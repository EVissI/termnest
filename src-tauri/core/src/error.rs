use serde::Serialize;

/// Machine-readable error category, used by the UI to pick an icon or an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidTarget,
    InvalidInput,
    KeyNotFound,
    KeyEncrypted,
    KeyInvalid,
    HostNotFound,
    ConnectionRefused,
    Timeout,
    Unreachable,
    AuthFailed,
    HostKeyChanged,
    HostKeyRejected,
    Protocol,
    SessionNotFound,
    NotFound,
    AlreadyExists,
    PermissionDenied,
    Storage,
    Remote,
    Cancelled,
}

/// Details for a host key mismatch, shown in the blocking warning dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostKeyInfo {
    pub host: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub known_fingerprint: Option<String>,
}

/// Error returned to the UI. `message` is a human-readable text in Russian and must
/// never contain secrets (passwords, passphrases, key material).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CoreError {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key: Option<Box<HostKeyInfo>>,
}

impl CoreError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            host_key: None,
        }
    }

    pub fn session_not_found() -> Self {
        Self::new(ErrorKind::SessionNotFound, "Сессия не найдена")
    }

    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Операция отменена")
    }

    pub fn storage(what: &str, err: impl std::fmt::Display) -> Self {
        Self::new(ErrorKind::Storage, format!("{what}: {err}"))
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    /// Whether retrying the same connection later may succeed (network-level problems).
    pub fn is_transient(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::HostNotFound
                | ErrorKind::ConnectionRefused
                | ErrorKind::Timeout
                | ErrorKind::Unreachable
        )
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
