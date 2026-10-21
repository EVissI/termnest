//! Tabs, splits, panels and windows.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use termnest_core::layout::{Bounds, Direction, Layout, PaneId, SplitId, TabId};
use termnest_core::{paths, ConnectSpec, CoreError, CoreResult, ErrorKind, SessionId, Target};

use super::AppStateRef;
use crate::state::AppState;
use crate::windows;

/// What a new tab or panel should connect to.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenSource {
    Host {
        id: String,
    },
    Target {
        target: String,
    },
    /// Same server as an existing panel.
    Pane {
        pane: PaneId,
    },
    Empty,
}

fn resolve_key(state: &AppState, own: Option<&str>) -> CoreResult<String> {
    if let Some(k) = own.filter(|k| !k.trim().is_empty()) {
        return Ok(k.to_owned());
    }
    if let Some(k) = state.settings().default_key_path {
        return Ok(k);
    }
    paths::default_key_path()
        .map(|p| p.display().to_string())
        .ok_or_else(|| {
            CoreError::new(
                ErrorKind::KeyNotFound,
                "Не найден ключ: укажите его у сервера или в настройках (ключ по умолчанию)",
            )
        })
}

pub fn spec_for_host(state: &AppState, id: &str) -> CoreResult<ConnectSpec> {
    let host = state
        .hosts
        .get(id)
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))?;
    Ok(ConnectSpec {
        label: host.name.clone(),
        host_id: Some(host.id.clone()),
        user: host.user.clone(),
        host: host.host.clone(),
        port: host.port,
        key_path: resolve_key(state, host.key_path.as_deref())?,
        switch_user: host.switch_user.clone(),
        switch_method: host.switch_method,
    })
}

fn spec_for(state: &AppState, source: &OpenSource) -> CoreResult<Option<ConnectSpec>> {
    match source {
        OpenSource::Host { id } => spec_for_host(state, id).map(Some),
        OpenSource::Target { target } => {
            let t: Target = target.parse()?;
            Ok(Some(ConnectSpec::from_target(
                &t,
                resolve_key(state, None)?,
                None,
            )))
        }
        OpenSource::Pane { pane } => Ok(state.layout().pane(*pane).and_then(|p| p.spec.clone())),
        OpenSource::Empty => Ok(None),
    }
}

/// The saved host may have been edited since the panel was created.
fn refreshed(state: &AppState, spec: ConnectSpec) -> ConnectSpec {
    match &spec.host_id {
        Some(id) => spec_for_host(state, id).unwrap_or(spec),
        None => spec,
    }
}

fn close_sessions(state: &AppState, ids: &[SessionId]) {
    for id in ids {
        state.transfers.cancel_session(*id);
        state.sessions.close(*id);
    }
}

/// Closes a window that has no tabs left, unless it is the only one.
fn close_if_empty(app: &AppHandle, state: &AppState, label: Option<String>) {
    let Some(label) = label else { return };
    if state.layout().windows.len() <= 1 {
        return;
    }
    if let Some(w) = app.get_webview_window(&label) {
        state
            .closing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(label);
        let _ = w.close();
    }
}

#[tauri::command]
pub fn layout_get(state: AppStateRef<'_>) -> Layout {
    state.layout().clone()
}

#[tauri::command]
pub fn tab_new(
    state: AppStateRef<'_>,
    window: String,
    source: OpenSource,
    index: Option<usize>,
) -> Result<(TabId, PaneId), CoreError> {
    let spec = spec_for(&state, &source)?;
    state.mutate_layout(|l| l.new_tab(&window, spec, index))
}

/// Opens a new tab right after the panel's tab with a new connection to the same server.
#[tauri::command]
pub fn tab_duplicate(state: AppStateRef<'_>, pane: PaneId) -> Result<(TabId, PaneId), CoreError> {
    let (window, index, spec) = {
        let layout = state.layout();
        let window = layout
            .pane_window(pane)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Панель не найдена"))?
            .to_owned();
        let w = layout
            .window(&window)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Окно не найдено"))?;
        let index = w
            .tabs
            .iter()
            .position(|t| t.panes().iter().any(|p| p.id == pane))
            .map(|i| i + 1);
        let spec = layout.pane(pane).and_then(|p| p.spec.clone());
        (window, index, spec)
    };
    if spec.is_none() {
        return Err(CoreError::new(
            ErrorKind::InvalidInput,
            "В панели нет подключения",
        ));
    }
    state.mutate_layout(|l| l.new_tab(&window, spec, index))
}

/// Sets what an empty (or finished) panel connects to.
#[tauri::command]
pub fn pane_open(
    state: AppStateRef<'_>,
    pane: PaneId,
    source: OpenSource,
) -> Result<(), CoreError> {
    let spec = spec_for(&state, &source)?;
    let old = state.layout().pane(pane).and_then(|p| p.session);
    state.mutate_layout(|l| {
        l.set_session(pane, None)?;
        l.release(pane)?;
        l.set_spec(pane, spec)
    })?;
    if let Some(id) = old {
        close_sessions(&state, &[id]);
    }
    Ok(())
}

/// Called by a panel once it knows its size: starts the connection if needed.
/// Returns the session to subscribe to (`None` for empty or held panels).
#[tauri::command]
pub fn pane_attach(
    state: AppStateRef<'_>,
    pane: PaneId,
    cols: u32,
    rows: u32,
) -> Result<Option<SessionId>, CoreError> {
    let (spec, session, held) = {
        let layout = state.layout();
        let p = layout
            .pane(pane)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Панель не найдена"))?;
        (p.spec.clone(), p.session, p.held)
    };
    if session.is_some() || held {
        return Ok(session);
    }
    let Some(spec) = spec else { return Ok(None) };
    let id = state.sessions.open(refreshed(&state, spec), cols, rows);
    state.mutate_layout(|l| l.set_session(pane, Some(id)))?;
    Ok(Some(id))
}

/// Explicit "connect" on a held panel, or reconnect of a finished session.
#[tauri::command]
pub fn pane_connect(
    state: AppStateRef<'_>,
    pane: PaneId,
    cols: u32,
    rows: u32,
) -> Result<Option<SessionId>, CoreError> {
    let session = state.layout().pane(pane).and_then(|p| p.session);
    if let Some(id) = session {
        if state.sessions.retry(id).is_ok() {
            return Ok(Some(id));
        }
        state.mutate_layout(|l| l.set_session(pane, None))?;
    }
    state.mutate_layout(|l| l.release(pane))?;
    pane_attach(state, pane, cols, rows)
}

#[tauri::command]
pub fn pane_split(
    state: AppStateRef<'_>,
    pane: PaneId,
    direction: Direction,
    source: OpenSource,
) -> Result<PaneId, CoreError> {
    let spec = spec_for(&state, &source)?;
    state.mutate_layout(|l| l.split(pane, direction, spec))
}

#[tauri::command]
pub async fn pane_close(
    app: AppHandle,
    state: AppStateRef<'_>,
    pane: PaneId,
) -> Result<(), CoreError> {
    let removed = state.mutate_layout(|l| l.close_pane(pane))?;
    close_sessions(&state, &removed.sessions);
    close_if_empty(&app, &state, removed.window_empty);
    Ok(())
}

#[tauri::command]
pub fn pane_focus(state: AppStateRef<'_>, pane: PaneId) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.focus_pane(pane))
}

#[tauri::command]
pub async fn tab_close(
    app: AppHandle,
    state: AppStateRef<'_>,
    tab: TabId,
) -> Result<(), CoreError> {
    let removed = state.mutate_layout(|l| l.close_tab(tab))?;
    close_sessions(&state, &removed.sessions);
    close_if_empty(&app, &state, removed.window_empty);
    Ok(())
}

#[tauri::command]
pub fn tab_activate(state: AppStateRef<'_>, tab: TabId) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.activate_tab(tab))
}

#[tauri::command]
pub fn tab_rename(
    state: AppStateRef<'_>,
    tab: TabId,
    title: Option<String>,
) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.rename_tab(tab, title))
}

#[tauri::command]
pub fn tab_set_color(
    state: AppStateRef<'_>,
    tab: TabId,
    color: Option<String>,
) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.set_tab_color(tab, color))
}

#[tauri::command]
pub fn split_set_ratio(
    state: AppStateRef<'_>,
    tab: TabId,
    split: SplitId,
    ratio: f64,
) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.set_ratio(tab, split, ratio))
}

fn move_tab_to(
    app: &AppHandle,
    state: &AppState,
    tab: TabId,
    window: &str,
    index: Option<usize>,
) -> CoreResult<()> {
    let emptied = state.mutate_layout(|l| l.move_tab(tab, window, index))?;
    if let Some(w) = app.get_webview_window(window) {
        let _ = w.set_focus();
    }
    close_if_empty(app, state, emptied);
    Ok(())
}

fn move_tab_to_new_window(
    app: &AppHandle,
    state: &AppState,
    tab: TabId,
    screen: Option<(f64, f64)>,
) -> CoreResult<String> {
    let source = state
        .layout()
        .tab_window(tab)
        .map(str::to_owned)
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Вкладка не найдена"))?;
    let (width, height) = state
        .layout()
        .window(&source)
        .and_then(|w| w.bounds)
        .map(|b| (b.width, b.height))
        .unwrap_or((1000, 650));
    let bounds = screen.map(|(x, y)| {
        let scale = app
            .get_webview_window(&source)
            .and_then(|w| w.scale_factor().ok())
            .unwrap_or(1.0);
        Bounds {
            x: (x / scale) as i32 - 120,
            y: (y / scale) as i32 - 16,
            width,
            height,
        }
    });
    let label = windows::new_label(app);
    let emptied = state.mutate_layout(|l| {
        l.add_window(&label, bounds);
        l.move_tab(tab, &label, None)
    })?;
    windows::create_window(app, &label, bounds, false)?;
    close_if_empty(app, state, emptied);
    Ok(label)
}

// Commands that create or close windows are async: on Windows a window must not be
// created from a synchronous command, which runs on the event loop thread.

/// Moves a tab within a window (reorder) or to another window. Never reconnects.
#[tauri::command]
pub async fn tab_move(
    app: AppHandle,
    state: AppStateRef<'_>,
    tab: TabId,
    window: String,
    index: Option<usize>,
) -> Result<(), CoreError> {
    move_tab_to(&app, &state, tab, &window, index)
}

/// Moves a tab into a new window, optionally placed at a screen point (physical pixels).
#[tauri::command]
pub async fn tab_to_new_window(
    app: AppHandle,
    state: AppStateRef<'_>,
    tab: TabId,
    screen_x: Option<f64>,
    screen_y: Option<f64>,
) -> Result<String, CoreError> {
    let screen = screen_x.zip(screen_y);
    move_tab_to_new_window(&app, &state, tab, screen)
}

/// End of a tab drag outside its tab bar: drop into the window under the cursor
/// or into a new window.
#[tauri::command]
pub async fn tab_drop(
    app: AppHandle,
    state: AppStateRef<'_>,
    tab: TabId,
    screen_x: f64,
    screen_y: f64,
) -> Result<(), CoreError> {
    let source = state.layout().tab_window(tab).map(str::to_owned);
    match windows::window_at(&app, screen_x, screen_y) {
        Some(target) if Some(&target) != source.as_ref() => {
            move_tab_to(&app, &state, tab, &target, None)
        }
        Some(_) => Ok(()),
        None => {
            let single_tab = source
                .as_deref()
                .and_then(|s| state.layout().window(s).map(|w| w.tabs.len() == 1))
                .unwrap_or(false);
            if single_tab {
                // Dragging the only tab out of its window: nothing to split off.
                return Ok(());
            }
            move_tab_to_new_window(&app, &state, tab, Some((screen_x, screen_y))).map(|_| ())
        }
    }
}

#[derive(Serialize)]
pub struct WindowInfo {
    label: String,
    number: usize,
    tabs: usize,
}

#[tauri::command]
pub fn window_list(state: AppStateRef<'_>) -> Vec<WindowInfo> {
    state
        .layout()
        .windows
        .iter()
        .enumerate()
        .map(|(i, w)| WindowInfo {
            label: w.label.clone(),
            number: i + 1,
            tabs: w.tabs.len(),
        })
        .collect()
}

#[tauri::command]
pub async fn window_new(app: AppHandle, state: AppStateRef<'_>) -> Result<String, CoreError> {
    let label = windows::new_label(&app);
    state.mutate_layout(|l| {
        l.add_window(&label, None);
        Ok(())
    })?;
    windows::create_window(&app, &label, None, false)?;
    Ok(label)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseAction {
    CloseSessions,
    MoveTabs,
}

/// Answer to the "window has active sessions" dialog.
#[tauri::command]
pub async fn window_close(
    app: AppHandle,
    state: AppStateRef<'_>,
    label: String,
    action: CloseAction,
) -> Result<(), CoreError> {
    if let CloseAction::MoveTabs = action {
        let target = state
            .layout()
            .windows
            .iter()
            .find(|w| w.label != label)
            .map(|w| w.label.clone())
            .ok_or_else(|| CoreError::new(ErrorKind::InvalidInput, "Нет другого окна"))?;
        state.mutate_layout(|l| {
            let tabs: Vec<TabId> = l
                .window(&label)
                .map(|w| w.tabs.iter().map(|t| t.id).collect())
                .unwrap_or_default();
            for t in tabs {
                l.move_tab(t, &target, None)?;
            }
            Ok(())
        })?;
        if let Some(w) = app.get_webview_window(&target) {
            let _ = w.set_focus();
        }
    }
    state
        .closing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.clone());
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct StartupInfo {
    warnings: Vec<String>,
    restore_pending: bool,
}

/// Data-file warnings (shown once) and whether restored panels wait for permission.
#[tauri::command]
pub fn startup_info(state: AppStateRef<'_>) -> StartupInfo {
    let warnings = std::mem::take(&mut *state.warnings.lock().unwrap_or_else(|e| e.into_inner()));
    let restore_pending = state
        .layout()
        .windows
        .iter()
        .any(|w| w.tabs.iter().any(|t| t.panes().iter().any(|p| p.held)));
    StartupInfo {
        warnings,
        restore_pending,
    }
}

/// "Reconnect all" after restoring in ask mode.
#[tauri::command]
pub fn restore_connect_all(state: AppStateRef<'_>) -> Result<(), CoreError> {
    state.mutate_layout(|l| {
        l.release_all();
        Ok(())
    })
}

/// Restores the saved layout on startup (or creates the main window).
pub fn restore_windows(app: &AppHandle, state: &AppState) -> CoreResult<()> {
    use termnest_core::settings::RestoreMode;

    let mode = state.settings().restore;
    let saved = match mode {
        RestoreMode::Off => None,
        _ => state.load_saved_layout().filter(|l| !l.windows.is_empty()),
    };
    let windows: Vec<(String, Option<Bounds>, bool)> = match saved {
        Some(mut layout) => {
            if mode == RestoreMode::Ask {
                layout.hold_all();
            }
            let list = layout
                .windows
                .iter()
                .map(|w| (w.label.clone(), w.bounds, w.maximized))
                .collect();
            *state.layout() = layout;
            list
        }
        None => {
            state.layout().add_window(windows::MAIN_LABEL, None);
            vec![(windows::MAIN_LABEL.to_owned(), None, false)]
        }
    };
    for (label, bounds, maximized) in windows {
        windows::create_window(app, &label, bounds, maximized)?;
    }
    Ok(())
}

/// Project override for one panel (kept in layout.json, not in hosts.toml).
#[tauri::command]
pub fn pane_set_project(
    state: AppStateRef<'_>,
    pane: PaneId,
    project: Option<String>,
) -> Result<(), CoreError> {
    state.mutate_layout(|l| l.set_pane_project(pane, project))
}
