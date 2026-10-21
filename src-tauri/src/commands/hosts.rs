use termnest_core::hosts::{valid_user_name, Host, HostsList};
use termnest_core::secret::SecretString;
use termnest_core::{credstore, keydeploy, monitor, paths, ssh, ssh_config, CoreError, ErrorKind};

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
    if let Some(host) = state.hosts.get(&id) {
        credstore::delete_host(&id, password_users(&host).iter().map(String::as_str));
    }
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

/// Users of a server that may have a saved password (login, working and quick users, root).
fn password_users(host: &Host) -> Vec<String> {
    let mut users = vec![host.user.clone()];
    for u in host
        .switch_user
        .iter()
        .chain(&host.quick_users)
        .map(String::as_str)
        .chain(["root"])
    {
        if !users.iter().any(|x| x == u) {
            users.push(u.to_owned());
        }
    }
    users
}

fn host(state: &crate::state::AppState, id: &str) -> Result<Host, CoreError> {
    state
        .hosts
        .get(id)
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))
}

/// Users of the server whose password is saved in the Windows Credential Manager.
#[tauri::command]
pub fn host_saved_passwords(state: AppStateRef<'_>, id: String) -> Result<Vec<String>, CoreError> {
    let host = host(&state, &id)?;
    Ok(password_users(&host)
        .into_iter()
        .filter(|u| credstore::exists(&id, u))
        .collect())
}

/// Saves the password of a server user (used to answer sudo/su prompts).
#[tauri::command]
pub fn host_password_save(
    state: AppStateRef<'_>,
    id: String,
    user: String,
    password: SecretString,
) -> Result<(), CoreError> {
    host(&state, &id)?;
    if !valid_user_name(&user) {
        return Err(CoreError::invalid("Некорректное имя пользователя"));
    }
    credstore::save(&id, &user, &password)
}

#[tauri::command]
pub fn host_password_delete(
    state: AppStateRef<'_>,
    id: String,
    user: String,
) -> Result<(), CoreError> {
    host(&state, &id)?;
    credstore::delete(&id, &user)
}

/// Becomes `user` in the session's shell; a saved password answers the prompt.
#[tauri::command]
pub fn session_switch_user(
    state: AppStateRef<'_>,
    session: u64,
    user: String,
) -> Result<(), CoreError> {
    if !valid_user_name(&user) {
        return Err(CoreError::invalid("Некорректное имя пользователя"));
    }
    let spec = state.sessions.spec(session)?;
    let saved = spec.host_id.as_deref().and_then(|id| state.hosts.get(id));
    let method = saved
        .as_ref()
        .map(|h| h.switch_method)
        .unwrap_or(spec.switch_method);
    let password = spec
        .host_id
        .as_deref()
        .and_then(|id| credstore::load(id, method.password_user(&spec.user, &user)));
    state
        .sessions
        .switch_user(session, method.command(&user), password)
}
