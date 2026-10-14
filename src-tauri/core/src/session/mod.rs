//! SSH sessions. A session lives here independently of any window or panel;
//! the UI only subscribes to its output and sends input by id.

mod connection;
mod output;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::mpsc;

use crate::error::{CoreError, CoreResult};
use crate::target::Target;

use output::OutputHub;
pub use output::OutputSink;

pub type SessionId = u64;

/// How much recent output each session keeps for replay to new subscribers.
const OUTPUT_BUFFER_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Error,
}

/// Receiver of session state changes (the app forwards them as `session://state`).
pub trait StateSink: Send + Sync + 'static {
    fn state_changed(&self, id: SessionId, state: SessionState, error: Option<&CoreError>);
}

#[derive(Debug, Clone)]
pub struct ConnectOptions {
    pub target: Target,
    pub key_path: PathBuf,
    pub cols: u32,
    pub rows: u32,
    pub keepalive_interval: Option<Duration>,
    pub connect_timeout: Duration,
}

impl ConnectOptions {
    pub fn new(target: Target, key_path: PathBuf, cols: u32, rows: u32) -> Self {
        Self {
            target,
            key_path,
            cols: cols.max(1),
            rows: rows.max(1),
            keepalive_interval: Some(Duration::from_secs(30)),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

pub(crate) enum Command {
    Data(Vec<u8>),
    Resize { cols: u32, rows: u32 },
    Close,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct SessionEntry {
    id: SessionId,
    state: Mutex<SessionState>,
    output: Mutex<OutputHub>,
    commands: mpsc::UnboundedSender<Command>,
    state_sink: Arc<dyn StateSink>,
}

impl SessionEntry {
    pub(crate) fn set_state(&self, state: SessionState, error: Option<&CoreError>) {
        *lock(&self.state) = state;
        self.state_sink.state_changed(self.id, state, error);
    }

    pub(crate) fn push_output(&self, data: &[u8]) {
        lock(&self.output).push(data);
    }
}

pub struct SessionManager {
    sessions: Mutex<HashMap<SessionId, Arc<SessionEntry>>>,
    next_id: AtomicU64,
    state_sink: Arc<dyn StateSink>,
}

impl SessionManager {
    pub fn new(state_sink: Arc<dyn StateSink>) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            state_sink,
        }
    }

    /// Registers a new session and starts connecting in the background.
    /// Progress is reported through the [`StateSink`]. Must be called inside a Tokio runtime.
    pub fn open(&self, options: ConnectOptions) -> SessionId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::unbounded_channel();
        let entry = Arc::new(SessionEntry {
            id,
            state: Mutex::new(SessionState::Connecting),
            output: Mutex::new(OutputHub::new(OUTPUT_BUFFER_BYTES)),
            commands: tx,
            state_sink: self.state_sink.clone(),
        });
        lock(&self.sessions).insert(id, entry.clone());
        tokio::spawn(connection::run(entry, options, rx));
        id
    }

    fn get(&self, id: SessionId) -> CoreResult<Arc<SessionEntry>> {
        lock(&self.sessions)
            .get(&id)
            .cloned()
            .ok_or_else(CoreError::session_not_found)
    }

    pub fn state(&self, id: SessionId) -> CoreResult<SessionState> {
        Ok(*lock(&self.get(id)?.state))
    }

    /// Subscribes to output. Buffered output is replayed to the sink first.
    pub fn subscribe(&self, id: SessionId, sink: Arc<dyn OutputSink>) -> CoreResult<u64> {
        Ok(lock(&self.get(id)?.output).subscribe(sink))
    }

    pub fn unsubscribe(&self, id: SessionId, subscription: u64) {
        if let Ok(entry) = self.get(id) {
            lock(&entry.output).unsubscribe(subscription);
        }
    }

    pub fn write(&self, id: SessionId, data: Vec<u8>) -> CoreResult<()> {
        if !data.is_empty() {
            // A send error only means the connection already ended; the state event says so.
            let _ = self.get(id)?.commands.send(Command::Data(data));
        }
        Ok(())
    }

    pub fn resize(&self, id: SessionId, cols: u32, rows: u32) -> CoreResult<()> {
        let _ = self.get(id)?.commands.send(Command::Resize {
            cols: cols.max(1),
            rows: rows.max(1),
        });
        Ok(())
    }

    /// Closes the session by user request and forgets it.
    pub fn close(&self, id: SessionId) {
        if let Some(entry) = lock(&self.sessions).remove(&id) {
            let _ = entry.commands.send(Command::Close);
        }
    }

    pub fn ids(&self) -> Vec<SessionId> {
        let mut ids: Vec<_> = lock(&self.sessions).keys().copied().collect();
        ids.sort_unstable();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder(Mutex<Vec<(SessionId, SessionState, Option<crate::ErrorKind>)>>);

    impl StateSink for Recorder {
        fn state_changed(&self, id: SessionId, state: SessionState, error: Option<&CoreError>) {
            self.0
                .lock()
                .unwrap()
                .push((id, state, error.map(|e| e.kind)));
        }
    }

    fn options(key_path: PathBuf) -> ConnectOptions {
        let mut o = ConnectOptions::new("me@127.0.0.1:1".parse().unwrap(), key_path, 80, 24);
        o.connect_timeout = Duration::from_secs(2);
        o
    }

    async fn wait_for_final_state(rec: &Recorder) -> (SessionState, Option<crate::ErrorKind>) {
        for _ in 0..200 {
            if let Some(&(_, state, kind)) = rec.0.lock().unwrap().last() {
                if state != SessionState::Connecting {
                    return (state, kind);
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("session did not leave the connecting state");
    }

    #[tokio::test]
    async fn missing_key_reports_error_state() {
        let rec = Arc::new(Recorder(Mutex::new(Vec::new())));
        let manager = SessionManager::new(rec.clone());
        let id = manager.open(options(PathBuf::from("definitely/missing/key")));

        let (state, kind) = wait_for_final_state(&rec).await;
        assert_eq!(state, SessionState::Error);
        assert_eq!(kind, Some(crate::ErrorKind::KeyNotFound));
        assert_eq!(manager.state(id).unwrap(), SessionState::Error);
    }

    #[tokio::test]
    async fn ids_are_unique_and_close_forgets_session() {
        let rec = Arc::new(Recorder(Mutex::new(Vec::new())));
        let manager = SessionManager::new(rec);
        let a = manager.open(options(PathBuf::from("missing-a")));
        let b = manager.open(options(PathBuf::from("missing-b")));
        assert_ne!(a, b);
        assert_eq!(manager.ids(), vec![a, b]);

        manager.close(a);
        assert_eq!(manager.ids(), vec![b]);
        assert_eq!(
            manager.write(a, b"x".to_vec()).unwrap_err().kind,
            crate::ErrorKind::SessionNotFound
        );
    }
}
