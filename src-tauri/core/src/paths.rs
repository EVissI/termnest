use std::path::{Path, PathBuf};

/// Home directory of the current user (`%USERPROFILE%` on Windows, `$HOME` elsewhere).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The user's `.ssh` directory.
pub fn ssh_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".ssh"))
}

/// Application data directory: `%APPDATA%\termnest` (or `~/.config/termnest` elsewhere).
pub fn app_dir() -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(appdata).join("termnest"));
    }
    home_dir().map(|h| h.join(".config").join("termnest"))
}

/// First existing private key among the OpenSSH default names.
pub fn default_key_path() -> Option<PathBuf> {
    let dir = ssh_dir()?;
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

/// Expands a leading `~` to the home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~") {
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
            if let Some(home) = home_dir() {
                return home.join(rest.trim_start_matches(['/', '\\']));
            }
        }
    }
    PathBuf::from(path)
}

/// Writes a file atomically: data goes to a temporary file in the same directory
/// which then replaces the target, so a crash never leaves a half-written config.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(".{file_name}.tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
pub(crate) mod test_dir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Temporary directory removed on drop.
    pub struct TestDir(PathBuf);

    impl TestDir {
        pub fn new() -> Self {
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "termnest-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TestDir(p)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_dir::TestDir;
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces() {
        let dir = TestDir::new();
        let file = dir.path().join("sub").join("a.toml");
        atomic_write(&file, b"one").unwrap();
        atomic_write(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(file.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn tilde_expansion() {
        assert_eq!(expand_tilde("/etc/x"), PathBuf::from("/etc/x"));
        assert_eq!(expand_tilde("~user/x"), PathBuf::from("~user/x"));
        if let Some(home) = home_dir() {
            assert_eq!(expand_tilde("~/.ssh/id"), home.join(".ssh/id"));
        }
    }
}
