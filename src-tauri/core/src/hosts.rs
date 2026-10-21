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

/// How to become another user after logging in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwitchMethod {
    /// `sudo -i` / `sudo -iu NAME` (asks for the login user's password if needed).
    #[default]
    Sudo,
    /// `su -` / `su - NAME` (asks for the target user's password).
    Su,
}

impl SwitchMethod {
    fn is_default(&self) -> bool {
        *self == Self::Sudo
    }

    /// Shell command that opens a login shell of `user`.
    pub fn command(self, user: &str) -> String {
        match (self, user == "root") {
            (Self::Sudo, true) => "sudo -i".to_owned(),
            (Self::Sudo, false) => format!("sudo -iu {user}"),
            (Self::Su, true) => "su -".to_owned(),
            (Self::Su, false) => format!("su - {user}"),
        }
    }
}

/// A Unix user name safe to put into a shell command.
pub fn valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
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
    /// Local project folder this server belongs to (Claude Code integration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// User to become right after login (`user` only logs in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switch_user: Option<String>,
    #[serde(default, skip_serializing_if = "SwitchMethod::is_default")]
    pub switch_method: SwitchMethod,
    /// Extra users offered in the panel's quick switch menu.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick_users: Vec<String>,
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
        self.project = crate::project::normalize(self.project.take());
        self.switch_user = self
            .switch_user
            .take()
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty());
        let mut quick: Vec<String> = Vec::new();
        for u in self.quick_users.drain(..) {
            let u = u.trim().to_owned();
            if !u.is_empty() && !quick.contains(&u) {
                quick.push(u);
            }
        }
        self.quick_users = quick;
        for u in self.switch_user.iter().chain(&self.quick_users) {
            if !valid_user_name(u) {
                return Err(CoreError::invalid(format!(
                    "Некорректное имя пользователя: {u}"
                )));
            }
        }
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
        // Becoming the login user itself is a no-op.
        if self.switch_user.as_deref() == Some(self.user.as_str()) {
            self.switch_user = None;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    /// Enclosing group (subgroups); `None` for a top-level group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
    /// Default project for the group's servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

impl Group {
    pub fn new(name: String, parent: Option<String>) -> Self {
        Self {
            name,
            parent,
            collapsed: false,
            project: None,
        }
    }
}

/// Whether `name` is `ancestor` itself or lies inside it (at any depth).
fn is_within(groups: &[Group], name: &str, ancestor: &str) -> bool {
    let mut current = Some(name.to_owned());
    // Bounded by the group count, so a broken file cannot loop forever.
    for _ in 0..=groups.len() {
        let Some(c) = current else { return false };
        if c == ancestor {
            return true;
        }
        current = groups
            .iter()
            .find(|g| g.name == c)
            .and_then(|g| g.parent.clone());
    }
    false
}

/// Drops parents that do not exist or form a cycle, so the groups always make a tree.
fn fix_tree(data: &mut HostsFile) {
    for i in 0..data.groups.len() {
        let Some(parent) = data.groups[i].parent.clone() else {
            continue;
        };
        let name = data.groups[i].name.clone();
        let exists = data.groups.iter().any(|g| g.name == parent);
        if !exists || is_within(&data.groups, &parent, &name) {
            data.groups[i].parent = None;
        }
    }
}

/// The group `name` and its enclosing groups, innermost first.
pub fn group_chain<'a>(groups: &'a [Group], name: &str) -> Vec<&'a Group> {
    let mut chain: Vec<&Group> = Vec::new();
    let mut current = Some(name);
    while let Some(c) = current {
        let Some(g) = groups.iter().find(|g| g.name == c) else {
            break;
        };
        if chain.iter().any(|x| x.name == g.name) {
            break;
        }
        chain.push(g);
        current = g.parent.as_deref();
    }
    chain
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostsFile {
    #[serde(default, rename = "group")]
    pub groups: Vec<Group>,
    #[serde(default, rename = "host")]
    pub hosts: Vec<Host>,
}

/// Hosts and groups as sent to the UI (the TOML file uses singular table names).
#[derive(Debug, Clone, Serialize)]
pub struct HostsList {
    pub groups: Vec<Group>,
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
        fix_tree(&mut data);
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
                    next.groups.push(Group::new(g.clone(), None));
                }
            }
        }
        fix_tree(&mut next);
        self.save(&next)?;
        *data = next;
        Ok(result)
    }

    pub fn snapshot(&self) -> HostsFile {
        self.lock().clone()
    }

    pub fn list(&self) -> HostsList {
        let data = self.lock();
        HostsList {
            groups: data.groups.clone(),
            hosts: data.hosts.clone(),
        }
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

    /// Creates a group, optionally inside `parent`.
    pub fn add_group(&self, name: &str, parent: Option<&str>) -> CoreResult<()> {
        let name = name.trim().to_owned();
        if name.is_empty() {
            return Err(CoreError::invalid("Пустое имя группы"));
        }
        self.modify(|data| {
            if data.groups.iter().any(|g| g.name == name) {
                return Err(CoreError::new(
                    ErrorKind::AlreadyExists,
                    "Группа с таким именем уже есть",
                ));
            }
            if let Some(p) = parent {
                if !data.groups.iter().any(|g| g.name == p) {
                    return Err(CoreError::new(ErrorKind::NotFound, "Группа не найдена"));
                }
            }
            data.groups
                .push(Group::new(name, parent.map(str::to_owned)));
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
            for g in data.groups.iter_mut() {
                if g.name == old {
                    g.name = new.clone();
                }
                if g.parent.as_deref() == Some(old) {
                    g.parent = Some(new.clone());
                }
            }
            for h in data.hosts.iter_mut() {
                if h.group.as_deref() == Some(old) {
                    h.group = Some(new.clone());
                }
            }
            Ok(())
        })
    }

    /// Deletes the group; its servers and subgroups move to the enclosing group.
    pub fn delete_group(&self, name: &str) -> CoreResult<()> {
        self.modify(|data| {
            let parent = data
                .groups
                .iter()
                .find(|g| g.name == name)
                .and_then(|g| g.parent.clone());
            data.groups.retain(|g| g.name != name);
            for g in data.groups.iter_mut() {
                if g.parent.as_deref() == Some(name) {
                    g.parent = parent.clone();
                }
            }
            for h in data.hosts.iter_mut() {
                if h.group.as_deref() == Some(name) {
                    h.group = parent.clone();
                }
            }
            Ok(())
        })
    }

    /// Moves a server into `group` (`None` - no group), placing it before the server
    /// `before` or, without it, after the last server of that group.
    pub fn move_host(&self, id: &str, group: Option<&str>, before: Option<&str>) -> CoreResult<()> {
        self.modify(|data| {
            if let Some(g) = group {
                if !data.groups.iter().any(|x| x.name == g) {
                    return Err(CoreError::new(ErrorKind::NotFound, "Группа не найдена"));
                }
            }
            let pos = data
                .hosts
                .iter()
                .position(|h| h.id == id)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))?;
            let mut host = data.hosts.remove(pos);
            host.group = group.map(str::to_owned);
            let at = before
                .and_then(|b| data.hosts.iter().position(|h| h.id == b))
                .or_else(|| {
                    data.hosts
                        .iter()
                        .rposition(|h| h.group == host.group)
                        .map(|i| i + 1)
                })
                .unwrap_or(data.hosts.len());
            data.hosts.insert(at, host);
            Ok(())
        })
    }

    /// Moves a group into `parent` (`None` - top level), placing it before the group
    /// `before` or, without it, at the end.
    pub fn move_group(
        &self,
        name: &str,
        parent: Option<&str>,
        before: Option<&str>,
    ) -> CoreResult<()> {
        self.modify(|data| {
            if let Some(p) = parent {
                if !data.groups.iter().any(|g| g.name == p) {
                    return Err(CoreError::new(ErrorKind::NotFound, "Группа не найдена"));
                }
                if is_within(&data.groups, p, name) {
                    return Err(CoreError::invalid(
                        "Нельзя переместить группу внутрь самой себя",
                    ));
                }
            }
            let pos = data
                .groups
                .iter()
                .position(|g| g.name == name)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Группа не найдена"))?;
            let mut group = data.groups.remove(pos);
            group.parent = parent.map(str::to_owned);
            let at = before
                .and_then(|b| data.groups.iter().position(|g| g.name == b))
                .unwrap_or(data.groups.len());
            data.groups.insert(at, group);
            Ok(())
        })
    }

    /// Links a server to a project folder (`None` unlinks).
    pub fn set_host_project(&self, id: &str, project: Option<String>) -> CoreResult<Host> {
        let project = crate::project::normalize(project);
        self.modify(|data| {
            let host = data
                .hosts
                .iter_mut()
                .find(|h| h.id == id)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сервер не найден"))?;
            host.project = project;
            Ok(host.clone())
        })
    }

    /// Sets the default project of a group (`None` unlinks).
    pub fn set_group_project(&self, name: &str, project: Option<String>) -> CoreResult<()> {
        let project = crate::project::normalize(project);
        self.modify(|data| {
            let group = data
                .groups
                .iter_mut()
                .find(|g| g.name == name)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Группа не найдена"))?;
            group.project = project;
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
                    project: None,
                    switch_user: None,
                    switch_method: SwitchMethod::Sudo,
                    quick_users: Vec::new(),
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
            project: None,
            switch_user: None,
            switch_method: SwitchMethod::Sudo,
            quick_users: Vec::new(),
        }
    }

    #[test]
    fn switch_user() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let mut h = host("a");
        h.user = "deploy".into();
        h.switch_user = Some(" root ".into());
        h.quick_users = vec!["www-data".into(), " ".into(), "www-data".into()];
        let saved = store.upsert(h.clone()).unwrap();
        assert_eq!(saved.switch_user.as_deref(), Some("root"));
        assert_eq!(saved.quick_users, ["www-data"]);

        h.switch_user = Some("deploy".into());
        assert_eq!(store.upsert(h.clone()).unwrap().switch_user, None);

        h.switch_user = Some("root; rm -rf /".into());
        assert_eq!(store.upsert(h).unwrap_err().kind, ErrorKind::InvalidInput);

        assert_eq!(SwitchMethod::Sudo.command("root"), "sudo -i");
        assert_eq!(SwitchMethod::Sudo.command("app"), "sudo -iu app");
        assert_eq!(SwitchMethod::Su.command("root"), "su -");
        assert_eq!(SwitchMethod::Su.command("app"), "su - app");
        assert!(!valid_user_name("-x"));
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
        assert_eq!(reloaded.groups, vec![Group::new("prod".into(), None)]);

        store.delete(&b.id).unwrap();
        assert_eq!(store.snapshot().hosts.len(), 1);
    }

    #[test]
    fn ui_list_uses_plural_keys() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        store.upsert(host("a")).unwrap();
        let json = serde_json::to_value(store.list()).unwrap();
        assert_eq!(json["hosts"].as_array().unwrap().len(), 1);
        assert_eq!(json["groups"].as_array().unwrap().len(), 1);
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

    fn names(store: &HostStore) -> Vec<(String, Option<String>)> {
        store
            .snapshot()
            .groups
            .into_iter()
            .map(|g| (g.name, g.parent))
            .collect()
    }

    #[test]
    fn subgroups() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let a = store.upsert(host("a")).unwrap();
        store.add_group("web", Some("prod")).unwrap();
        store.add_group("eu", Some("web")).unwrap();
        assert_eq!(
            store.add_group("web", None).unwrap_err().kind,
            ErrorKind::AlreadyExists
        );
        assert_eq!(
            store.add_group("x", Some("missing")).unwrap_err().kind,
            ErrorKind::NotFound
        );

        // A group cannot move into itself or its own subgroup.
        assert!(store.move_group("prod", Some("eu"), None).is_err());
        assert!(store.move_group("web", Some("web"), None).is_err());

        store.rename_group("web", "www").unwrap();
        assert_eq!(names(&store)[2], ("eu".into(), Some("www".into())));

        let chain: Vec<_> = group_chain(&store.snapshot().groups, "eu")
            .into_iter()
            .map(|g| g.name.clone())
            .collect();
        assert_eq!(chain, ["eu", "www", "prod"]);

        // Deleting a group lifts its content one level up.
        store.move_host(&a.id, Some("www"), None).unwrap();
        store.delete_group("www").unwrap();
        assert_eq!(store.get(&a.id).unwrap().group.as_deref(), Some("prod"));
        assert_eq!(names(&store)[1], ("eu".into(), Some("prod".into())));
    }

    #[test]
    fn move_order() {
        let dir = TestDir::new();
        let store = HostStore::load(dir.path().join("h.toml")).unwrap();
        let a = store.upsert(host("a")).unwrap();
        let b = store.upsert(host("b")).unwrap();
        let mut c = host("c");
        c.group = None;
        let c = store.upsert(c).unwrap();
        let order = |s: &HostStore| {
            s.snapshot()
                .hosts
                .into_iter()
                .map(|h| (h.name, h.group))
                .collect::<Vec<_>>()
        };

        store.move_host(&b.id, Some("prod"), Some(&a.id)).unwrap();
        assert_eq!(order(&store)[0].0, "b");
        // Without `before` a server goes after the last one of its new group.
        store.move_host(&c.id, Some("prod"), None).unwrap();
        assert_eq!(
            order(&store),
            vec![
                ("b".into(), Some("prod".into())),
                ("a".into(), Some("prod".into())),
                ("c".into(), Some("prod".into())),
            ]
        );
        store.move_host(&a.id, None, None).unwrap();
        assert_eq!(order(&store)[2], ("a".into(), None));

        store.add_group("dev", None).unwrap();
        store.add_group("test", None).unwrap();
        store.move_group("test", None, Some("prod")).unwrap();
        store.move_group("prod", Some("dev"), None).unwrap();
        assert_eq!(
            names(&store),
            vec![
                ("test".into(), None),
                ("dev".into(), None),
                ("prod".into(), Some("dev".into())),
            ]
        );
    }

    #[test]
    fn broken_tree_is_repaired_on_load() {
        let dir = TestDir::new();
        let path = dir.path().join("h.toml");
        std::fs::write(
            &path,
            "[[group]]\nname = \"a\"\nparent = \"b\"\n[[group]]\nname = \"b\"\nparent = \"a\"\n[[group]]\nname = \"c\"\nparent = \"gone\"\n",
        )
        .unwrap();
        let store = HostStore::load(path).unwrap();
        let groups = names(&store);
        assert_eq!(groups[2], ("c".into(), None));
        assert!(
            groups[0].1.is_none() || groups[1].1.is_none(),
            "cycle broken"
        );
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
