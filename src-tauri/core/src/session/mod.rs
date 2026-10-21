//! SSH sessions. A session lives here independently of any window or panel;
//! the UI only subscribes to its output and sends input by id.

mod autopass;
mod output;
mod runner;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use russh::client::Handle;
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::ssh::{Client, SshContext};
use crate::target::Target;

use output::OutputHub;
pub use output::OutputSink;

pub type SessionId = u64;

/// How much recent output each session keeps for replay to new subscribers.
const OUTPUT_BUFFER_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Error,
}

/// Where to connect. Stored in the layout so a panel can be restored later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectSpec {
    /// Display name (host name from the server list or the target string).
    pub label: String,
    /// Saved host this spec came from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub key_path: String,
    /// User to become right after the shell opens (also after a reconnect).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_user: Option<String>,
    #[serde(default)]
    pub switch_method: crate::hosts::SwitchMethod,
}

impl ConnectSpec {
    /// Typed into a fresh shell to become `switch_user`.
    pub fn startup_command(&self) -> Option<String> {
        let user = self.switch_user.as_deref()?;
        crate::hosts::valid_user_name(user).then(|| self.switch_method.command(user))
    }

    /// The startup switch with its saved password, if any.
    fn startup_switch(&self) -> Option<autopass::Switch> {
        let command = self.startup_command()?;
        let target = self.switch_user.as_deref()?;
        let password = self.host_id.as_deref().and_then(|host| {
            crate::credstore::load(host, self.switch_method.password_user(&self.user, target))
        });
        Some(autopass::Switch { command, password })
    }

    pub fn target(&self) -> Target {
        Target {
            user: self.user.clone(),
            host: self.host.clone(),
            port: self.port,
        }
    }

    pub fn from_target(target: &Target, key_path: String, label: Option<String>) -> Self {
        Self {
            label: label.unwrap_or_else(|| target.to_string()),
            host_id: None,
            user: target.user.clone(),
            host: target.host.clone(),
            port: target.port,
            key_path,
            switch_user: None,
            switch_method: Default::default(),
        }
    }
}

/// Current status of a session, sent to the UI on every change.
#[derive(Debug, Clone, Serialize)]
pub struct SessionStatus {
    pub id: SessionId,
    pub label: String,
    pub state: SessionState,
    pub error: Option<CoreError>,
    /// Reconnect: attempt number and delay until the next try.
    pub attempt: u32,
    pub retry_in_ms: Option<u64>,
    /// Exit code of the remote shell if it ended by itself.
    pub exit_code: Option<u32>,
}

/// Receiver of session state changes (the app forwards them as `session://state`).
pub trait StateSink: Send + Sync + 'static {
    fn state_changed(&self, status: &SessionStatus);
}

pub(crate) enum Command {
    Data(Vec<u8>),
    /// Type a user switch command and answer its password prompt.
    Switch(autopass::Switch),
    Resize {
        cols: u32,
        rows: u32,
    },
    /// Connect again after an error or after the shell exited.
    Retry,
    /// Skip the remaining reconnect delay.
    ReconnectNow,
    /// Stop reconnecting and stay disconnected.
    CancelReconnect,
    Close,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct SessionEntry {
    spec: ConnectSpec,
    status: Mutex<SessionStatus>,
    output: Mutex<OutputHub>,
    commands: mpsc::UnboundedSender<Command>,
    state_sink: Arc<dyn StateSink>,
    handle: Mutex<Option<Arc<Handle<Client>>>>,
    sftp: tokio::sync::Mutex<Option<Arc<SftpSession>>>,
}

impl SessionEntry {
    pub(crate) fn publish(&self, update: impl FnOnce(&mut SessionStatus)) {
        let snapshot = {
            let mut status = lock(&self.status);
            update(&mut status);
            status.clone()
        };
        self.state_sink.state_changed(&snapshot);
    }

    pub(crate) fn push_output(&self, data: &[u8]) {
        lock(&self.output).push(data);
    }

    /// A gray status line in the terminal (seen by all subscribers and replays).
    pub(crate) fn notice(&self, text: &str, color: u8) {
        self.push_output(format!("\r\n\x1b[{color}m[{text}]\x1b[0m\r\n").as_bytes());
    }

    pub(crate) fn set_handle(&self, handle: Option<Arc<Handle<Client>>>) {
        *lock(&self.handle) = handle;
    }

    pub(crate) async fn drop_sftp(&self) {
        *self.sftp.lock().await = None;
    }
}

pub struct SessionManager {
    sessions: Mutex<HashMap<SessionId, Arc<SessionEntry>>>,
    next_id: AtomicU64,
    state_sink: Arc<dyn StateSink>,
    ctx: Arc<SshContext>,
    /// Session tasks run here, so `open` works from any thread (sync commands
    /// run on the UI thread without a runtime context).
    runtime: tokio::runtime::Handle,
}

impl SessionManager {
    pub fn new(
        state_sink: Arc<dyn StateSink>,
        ctx: Arc<SshContext>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            state_sink,
            ctx,
            runtime,
        }
    }

    pub fn context(&self) -> &Arc<SshContext> {
        &self.ctx
    }

    /// Registers a new session and starts connecting in the background.
    /// Progress is reported through the [`StateSink`].
    pub fn open(&self, spec: ConnectSpec, cols: u32, rows: u32) -> SessionId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let entry = Arc::new(SessionEntry {
            status: Mutex::new(SessionStatus {
                id,
                label: spec.label.clone(),
                state: SessionState::Connecting,
                error: None,
                attempt: 0,
                retry_in_ms: None,
                exit_code: None,
            }),
            spec,
            output: Mutex::new(OutputHub::new(OUTPUT_BUFFER_BYTES)),
            commands: tx,
            state_sink: self.state_sink.clone(),
            handle: Mutex::new(None),
            sftp: tokio::sync::Mutex::new(None),
        });
        lock(&self.sessions).insert(id, entry.clone());
        self.runtime.spawn(runner::run(
            entry,
            self.ctx.clone(),
            rx,
            (cols.max(1), rows.max(1)),
        ));
        id
    }

    fn get(&self, id: SessionId) -> CoreResult<Arc<SessionEntry>> {
        lock(&self.sessions)
            .get(&id)
            .cloned()
            .ok_or_else(CoreError::session_not_found)
    }

    pub fn status(&self, id: SessionId) -> CoreResult<SessionStatus> {
        Ok(lock(&self.get(id)?.status).clone())
    }

    pub fn statuses(&self) -> Vec<SessionStatus> {
        let mut all: Vec<_> = lock(&self.sessions)
            .values()
            .map(|e| lock(&e.status).clone())
            .collect();
        all.sort_by_key(|s| s.id);
        all
    }

    pub fn spec(&self, id: SessionId) -> CoreResult<ConnectSpec> {
        Ok(self.get(id)?.spec.clone())
    }

    /// Subscribes to output. Buffered output is replayed to the sink first.
    pub fn subscribe(&self, id: SessionId, sink: Arc<dyn OutputSink>) -> CoreResult<u64> {
        Ok(lock(&self.get(id)?.output).subscribe(sink))
    }

    /// Recent output as plain text (escape sequences removed), at most `lines` lines.
    pub fn recent_output(&self, id: SessionId, lines: usize) -> CoreResult<String> {
        let raw = lock(&self.get(id)?.output).tail(OUTPUT_BUFFER_BYTES);
        Ok(crate::textutil::tail_lines(
            &crate::textutil::plain_text(&raw),
            lines,
        ))
    }

    pub fn unsubscribe(&self, id: SessionId, subscription: u64) {
        if let Ok(entry) = self.get(id) {
            lock(&entry.output).unsubscribe(subscription);
        }
    }

    fn send(&self, id: SessionId, cmd: Command) -> CoreResult<()> {
        // A send error only means the runner already ended; the state event says so.
        let _ = self.get(id)?.commands.send(cmd);
        Ok(())
    }

    pub fn write(&self, id: SessionId, data: Vec<u8>) -> CoreResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.send(id, Command::Data(data))
    }

    /// Types a user switch command; a saved password answers the prompt it shows.
    pub fn switch_user(
        &self,
        id: SessionId,
        command: String,
        password: Option<crate::secret::SecretString>,
    ) -> CoreResult<()> {
        self.send(id, Command::Switch(autopass::Switch { command, password }))
    }

    /// Writes the same input to several sessions (broadcast, snippets). Unknown ids are skipped.
    pub fn write_many(&self, ids: &[SessionId], data: &[u8]) {
        for id in ids {
            let _ = self.write(*id, data.to_vec());
        }
    }

    pub fn resize(&self, id: SessionId, cols: u32, rows: u32) -> CoreResult<()> {
        self.send(
            id,
            Command::Resize {
                cols: cols.max(1),
                rows: rows.max(1),
            },
        )
    }

    pub fn retry(&self, id: SessionId) -> CoreResult<()> {
        self.send(id, Command::Retry)
    }

    pub fn reconnect_now(&self, id: SessionId) -> CoreResult<()> {
        self.send(id, Command::ReconnectNow)
    }

    pub fn cancel_reconnect(&self, id: SessionId) -> CoreResult<()> {
        self.send(id, Command::CancelReconnect)
    }

    /// Closes the session by user request and forgets it.
    pub fn close(&self, id: SessionId) {
        if let Some(entry) = lock(&self.sessions).remove(&id) {
            let _ = entry.commands.send(Command::Close);
        }
    }

    pub fn close_all(&self) {
        let all: Vec<_> = lock(&self.sessions).drain().map(|(_, e)| e).collect();
        for entry in all {
            let _ = entry.commands.send(Command::Close);
        }
    }

    pub fn ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<_> = lock(&self.sessions).keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// SFTP over the session's existing SSH connection (a separate channel, no new login).
    pub async fn sftp(&self, id: SessionId) -> CoreResult<Arc<SftpSession>> {
        let entry = self.get(id)?;
        let mut slot = entry.sftp.lock().await;
        if let Some(s) = slot.as_ref() {
            return Ok(s.clone());
        }
        let handle = lock(&entry.handle)
            .clone()
            .ok_or_else(|| CoreError::new(ErrorKind::Unreachable, "Сессия не подключена"))?;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(crate::ssh::map_ssh_error)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(crate::ssh::map_ssh_error)?;
        let sftp = SftpSession::new(channel.into_stream()).await.map_err(|e| {
            CoreError::new(
                ErrorKind::Protocol,
                format!("SFTP недоступен на сервере: {e}"),
            )
        })?;
        let sftp = Arc::new(sftp);
        *slot = Some(sftp.clone());
        Ok(sftp)
    }

    /// Runs a command on a separate exec channel of the session's connection.
    pub async fn exec(&self, id: SessionId, command: &str) -> CoreResult<crate::ssh::ExecOutput> {
        let entry = self.get(id)?;
        let handle = lock(&entry.handle)
            .clone()
            .ok_or_else(|| CoreError::new(ErrorKind::Unreachable, "Сессия не подключена"))?;
        crate::ssh::exec(&handle, command, None).await
    }

    /// Forgets the cached SFTP session (after a protocol error).
    pub async fn reset_sftp(&self, id: SessionId) {
        if let Ok(entry) = self.get(id) {
            entry.drop_sftp().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hostkeys::KnownHosts;
    use crate::paths::test_dir::TestDir;
    use crate::prompt::NoPrompts;
    use crate::secret::PassphraseCache;
    use std::time::Duration;

    struct Recorder(Mutex<Vec<(SessionId, SessionState, Option<ErrorKind>)>>);

    impl StateSink for Recorder {
        fn state_changed(&self, s: &SessionStatus) {
            self.0
                .lock()
                .unwrap()
                .push((s.id, s.state, s.error.as_ref().map(|e| e.kind)));
        }
    }

    fn manager(dir: &TestDir) -> (SessionManager, Arc<Recorder>) {
        let rec = Arc::new(Recorder(Mutex::new(Vec::new())));
        let ctx = Arc::new(SshContext::new(
            Arc::new(NoPrompts),
            Arc::new(KnownHosts::load(dir.path().join("kh")).unwrap()),
            Arc::new(PassphraseCache::default()),
            30,
        ));
        (
            SessionManager::new(rec.clone(), ctx, tokio::runtime::Handle::current()),
            rec,
        )
    }

    fn spec(key_path: &str) -> ConnectSpec {
        ConnectSpec::from_target(&"me@127.0.0.1:1".parse().unwrap(), key_path.into(), None)
    }

    async fn wait_for_final_state(rec: &Recorder) -> (SessionState, Option<ErrorKind>) {
        for _ in 0..200 {
            if let Some(&(_, state, kind)) = rec.0.lock().unwrap().last() {
                if state != SessionState::Connecting {
                    return (state, kind);
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("session did not leave the connecting state");
    }

    #[tokio::test]
    async fn missing_key_reports_error_state() {
        let dir = TestDir::new();
        let (manager, rec) = manager(&dir);
        let id = manager.open(spec("definitely/missing/key"), 80, 24);

        let (state, kind) = wait_for_final_state(&rec).await;
        assert_eq!(state, SessionState::Error);
        assert_eq!(kind, Some(ErrorKind::KeyNotFound));
        assert_eq!(manager.status(id).unwrap().state, SessionState::Error);
    }

    #[tokio::test]
    async fn ids_are_unique_and_close_forgets_session() {
        let dir = TestDir::new();
        let (manager, _rec) = manager(&dir);
        let a = manager.open(spec("missing-a"), 80, 24);
        let b = manager.open(spec("missing-b"), 80, 24);
        assert_ne!(a, b);
        assert_eq!(manager.ids(), vec![a, b]);

        manager.close(a);
        assert_eq!(manager.ids(), vec![b]);
        assert_eq!(
            manager.write(a, b"x".to_vec()).unwrap_err().kind,
            ErrorKind::SessionNotFound
        );
    }
}
