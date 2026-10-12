//! Portable core for BHTerminal.
//!
//! Everything here compiles unchanged on macOS, Windows and Linux: no UI, no
//! Tauri, and no platform discovery — the shell injects paths rather than the
//! core guessing them. The one place a platform difference is unavoidable is
//! ssh connection multiplexing, which Windows OpenSSH does not implement; see
//! `ssh::multiplexing_supported`.

pub mod backup;
pub mod crypto;
pub mod import;
pub mod model;
pub mod prompt;
pub mod secrets;
pub mod sftp;
pub mod ssh;
pub mod store;
pub mod title;
pub mod update;

pub use model::{AuthMethod, ConnectionType, Host, HostFolder, Snippet, TreeItem, TunnelKind, TunnelRule};
pub use secrets::SecretVault;
pub use store::SessionStore;
