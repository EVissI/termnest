use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Deserializer};
use zeroize::Zeroizing;

/// A password or passphrase. The memory is wiped on drop; it can be deserialized
/// from the UI but never serialized or printed.
#[derive(Clone, Default)]
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(SecretString::new(s))
    }
}

/// Passphrases the user chose to remember, kept in memory until the app exits.
#[derive(Default)]
pub struct PassphraseCache {
    entries: Mutex<HashMap<PathBuf, SecretString>>,
}

impl PassphraseCache {
    pub fn get(&self, key: &Path) -> Option<SecretString> {
        self.lock().get(key).cloned()
    }

    pub fn insert(&self, key: &Path, passphrase: SecretString) {
        self.lock().insert(key.to_path_buf(), passphrase);
    }

    pub fn remove(&self, key: &Path) {
        self.lock().remove(key);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PathBuf, SecretString>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak() {
        let s = SecretString::new("hunter2".into());
        assert!(!format!("{s:?}").contains("hunter2"));
    }

    #[test]
    fn deserializes_from_json_string() {
        let s: SecretString = serde_json::from_str("\"pw\"").unwrap();
        assert_eq!(s.expose(), "pw");
    }

    #[test]
    fn cache_roundtrip() {
        let c = PassphraseCache::default();
        let p = Path::new("k");
        assert!(c.get(p).is_none());
        c.insert(p, SecretString::new("x".into()));
        assert_eq!(c.get(p).unwrap().expose(), "x");
        c.remove(p);
        assert!(c.get(p).is_none());
    }
}
