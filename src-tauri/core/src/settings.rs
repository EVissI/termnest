//! Application settings (`settings.toml`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::paths::atomic_write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WindowCloseAction {
    /// Ask what to do with the window's sessions.
    #[default]
    Ask,
    /// Close the window together with its sessions.
    CloseSessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RestoreMode {
    /// Restore the layout and reconnect all panels.
    #[default]
    Auto,
    /// Restore the layout and ask before reconnecting.
    Ask,
    /// Start with an empty window.
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: String,
    pub font_family: String,
    pub font_size: u16,
    pub line_height: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: bool,
    pub scrollback: u32,
    pub copy_on_select: bool,
    pub warn_multiline_paste: bool,
    /// SSH keepalive interval in seconds, 0 disables it.
    pub keepalive_secs: u32,
    pub on_window_close: WindowCloseAction,
    pub restore: RestoreMode,
    /// Key used for quick connect and hosts without their own key.
    pub default_key_path: Option<String>,
    /// Private keys added manually (outside `~/.ssh`).
    pub extra_keys: Vec<String>,
    /// Hotkey overrides: action id -> combination like "Ctrl+Shift+T".
    pub hotkeys: BTreeMap<String, String>,
    pub sidebar_visible: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "termnest-dark".into(),
            font_family: "'Cascadia Mono', Consolas, 'Courier New', monospace".into(),
            font_size: 14,
            line_height: 1.0,
            cursor_style: CursorStyle::Block,
            cursor_blink: true,
            scrollback: 10_000,
            copy_on_select: false,
            warn_multiline_paste: true,
            keepalive_secs: 30,
            on_window_close: WindowCloseAction::Ask,
            restore: RestoreMode::Auto,
            default_key_path: None,
            extra_keys: Vec::new(),
            hotkeys: BTreeMap::new(),
            sidebar_visible: true,
        }
    }
}

impl Settings {
    fn sanitize(&mut self) {
        self.font_size = self.font_size.clamp(6, 48);
        if !self.line_height.is_finite() {
            self.line_height = 1.0;
        }
        self.line_height = self.line_height.clamp(0.8, 2.0);
        self.scrollback = self.scrollback.clamp(100, 200_000);
        self.keepalive_secs = self.keepalive_secs.min(3600);
        self.default_key_path = self
            .default_key_path
            .take()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty());
        self.extra_keys.retain(|k| !k.trim().is_empty());
        self.extra_keys.dedup();
    }
}

pub struct SettingsStore {
    path: PathBuf,
    data: Mutex<Settings>,
}

impl SettingsStore {
    /// Loads settings; a broken file is reported and replaced by defaults in memory.
    pub fn load(path: PathBuf) -> (Self, Option<CoreError>) {
        let (mut data, err) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Settings>(&text) {
                Ok(s) => (s, None),
                Err(e) => (
                    Settings::default(),
                    Some(CoreError::storage(
                        "Ошибка в settings.toml, используются значения по умолчанию",
                        e,
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (
                Settings::default(),
                Some(CoreError::storage("Не удалось прочитать settings.toml", e)),
            ),
        };
        data.sanitize();
        (
            Self {
                path,
                data: Mutex::new(data),
            },
            err,
        )
    }

    pub fn get(&self) -> Settings {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set(&self, mut settings: Settings) -> CoreResult<Settings> {
        settings.sanitize();
        let text = toml::to_string_pretty(&settings)
            .map_err(|e| CoreError::storage("Не удалось сериализовать настройки", e))?;
        atomic_write(&self.path, text.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить settings.toml", e))?;
        *self.data.lock().unwrap_or_else(|e| e.into_inner()) = settings.clone();
        Ok(settings)
    }

    pub fn update(&self, f: impl FnOnce(&mut Settings)) -> CoreResult<Settings> {
        let mut s = self.get();
        f(&mut s);
        self.set(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_dir::TestDir;

    #[test]
    fn defaults_roundtrip_and_partial_files() {
        let dir = TestDir::new();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            "font_size = 99\n[hotkeys]\n\"tab.new\" = \"Ctrl+T\"\n",
        )
        .unwrap();
        let (store, err) = SettingsStore::load(path.clone());
        assert!(err.is_none());
        let s = store.get();
        assert_eq!(s.font_size, 48, "clamped");
        assert_eq!(s.keepalive_secs, 30, "missing field gets default");
        assert_eq!(s.hotkeys.get("tab.new").unwrap(), "Ctrl+T");

        store.update(|s| s.restore = RestoreMode::Ask).unwrap();
        let (again, _) = SettingsStore::load(path);
        assert_eq!(again.get().restore, RestoreMode::Ask);
    }

    #[test]
    fn broken_file_falls_back_to_defaults() {
        let dir = TestDir::new();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "font_size = \"big\"").unwrap();
        let (store, err) = SettingsStore::load(path);
        assert!(err.is_some());
        assert_eq!(store.get(), Settings::default());
    }
}
