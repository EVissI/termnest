use tauri::{AppHandle, Emitter};
use termnest_core::transfer::{TransferSink, TransferStatus};
use termnest_core::{SessionStatus, StateSink};

pub const SESSION_STATE_EVENT: &str = "session://state";
pub const LAYOUT_CHANGED_EVENT: &str = "layout://changed";
pub const SETTINGS_CHANGED_EVENT: &str = "settings://changed";
pub const HOSTS_CHANGED_EVENT: &str = "hosts://changed";
pub const SNIPPETS_CHANGED_EVENT: &str = "snippets://changed";
pub const TRANSFER_EVENT: &str = "sftp://transfer";
pub const WINDOW_CLOSE_REQUESTED_EVENT: &str = "window://close-requested";

/// Forwards session state changes to all windows as `session://state`.
pub struct TauriStateSink(pub AppHandle);

impl StateSink for TauriStateSink {
    fn state_changed(&self, status: &SessionStatus) {
        let _ = self.0.emit(SESSION_STATE_EVENT, status);
    }
}

/// Forwards transfer progress as `sftp://transfer`.
pub struct TauriTransferSink(pub AppHandle);

impl TransferSink for TauriTransferSink {
    fn transfer_changed(&self, status: &TransferStatus) {
        let _ = self.0.emit(TRANSFER_EVENT, status);
    }
}
