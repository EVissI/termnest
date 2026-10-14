use std::path::PathBuf;

use termnest_core::keydeploy::{DisableReport, InstallReport};
use termnest_core::keys::{self, KeyInfo, KeyKind};
use termnest_core::secret::SecretString;
use termnest_core::{paths, CoreError, ErrorKind, Target};

use super::AppStateRef;
use crate::events::SETTINGS_CHANGED_EVENT;

#[tauri::command]
pub fn keys_list(state: AppStateRef<'_>) -> Vec<KeyInfo> {
    keys::list(&state.settings().extra_keys)
}

#[tauri::command]
pub fn keys_ssh_dir() -> Option<String> {
    paths::ssh_dir().map(|p| p.display().to_string())
}

/// Adds a key file from outside `~/.ssh` to the list.
#[tauri::command]
pub fn keys_add(state: AppStateRef<'_>, path: String) -> Result<KeyInfo, CoreError> {
    let info = keys::validate(&PathBuf::from(&path))?;
    let settings = state.settings.update(|s| {
        if !s.extra_keys.contains(&path) {
            s.extra_keys.push(path.clone());
        }
    })?;
    state.emit(SETTINGS_CHANGED_EVENT, settings);
    Ok(info)
}

#[tauri::command]
pub fn keys_forget(state: AppStateRef<'_>, path: String) -> Result<(), CoreError> {
    let settings = state
        .settings
        .update(|s| s.extra_keys.retain(|k| k != &path))?;
    state.emit(SETTINGS_CHANGED_EVENT, settings);
    Ok(())
}

#[tauri::command]
pub fn keys_public(path: String) -> Result<String, CoreError> {
    keys::public_key_line(&PathBuf::from(path))
}

/// Generates a key pair in `~/.ssh/<name>`.
#[tauri::command]
pub async fn keys_generate(
    name: String,
    kind: KeyKind,
    comment: String,
    passphrase: Option<SecretString>,
    overwrite: bool,
) -> Result<KeyInfo, CoreError> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.contains(['/', '\\', ':']) || name.ends_with(".pub") {
        return Err(CoreError::invalid("Некорректное имя файла ключа"));
    }
    let dir = paths::ssh_dir()
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Не найдена домашняя папка"))?;
    let path = dir.join(name);
    // RSA 4096 generation takes a while: keep it off the async runtime.
    tauri::async_runtime::spawn_blocking(move || {
        keys::generate(&path, kind, comment.trim(), passphrase.as_ref(), overwrite)
    })
    .await
    .map_err(|e| {
        CoreError::new(
            ErrorKind::KeyInvalid,
            format!("Ошибка генерации ключа: {e}"),
        )
    })?
}

/// Installs the public key on a server using a one-time password.
#[tauri::command]
pub async fn deploy_install(
    state: AppStateRef<'_>,
    target: String,
    password: SecretString,
    key_path: String,
) -> Result<InstallReport, CoreError> {
    let target: Target = target.parse()?;
    if password.is_empty() {
        return Err(CoreError::invalid("Введите пароль от сервера"));
    }
    state
        .deploys
        .install(&target, password, PathBuf::from(key_path), &state.ctx)
        .await
}

#[tauri::command]
pub async fn deploy_disable_password(
    state: AppStateRef<'_>,
    deploy_id: u64,
) -> Result<DisableReport, CoreError> {
    state.deploys.disable_password(deploy_id, &state.ctx).await
}

#[tauri::command]
pub async fn deploy_finish(state: AppStateRef<'_>, deploy_id: u64) -> Result<(), CoreError> {
    state.deploys.finish(deploy_id).await;
    Ok(())
}
