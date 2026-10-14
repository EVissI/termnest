use std::path::PathBuf;
use std::sync::Arc;

use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;
use termnest_core::session::ConnectOptions;
use termnest_core::{
    paths, CoreError, ErrorKind, OutputSink, SessionId, SessionManager, SessionState, Target,
};

/// Sends terminal output to the webview as raw bytes (an `ArrayBuffer` on the JS side).
struct ChannelSink(Channel<InvokeResponseBody>);

impl OutputSink for ChannelSink {
    fn send(&self, data: &[u8]) -> bool {
        self.0.send(InvokeResponseBody::Raw(data.to_vec())).is_ok()
    }
}

#[tauri::command]
pub async fn session_open(
    manager: State<'_, SessionManager>,
    target: String,
    key_path: String,
    cols: u32,
    rows: u32,
) -> Result<SessionId, CoreError> {
    let target: Target = target.parse()?;
    let key_path = key_path.trim();
    if key_path.is_empty() {
        return Err(CoreError::new(
            ErrorKind::KeyNotFound,
            "Не указан путь к ключу",
        ));
    }
    let options = ConnectOptions::new(target, PathBuf::from(key_path), cols, rows);
    Ok(manager.open(options))
}

#[tauri::command]
pub fn session_subscribe(
    manager: State<'_, SessionManager>,
    id: SessionId,
    on_output: Channel<InvokeResponseBody>,
) -> Result<u64, CoreError> {
    manager.subscribe(id, Arc::new(ChannelSink(on_output)))
}

#[tauri::command]
pub fn session_unsubscribe(manager: State<'_, SessionManager>, id: SessionId, subscription: u64) {
    manager.unsubscribe(id, subscription);
}

#[tauri::command]
pub fn session_write(
    manager: State<'_, SessionManager>,
    id: SessionId,
    data: Vec<u8>,
) -> Result<(), CoreError> {
    manager.write(id, data)
}

#[tauri::command]
pub fn session_resize(
    manager: State<'_, SessionManager>,
    id: SessionId,
    cols: u32,
    rows: u32,
) -> Result<(), CoreError> {
    manager.resize(id, cols, rows)
}

#[tauri::command]
pub fn session_close(manager: State<'_, SessionManager>, id: SessionId) {
    manager.close(id);
}

#[tauri::command]
pub fn session_state(
    manager: State<'_, SessionManager>,
    id: SessionId,
) -> Result<SessionState, CoreError> {
    manager.state(id)
}

#[tauri::command]
pub fn default_key_path() -> Option<String> {
    paths::default_key_path().map(|p| p.display().to_string())
}
