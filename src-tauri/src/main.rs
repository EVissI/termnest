#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod prompts;
mod state;
mod windows;

use std::sync::Arc;

use tauri::Manager;

use crate::commands::{hosts, keys, layout, session, settings, sftp, snippets};
use crate::state::AppState;

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let state = AppState::init(&handle).map_err(|e| e.message)?;
            app.manage(state.clone());
            layout::restore_windows(&handle, &state).map_err(|e| e.message)?;
            Ok(())
        })
        .on_window_event(windows::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            session::session_subscribe,
            session::session_unsubscribe,
            session::session_write,
            session::session_write_many,
            session::session_resize,
            session::session_retry,
            session::session_reconnect_now,
            session::session_cancel_reconnect,
            session::session_statuses,
            session::prompt_respond,
            session::prompt_pending,
            session::known_hosts_list,
            session::known_hosts_remove,
            layout::layout_get,
            layout::tab_new,
            layout::tab_duplicate,
            layout::tab_close,
            layout::tab_activate,
            layout::tab_rename,
            layout::tab_set_color,
            layout::tab_move,
            layout::tab_to_new_window,
            layout::tab_drop,
            layout::pane_open,
            layout::pane_attach,
            layout::pane_connect,
            layout::pane_split,
            layout::pane_close,
            layout::pane_focus,
            layout::pane_set_project,
            layout::split_set_ratio,
            layout::window_list,
            layout::window_new,
            layout::window_close,
            layout::startup_info,
            layout::restore_connect_all,
            hosts::hosts_get,
            hosts::hosts_upsert,
            hosts::hosts_duplicate,
            hosts::hosts_delete,
            hosts::group_add,
            hosts::group_rename,
            hosts::group_delete,
            hosts::group_set_collapsed,
            hosts::hosts_import_ssh_config,
            hosts::host_set_project,
            hosts::group_set_project,
            hosts::project_info,
            keys::keys_list,
            keys::keys_ssh_dir,
            keys::keys_add,
            keys::keys_forget,
            keys::keys_public,
            keys::keys_generate,
            keys::deploy_install,
            keys::deploy_disable_password,
            keys::deploy_finish,
            settings::settings_get,
            settings::settings_set,
            settings::app_info,
            settings::passphrases_clear,
            snippets::snippets_list,
            snippets::snippets_upsert,
            snippets::snippets_delete,
            sftp::sftp_home,
            sftp::sftp_realpath,
            sftp::sftp_list,
            sftp::sftp_mkdir,
            sftp::sftp_rename,
            sftp::sftp_remove,
            sftp::sftp_chmod,
            sftp::sftp_upload,
            sftp::sftp_download,
            sftp::transfers_list,
            sftp::transfer_cancel,
            sftp::transfers_clear,
            sftp::session_cwd,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start termnest");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                state.save_layout_now();
                state.sessions.close_all();
            }
        }
    });
}
