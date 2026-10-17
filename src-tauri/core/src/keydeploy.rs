//! Installing a public key on a server (like `ssh-copy-id`) and, optionally,
//! disabling password logins afterwards.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use russh::client::Handle;
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::secret::SecretString;
use crate::ssh::{self, Auth, Client, SshContext};
use crate::target::Target;

pub const DROP_IN_PATH: &str = "/etc/ssh/sshd_config.d/00-termnest-keys-only.conf";

/// Reads the public key line from stdin and appends it to `authorized_keys` unless the
/// same key (type + data, comment ignored) is already there. The key never appears in
/// the command line. Wrapped in `sh -c` so it works with any login shell.
const INSTALL_SCRIPT: &str = r#"sh -c 'umask 077; IFS= read -r key || [ -n "$key" ]; set -- $key; body="$1 $2"; d="$HOME/.ssh"; f="$d/authorized_keys"; mkdir -p "$d" && chmod 700 "$d" && touch "$f" && chmod 600 "$f" || exit 3; if grep -qF -- "$body" "$f"; then echo TERMNEST_EXISTS; exit 0; fi; if [ -s "$f" ] && [ -n "$(tail -c1 "$f")" ]; then echo >> "$f"; fi; printf "%s\n" "$key" >> "$f" && echo TERMNEST_ADDED'"#;

#[derive(Debug, Clone, Serialize)]
pub struct InstallReport {
    /// Id of the kept password connection, used for the optional next step.
    pub deploy_id: u64,
    /// `false` if the key was already present.
    pub added: bool,
    pub verified: bool,
    pub verify_error: Option<String>,
    /// root, or passwordless sudo is available (password login can be disabled).
    pub can_disable_password: bool,
    pub privilege_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisableReport {
    pub steps: Vec<String>,
    /// Output of `sshd -T | grep passwordauthentication` after the change.
    pub effective: String,
}

struct Deploy {
    handle: Arc<Handle<Client>>,
    target: Target,
    key_path: PathBuf,
    verified: bool,
    sudo_prefix: Option<String>,
}

/// Keeps the password connection of each wizard run open until the wizard closes,
/// so disabling password logins never relies on a fresh password login.
#[derive(Default)]
pub struct DeployManager {
    runs: Mutex<HashMap<u64, Deploy>>,
    next: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl DeployManager {
    pub async fn install(
        &self,
        target: &Target,
        password: SecretString,
        key_path: PathBuf,
        ctx: &SshContext,
    ) -> CoreResult<InstallReport> {
        let public_line = crate::keys::public_key_line(&key_path)?;
        if public_line.contains(['\n', '\r', '\'']) {
            return Err(CoreError::new(
                ErrorKind::KeyInvalid,
                "Некорректный публичный ключ",
            ));
        }

        let handle = ssh::connect(target, Auth::Password(&password), ctx).await?;
        // The password is not needed any more: wipe it right away.
        drop(password);
        let handle = Arc::new(handle);

        let out = ssh::exec(
            &handle,
            INSTALL_SCRIPT,
            Some(format!("{public_line}\n").as_bytes()),
        )
        .await?;
        let added = if out.stdout.contains("TERMNEST_ADDED") {
            true
        } else if out.stdout.contains("TERMNEST_EXISTS") {
            false
        } else {
            ssh::disconnect(&handle).await;
            return Err(CoreError::new(
                ErrorKind::Remote,
                format!(
                    "Не удалось записать ключ в ~/.ssh/authorized_keys (код {}): {}",
                    out.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into()),
                    out.stderr.trim()
                ),
            ));
        };

        let (verified, verify_error) = match verify_key_login(target, &key_path, ctx).await {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e.message)),
        };

        let (sudo_prefix, privilege_note) = detect_privileges(&handle).await;
        let deploy_id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let can_disable_password = verified && sudo_prefix.is_some();
        lock(&self.runs).insert(
            deploy_id,
            Deploy {
                handle,
                target: target.clone(),
                key_path,
                verified,
                sudo_prefix,
            },
        );
        Ok(InstallReport {
            deploy_id,
            added,
            verified,
            verify_error,
            can_disable_password,
            privilege_note,
        })
    }

    /// Disables password logins with a drop-in file, verifying key login with a new
    /// connection and rolling back on any failure. The original connection stays open.
    pub async fn disable_password(
        &self,
        deploy_id: u64,
        ctx: &SshContext,
    ) -> CoreResult<DisableReport> {
        let (handle, target, key_path, verified, prefix) = {
            let runs = lock(&self.runs);
            let d = runs.get(&deploy_id).ok_or_else(|| {
                CoreError::new(ErrorKind::NotFound, "Сеанс мастера закрыт, начните заново")
            })?;
            (
                d.handle.clone(),
                d.target.clone(),
                d.key_path.clone(),
                d.verified,
                d.sudo_prefix.clone(),
            )
        };
        if !verified {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Вход по ключу не подтверждён, отключать пароль небезопасно",
            ));
        }
        let Some(p) = prefix else {
            return Err(CoreError::new(
                ErrorKind::PermissionDenied,
                manual_instructions(),
            ));
        };

        let mut steps = Vec::new();
        apply_drop_in(
            &handle,
            &p,
            Some(KEYS_ONLY),
            &DropInCheck {
                target: &target,
                key_path: &key_path,
                ctx,
            },
            &mut steps,
        )
        .await?;
        let effective = ssh::exec(
            &handle,
            &format!(
                "{p}sh -c '{SBIN}; sshd -T 2>/dev/null | grep -i \"^passwordauthentication\"'"
            ),
            None,
        )
        .await
        .map(|o| o.stdout.trim().to_owned())
        .unwrap_or_default();
        Ok(DisableReport { steps, effective })
    }

    /// Closes the wizard's password connection.
    pub async fn finish(&self, deploy_id: u64) {
        let run = lock(&self.runs).remove(&deploy_id);
        if let Some(d) = run {
            ssh::disconnect(&d.handle).await;
        }
    }
}

const KEYS_ONLY: &str = "# Created by termnest: key-only SSH logins.\nPasswordAuthentication no\nKbdInteractiveAuthentication no\n";
const ALLOW_PASSWORD: &str =
    "# Created by termnest: password logins allowed.\nPasswordAuthentication yes\n";
const SBIN: &str = "PATH=$PATH:/usr/sbin:/sbin";

/// How to confirm that key login still works after an sshd change.
struct DropInCheck<'a> {
    target: &'a Target,
    key_path: &'a std::path::Path,
    ctx: &'a SshContext,
}

/// Writes (`Some`) or removes (`None`) the termnest drop-in, checks the config with
/// `sshd -t`, reloads sshd and opens a new key connection. Any failure restores the
/// previous file, so a mistake never locks the user out.
async fn apply_drop_in(
    handle: &Handle<Client>,
    p: &str,
    content: Option<&str>,
    check: &DropInCheck<'_>,
    steps: &mut Vec<String>,
) -> CoreResult<()> {
    let run = |cmd: String| async move { ssh::exec(handle, &cmd, None).await };
    let write = |text: String| async move {
        ssh::exec(
            handle,
            &format!("{p}sh -c 'cat > {DROP_IN_PATH} && chmod 644 {DROP_IN_PATH}'"),
            Some(text.as_bytes()),
        )
        .await
    };
    let remove = format!("{p}rm -f {DROP_IN_PATH}");
    let reload = format!(
        "{p}sh -c 'systemctl reload ssh 2>/dev/null || systemctl reload sshd 2>/dev/null || service ssh reload 2>/dev/null || service sshd reload'"
    );

    if content.is_some() {
        // sshd must read the drop-in directory, otherwise the file has no effect.
        let include = run(format!(
            "{p}sh -c 'grep -Eiq \"^[[:space:]]*Include[[:space:]]+/etc/ssh/sshd_config.d/\\*\\.conf\" /etc/ssh/sshd_config'"
        ))
        .await?;
        if include.exit_code != Some(0) {
            return Err(CoreError::new(
                ErrorKind::Remote,
                format!(
                    "В /etc/ssh/sshd_config нет строки «Include /etc/ssh/sshd_config.d/*.conf», drop-in не сработает. {}",
                    manual_instructions()
                ),
            ));
        }
        steps.push("Найден Include /etc/ssh/sshd_config.d/*.conf".to_owned());
    }

    let previous = run(format!("{p}cat {DROP_IN_PATH}")).await?;
    let previous = (previous.exit_code == Some(0)).then_some(previous.stdout);
    let restore = || async {
        let _ = match &previous {
            Some(text) => write(text.clone()).await,
            None => run(remove.clone()).await,
        };
    };

    match content {
        Some(text) => {
            let out = write(text.to_owned()).await?;
            if out.exit_code != Some(0) {
                restore().await;
                return Err(CoreError::new(
                    ErrorKind::Remote,
                    format!("Не удалось записать {DROP_IN_PATH}: {}", out.stderr.trim()),
                ));
            }
            steps.push(format!("Записан {DROP_IN_PATH}"));
        }
        None => {
            let out = run(remove.clone()).await?;
            if out.exit_code != Some(0) {
                return Err(CoreError::new(
                    ErrorKind::Remote,
                    format!("Не удалось удалить {DROP_IN_PATH}: {}", out.stderr.trim()),
                ));
            }
            steps.push(format!("Удалён {DROP_IN_PATH}"));
        }
    }

    let test = run(format!("{p}sh -c '{SBIN}; sshd -t'")).await?;
    if test.exit_code != Some(0) {
        restore().await;
        return Err(CoreError::new(
            ErrorKind::Remote,
            format!(
                "sshd -t нашёл ошибку, изменения откатаны: {}",
                test.stderr.trim()
            ),
        ));
    }
    steps.push("Проверка конфигурации sshd -t пройдена".to_owned());

    let reloaded = run(reload.clone()).await?;
    if reloaded.exit_code != Some(0) {
        restore().await;
        return Err(CoreError::new(
            ErrorKind::Remote,
            format!(
                "Не удалось перезагрузить sshd, изменения откатаны: {}",
                reloaded.stderr.trim()
            ),
        ));
    }
    steps.push("sshd перезагружен".to_owned());

    if let Err(e) = verify_key_login(check.target, check.key_path, check.ctx).await {
        restore().await;
        let _ = run(reload).await;
        return Err(CoreError::new(
            ErrorKind::Remote,
            format!(
                "После изменения вход по ключу не работает ({}), изменения откатаны",
                e.message
            ),
        ));
    }
    steps.push("Новое подключение по ключу успешно".to_owned());
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct PasswordLoginReport {
    pub steps: Vec<String>,
    /// Methods the server offers now (checked with a new connection).
    pub methods: Vec<String>,
    /// Someone can log in without a key (password or keyboard-interactive).
    pub password_login: bool,
}

/// Forbids (`allow == false`) or allows password logins on a server, connecting with
/// the server's key. The result is checked against what the server really offers.
pub async fn set_password_login(
    target: &Target,
    key_path: &std::path::Path,
    allow: bool,
    ctx: &SshContext,
) -> CoreResult<PasswordLoginReport> {
    let handle = ssh::connect(target, Auth::Key(key_path), ctx).await?;
    let result = async {
        let (prefix, note) = detect_privileges(&handle).await;
        let Some(p) = prefix else {
            return Err(CoreError::new(
                ErrorKind::PermissionDenied,
                format!("{note}. {}", manual_instructions()),
            ));
        };
        let mut steps = vec![note];
        let check = DropInCheck {
            target,
            key_path,
            ctx,
        };
        if allow {
            // Our own file is the usual reason: removing it is the smallest change.
            let ours = ssh::exec(&handle, &format!("{p}test -e {DROP_IN_PATH}"), None).await?;
            if ours.exit_code == Some(0) {
                apply_drop_in(&handle, &p, None, &check, &mut steps).await?;
            }
            let methods = ssh::auth_methods(target, ctx).await?;
            if !ssh::allows_password(&methods) {
                apply_drop_in(&handle, &p, Some(ALLOW_PASSWORD), &check, &mut steps).await?;
            }
        } else {
            apply_drop_in(&handle, &p, Some(KEYS_ONLY), &check, &mut steps).await?;
        }
        let methods = ssh::auth_methods(target, ctx).await?;
        let password_login = ssh::allows_password(&methods);
        if password_login != allow {
            steps.push(
                "Сервер всё ещё ведёт себя иначе: настройку переопределяет другой файл или Match-блок sshd_config"
                    .to_owned(),
            );
        }
        Ok(PasswordLoginReport {
            steps,
            methods,
            password_login,
        })
    }
    .await;
    ssh::disconnect(&handle).await;
    result
}

async fn verify_key_login(
    target: &Target,
    key_path: &std::path::Path,
    ctx: &SshContext,
) -> CoreResult<()> {
    let handle = ssh::connect(target, Auth::Key(key_path), ctx).await?;
    ssh::disconnect(&handle).await;
    Ok(())
}

/// Returns the command prefix for root actions (`""` for root, `"sudo -n "` with
/// passwordless sudo) or `None` with an explanation.
async fn detect_privileges(handle: &Handle<Client>) -> (Option<String>, String) {
    if let Ok(o) = ssh::exec(handle, "id -u", None).await {
        if o.stdout.trim() == "0" {
            return (Some(String::new()), "Пользователь root".into());
        }
    }
    if let Ok(o) = ssh::exec(handle, "sudo -n true", None).await {
        if o.exit_code == Some(0) {
            return (Some("sudo -n ".into()), "Есть sudo без пароля".into());
        }
    }
    (
        None,
        "Нет прав root и sudo без пароля: вход по паролю можно отключить только вручную".into(),
    )
}

pub fn manual_instructions() -> String {
    format!(
        "Вручную: от root создайте {DROP_IN_PATH} со строкой «PasswordAuthentication no» \
         (или «yes», чтобы разрешить пароль), выполните «sshd -t», затем «systemctl reload ssh» \
         и, не закрывая текущую сессию, проверьте вход по ключу в новом окне."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_script_has_no_inner_single_quotes() {
        // The whole script is a single-quoted `sh -c` argument.
        let inner = INSTALL_SCRIPT
            .strip_prefix("sh -c '")
            .and_then(|s| s.strip_suffix('\''))
            .unwrap();
        assert!(!inner.contains('\''));
    }
}
