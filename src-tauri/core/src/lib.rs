//! Core logic of termnest: SSH sessions, stored data and everything that does not
//! depend on the UI shell.
//!
//! This crate intentionally has no dependency on Tauri, so it can be built and tested
//! on any platform (including the Linux build container).

pub mod error;
pub mod hostkeys;
pub mod hosts;
pub mod ids;
pub mod keydeploy;
pub mod keys;
pub mod layout;
pub mod paths;
pub mod project;
pub mod prompt;
pub mod secret;
pub mod session;
pub mod settings;
pub mod sftp;
pub mod snippets;
pub mod ssh;
pub mod ssh_config;
pub mod target;
pub mod transfer;

pub use error::{CoreError, CoreResult, ErrorKind};
pub use russh_sftp::client::SftpSession;
pub use session::{
    ConnectSpec, OutputSink, SessionId, SessionManager, SessionState, SessionStatus, StateSink,
};
pub use target::Target;
