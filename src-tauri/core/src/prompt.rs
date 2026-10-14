//! Questions the core needs the user to answer while an operation is running
//! (unknown host key, key passphrase, file name conflict).

use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

use crate::secret::SecretString;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptRequest {
    /// First connection to a host: show the fingerprint and ask whether to trust it.
    HostKeyUnknown {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
    },
    /// The private key is encrypted.
    Passphrase {
        key_path: String,
        /// The previous attempt was wrong.
        retry: bool,
    },
    /// A file transfer target already exists.
    FileConflict {
        name: String,
        /// Destination is on the server (upload) or local (download).
        remote: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictAction {
    Replace,
    Skip,
    Rename,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptReply {
    Cancel,
    Confirm,
    Passphrase {
        value: SecretString,
        remember: bool,
    },
    Conflict {
        action: ConflictAction,
        apply_to_all: bool,
    },
}

/// Shows prompts to the user. The receiver resolves with the answer; if the sender is
/// dropped (no window to answer) it is treated as a cancel.
pub trait Prompter: Send + Sync + 'static {
    fn ask(&self, request: PromptRequest) -> oneshot::Receiver<PromptReply>;
}

/// Asks and waits; any failure to get an answer is a cancel.
pub async fn ask(prompter: &dyn Prompter, request: PromptRequest) -> PromptReply {
    prompter.ask(request).await.unwrap_or(PromptReply::Cancel)
}

/// Prompter that cancels everything: for tests and non-interactive use.
pub struct NoPrompts;

impl Prompter for NoPrompts {
    fn ask(&self, _request: PromptRequest) -> oneshot::Receiver<PromptReply> {
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(PromptReply::Cancel);
        rx
    }
}
