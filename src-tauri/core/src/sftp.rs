//! Remote file operations over an SFTP session.

use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType, StatusCode};
use serde::Serialize;

use crate::error::{CoreError, CoreResult, ErrorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    /// Unix time, seconds.
    pub modified: Option<u32>,
    /// Permission bits (`0o7777` part of the mode).
    pub mode: Option<u32>,
    pub owner: Option<String>,
    pub group: Option<String>,
}

pub fn map_sftp_error(e: SftpError, what: &str) -> CoreError {
    match &e {
        SftpError::Status(s) => {
            let kind = match s.status_code {
                StatusCode::NoSuchFile => ErrorKind::NotFound,
                StatusCode::PermissionDenied => ErrorKind::PermissionDenied,
                _ => ErrorKind::Remote,
            };
            let reason = match s.status_code {
                StatusCode::NoSuchFile => "нет такого файла или папки".to_owned(),
                StatusCode::PermissionDenied => "нет доступа".to_owned(),
                _ => s.error_message.clone(),
            };
            CoreError::new(kind, format!("{what}: {reason}"))
        }
        SftpError::Timeout => CoreError::new(ErrorKind::Timeout, format!("{what}: таймаут SFTP")),
        _ => CoreError::new(ErrorKind::Remote, format!("{what}: {e}")),
    }
}

/// Joins a remote directory and a name with `/`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// Parent of a remote path (`/` for top-level entries).
pub fn parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(i) => trimmed[..i].to_owned(),
    }
}

fn kind_of(attrs: &FileAttributes) -> EntryKind {
    match attrs.file_type() {
        FileType::Dir => EntryKind::Dir,
        FileType::File => EntryKind::File,
        FileType::Symlink => EntryKind::Symlink,
        FileType::Other => EntryKind::Other,
    }
}

pub async fn realpath(sftp: &SftpSession, path: &str) -> CoreResult<String> {
    sftp.canonicalize(path)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Путь {path}")))
}

/// Lists a directory: folders first, then by name. Symlinks to folders count as folders.
pub async fn list(sftp: &SftpSession, dir: &str) -> CoreResult<Vec<RemoteEntry>> {
    let rd = sftp
        .read_dir(dir)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Папка {dir}")))?;
    let mut out = Vec::new();
    for entry in rd {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let attrs = entry.metadata();
        let path = join(dir, &name);
        let mut kind = kind_of(&attrs);
        if kind == EntryKind::Symlink {
            if let Ok(target) = sftp.metadata(path.clone()).await {
                if kind_of(&target) == EntryKind::Dir {
                    kind = EntryKind::Dir;
                }
            }
        }
        out.push(RemoteEntry {
            name,
            path,
            kind,
            size: attrs.size.unwrap_or(0),
            modified: attrs.mtime,
            mode: attrs.permissions.map(|p| p & 0o7777),
            owner: attrs
                .user
                .clone()
                .or_else(|| attrs.uid.map(|u| u.to_string())),
            group: attrs
                .group
                .clone()
                .or_else(|| attrs.gid.map(|g| g.to_string())),
        });
    }
    out.sort_by(|a, b| {
        let ad = a.kind == EntryKind::Dir;
        let bd = b.kind == EntryKind::Dir;
        bd.cmp(&ad)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

pub async fn stat(sftp: &SftpSession, path: &str) -> CoreResult<Option<FileAttributes>> {
    match sftp.metadata(path).await {
        Ok(a) => Ok(Some(a)),
        Err(SftpError::Status(s)) if s.status_code == StatusCode::NoSuchFile => Ok(None),
        Err(e) => Err(map_sftp_error(e, &format!("Файл {path}"))),
    }
}

pub async fn mkdir(sftp: &SftpSession, path: &str) -> CoreResult<()> {
    sftp.create_dir(path)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Создание папки {path}")))
}

pub async fn rename(sftp: &SftpSession, from: &str, to: &str) -> CoreResult<()> {
    if stat(sftp, to).await?.is_some() {
        return Err(CoreError::new(
            ErrorKind::AlreadyExists,
            format!("{to} уже существует"),
        ));
    }
    sftp.rename(from, to)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Переименование {from}")))
}

pub async fn chmod(sftp: &SftpSession, path: &str, mode: u32) -> CoreResult<()> {
    let mut attrs = FileAttributes::empty();
    attrs.permissions = Some(mode & 0o7777);
    sftp.set_metadata(path, attrs)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Права {path}")))
}

/// Deletes a file, a symlink or a whole directory tree.
pub async fn remove(sftp: &SftpSession, path: &str) -> CoreResult<()> {
    let attrs = sftp
        .symlink_metadata(path)
        .await
        .map_err(|e| map_sftp_error(e, &format!("Удаление {path}")))?;
    if kind_of(&attrs) != EntryKind::Dir {
        return sftp
            .remove_file(path)
            .await
            .map_err(|e| map_sftp_error(e, &format!("Удаление {path}")));
    }
    // Depth-first without recursion in async code.
    let mut stack = vec![(path.to_owned(), false)];
    while let Some((dir, visited)) = stack.pop() {
        if visited {
            sftp.remove_dir(dir.clone())
                .await
                .map_err(|e| map_sftp_error(e, &format!("Удаление {dir}")))?;
            continue;
        }
        stack.push((dir.clone(), true));
        let rd = sftp
            .read_dir(dir.clone())
            .await
            .map_err(|e| map_sftp_error(e, &format!("Удаление {dir}")))?;
        for entry in rd {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let child = join(&dir, &name);
            if kind_of(&entry.metadata()) == EntryKind::Dir {
                stack.push((child, false));
            } else {
                sftp.remove_file(child.clone())
                    .await
                    .map_err(|e| map_sftp_error(e, &format!("Удаление {child}")))?;
            }
        }
    }
    Ok(())
}

/// A name not used in `taken`: `name (1).ext`, `name (2).ext`, ...
pub fn free_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/home/u", "a"), "/home/u/a");
        assert_eq!(parent("/home/u/a"), "/home/u");
        assert_eq!(parent("/home"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(parent("/home/u/"), "/home");
    }

    #[test]
    fn free_names() {
        let taken = ["a.txt", "a (1).txt"];
        assert_eq!(free_name("a.txt", |n| taken.contains(&n)), "a (2).txt");
        assert_eq!(free_name("dir", |_| false), "dir (1)");
        assert_eq!(free_name(".bashrc", |_| false), ".bashrc (1)");
    }
}
