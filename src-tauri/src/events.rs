use serde::Serialize;
use tauri::{AppHandle, Emitter};
use termnest_core::{CoreError, SessionId, SessionState, StateSink};

pub const SESSION_STATE_EVENT: &str = "session://state";

#[derive(Clone, Serialize)]
struct SessionStatePayload<'a> {
    id: SessionId,
    state: SessionState,
    error: Option<&'a CoreError>,
}

/// Forwards session state changes to all windows as `session://state`.
pub struct TauriStateSink {
    app: AppHandle,
}

impl TauriStateSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl StateSink for TauriStateSink {
    fn state_changed(&self, id: SessionId, state: SessionState, error: Option<&CoreError>) {
        let _ = self.app.emit(
            SESSION_STATE_EVENT,
            SessionStatePayload { id, state, error },
        );
    }
}
