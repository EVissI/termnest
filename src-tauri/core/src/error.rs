use serde::Serialize;

/// Machine-readable error category, used by the UI to pick an icon or an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    InvalidTarget,
    KeyNotFound,
    KeyEncrypted,
    KeyInvalid,
    HostNotFound,
    ConnectionRefused,
    Timeout,
    Unreachable,
    AuthFailed,
    Protocol,
    SessionNotFound,
}

/// Error returned to the UI. `message` is a human-readable text in Russian and must
/// never contain secrets (passwords, passphrases, key material).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CoreError {
    pub kind: ErrorKind,
    pub message: String,
}

impl CoreError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn session_not_found() -> Self {
        Self::new(ErrorKind::SessionNotFound, "Сессия не найдена")
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
