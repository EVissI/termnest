#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;

use std::sync::Arc;

use tauri::Manager;
use termnest_core::SessionManager;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let sink = events::TauriStateSink::new(app.handle().clone());
            app.manage(SessionManager::new(Arc::new(sink)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::session::session_open,
            commands::session::session_subscribe,
            commands::session::session_unsubscribe,
            commands::session::session_write,
            commands::session::session_resize,
            commands::session::session_close,
            commands::session::session_state,
            commands::session::default_key_path,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start termnest");
}
