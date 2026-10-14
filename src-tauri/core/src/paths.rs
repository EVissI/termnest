use std::path::PathBuf;

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

/// First existing private key among the OpenSSH default names.
pub fn default_key_path() -> Option<PathBuf> {
    let dir = ssh_dir()?;
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}
