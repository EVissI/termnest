//! termnest's own known_hosts file (OpenSSH format).

use std::path::PathBuf;
use std::sync::Mutex;

use russh::keys::{HashAlg, PublicKey};
use serde::Serialize;

use crate::error::{CoreError, CoreResult};
use crate::paths::atomic_write;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    Trusted,
    Unknown,
    /// A different key of the same type is stored for this host.
    Changed {
        known_fingerprint: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct KnownHostEntry {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
}

pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Host field as written in known_hosts: `host` for port 22, `[host]:port` otherwise.
fn host_pattern(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_ascii_lowercase()
    } else {
        format!("[{}]:{}", host.to_ascii_lowercase(), port)
    }
}

fn parse_host_pattern(pattern: &str) -> Option<(String, u16)> {
    if let Some(rest) = pattern.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        Some((host.to_ascii_lowercase(), port.parse().ok()?))
    } else {
        Some((pattern.to_ascii_lowercase(), 22))
    }
}

struct Line {
    raw: String,
    parsed: Option<ParsedLine>,
}

struct ParsedLine {
    hosts: Vec<(String, u16)>,
    key: PublicKey,
}

fn parse_line(raw: &str) -> Option<ParsedLine> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
        return None;
    }
    let mut parts = line.split_whitespace();
    let hosts_field = parts.next()?;
    if hosts_field.starts_with('|') {
        // Hashed entries are not written by termnest; ignore them.
        return None;
    }
    let key_type = parts.next()?;
    let key_data = parts.next()?;
    let key = PublicKey::from_openssh(&format!("{key_type} {key_data}")).ok()?;
    let hosts = hosts_field
        .split(',')
        .filter_map(parse_host_pattern)
        .collect();
    Some(ParsedLine { hosts, key })
}

pub struct KnownHosts {
    path: PathBuf,
    lines: Mutex<Vec<Line>>,
}

impl KnownHosts {
    /// Loads the file; a missing file is an empty store.
    pub fn load(path: PathBuf) -> CoreResult<Self> {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(CoreError::storage("Не удалось прочитать known_hosts", e)),
        };
        let lines = text
            .lines()
            .map(|raw| Line {
                raw: raw.to_owned(),
                parsed: parse_line(raw),
            })
            .collect();
        Ok(Self {
            path,
            lines: Mutex::new(lines),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Line>> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> HostKeyStatus {
        let wanted = (host.to_ascii_lowercase(), port);
        let mut changed = None;
        for line in self.lock().iter() {
            let Some(p) = &line.parsed else { continue };
            if !p.hosts.contains(&wanted) {
                continue;
            }
            if p.key.key_data() == key.key_data() {
                return HostKeyStatus::Trusted;
            }
            if p.key.algorithm() == key.algorithm() && changed.is_none() {
                changed = Some(fingerprint(&p.key));
            }
        }
        match changed {
            Some(known_fingerprint) => HostKeyStatus::Changed { known_fingerprint },
            None => HostKeyStatus::Unknown,
        }
    }

    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> CoreResult<()> {
        let encoded = key
            .to_openssh()
            .map_err(|e| CoreError::storage("Не удалось закодировать ключ хоста", e))?;
        // Drop the comment if any: "type base64 comment" -> "type base64".
        let mut fields = encoded.split_whitespace();
        let key_text = match (fields.next(), fields.next()) {
            (Some(t), Some(d)) => format!("{t} {d}"),
            _ => encoded.clone(),
        };
        let raw = format!("{} {}", host_pattern(host, port), key_text);
        {
            let mut lines = self.lock();
            lines.push(Line {
                parsed: parse_line(&raw),
                raw,
            });
        }
        self.save()
    }

    /// Removes all entries for the host (only that host is dropped from multi-host lines).
    pub fn remove(&self, host: &str, port: u16) -> CoreResult<usize> {
        let wanted = (host.to_ascii_lowercase(), port);
        let mut removed = 0;
        {
            let mut lines = self.lock();
            let mut kept = Vec::with_capacity(lines.len());
            for line in lines.drain(..) {
                let Some(p) = &line.parsed else {
                    kept.push(line);
                    continue;
                };
                if !p.hosts.contains(&wanted) {
                    kept.push(line);
                    continue;
                }
                removed += 1;
                let others: Vec<_> = p.hosts.iter().filter(|h| **h != wanted).collect();
                if !others.is_empty() {
                    let mut fields = line.raw.split_whitespace().skip(1);
                    let rest: Vec<&str> = fields.by_ref().collect();
                    let hosts: Vec<String> =
                        others.iter().map(|(h, p)| host_pattern(h, *p)).collect();
                    let raw = format!("{} {}", hosts.join(","), rest.join(" "));
                    kept.push(Line {
                        parsed: parse_line(&raw),
                        raw,
                    });
                }
            }
            *lines = kept;
        }
        if removed > 0 {
            self.save()?;
        }
        Ok(removed)
    }

    pub fn entries(&self) -> Vec<KnownHostEntry> {
        let mut out = Vec::new();
        for line in self.lock().iter() {
            let Some(p) = &line.parsed else { continue };
            for (host, port) in &p.hosts {
                out.push(KnownHostEntry {
                    host: host.clone(),
                    port: *port,
                    key_type: p.key.algorithm().to_string(),
                    fingerprint: fingerprint(&p.key),
                });
            }
        }
        out
    }

    fn save(&self) -> CoreResult<()> {
        let mut text = String::new();
        for line in self.lock().iter() {
            text.push_str(&line.raw);
            text.push('\n');
        }
        atomic_write(&self.path, text.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить known_hosts", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_dir::TestDir;
    use russh::keys::{Algorithm, PrivateKey};

    fn key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }

    #[test]
    fn unknown_then_trusted_then_changed() {
        let dir = TestDir::new();
        let path = dir.path().join("known_hosts");
        let kh = KnownHosts::load(path.clone()).unwrap();
        let k1 = key();
        let k2 = key();

        assert_eq!(kh.check("Example.com", 22, &k1), HostKeyStatus::Unknown);
        kh.add("Example.com", 22, &k1).unwrap();
        assert_eq!(kh.check("example.com", 22, &k1), HostKeyStatus::Trusted);
        assert_eq!(kh.check("example.com", 2222, &k1), HostKeyStatus::Unknown);
        assert!(matches!(
            kh.check("example.com", 22, &k2),
            HostKeyStatus::Changed { .. }
        ));

        // Persisted in OpenSSH format and reloadable.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("example.com ssh-ed25519 "));
        let reloaded = KnownHosts::load(path).unwrap();
        assert_eq!(
            reloaded.check("example.com", 22, &k1),
            HostKeyStatus::Trusted
        );
    }

    #[test]
    fn non_default_port_and_remove() {
        let dir = TestDir::new();
        let kh = KnownHosts::load(dir.path().join("kh")).unwrap();
        let k = key();
        kh.add("10.0.0.1", 2222, &k).unwrap();
        kh.add("other", 22, &k).unwrap();
        assert_eq!(kh.entries()[0].port, 2222);
        assert_eq!(kh.remove("10.0.0.1", 2222).unwrap(), 1);
        assert_eq!(kh.check("10.0.0.1", 2222, &k), HostKeyStatus::Unknown);
        assert_eq!(kh.check("other", 22, &k), HostKeyStatus::Trusted);
    }

    #[test]
    fn keeps_unrelated_lines_and_splits_multi_host_lines() {
        let dir = TestDir::new();
        let path = dir.path().join("kh");
        let k = key();
        let encoded = k.to_openssh().unwrap();
        std::fs::write(
            &path,
            format!("# comment\n|1|hashed|entry ssh-ed25519 AAAA\na,b {encoded}\n"),
        )
        .unwrap();
        let kh = KnownHosts::load(path.clone()).unwrap();
        assert_eq!(kh.check("b", 22, &k), HostKeyStatus::Trusted);
        kh.remove("a", 22).unwrap();
        assert_eq!(kh.check("a", 22, &k), HostKeyStatus::Unknown);
        assert_eq!(kh.check("b", 22, &k), HostKeyStatus::Trusted);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# comment"));
        assert!(text.contains("|1|hashed"));
    }
}
