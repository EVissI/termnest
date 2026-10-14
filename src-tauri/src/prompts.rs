//! Delivers core prompts (host key, passphrase, file conflict) to the windows and
//! routes the answer back.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use termnest_core::prompt::{PromptReply, PromptRequest, Prompter};
use tokio::sync::oneshot;

pub const PROMPT_REQUEST_EVENT: &str = "prompt://request";
pub const PROMPT_RESOLVED_EVENT: &str = "prompt://resolved";

#[derive(Clone, Serialize)]
pub struct PendingPrompt {
    pub id: u64,
    pub request: PromptRequest,
}

pub struct PromptHub {
    app: AppHandle,
    next: AtomicU64,
    pending: Mutex<BTreeMap<u64, (PromptRequest, oneshot::Sender<PromptReply>)>>,
}

impl PromptHub {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            next: AtomicU64::new(1),
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<u64, (PromptRequest, oneshot::Sender<PromptReply>)>>
    {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Prompts still waiting for an answer (for a window that opened later).
    pub fn pending(&self) -> Vec<PendingPrompt> {
        self.lock()
            .iter()
            .map(|(id, (request, _))| PendingPrompt {
                id: *id,
                request: request.clone(),
            })
            .collect()
    }

    pub fn respond(&self, id: u64, reply: PromptReply) {
        if let Some((_, tx)) = self.lock().remove(&id) {
            let _ = tx.send(reply);
        }
        let _ = self.app.emit(PROMPT_RESOLVED_EVENT, id);
    }
}

impl Prompter for PromptHub {
    fn ask(&self, request: PromptRequest) -> oneshot::Receiver<PromptReply> {
        let (tx, rx) = oneshot::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.lock().insert(id, (request.clone(), tx));
        let _ = self
            .app
            .emit(PROMPT_REQUEST_EVENT, PendingPrompt { id, request });
        rx
    }
}
