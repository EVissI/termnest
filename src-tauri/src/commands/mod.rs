//! Tauri commands: a thin layer that converts IPC arguments and delegates to the core.

pub mod hosts;
pub mod keys;
pub mod layout;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod snippets;

use std::sync::Arc;

use tauri::State;

use crate::state::AppState;

pub type AppStateRef<'a> = State<'a, Arc<AppState>>;
