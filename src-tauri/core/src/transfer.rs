//! SFTP transfer queue: uploads and downloads of files and whole folders with
//! progress, speed, cancellation and name conflict handling.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh_sftp::client::SftpSession;
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::prompt::{self, ConflictAction, PromptReply, PromptRequest, Prompter};
use crate::session::SessionId;
use crate::sftp::{self, map_sftp_error, EntryKind};

pub type TransferId = u64;

const CHUNK: usize = 256 * 1024;
const PUBLISH_EVERY: Duration = Duration::from_millis(200);
/// Parallel transfers per session.
const PER_SESSION: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferStatus {
    pub id: TransferId,
    pub session: SessionId,
    pub direction: Direction,
    pub name: String,
    pub destination: String,
    pub state: TransferState,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub files_total: u32,
    pub files_done: u32,
    pub skipped: u32,
    pub current_file: Option<String>,
    pub speed_bps: u64,
    pub error: Option<String>,
}

pub trait TransferSink: Send + Sync + 'static {
    fn transfer_changed(&self, status: &TransferStatus);
}

struct Job {
    status: Mutex<TransferStatus>,
    cancel: AtomicBool,
}

/// "Apply to all" answer shared by the jobs of one upload/download request.
type BatchPolicy = Arc<tokio::sync::Mutex<Option<ConflictAction>>>;

pub struct TransferManager {
    jobs: Mutex<HashMap<TransferId, Arc<Job>>>,
    next: AtomicU64,
    sink: Arc<dyn TransferSink>,
    prompter: Arc<dyn Prompter>,
    limits: Mutex<HashMap<SessionId, Arc<Semaphore>>>,
    runtime: tokio::runtime::Handle,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Item to transfer: a top-level file or folder.
pub enum Source {
    Local(PathBuf),
    Remote { path: String, is_dir: bool },
}

impl TransferManager {
    pub fn new(
        sink: Arc<dyn TransferSink>,
        prompter: Arc<dyn Prompter>,
        runtime: tokio::runtime::Handle,
    ) -> Arc<Self> {
        Arc::new(Self {
            jobs: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            sink,
            prompter,
            limits: Mutex::new(HashMap::new()),
            runtime,
        })
    }

    pub fn list(&self) -> Vec<TransferStatus> {
        let mut all: Vec<_> = lock(&self.jobs)
            .values()
            .map(|j| lock(&j.status).clone())
            .collect();
        all.sort_by_key(|s| s.id);
        all
    }

    pub fn cancel(&self, id: TransferId) {
        if let Some(job) = lock(&self.jobs).get(&id) {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn cancel_session(&self, session: SessionId) {
        for job in lock(&self.jobs).values() {
            if lock(&job.status).session == session {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// Forgets finished, failed and cancelled transfers.
    pub fn clear_finished(&self) {
        lock(&self.jobs).retain(|_, j| {
            matches!(
                lock(&j.status).state,
                TransferState::Queued | TransferState::Running
            )
        });
    }

    fn limit(&self, session: SessionId) -> Arc<Semaphore> {
        lock(&self.limits)
            .entry(session)
            .or_insert_with(|| Arc::new(Semaphore::new(PER_SESSION)))
            .clone()
    }

    /// Queues transfers (one per source) into `destination` (a remote folder for
    /// uploads, a local folder for downloads).
    pub fn enqueue(
        self: &Arc<Self>,
        sftp: Arc<SftpSession>,
        session: SessionId,
        sources: Vec<Source>,
        destination: String,
    ) -> Vec<TransferId> {
        let policy: BatchPolicy = Arc::new(tokio::sync::Mutex::new(None));
        let mut ids = Vec::new();
        for source in sources {
            let id = self.next.fetch_add(1, Ordering::Relaxed);
            let (direction, name) = match &source {
                Source::Local(p) => (
                    Direction::Upload,
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
                Source::Remote { path, .. } => (
                    Direction::Download,
                    path.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(path)
                        .to_owned(),
                ),
            };
            let job = Arc::new(Job {
                status: Mutex::new(TransferStatus {
                    id,
                    session,
                    direction,
                    name,
                    destination: destination.clone(),
                    state: TransferState::Queued,
                    total_bytes: 0,
                    done_bytes: 0,
                    files_total: 0,
                    files_done: 0,
                    skipped: 0,
                    current_file: None,
                    speed_bps: 0,
                    error: None,
                }),
                cancel: AtomicBool::new(false),
            });
            lock(&self.jobs).insert(id, job.clone());
            self.sink.transfer_changed(&lock(&job.status).clone());
            ids.push(id);

            let this = self.clone();
            let sftp = sftp.clone();
            let destination = destination.clone();
            let policy = policy.clone();
            let limit = self.limit(session);
            self.runtime.spawn(async move {
                let _permit = limit.acquire_owned().await;
                let mut run = Run::new(&this, job.clone(), sftp, policy);
                run.set(|s| s.state = TransferState::Running);
                let result = match source {
                    Source::Local(path) => run.upload(&path, &destination).await,
                    Source::Remote { path, is_dir } => {
                        run.download(&path, is_dir, Path::new(&destination)).await
                    }
                };
                let cancelled = job.cancel.load(Ordering::Relaxed);
                run.set(|s| {
                    s.current_file = None;
                    s.speed_bps = 0;
                    match (&result, cancelled) {
                        (_, true) => s.state = TransferState::Cancelled,
                        (Ok(()), false) => s.state = TransferState::Done,
                        (Err(e), false) => {
                            s.state = TransferState::Failed;
                            s.error = Some(e.message.clone());
                        }
                    }
                });
            });
        }
        ids
    }
}

struct Run<'a> {
    manager: &'a TransferManager,
    job: Arc<Job>,
    sftp: Arc<SftpSession>,
    policy: BatchPolicy,
    last_publish: Instant,
    speed_window: (Instant, u64),
}

enum Resolution {
    Write(String),
    Skip,
}

fn cancelled() -> CoreError {
    CoreError::cancelled()
}

/// Remote names may contain characters Windows does not allow.
fn local_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

impl<'a> Run<'a> {
    fn new(
        manager: &'a TransferManager,
        job: Arc<Job>,
        sftp: Arc<SftpSession>,
        policy: BatchPolicy,
    ) -> Self {
        let now = Instant::now();
        Self {
            manager,
            job,
            sftp,
            policy,
            last_publish: now,
            speed_window: (now, 0),
        }
    }

    fn set(&mut self, f: impl FnOnce(&mut TransferStatus)) {
        let snapshot = {
            let mut s = lock(&self.job.status);
            f(&mut s);
            s.clone()
        };
        self.last_publish = Instant::now();
        self.manager.sink.transfer_changed(&snapshot);
    }

    fn check_cancel(&self) -> CoreResult<()> {
        if self.job.cancel.load(Ordering::Relaxed) {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    fn progress(&mut self, bytes: u64) {
        let now = Instant::now();
        let mut status = lock(&self.job.status);
        status.done_bytes += bytes;
        let elapsed = now.duration_since(self.speed_window.0);
        if elapsed >= Duration::from_millis(500) {
            let delta = status.done_bytes - self.speed_window.1;
            let current = (delta as f64 / elapsed.as_secs_f64()) as u64;
            status.speed_bps = if status.speed_bps == 0 {
                current
            } else {
                (status.speed_bps * 2 + current) / 3
            };
            self.speed_window = (now, status.done_bytes);
        }
        if now.duration_since(self.last_publish) >= PUBLISH_EVERY {
            let snapshot = status.clone();
            drop(status);
            self.last_publish = now;
            self.manager.sink.transfer_changed(&snapshot);
        }
    }

    async fn resolve_conflict(&mut self, name: &str, remote: bool) -> CoreResult<ConflictAction> {
        let mut policy = self.policy.lock().await;
        if let Some(action) = *policy {
            return Ok(action);
        }
        let reply = prompt::ask(
            self.manager.prompter.as_ref(),
            PromptRequest::FileConflict {
                name: name.to_owned(),
                remote,
            },
        )
        .await;
        match reply {
            PromptReply::Conflict {
                action,
                apply_to_all,
            } => {
                if apply_to_all {
                    *policy = Some(action);
                }
                Ok(action)
            }
            _ => {
                self.job.cancel.store(true, Ordering::Relaxed);
                Err(cancelled())
            }
        }
    }

    async fn remote_target(&mut self, dir: &str, name: &str) -> CoreResult<Resolution> {
        let path = sftp::join(dir, name);
        if sftp::stat(&self.sftp, &path).await?.is_none() {
            return Ok(Resolution::Write(path));
        }
        match self.resolve_conflict(&path, true).await? {
            ConflictAction::Replace => Ok(Resolution::Write(path)),
            ConflictAction::Skip => Ok(Resolution::Skip),
            ConflictAction::Rename => {
                let entries = sftp::list(&self.sftp, dir).await?;
                let fresh = sftp::free_name(name, |n| entries.iter().any(|e| e.name == n));
                Ok(Resolution::Write(sftp::join(dir, &fresh)))
            }
        }
    }

    async fn local_target(&mut self, dir: &Path, name: &str) -> CoreResult<Resolution> {
        let name = local_name(name);
        let path = dir.join(&name);
        if !path.exists() {
            return Ok(Resolution::Write(path.display().to_string()));
        }
        match self
            .resolve_conflict(&path.display().to_string(), false)
            .await?
        {
            ConflictAction::Replace => Ok(Resolution::Write(path.display().to_string())),
            ConflictAction::Skip => Ok(Resolution::Skip),
            ConflictAction::Rename => {
                let fresh = sftp::free_name(&name, |n| dir.join(n).exists());
                Ok(Resolution::Write(dir.join(fresh).display().to_string()))
            }
        }
    }

    async fn upload(&mut self, local: &Path, remote_dir: &str) -> CoreResult<()> {
        let meta = tokio::fs::metadata(local).await.map_err(|e| {
            CoreError::new(ErrorKind::NotFound, format!("{}: {e}", local.display()))
        })?;
        let name = local
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        if !meta.is_dir() {
            self.set(|s| {
                s.total_bytes = meta.len();
                s.files_total = 1;
            });
            return match self.remote_target(remote_dir, &name).await? {
                Resolution::Write(dest) => self.upload_file(local, &dest).await,
                Resolution::Skip => {
                    self.set(|s| s.skipped += 1);
                    Ok(())
                }
            };
        }

        // Plan the folder: (relative dir paths, (local file, relative path, size)).
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut stack = vec![(local.to_path_buf(), String::new())];
        while let Some((dir, rel)) = stack.pop() {
            self.check_cancel()?;
            let mut rd = tokio::fs::read_dir(&dir)
                .await
                .map_err(|e| CoreError::storage(&dir.display().to_string(), e))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| CoreError::storage("Чтение папки", e))?
            {
                let entry_name = entry.file_name().to_string_lossy().into_owned();
                let child_rel = if rel.is_empty() {
                    entry_name.clone()
                } else {
                    format!("{rel}/{entry_name}")
                };
                let ft = entry
                    .file_type()
                    .await
                    .map_err(|e| CoreError::storage("Чтение папки", e))?;
                if ft.is_dir() {
                    dirs.push(child_rel.clone());
                    stack.push((entry.path(), child_rel));
                } else if ft.is_file() {
                    let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                    files.push((entry.path(), child_rel, size));
                }
            }
        }
        dirs.sort();
        let total: u64 = files.iter().map(|f| f.2).sum();
        let count = files.len() as u32;
        self.set(|s| {
            s.total_bytes = total;
            s.files_total = count;
        });

        let top = sftp::join(remote_dir, &name);
        match sftp::stat(&self.sftp, &top).await? {
            None => sftp::mkdir(&self.sftp, &top).await?,
            Some(a) if a.is_dir() => {}
            Some(_) => {
                return Err(CoreError::new(
                    ErrorKind::AlreadyExists,
                    format!("{top} уже существует и это не папка"),
                ));
            }
        }
        for d in &dirs {
            self.check_cancel()?;
            let path = format!("{top}/{d}");
            if sftp::stat(&self.sftp, &path).await?.is_none() {
                sftp::mkdir(&self.sftp, &path).await?;
            }
        }
        for (path, rel, size) in files {
            self.check_cancel()?;
            let (sub, file_name) = match rel.rsplit_once('/') {
                Some((d, f)) => (format!("{top}/{d}"), f.to_owned()),
                None => (top.clone(), rel.clone()),
            };
            match self.remote_target(&sub, &file_name).await? {
                Resolution::Write(dest) => self.upload_file(&path, &dest).await?,
                Resolution::Skip => {
                    self.set(|s| {
                        s.skipped += 1;
                        s.done_bytes += size;
                    });
                }
            }
        }
        Ok(())
    }

    async fn upload_file(&mut self, local: &Path, remote: &str) -> CoreResult<()> {
        self.set(|s| s.current_file = Some(remote.to_owned()));
        let mut src = tokio::fs::File::open(local)
            .await
            .map_err(|e| CoreError::storage(&local.display().to_string(), e))?;
        let mut dst = self
            .sftp
            .create(remote)
            .await
            .map_err(|e| map_sftp_error(e, &format!("Запись {remote}")))?;
        let mut buf = vec![0u8; CHUNK];
        let result: CoreResult<()> = async {
            loop {
                self.check_cancel()?;
                let n = src
                    .read(&mut buf)
                    .await
                    .map_err(|e| CoreError::storage("Чтение файла", e))?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n]).await.map_err(|e| {
                    CoreError::new(ErrorKind::Remote, format!("Запись {remote}: {e}"))
                })?;
                self.progress(n as u64);
            }
            dst.shutdown()
                .await
                .map_err(|e| CoreError::new(ErrorKind::Remote, format!("Запись {remote}: {e}")))
        }
        .await;
        if result.is_err() {
            drop(dst);
            let _ = self.sftp.remove_file(remote).await;
            return result;
        }
        self.set(|s| s.files_done += 1);
        Ok(())
    }

    async fn download(&mut self, remote: &str, is_dir: bool, local_dir: &Path) -> CoreResult<()> {
        let name = remote
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(remote)
            .to_owned();
        if !is_dir {
            let size = sftp::stat(&self.sftp, remote)
                .await?
                .and_then(|a| a.size)
                .unwrap_or(0);
            self.set(|s| {
                s.total_bytes = size;
                s.files_total = 1;
            });
            return match self.local_target(local_dir, &name).await? {
                Resolution::Write(dest) => self.download_file(remote, Path::new(&dest)).await,
                Resolution::Skip => {
                    self.set(|s| s.skipped += 1);
                    Ok(())
                }
            };
        }

        let mut dirs = Vec::new();
        let mut files = Vec::new();
        let mut stack = vec![(remote.to_owned(), String::new())];
        while let Some((dir, rel)) = stack.pop() {
            self.check_cancel()?;
            for entry in sftp::list(&self.sftp, &dir).await? {
                let child_rel = if rel.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{rel}/{}", entry.name)
                };
                match entry.kind {
                    EntryKind::Dir => {
                        dirs.push(child_rel.clone());
                        // Do not follow symlinked folders: avoids loops.
                        let real_dir = sftp::stat(&self.sftp, &entry.path)
                            .await?
                            .is_some_and(|a| a.is_dir());
                        let is_link = self
                            .sftp
                            .symlink_metadata(entry.path.clone())
                            .await
                            .is_ok_and(|a| a.is_symlink());
                        if real_dir && !is_link {
                            stack.push((entry.path.clone(), child_rel));
                        }
                    }
                    EntryKind::File => files.push((entry.path.clone(), child_rel, entry.size)),
                    _ => {}
                }
            }
        }
        let total: u64 = files.iter().map(|f| f.2).sum();
        let count = files.len() as u32;
        self.set(|s| {
            s.total_bytes = total;
            s.files_total = count;
        });

        let top = local_dir.join(local_name(&name));
        tokio::fs::create_dir_all(&top)
            .await
            .map_err(|e| CoreError::storage(&top.display().to_string(), e))?;
        for d in &dirs {
            let p = d
                .split('/')
                .fold(top.clone(), |acc, part| acc.join(local_name(part)));
            tokio::fs::create_dir_all(&p)
                .await
                .map_err(|e| CoreError::storage(&p.display().to_string(), e))?;
        }
        for (path, rel, size) in files {
            self.check_cancel()?;
            let mut parts: Vec<&str> = rel.split('/').collect();
            let file_name = parts.pop().unwrap_or_default().to_owned();
            let dir = parts
                .iter()
                .fold(top.clone(), |acc, part| acc.join(local_name(part)));
            match self.local_target(&dir, &file_name).await? {
                Resolution::Write(dest) => self.download_file(&path, Path::new(&dest)).await?,
                Resolution::Skip => self.set(|s| {
                    s.skipped += 1;
                    s.done_bytes += size;
                }),
            }
        }
        Ok(())
    }

    async fn download_file(&mut self, remote: &str, local: &Path) -> CoreResult<()> {
        self.set(|s| s.current_file = Some(remote.to_owned()));
        let mut src = self
            .sftp
            .open(remote)
            .await
            .map_err(|e| map_sftp_error(e, &format!("Чтение {remote}")))?;
        let mut dst = tokio::fs::File::create(local)
            .await
            .map_err(|e| CoreError::storage(&local.display().to_string(), e))?;
        let mut buf = vec![0u8; CHUNK];
        let result: CoreResult<()> = async {
            loop {
                self.check_cancel()?;
                let n = src.read(&mut buf).await.map_err(|e| {
                    CoreError::new(ErrorKind::Remote, format!("Чтение {remote}: {e}"))
                })?;
                if n == 0 {
                    break;
                }
                dst.write_all(&buf[..n])
                    .await
                    .map_err(|e| CoreError::storage("Запись файла", e))?;
                self.progress(n as u64);
            }
            dst.flush()
                .await
                .map_err(|e| CoreError::storage("Запись файла", e))
        }
        .await;
        if result.is_err() {
            drop(dst);
            let _ = tokio::fs::remove_file(local).await;
            return result;
        }
        self.set(|s| s.files_done += 1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::local_name;

    #[test]
    fn local_names_are_sanitized() {
        assert_eq!(local_name("a:b*c?.txt"), "a_b_c_.txt");
        assert_eq!(local_name("normal.log"), "normal.log");
    }
}
