//! Command snippets (`snippets.toml`) with `{{variable}}` placeholders.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult, ErrorKind};
use crate::ids::new_id;
use crate::paths::atomic_write;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Send Enter after the command.
    #[serde(default)]
    pub press_enter: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SnippetsFile {
    #[serde(default, rename = "snippet")]
    snippets: Vec<Snippet>,
}

/// Variable names in order of first appearance: `{{name}}`.
pub fn variables(command: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = after[..end].trim();
        if !name.is_empty() && !name.contains('{') && !out.iter().any(|v| v == name) {
            out.push(name.to_owned());
        }
        rest = &after[end + 2..];
    }
    out
}

/// Substitutes `{{name}}` with values; unknown variables are left as they are.
pub fn render(command: &str, values: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(command.len());
    let mut rest = command;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = after[..end].trim();
                match values.get(name) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 2 + end + 2]),
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

pub struct SnippetStore {
    path: PathBuf,
    data: Mutex<SnippetsFile>,
}

impl SnippetStore {
    pub fn load(path: PathBuf) -> CoreResult<Self> {
        let mut data: SnippetsFile = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|e| CoreError::storage("Ошибка в snippets.toml", e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SnippetsFile::default(),
            Err(e) => return Err(CoreError::storage("Не удалось прочитать snippets.toml", e)),
        };
        for s in &mut data.snippets {
            if s.id.is_empty() {
                s.id = new_id();
            }
        }
        Ok(Self {
            path,
            data: Mutex::new(data),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SnippetsFile> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<Snippet> {
        self.lock().snippets.clone()
    }

    fn save(&self, data: &SnippetsFile) -> CoreResult<()> {
        let text = toml::to_string_pretty(data)
            .map_err(|e| CoreError::storage("Не удалось сериализовать сниппеты", e))?;
        atomic_write(&self.path, text.as_bytes())
            .map_err(|e| CoreError::storage("Не удалось сохранить snippets.toml", e))
    }

    pub fn upsert(&self, mut snippet: Snippet) -> CoreResult<Snippet> {
        snippet.name = snippet.name.trim().to_owned();
        if snippet.name.is_empty() {
            return Err(CoreError::invalid("Укажите название сниппета"));
        }
        if snippet.command.trim().is_empty() {
            return Err(CoreError::invalid("Команда не может быть пустой"));
        }
        snippet.tags = snippet
            .tags
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        snippet.tags.dedup();

        let mut data = self.lock();
        let mut next = data.clone();
        if snippet.id.is_empty() {
            snippet.id = new_id();
            next.snippets.push(snippet.clone());
        } else {
            let slot = next
                .snippets
                .iter_mut()
                .find(|s| s.id == snippet.id)
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "Сниппет не найден"))?;
            *slot = snippet.clone();
        }
        self.save(&next)?;
        *data = next;
        Ok(snippet)
    }

    pub fn delete(&self, id: &str) -> CoreResult<()> {
        let mut data = self.lock();
        let mut next = data.clone();
        next.snippets.retain(|s| s.id != id);
        self.save(&next)?;
        *data = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_and_render() {
        let cmd = "docker logs {{ container }} --tail {{lines}} | grep {{container}} {{";
        assert_eq!(variables(cmd), vec!["container", "lines"]);
        let mut v = HashMap::new();
        v.insert("container".to_owned(), "web".to_owned());
        assert_eq!(
            render(cmd, &v),
            "docker logs web --tail {{lines}} | grep web {{"
        );
        assert!(variables("no vars").is_empty());
    }

    #[test]
    fn store_crud() {
        let dir = crate::paths::test_dir::TestDir::new();
        let path = dir.path().join("snippets.toml");
        let store = SnippetStore::load(path.clone()).unwrap();
        let s = store
            .upsert(Snippet {
                id: String::new(),
                name: " Uptime ".into(),
                command: "uptime".into(),
                tags: vec!["sys".into(), " ".into()],
                press_enter: true,
            })
            .unwrap();
        assert_eq!(s.name, "Uptime");
        assert_eq!(s.tags, vec!["sys"]);
        assert_eq!(
            SnippetStore::load(path.clone()).unwrap().list(),
            vec![s.clone()]
        );
        store.delete(&s.id).unwrap();
        assert!(SnippetStore::load(path).unwrap().list().is_empty());
        assert_eq!(
            store
                .upsert(Snippet {
                    id: String::new(),
                    name: "x".into(),
                    command: " ".into(),
                    tags: vec![],
                    press_enter: false
                })
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput
        );
    }
}
