//! Window creation, geometry tracking and close handling.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{
    AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent,
};
use termnest_core::layout::Bounds;
use termnest_core::settings::WindowCloseAction;
use termnest_core::{CoreError, CoreResult, ErrorKind};

use crate::events::WINDOW_CLOSE_REQUESTED_EVENT;
use crate::state::AppState;

pub const MAIN_LABEL: &str = "main";
const DEFAULT_SIZE: (f64, f64) = (1100.0, 700.0);

static WINDOW_COUNTER: AtomicU32 = AtomicU32::new(1);

pub fn new_label(app: &AppHandle) -> String {
    loop {
        let n = WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let label = format!("w{n}");
        if app.get_webview_window(&label).is_none() {
            return label;
        }
    }
}

/// Keeps generated labels from colliding with restored ones.
pub fn reserve_label(label: &str) {
    if let Some(n) = label.strip_prefix('w').and_then(|n| n.parse::<u32>().ok()) {
        WINDOW_COUNTER.fetch_max(n + 1, Ordering::Relaxed);
    }
}

fn on_screen(app: &AppHandle, b: &Bounds) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return true;
    };
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let pos = m.position().to_logical::<f64>(scale);
        let size = m.size().to_logical::<f64>(scale);
        let (x, y) = (b.x as f64 + 40.0, b.y as f64 + 20.0);
        x >= pos.x && y >= pos.y && x < pos.x + size.width && y < pos.y + size.height
    })
}

/// Creates a window showing the layout entry `label` (which must already exist).
pub fn create_window(
    app: &AppHandle,
    label: &str,
    bounds: Option<Bounds>,
    maximized: bool,
) -> CoreResult<WebviewWindow> {
    reserve_label(label);
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title("termnest")
        .min_inner_size(480.0, 300.0)
        .background_color(tauri::window::Color(20, 22, 27, 255))
        .visible(true);
    match bounds.filter(|b| on_screen(app, b)) {
        Some(b) => {
            builder = builder
                .position(b.x as f64, b.y as f64)
                .inner_size(b.width.max(480) as f64, b.height.max(300) as f64);
        }
        None => {
            builder = builder.inner_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
            if bounds.is_none() && label == MAIN_LABEL {
                builder = builder.center();
            }
        }
    }
    if maximized {
        builder = builder.maximized(true);
    }
    builder
        .build()
        .map_err(|e| CoreError::new(ErrorKind::Storage, format!("Не удалось открыть окно: {e}")))
}

fn read_bounds(window: &Window) -> Option<(Bounds, bool)> {
    let scale = window.scale_factor().ok()?;
    let pos = window.outer_position().ok()?.to_logical::<f64>(scale);
    let size = window.inner_size().ok()?.to_logical::<f64>(scale);
    let maximized = window.is_maximized().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    if minimized {
        return None;
    }
    Some((
        Bounds {
            x: pos.x.round() as i32,
            y: pos.y.round() as i32,
            width: size.width.round() as u32,
            height: size.height.round() as u32,
        },
        maximized,
    ))
}

#[derive(Clone, Serialize)]
struct CloseRequest {
    sessions: usize,
    other_windows: bool,
}

pub fn handle_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let label = window.label().to_owned();
    match event {
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            if let Some((bounds, maximized)) = read_bounds(window) {
                state.mutate_layout_quiet(|l| l.set_bounds(&label, bounds, maximized));
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            let confirmed = state
                .closing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&label);
            if confirmed {
                return;
            }
            let (sessions, other_windows) = {
                let layout = state.layout();
                let sessions = layout
                    .window(&label)
                    .map(|w| w.sessions().len())
                    .unwrap_or(0);
                (sessions, layout.windows.len() > 1)
            };
            if sessions > 0 && state.settings().on_window_close == WindowCloseAction::Ask {
                api.prevent_close();
                let _ = tauri::Emitter::emit_to(
                    window,
                    tauri::EventTarget::webview_window(&label),
                    WINDOW_CLOSE_REQUESTED_EVENT,
                    CloseRequest {
                        sessions,
                        other_windows,
                    },
                );
            }
        }
        WindowEvent::Destroyed => {
            state
                .closing
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&label);
            let others = app
                .webview_windows()
                .keys()
                .filter(|l| **l != label)
                .count();
            if others == 0 {
                // Last window: keep it in the layout so it can be restored next time.
                state.save_layout_now();
                state.sessions.close_all();
            } else {
                let mut closed = Vec::new();
                let _ = state.mutate_layout(|l| {
                    closed = l.remove_window(&label);
                    Ok(())
                });
                for id in closed {
                    state.transfers.cancel_session(id);
                    state.sessions.close(id);
                }
            }
        }
        _ => {}
    }
}

/// Window under a screen point (physical pixels), if any.
pub fn window_at(app: &AppHandle, x: f64, y: f64) -> Option<String> {
    let mut found = None;
    for (label, w) in app.webview_windows() {
        let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) else {
            continue;
        };
        if w.is_minimized().unwrap_or(false) || !w.is_visible().unwrap_or(true) {
            continue;
        }
        let inside = x >= pos.x as f64
            && y >= pos.y as f64
            && x < pos.x as f64 + size.width as f64
            && y < pos.y as f64 + size.height as f64;
        if inside {
            // Prefer the focused window when windows overlap.
            if w.is_focused().unwrap_or(false) || found.is_none() {
                found = Some(label);
            }
        }
    }
    found
}
