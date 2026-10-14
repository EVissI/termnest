//! Saved servers and groups (`hosts.toml`).

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::ids::new_id;
use crate::paths::atomic_write;
use crate::ssh_config::SshConfigHost;
use crate::target::Target;

fn default_port() -> u16 {
    22
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl Host {
    pub fn target(&self) -> Target {
        Target {
            user: self.user.clone(),
            host: self.host.clone(),
            port: self.port,
        }
    }

    fn normalize(&mut self) -> CoreResult<()> {
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.user = self.user.trim().to_owned();
        self.note = self.note.trim().to_owned();
        self.key_path = self
            .key_path
            .take()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty());
        self.group = self
            .group
            .take()
            .map(|g| g.trim().to_owned())
            .filter(|g| !g.is_empty());
        if self.host.is_empty() || self.host.chars().any(char::is_whitespace) {
            return Err(CoreError::invalid("Укажите адрес сервера без пробелов"));
        }
        if self.user.is_empty() || self.user.chars().any(char::is_whitespace) {
            return Err(CoreError::invalid("Укажите пользователя"));
        }
        if self.port == 0 {
            return Err(CoreError::invalid("Порт должен быть от 1 до 65535"));
        }
        if self.name.is_empty() {
            self.name = self.host.clone();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostsFile {
    #[serde(default, rename = "group")]
    pub groups: Vec<Group>,
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
}

pub struct HostStore {
    path: PathBuf,
    data: Mutex<HostsFile>,
}

impl HostStore {
    pub fn load(path: PathBuf) -> CoreResult<Self> {
        let mut data: HostsFile = match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).map_err(|e| CoreError::storage("Ошибка в hosts.toml", e))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HostsFile::default(),
            Err(e) => return Err(CoreError::storage("Не удалось прочитать hosts.toml", e)),
        };
        for h in &mut data.hosts {
            if h.id.is_empty() {
                h.id = new_id();
            }
        }
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HostsFile> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn save(&self, data: &HostsFile) -> CoreResult<()> {
        let text = toml::to_string_pretty(data)
            .map_err(|e| CoreError::storage("Не удалось сериализовать hosts.toml", e))?;
        atomic_write(&self.path, text.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить hosts.toml", e))
    }

    /// Applies a change and saves; on a save error the in-memory state is rolled back.
    fn modify<T>(&self, f: impl FnOnce(&mut HostsFile) -> CoreResult<T>) -> CoreResult<T> {
        let mut data = self.lock();
        let mut next = data.clone();
        let result = f(&mut next)?;
        // Every group referenced by a host must exist.
        for h in &next.hosts {
            if let Some(g) = &h.group {
                if !next.groups.iter().any(|x| &x.name == g) {
                    next.groups.push(Group {
                        name: g.clone(),
                        collapsed: false,
                    });
                }
            }
        }
        self.save(&next)?;
        *data = next;
        Ok(result)
    }

    pub fn snapshot(&self) -> HostsFile {
        self.lock().clone()
    }

    pub fn get(&self, id: &str) -> Option<Host> {
        self.lock().hosts.iter().find(|h| h.id == id).cloned()
    }

    /// Creates (empty id) or updates a host. Returns the stored host.
    pub fn upsert(&self, mut host: Host) -> CoreResult<Host> {
        host.normalize()?;
        self.modify(|data| {
            if host.id.is_empty() {
                host.id = new_id();
                data.hosts.push(host.clone());
            } else {
                let slot = data
                    .hosts
                    .iter_mut()
                    .find(|h| h.id == host.id)
                    .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))?;
                *slot = host.clone();
            }
            Ok(host)
        })
    }

    pub fn duplicate(&self, id: &str) -> CoreResult<Host> {
        self.modify(|data| {
            let pos = data
                .hosts
                .iter()
                .position(|h| h.id == id)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))?;
            let mut copy = data.hosts[pos].clone();
            copy.id = new_id();
            copy.name = format!("{} (копия)", copy.name);
            data.hosts.insert(pos + 1, copy.clone());
            Ok(copy)
        })
    }

    pub fn delete(&self, id: &str) -> CoreResult<()> {
        self.modify(|data| {
            data.hosts.retain(|h| h.id != id);
            Ok(())
        })
    }

    pub fn add_group(&self, name: &str) -> CoreResult<()> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(CoreError::invalid("Пустое имя группы"));
        }
        self.modify(|data| {
            if !data.groups.iter().any(|g| g.name == name) {
                data.groups.push(Group {
                    name,
                    collapsed: false,
                });
            }
            Ok(())
        })
    }

    pub fn rename_group(&self, old: &str, new: &str) -> CoreResult<()> {
        let new = new.trim().to_owned();
        if new.is_empty() {
            return Err(CoreError::invalid("Пустое имя группы"));
        }
        self.modify(|data| {
            if data.groups.iter().any(|g| g.name == new) && old != new {
                return Err(CoreError::new(
                    ErrorKind::AlreadyExists,
                    "Группа с таким именем уже есть",
                ));
            }
            for g in data.groups.iter_mut().filter(|g| g.name == old) {
                g.name = new.clone();
            }
            for h in data.hosts.iter_mut() {
                if h.group.as_deref() == Some(old) {
                    h.group = Some(new.clone());
                }
            }
            Ok(())
        })
    }

    /// Deletes the group; its hosts become ungrouped.
    pub fn delete_group(&self, name: &str) -> CoreResult<()> {
        self.modify(|data| {
            data.groups.retain(|g| g.name != name);
            for h in data.hosts.iter_mut() {
                if h.group.as_deref() == Some(name) {
                    h.group = None;
                }
            }
            Ok(())
        })
    }

    pub fn set_group_collapsed(&self, name: &str, collapsed: bool) -> CoreResult<()> {
        self.modify(|data| {
            for g in data.groups.iter_mut().filter(|g| g.name == name) {
                g.collapsed = collapsed;
            }
            Ok(())
        })
    }

    /// Imports hosts from `~/.ssh/config` entries, skipping ones that already exist
    /// (same host, port and user). Returns the number of added hosts.
    pub fn import(
        &self,
        entries: &[SshConfigHost],
        group: &str,
        default_user: &str,
    ) -> CoreResult<usize> {
        self.modify(|data| {
            let mut added = 0;
            for e in entries {
                let user = e.user.clone().unwrap_or_else(|| default_user.to_owned());
                let port = e.port.unwrap_or(22);
                let exists = data
                    .hosts
                    .iter()
                    .any(|h| h.host == e.hostname && h.port == port && h.user == user);
                if exists || user.is_empty() {
                    continue;
                }
                data.hosts.push(Host {
                    id: new_id(),
                    name: e.alias.clone(),
                    host: e.hostname.clone(),
                    port,
                    user,
                    key_path: e.identity_file.clone(),
                    group: Some(group.to_owned()),
                    note: String::new(),
                });
                added += 1;
            }
            Ok(added)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_dir::TestDir;

    fn host(name: &str) -> Host {
        Host {
            id: String::new(),
            name: name.into(),
            host: format!("{name}.example.com"),
            port: 22,
            user: "root".into(),
            key_path: Some(" ".into()),
            group: Some("prod".into()),
            note: String::new(),
        }
    }

    #[test]
    fn crud_and_persistence() {
        let dir = TestDir::new();
        let path = dir.path().join("hosts.toml");
        let store = HostStore::load(path.clone()).unwrap();

        let a = store.upsert(host("a")).unwrap();
        assert!(!a.id.is_empty());
        assert_eq!(a.key_path, None, "blank key path is dropped");
        let b = store.duplicate(&a.id).unwrap();
        assert_eq!(b.name, "a (копия)");

        let mut edited = a.clone();
        edited.port = 2222;
        store.upsert(edited).unwrap();

        let reloaded = HostStore::load(path).unwrap().snapshot();
        assert_eq!(reloaded.hosts.len(), 2);
        assert_eq!(reloaded.hosts[0].port, 2222);
        assert_eq!(
            reloaded.groups,
            vec![Group {
                name: "prod".into(),
                collapsed: false
            }]
        );

        store.delete(&b.id).unwrap();
        assert_eq!(store.snapshot().hosts.len(), 1);
    }

    #[test]
    fn validation() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let mut bad = host("x");
        bad.user = " ".into();
        assert_eq!(store.upsert(bad).unwrap_err().kind, ErrorKind::InvalidInput);
        let mut unnamed = host("x");
        unnamed.name = "".into();
        assert_eq!(store.upsert(unnamed).unwrap().name, "x.example.com");
    }

    #[test]
    fn groups() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let a = store.upsert(host("a")).unwrap();
        store.rename_group("prod", "production").unwrap();
        assert_eq!(
            store.get(&a.id).unwrap().group.as_deref(),
            Some("production")
        );
        store.set_group_collapsed("production", true).unwrap();
        assert!(store.snapshot().groups[0].collapsed);
        store.delete_group("production").unwrap();
        assert_eq!(store.get(&a.id).unwrap().group, None);
        assert!(store.snapshot().groups.is_empty());
    }

    #[test]
    fn import_skips_duplicates() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let entries = crate::ssh_config::parse("Host one\n HostName 1.1.1.1\n User u\nHost two\n");
        assert_eq!(store.import(&entries, "ssh config", "me").unwrap(), 2);
        assert_eq!(store.import(&entries, "ssh config", "me").unwrap(), 0);
        let two = store
            .snapshot()
            .hosts
            .into_iter()
            .find(|h| h.name == "two")
            .unwrap();
        assert_eq!(two.user, "me");
        assert_eq!(two.host, "two");
    }
}
