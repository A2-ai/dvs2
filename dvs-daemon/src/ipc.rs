//! IPC (Inter-Process Communication) for daemon control.

use crate::DaemonError;
use std::path::PathBuf;

/// Commands that can be sent to the daemon.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DaemonCommand {
    /// Stop the daemon.
    Stop,
    /// Get daemon status.
    Status,
    /// Add a path to watch.
    AddWatch(PathBuf),
    /// Remove a path from watching.
    RemoveWatch(PathBuf),
    /// Trigger a manual sync.
    Sync,
    /// Reload configuration.
    ReloadConfig,
}

/// Response from the daemon.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DaemonResponse {
    /// Command succeeded.
    Ok,
    /// Command succeeded with message.
    OkWithMessage(String),
    /// Command failed with error.
    Error(String),
    /// Status response.
    Status(DaemonStatus),
}

/// Daemon status information.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DaemonStatus {
    /// Whether the daemon is running.
    pub running: bool,
    /// Number of paths being watched.
    pub watch_count: usize,
    /// Uptime in seconds.
    pub uptime_secs: u64,
    /// Number of events processed.
    pub events_processed: u64,
}

/// Client for communicating with the daemon.
pub struct DaemonClient {
    _socket_path: PathBuf,
}

impl DaemonClient {
    /// Create a new daemon client.
    pub fn new(_socket_path: PathBuf) -> Self {
        todo!("Create daemon client")
    }

    /// Connect to the daemon.
    pub async fn connect(&mut self) -> Result<(), DaemonError> {
        todo!("Connect to daemon")
    }

    /// Send a command to the daemon.
    pub async fn send_command(
        &mut self,
        _command: DaemonCommand,
    ) -> Result<DaemonResponse, DaemonError> {
        todo!("Send command to daemon")
    }

    /// Check if connected to daemon.
    pub fn is_connected(&self) -> bool {
        todo!("Check if connected")
    }
}

/// Server for receiving daemon commands.
pub struct DaemonServer {
    _socket_path: PathBuf,
}

impl DaemonServer {
    /// Create a new daemon server.
    pub fn new(_socket_path: PathBuf) -> Self {
        todo!("Create daemon server")
    }

    /// Start listening for commands.
    pub async fn listen(&mut self) -> Result<(), DaemonError> {
        todo!("Start listening for commands")
    }

    /// Get the next command (blocks until command is available).
    pub async fn next_command(&mut self) -> Option<DaemonCommand> {
        todo!("Get next command")
    }

    /// Send a response to the client.
    pub async fn send_response(&mut self, _response: DaemonResponse) -> Result<(), DaemonError> {
        todo!("Send response to client")
    }
}

/// Get the default socket path for the daemon.
pub fn default_socket_path() -> PathBuf {
    todo!("Get default socket path")
}
