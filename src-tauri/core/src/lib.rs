//! Core logic of termnest: SSH sessions and everything that does not depend on the UI shell.
//!
//! This crate intentionally has no dependency on Tauri, so it can be built and tested
//! on any platform (including the Linux build container).

pub mod error;
pub mod paths;
pub mod session;
pub mod target;

pub use error::{CoreError, ErrorKind};
pub use session::{OutputSink, SessionId, SessionManager, SessionState, StateSink};
pub use target::Target;
