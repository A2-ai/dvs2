//! DVS Daemon - Background file watching and auto-sync service.
//!
//! This crate provides a daemon that watches for file changes and automatically
//! syncs tracked files with the DVS storage.

pub mod config;
pub mod handler;
pub mod ipc;
pub mod watcher;

pub use config::DaemonConfig;
pub use handler::EventHandler;
pub use ipc::{DaemonClient, DaemonServer};
pub use watcher::FileWatcher;

/// Daemon error types.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// File watcher error.
    #[error("watcher error: {0}")]
    WatcherError(String),

    /// IPC communication error.
    #[error("IPC error: {0}")]
    IpcError(String),

    /// Configuration error.
    #[error("config error: {0}")]
    ConfigError(String),

    /// DVS core operation failed.
    #[error("dvs error: {0}")]
    DvsError(#[from] dvs_core::DvsError),

    /// IO error.
    #[error("io error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Start the daemon with the given configuration.
pub async fn start_daemon(_config: DaemonConfig) -> Result<(), DaemonError> {
    todo!("Start the daemon")
}

/// Stop the daemon gracefully.
pub async fn stop_daemon() -> Result<(), DaemonError> {
    todo!("Stop the daemon")
}

/// Check if the daemon is running.
pub fn is_daemon_running() -> bool {
    todo!("Check if daemon is running")
}
