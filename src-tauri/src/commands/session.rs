use std::sync::Arc;

use tauri::ipc::{Channel, InvokeResponseBody};
use termnest_core::prompt::PromptReply;
use termnest_core::{CoreError, OutputSink, SessionId, SessionStatus};

use super::AppStateRef;
use crate::prompts::PendingPrompt;

/// Sends terminal output to the webview as raw bytes (an `ArrayBuffer` on the JS side).
struct ChannelSink(Channel<InvokeResponseBody>);

impl OutputSink for ChannelSink {
    fn send(&self, data: &[u8]) -> bool {
        self.0.send(InvokeResponseBody::Raw(data.to_vec())).is_ok()
    }
}

#[tauri::command]
pub fn session_subscribe(
    state: AppStateRef<'_>,
    id: SessionId,
    on_output: Channel<InvokeResponseBody>,
) -> Result<u64, CoreError> {
    state
        .sessions
        .subscribe(id, Arc::new(ChannelSink(on_output)))
}

#[tauri::command]
pub fn session_unsubscribe(state: AppStateRef<'_>, id: SessionId, subscription: u64) {
    state.sessions.unsubscribe(id, subscription);
}

#[tauri::command]
pub fn session_write(
    state: AppStateRef<'_>,
    id: SessionId,
    data: Vec<u8>,
) -> Result<(), CoreError> {
    state.sessions.write(id, data)
}

/// Same input to several sessions (broadcast mode, snippets).
#[tauri::command]
pub fn session_write_many(state: AppStateRef<'_>, ids: Vec<SessionId>, data: Vec<u8>) {
    state.sessions.write_many(&ids, &data);
}

#[tauri::command]
pub fn session_resize(
    state: AppStateRef<'_>,
    id: SessionId,
    cols: u32,
    rows: u32,
) -> Result<(), CoreError> {
    state.sessions.resize(id, cols, rows)
}

#[tauri::command]
pub fn session_retry(state: AppStateRef<'_>, id: SessionId) -> Result<(), CoreError> {
    state.sessions.retry(id)
}

#[tauri::command]
pub fn session_reconnect_now(state: AppStateRef<'_>, id: SessionId) -> Result<(), CoreError> {
    state.sessions.reconnect_now(id)
}

#[tauri::command]
pub fn session_cancel_reconnect(state: AppStateRef<'_>, id: SessionId) -> Result<(), CoreError> {
    state.sessions.cancel_reconnect(id)
}

#[tauri::command]
pub fn session_statuses(state: AppStateRef<'_>) -> Vec<SessionStatus> {
    state.sessions.statuses()
}

#[tauri::command]
pub fn prompt_respond(state: AppStateRef<'_>, id: u64, reply: PromptReply) {
    state.prompts.respond(id, reply);
}

#[tauri::command]
pub fn prompt_pending(state: AppStateRef<'_>) -> Vec<PendingPrompt> {
    state.prompts.pending()
}

#[derive(serde::Serialize)]
pub struct KnownHost {
    host: String,
    port: u16,
    key_type: String,
    fingerprint: String,
}

#[tauri::command]
pub fn known_hosts_list(state: AppStateRef<'_>) -> Vec<KnownHost> {
    state
        .known_hosts
        .entries()
        .into_iter()
        .map(|e| KnownHost {
            host: e.host,
            port: e.port,
            key_type: e.key_type,
            fingerprint: e.fingerprint,
        })
        .collect()
}

/// Deletes the stored host key (the only way past a "host key changed" block).
#[tauri::command]
pub fn known_hosts_remove(
    state: AppStateRef<'_>,
    host: String,
    port: u16,
) -> Result<usize, CoreError> {
    state.known_hosts.remove(&host, port)
}
