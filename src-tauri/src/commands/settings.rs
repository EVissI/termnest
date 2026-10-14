use serde::Serialize;
use termnest_core::settings::Settings;
use termnest_core::CoreError;

use super::AppStateRef;
use crate::events::SETTINGS_CHANGED_EVENT;

#[tauri::command]
pub fn settings_get(state: AppStateRef<'_>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn settings_set(state: AppStateRef<'_>, settings: Settings) -> Result<Settings, CoreError> {
    let saved = state.settings.set(settings)?;
    state.ctx.set_keepalive_secs(saved.keepalive_secs);
    state.emit(SETTINGS_CHANGED_EVENT, saved.clone());
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
