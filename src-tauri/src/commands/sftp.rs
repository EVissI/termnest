use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use termnest_core::sftp::{self, RemoteEntry};
use termnest_core::transfer::{Source, TransferId, TransferStatus};
use termnest_core::{CoreError, CoreResult, ErrorKind, SessionId, SftpSession};

use super::AppStateRef;
use crate::state::AppState;

/// Runs an SFTP operation; on a transport-level failure the cached SFTP channel is
/// dropped so the next call opens a fresh one.
async fn with_sftp<T, F, Fut>(state: &AppState, session: SessionId, op: F) -> CoreResult<T>
where
    F: FnOnce(Arc<SftpSession>) -> Fut,
    Fut: std::future::Future<Output = CoreResult<T>>,
{
    let sftp = state.sessions.sftp(session).await?;
    let result = op(sftp).await;
    if let Err(e) = &result {
        if matches!(
            e.kind,
            ErrorKind::Remote | ErrorKind::Timeout | ErrorKind::Protocol
        ) {
            state.sessions.reset_sftp(session).await;
        }
    }
    result
}

/// Home directory (the SFTP start folder).
#[tauri::command]
pub async fn sftp_home(state: AppStateRef<'_>, session: SessionId) -> Result<String, CoreError> {
    with_sftp(
        &state,
        session,
        |s| async move { sftp::realpath(&s, ".").await },
    )
    .await
}

#[tauri::command]
pub async fn sftp_realpath(
    state: AppStateRef<'_>,
    session: SessionId,
    path: String,
) -> Result<String, CoreError> {
    with_sftp(&state, session, |s| async move {
        sftp::realpath(&s, &path).await
    })
    .await
}

#[tauri::command]
pub async fn sftp_list(
    state: AppStateRef<'_>,
    session: SessionId,
    path: String,
) -> Result<Vec<RemoteEntry>, CoreError> {
    with_sftp(
        &state,
        session,
        |s| async move { sftp::list(&s, &path).await },
    )
    .await
}

#[tauri::command]
pub async fn sftp_mkdir(
    state: AppStateRef<'_>,
    session: SessionId,
    path: String,
) -> Result<(), CoreError> {
    with_sftp(
        &state,
        session,
        |s| async move { sftp::mkdir(&s, &path).await },
    )
    .await
}

#[tauri::command]
pub async fn sftp_rename(
    state: AppStateRef<'_>,
    session: SessionId,
    from: String,
    to: String,
) -> Result<(), CoreError> {
    with_sftp(&state, session, |s| async move {
        sftp::rename(&s, &from, &to).await
    })
    .await
}

#[tauri::command]
pub async fn sftp_remove(
    state: AppStateRef<'_>,
    session: SessionId,
    paths: Vec<String>,
) -> Result<(), CoreError> {
    with_sftp(&state, session, |s| async move {
        for p in &paths {
            sftp::remove(&s, p).await?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn sftp_chmod(
    state: AppStateRef<'_>,
    session: SessionId,
    path: String,
    mode: u32,
) -> Result<(), CoreError> {
    with_sftp(&state, session, |s| async move {
        sftp::chmod(&s, &path, mode).await
    })
    .await
}

#[tauri::command]
pub async fn sftp_upload(
    state: AppStateRef<'_>,
    session: SessionId,
    local_paths: Vec<String>,
    remote_dir: String,
) -> Result<Vec<TransferId>, CoreError> {
    let sftp = state.sessions.sftp(session).await?;
    let sources = local_paths
        .into_iter()
        .map(|p| Source::Local(PathBuf::from(p)))
        .collect();
    Ok(state.transfers.enqueue(sftp, session, sources, remote_dir))
}

#[derive(Deserialize)]
pub struct RemoteItem {
    path: String,
    is_dir: bool,
}

#[tauri::command]
pub async fn sftp_download(
    state: AppStateRef<'_>,
    session: SessionId,
    items: Vec<RemoteItem>,
    local_dir: String,
) -> Result<Vec<TransferId>, CoreError> {
    if !std::path::Path::new(&local_dir).is_dir() {
        return Err(CoreError::new(
            ErrorKind::NotFound,
            "Папка для скачивания не найдена",
        ));
    }
    let sftp = state.sessions.sftp(session).await?;
    let sources = items
        .into_iter()
        .map(|i| Source::Remote {
            path: i.path,
            is_dir: i.is_dir,
        })
        .collect();
    Ok(state.transfers.enqueue(sftp, session, sources, local_dir))
}

#[tauri::command]
pub fn transfers_list(state: AppStateRef<'_>) -> Vec<TransferStatus> {
    state.transfers.list()
}

#[tauri::command]
pub fn transfer_cancel(state: AppStateRef<'_>, id: TransferId) {
    state.transfers.cancel(id);
}

#[tauri::command]
pub fn transfers_clear(state: AppStateRef<'_>) {
    state.transfers.clear_finished();
}

/// Current directory of the shell, if the server reports it via `/proc` for the
/// session's login shell. Best effort; `None` when unknown.
#[tauri::command]
pub async fn session_cwd(
    state: AppStateRef<'_>,
    session: SessionId,
) -> Result<Option<String>, CoreError> {
    // The shell started by our PTY request is the newest login shell of this user
    // with a terminal; its cwd is readable through /proc on Linux.
    let script = "sh -c 'for p in $(ps -o pid= -u \"$(id -u)\" --sort=-start_time 2>/dev/null); do t=$(readlink /proc/$p/fd/0 2>/dev/null); case \"$t\" in /dev/pts/*) c=$(readlink /proc/$p/cwd 2>/dev/null) && [ -n \"$c\" ] && echo \"$c\" && exit 0;; esac; done'";
    let out = state.sessions.exec(session, script).await?;
    let line = out.stdout.lines().next().unwrap_or("").trim().to_owned();
    Ok((!line.is_empty()).then_some(line))
}
