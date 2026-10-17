use termnest_core::hosts::{Host, HostsList};
use termnest_core::{keydeploy, monitor, paths, ssh, ssh_config, CoreError, ErrorKind};

use super::AppStateRef;

#[tauri::command]
pub fn hosts_get(state: AppStateRef<'_>) -> HostsList {
    state.hosts.list()
}

#[tauri::command]
pub fn hosts_upsert(state: AppStateRef<'_>, host: Host) -> Result<Host, CoreError> {
    let saved = state.hosts.upsert(host)?;
    state.hosts_changed();
    Ok(saved)
}

#[tauri::command]
pub fn hosts_duplicate(state: AppStateRef<'_>, id: String) -> Result<Host, CoreError> {
    let copy = state.hosts.duplicate(&id)?;
    state.hosts_changed();
    Ok(copy)
}

#[tauri::command]
pub fn hosts_delete(state: AppStateRef<'_>, id: String) -> Result<(), CoreError> {
    state.hosts.delete(&id)?;
    state.hosts_changed();
    Ok(())
}

#[tauri::command]
pub fn group_add(
    state: AppStateRef<'_>,
    name: String,
    parent: Option<String>,
) -> Result<(), CoreError> {
    state.hosts.add_group(&name, parent.as_deref())?;
    state.hosts_changed();
    Ok(())
}

#[tauri::command]
pub fn group_rename(state: AppStateRef<'_>, old: String, new: String) -> Result<(), CoreError> {
    state.hosts.rename_group(&old, &new)?;
    state.hosts_changed();
    Ok(())
}

#[tauri::command]
pub fn group_delete(state: AppStateRef<'_>, name: String) -> Result<(), CoreError> {
    state.hosts.delete_group(&name)?;
    state.hosts_changed();
    Ok(())
}

/// Drag and drop: a server into a group (`None` - no group) before another server.
#[tauri::command]
pub fn host_move(
    state: AppStateRef<'_>,
    id: String,
    group: Option<String>,
    before: Option<String>,
) -> Result<(), CoreError> {
    state
        .hosts
        .move_host(&id, group.as_deref(), before.as_deref())?;
    state.hosts_changed();
    Ok(())
}

/// Drag and drop: a group into another group (`None` - top level) before a sibling.
#[tauri::command]
pub fn group_move(
    state: AppStateRef<'_>,
    name: String,
    parent: Option<String>,
    before: Option<String>,
) -> Result<(), CoreError> {
    state
        .hosts
        .move_group(&name, parent.as_deref(), before.as_deref())?;
    state.hosts_changed();
    Ok(())
}

#[tauri::command]
pub fn group_set_collapsed(
    state: AppStateRef<'_>,
    name: String,
    collapsed: bool,
) -> Result<(), CoreError> {
    state.hosts.set_group_collapsed(&name, collapsed)?;
    state.hosts_changed();
    Ok(())
}

/// Imports hosts from `~/.ssh/config` into the "ssh config" group. Returns the number added.
#[tauri::command]
pub fn hosts_import_ssh_config(state: AppStateRef<'_>) -> Result<usize, CoreError> {
    let path = paths::ssh_dir()
        .map(|d| d.join("config"))
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Не найдена папка .ssh"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| {
        CoreError::new(
            ErrorKind::NotFound,
            format!("Не удалось прочитать {}: {e}", path.display()),
        )
    })?;
    let entries = ssh_config::parse(&text);
    let default_user = std::env::var("USERNAME").unwrap_or_default();
    let added = state.hosts.import(&entries, "ssh config", &default_user)?;
    state.hosts_changed();
    Ok(added)
}

/// Links a server to a local project folder (`None` unlinks).
#[tauri::command]
pub fn host_set_project(
    state: AppStateRef<'_>,
    id: String,
    project: Option<String>,
) -> Result<(), CoreError> {
    state.hosts.set_host_project(&id, project)?;
    state.hosts_changed();
    Ok(())
}

/// Sets the default project of a group (`None` unlinks).
#[tauri::command]
pub fn group_set_project(
    state: AppStateRef<'_>,
    name: String,
    project: Option<String>,
) -> Result<(), CoreError> {
    state.hosts.set_group_project(&name, project)?;
    state.hosts_changed();
    Ok(())
}

/// Whether a project folder exists and is a git repository.
#[tauri::command]
pub fn project_info(path: String) -> termnest_core::project::ProjectInfo {
    termnest_core::project::info(&path)
}

#[derive(serde::Serialize)]
pub struct PasswordLoginProbe {
    pub methods: Vec<String>,
    pub password_login: bool,
}

/// Asks the server itself which login methods it offers (no credentials are sent).
#[tauri::command]
pub async fn host_password_login(
    state: AppStateRef<'_>,
    id: String,
) -> Result<PasswordLoginProbe, CoreError> {
    let spec = super::layout::spec_for_host(&state, &id)?;
    let methods = ssh::auth_methods(&spec.target(), &state.ctx).await?;
    Ok(PasswordLoginProbe {
        password_login: ssh::allows_password(&methods),
        methods,
    })
}

/// Forbids or allows password logins on the server (needs root or passwordless sudo).
#[tauri::command]
pub async fn host_set_password_login(
    state: AppStateRef<'_>,
    id: String,
    allow: bool,
) -> Result<keydeploy::PasswordLoginReport, CoreError> {
    let spec = super::layout::spec_for_host(&state, &id)?;
    keydeploy::set_password_login(
        &spec.target(),
        std::path::Path::new(&spec.key_path),
        allow,
        &state.ctx,
    )
    .await
}

/// Opens the connection of a server information window: a saved server (`host`) or
/// the server of an open session (`session`, also for quick connections).
#[tauri::command]
pub async fn monitor_open(
    state: AppStateRef<'_>,
    host: Option<String>,
    session: Option<u64>,
) -> Result<monitor::MonitorOpened, CoreError> {
    let spec = match (host, session) {
        (Some(id), _) => super::layout::spec_for_host(&state, &id)?,
        (None, Some(s)) => state.sessions.spec(s)?,
        (None, None) => return Err(CoreError::invalid("Не указан сервер")),
    };
    state
        .monitors
        .open(&spec.target(), spec.key_path, &state.ctx)
        .await
}

/// Collects resource usage and authorized keys (fixed read-only script).
#[tauri::command]
pub async fn monitor_sample(
    state: AppStateRef<'_>,
    id: u64,
) -> Result<monitor::ServerInfo, CoreError> {
    state.monitors.sample(id).await
}

#[tauri::command]
pub async fn monitor_close(state: AppStateRef<'_>, id: u64) -> Result<(), CoreError> {
    state.monitors.close(id).await;
    Ok(())
}
