//! Application state shared by all commands.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use termnest_core::hostkeys::KnownHosts;
use termnest_core::hosts::HostStore;
use termnest_core::keydeploy::DeployManager;
use termnest_core::layout::{Layout, LayoutFile};
use termnest_core::secret::PassphraseCache;
use termnest_core::settings::{Settings, SettingsStore};
use termnest_core::snippets::SnippetStore;
use termnest_core::ssh::SshContext;
use termnest_core::transfer::TransferManager;
use termnest_core::{paths, CoreError, CoreResult, ErrorKind, SessionManager};
use tokio::sync::mpsc;

use crate::events::{self, TauriStateSink, TauriTransferSink, LAYOUT_CHANGED_EVENT};
use crate::prompts::PromptHub;

pub struct AppState {
    pub app: AppHandle,
    pub dir: PathBuf,
    pub sessions: Arc<SessionManager>,
    pub ctx: Arc<SshContext>,
    pub known_hosts: Arc<KnownHosts>,
    pub hosts: HostStore,
    pub settings: SettingsStore,
    pub snippets: SnippetStore,
    pub prompts: Arc<PromptHub>,
    pub deploys: DeployManager,
    pub monitors: termnest_core::monitor::MonitorManager,
    pub transfers: Arc<TransferManager>,
    pub mcp: crate::mcp::McpHub,
    pub agents: Arc<termnest_core::agents::AgentManager>,
    layout: Mutex<Layout>,
    layout_file: LayoutFile,
    save_tx: mpsc::UnboundedSender<()>,
    /// Windows whose close was confirmed by the user (or that need no confirmation).
    pub closing: Mutex<HashSet<String>>,
    /// Problems found while loading data files, shown once in the UI.
    pub warnings: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Moves a broken config aside so that it is not overwritten, and reports it.
fn quarantine(path: &Path, err: &CoreError, warnings: &mut Vec<String>) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut backup = path.as_os_str().to_owned();
    backup.push(format!(".broken-{stamp}"));
    let moved = std::fs::rename(path, &backup).is_ok();
    warnings.push(if moved {
        format!(
            "{} Файл сохранён как {}.",
            err.message,
            PathBuf::from(backup).display()
        )
    } else {
        err.message.clone()
    });
}

impl AppState {
    pub fn init(app: &AppHandle) -> CoreResult<Arc<Self>> {
        let dir = paths::app_dir().ok_or_else(|| {
            CoreError::new(ErrorKind::Storage, "Не удалось определить папку %APPDATA%")
        })?;
        std::fs::create_dir_all(&dir)
            .map_err(|e| CoreError::storage("Не удалось создать папку данных", e))?;
        let mut warnings = Vec::new();

        let (settings, err) = SettingsStore::load(dir.join("settings.toml"));
        if let Some(e) = err {
            warnings.push(e.message);
        }

        let hosts_path = dir.join("hosts.toml");
        let hosts = HostStore::load(hosts_path.clone()).or_else(|e| {
            quarantine(&hosts_path, &e, &mut warnings);
            HostStore::load(hosts_path.clone())
        })?;

        let snippets_path = dir.join("snippets.toml");
        let snippets = SnippetStore::load(snippets_path.clone()).or_else(|e| {
            quarantine(&snippets_path, &e, &mut warnings);
            SnippetStore::load(snippets_path.clone())
        })?;

        let known_hosts = Arc::new(KnownHosts::load(dir.join("known_hosts"))?);
        let prompts = Arc::new(PromptHub::new(app.clone()));
        let ctx = Arc::new(SshContext::new(
            prompts.clone(),
            known_hosts.clone(),
            Arc::new(PassphraseCache::default()),
            settings.get().keepalive_secs,
        ));
        // Tauri's Tokio runtime: commands may run on the UI thread, so tasks are
        // spawned through an explicit handle.
        let tauri::async_runtime::RuntimeHandle::Tokio(runtime) = tauri::async_runtime::handle();
        let sessions = Arc::new(SessionManager::new(
            Arc::new(TauriStateSink(app.clone())),
            ctx.clone(),
            runtime.clone(),
        ));
        let runtime_handle = runtime.clone();
        let transfers = TransferManager::new(
            Arc::new(TauriTransferSink(app.clone())),
            prompts.clone(),
            runtime,
        );

        let agents = termnest_core::agents::AgentManager::load(
            dir.join("agents"),
            settings.get().claude.max_agents as usize,
            Arc::new(events::TauriAgentSink(app.clone())),
            runtime_handle.clone(),
        );

        let (save_tx, save_rx) = mpsc::unbounded_channel();
        let state = Arc::new(Self {
            app: app.clone(),
            layout_file: LayoutFile::new(dir.join("layout.json")),
            dir,
            sessions,
            ctx,
            known_hosts,
            hosts,
            settings,
            snippets,
            prompts,
            deploys: DeployManager::default(),
            monitors: Default::default(),
            transfers,
            mcp: crate::mcp::McpHub::default(),
            agents,
            layout: Mutex::new(Layout::default()),
            save_tx,
            closing: Mutex::new(HashSet::new()),
            warnings: Mutex::new(warnings),
        });
        tauri::async_runtime::spawn(save_loop(Arc::downgrade(&state), save_rx));
        Ok(state)
    }

    pub fn settings(&self) -> Settings {
        self.settings.get()
    }

    pub fn load_saved_layout(&self) -> Option<Layout> {
        self.layout_file.load()
    }

    pub fn layout(&self) -> MutexGuard<'_, Layout> {
        lock(&self.layout)
    }

    /// Changes the layout, notifies all windows and schedules a save.
    pub fn mutate_layout<T>(&self, f: impl FnOnce(&mut Layout) -> CoreResult<T>) -> CoreResult<T> {
        let (result, snapshot) = {
            let mut layout = lock(&self.layout);
            let result = f(&mut layout)?;
            (result, layout.clone())
        };
        let _ = self.app.emit(LAYOUT_CHANGED_EVENT, &snapshot);
        self.schedule_save();
        Ok(result)
    }

    /// Changes the layout without notifying windows (window geometry).
    pub fn mutate_layout_quiet(&self, f: impl FnOnce(&mut Layout)) {
        f(&mut lock(&self.layout));
        self.schedule_save();
    }

    pub fn schedule_save(&self) {
        let _ = self.save_tx.send(());
    }

    pub fn save_layout_now(&self) {
        let snapshot = lock(&self.layout).clone();
        if let Err(e) = self.layout_file.save(&snapshot) {
            log::warn!("{e}");
        }
    }

    pub fn emit<S: serde::Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.app.emit(event, payload);
    }

    pub fn hosts_changed(&self) {
        self.emit(events::HOSTS_CHANGED_EVENT, self.hosts.list());
    }
}

/// Saves the layout at most once per second while it keeps changing.
async fn save_loop(state: std::sync::Weak<AppState>, mut rx: mpsc::UnboundedReceiver<()>) {
    while rx.recv().await.is_some() {
        tokio::time::sleep(Duration::from_millis(1000)).await;
        while rx.try_recv().is_ok() {}
        match state.upgrade() {
            Some(s) => s.save_layout_now(),
            None => return,
        }
    }
}
