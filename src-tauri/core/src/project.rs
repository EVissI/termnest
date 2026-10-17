//! Local projects (repositories) linked to servers and panels.

use std::path::Path;

use serde::Serialize;

use crate::hosts::{Group, Host};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectInfo {
    pub path: String,
    /// Folder name, shown as the panel label.
    pub name: String,
    pub exists: bool,
    /// A `.git` directory or file (worktree) is present.
    pub is_git: bool,
}

/// Trims a project path; empty means "no project".
pub fn normalize(path: Option<String>) -> Option<String> {
    path.map(|p| p.trim().trim_end_matches(['/', '\\']).to_owned())
        .filter(|p| !p.is_empty())
}

pub fn info(path: &str) -> ProjectInfo {
    let p = Path::new(path);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned());
    ProjectInfo {
        path: path.to_owned(),
        name,
        exists: p.is_dir(),
        is_git: p.join(".git").exists(),
    }
}

/// Effective project: panel override, then the server's, then its group's (the
/// nearest enclosing group with a project).
pub fn resolve<'a>(
    pane_project: Option<&'a str>,
    host: Option<&'a Host>,
    groups: &'a [Group],
) -> Option<&'a str> {
    if let Some(p) = pane_project {
        return Some(p);
    }
    let host = host?;
    if let Some(p) = host.project.as_deref() {
        return Some(p);
    }
    let group = host.group.as_deref()?;
    crate::hosts::group_chain(groups, group)
        .into_iter()
        .find_map(|g| g.project.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_dir::TestDir;

    fn host(project: Option<&str>, group: Option<&str>) -> Host {
        Host {
            id: "h".into(),
            name: "h".into(),
            host: "h".into(),
            port: 22,
            user: "u".into(),
            key_path: None,
            group: group.map(Into::into),
            note: String::new(),
            project: project.map(Into::into),
        }
    }

    #[test]
    fn resolution_order() {
        let mut prod = Group::new("prod".into(), None);
        prod.project = Some("G".into());
        let groups = vec![prod, Group::new("web".into(), Some("prod".into()))];
        let with_own = host(Some("H"), Some("prod"));
        let group_only = host(None, Some("prod"));
        let nothing = host(None, None);
        assert_eq!(resolve(Some("P"), Some(&with_own), &groups), Some("P"));
        assert_eq!(resolve(None, Some(&with_own), &groups), Some("H"));
        assert_eq!(resolve(None, Some(&group_only), &groups), Some("G"));
        let subgroup = host(None, Some("web"));
        assert_eq!(resolve(None, Some(&subgroup), &groups), Some("G"));
        assert_eq!(resolve(None, Some(&nothing), &groups), None);
        assert_eq!(resolve(None, None, &groups), None);
    }

    #[test]
    fn info_detects_git() {
        let dir = TestDir::new();
        let path = dir.path().display().to_string();
        let plain = info(&path);
        assert!(plain.exists && !plain.is_git);
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(info(&path).is_git);
        assert!(!info(&format!("{path}/missing")).exists);
    }

    #[test]
    fn normalize_trims() {
        assert_eq!(normalize(Some(" D:\\p\\ ".into())), Some("D:\\p".into()));
        assert_eq!(normalize(Some("  ".into())), None);
    }
}
