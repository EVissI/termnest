//! Local SSH keys: listing, reading public keys, generation.

use std::path::{Path, PathBuf};

use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::secret::SecretString;

#[derive(Debug, Clone, Serialize)]
pub struct KeyInfo {
    pub path: String,
    pub name: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub comment: String,
    pub encrypted: bool,
    /// Added manually (outside `~/.ssh`).
    pub manual: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Ed25519,
    Rsa4096,
}

const NOT_KEYS: [&str; 5] = [
    "known_hosts",
    "known_hosts.old",
    "config",
    "authorized_keys",
    "authorized_keys2",
];

/// What we could learn about a private key file without its passphrase.
struct Inspected {
    public: Option<PublicKey>,
    encrypted: bool,
}

fn inspect(path: &Path) -> Option<Inspected> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if !text.contains("PRIVATE KEY-----") {
        return None;
    }
    // OpenSSH format keeps the public key unencrypted.
    if let Ok(k) = PrivateKey::from_openssh(&text) {
        return Some(Inspected {
            public: Some(k.public_key().clone()),
            encrypted: k.is_encrypted(),
        });
    }
    // Legacy PEM formats.
    match russh::keys::decode_secret_key(&text, None) {
        Ok(k) => Some(Inspected {
            public: Some(k.public_key().clone()),
            encrypted: false,
        }),
        Err(russh::keys::Error::KeyIsEncrypted) => Some(Inspected {
            public: read_pub_file(path),
            encrypted: true,
        }),
        Err(_) => None,
    }
}

fn pub_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".pub");
    PathBuf::from(s)
}

fn read_pub_file(path: &Path) -> Option<PublicKey> {
    let text = std::fs::read_to_string(pub_path(path)).ok()?;
    PublicKey::from_openssh(text.trim()).ok()
}

fn info(path: &Path, manual: bool) -> Option<KeyInfo> {
    let inspected = inspect(path)?;
    let public = inspected.public.or_else(|| read_pub_file(path));
    let (algorithm, fingerprint, comment) = match &public {
        Some(p) => (
            p.algorithm().to_string(),
            p.fingerprint(HashAlg::Sha256).to_string(),
            p.comment().to_string(),
        ),
        None => ("?".into(), "неизвестен (нет .pub)".into(), String::new()),
    };
    Some(KeyInfo {
        path: path.display().to_string(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        algorithm,
        fingerprint,
        comment,
        encrypted: inspected.encrypted,
        manual,
    })
}

/// Keys from `~/.ssh` plus manually added ones.
pub fn list(extra: &[String]) -> Vec<KeyInfo> {
    let mut out = Vec::new();
    if let Some(dir) = crate::paths::ssh_dir() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut paths: Vec<PathBuf> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    !name.ends_with(".pub") && !NOT_KEYS.contains(&name.as_str())
                })
                .collect();
            paths.sort();
            out.extend(paths.iter().filter_map(|p| info(p, false)));
        }
    }
    for e in extra {
        let p = PathBuf::from(e);
        if out.iter().any(|k| Path::new(&k.path) == p) {
            continue;
        }
        if let Some(k) = info(&p, true) {
            out.push(k);
        }
    }
    out
}

/// Checks that the file is a private key we can use; returns its info.
pub fn validate(path: &Path) -> CoreResult<KeyInfo> {
    if !path.is_file() {
        return Err(CoreError::new(
            ErrorKind::KeyNotFound,
            format!("Файл не найден: {}", path.display()),
        ));
    }
    info(path, true)
        .ok_or_else(|| CoreError::new(ErrorKind::KeyInvalid, "Файл не похож на приватный SSH-ключ"))
}

/// Public key in `authorized_keys` format. Uses the `.pub` file or the key itself.
pub fn public_key_line(path: &Path) -> CoreResult<String> {
    let public = inspect(path)
        .and_then(|i| i.public)
        .or_else(|| read_pub_file(path))
        .ok_or_else(|| {
            CoreError::new(
                ErrorKind::KeyInvalid,
                format!("Не удалось получить публичный ключ для {}", path.display()),
            )
        })?;
    public.to_openssh().map_err(|e| {
        CoreError::new(
            ErrorKind::KeyInvalid,
            format!("Ошибка кодирования ключа: {e}"),
        )
    })
}

/// Generates a key pair and writes it in OpenSSH format. Existing files are only
/// replaced when `overwrite` is set.
pub fn generate(
    path: &Path,
    kind: KeyKind,
    comment: &str,
    passphrase: Option<&SecretString>,
    overwrite: bool,
) -> CoreResult<KeyInfo> {
    let public_path = pub_path(path);
    if !overwrite && (path.exists() || public_path.exists()) {
        return Err(CoreError::new(
            ErrorKind::AlreadyExists,
            format!("Файл {} уже существует", path.display()),
        ));
    }
    let mut rng = rand::rng();
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519),
        KeyKind::Rsa4096 => RsaKeypair::random(&mut rng, 4096)
            .and_then(|kp| PrivateKey::new(KeypairData::from(kp), comment)),
    }
    .map_err(|e| {
        CoreError::new(
            ErrorKind::KeyInvalid,
            format!("Не удалось сгенерировать ключ: {e}"),
        )
    })?;
    key.set_comment(comment);
    let public_line = key
        .public_key()
        .to_openssh()
        .map_err(|e| CoreError::new(ErrorKind::KeyInvalid, e.to_string()))?;

    if let Some(p) = passphrase.filter(|p| !p.is_empty()) {
        key = key.encrypt(&mut rng, p.expose()).map_err(|e| {
            CoreError::new(
                ErrorKind::KeyInvalid,
                format!("Не удалось зашифровать ключ: {e}"),
            )
        })?;
    }
    let private_text = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| CoreError::new(ErrorKind::KeyInvalid, e.to_string()))?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| CoreError::storage("Не удалось создать папку", e))?;
    }
    write_private(path, private_text.as_bytes())?;
    crate::paths::atomic_write(&public_path, format!("{public_line}\n").as_bytes())
        .map_err(|e| CoreError::storage("Не удалось записать публичный ключ", e))?;
    validate(path)
}

/// Writes the private key so that it is never readable by other users:
/// the temporary file gets restricted permissions before it is moved in place.
fn write_private(path: &Path, data: &[u8]) -> CoreResult<()> {
    use std::io::Write;
    let tmp = {
        let mut s = path.as_os_str().to_owned();
        s.push(".tmp");
        PathBuf::from(s)
    };
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        restrict_to_owner(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CoreError::storage("Не удалось записать приватный ключ", e)
    })
}

/// Removes inherited permissions and grants access only to the current user.
#[cfg(windows)]
pub fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let user =
        std::env::var("USERNAME").map_err(|_| std::io::Error::other("USERNAME is not set"))?;
    let domain = std::env::var("USERDOMAIN").ok();
    let account = match domain {
        Some(d) if !d.is_empty() => format!("{d}\\{user}"),
        _ => user,
    };
    let status = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("{account}:F"))
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("icacls failed to set permissions"))
    }
}

#[cfg(not(windows))]
pub fn restrict_to_owner(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_dir::TestDir;

    #[test]
    fn generate_inspect_and_refuse_overwrite() {
        let dir = TestDir::new();
        let path = dir.path().join("id_test");
        let k = generate(&path, KeyKind::Ed25519, "me@test", None, false).unwrap();
        assert_eq!(k.algorithm, "ssh-ed25519");
        assert!(k.fingerprint.starts_with("SHA256:"));
        assert!(!k.encrypted);
        assert_eq!(k.comment, "me@test");

        let line = public_key_line(&path).unwrap();
        assert!(line.starts_with("ssh-ed25519 "));
        let pub_file = std::fs::read_to_string(pub_path(&path)).unwrap();
        assert_eq!(pub_file.trim(), line);

        let err = generate(&path, KeyKind::Ed25519, "", None, false).unwrap_err();
        assert_eq!(err.kind, ErrorKind::AlreadyExists);
        let again = generate(&path, KeyKind::Ed25519, "", None, true).unwrap();
        assert_ne!(again.fingerprint, k.fingerprint);
    }

    #[test]
    fn encrypted_key_is_detected_and_loadable() {
        let dir = TestDir::new();
        let path = dir.path().join("id_enc");
        let pass = SecretString::new("correct horse".into());
        let k = generate(&path, KeyKind::Ed25519, "", Some(&pass), false).unwrap();
        assert!(k.encrypted);
        assert!(
            public_key_line(&path).is_ok(),
            "public part readable without passphrase"
        );
        assert!(matches!(
            russh::keys::load_secret_key(&path, None),
            Err(russh::keys::Error::KeyIsEncrypted)
        ));
        russh::keys::load_secret_key(&path, Some("correct horse")).unwrap();
    }

    #[test]
    fn non_keys_are_rejected() {
        let dir = TestDir::new();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hello").unwrap();
        assert_eq!(validate(&path).unwrap_err().kind, ErrorKind::KeyInvalid);
    }
}
