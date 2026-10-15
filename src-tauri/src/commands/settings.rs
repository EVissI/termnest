use serde::Serialize;
use termnest_core::settings::Settings;
use termnest_core::CoreError;

use super::AppStateRef;
use crate::events::SETTINGS_CHANGED_EVENT;
use crate::mcp::{LogEntry, McpHub, McpStatus};

#[tauri::command]
pub fn settings_get(state: AppStateRef<'_>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn settings_set(state: AppStateRef<'_>, settings: Settings) -> Result<Settings, CoreError> {
    let before = state.settings().claude;
    let saved = state.settings.set(settings)?;
    state.ctx.set_keepalive_secs(saved.keepalive_secs);
    state.emit(SETTINGS_CHANGED_EVENT, saved.clone());
    // Restart the MCP server only when its own settings changed.
    if before.mcp_enabled != saved.claude.mcp_enabled || before.mcp_port != saved.claude.mcp_port {
        let state = state.inner().clone();
        let settings = saved.clone();
        tauri::async_runtime::spawn(async move {
            state.mcp.apply(&state, &settings).await;
        });
    }
    Ok(saved)
}

#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    data_dir: String,
}

#[tauri::command]
pub fn app_info(state: AppStateRef<'_>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        data_dir: state.dir.display().to_string(),
    }
}

/// Forgets passphrases remembered during this run.
#[tauri::command]
pub fn passphrases_clear(state: AppStateRef<'_>) {
    state.ctx.passphrases.clear();
}

#[tauri::command]
pub async fn mcp_status(state: AppStateRef<'_>) -> Result<McpStatus, CoreError> {
    Ok(state.mcp.status(&state).await)
}

/// New token; the running server is restarted with it (old clients stop working).
#[tauri::command]
pub async fn mcp_regenerate_token(state: AppStateRef<'_>) -> Result<McpStatus, CoreError> {
    McpHub::regenerate_token(&state.dir)?;
    let settings = state.settings();
    state.mcp.apply(state.inner(), &settings).await;
    Ok(state.mcp.status(&state).await)
}

#[tauri::command]
pub fn mcp_journal(state: AppStateRef<'_>) -> Vec<LogEntry> {
    state.mcp.log()
}

/// A window's answer to `read_selection`.
#[tauri::command]
pub fn mcp_selection_reply(state: AppStateRef<'_>, id: u64, text: String) {
    state.mcp.selection_reply(id, text);
}
