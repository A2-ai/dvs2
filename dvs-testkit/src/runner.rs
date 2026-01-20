//! Interface runner trait and implementations.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use crate::repo::TestRepo;
use crate::snapshot::WorkspaceSnapshot;

/// Operation to run through an interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Op {
    /// The kind of operation.
    pub kind: OpKind,
    /// Arguments for the operation.
    pub args: Vec<String>,
}

/// Kind of DVS operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// Initialize DVS.
    Init,
    /// Add files to DVS.
    Add,
    /// Get files from storage.
    Get,
    /// Check file status.
    Status,
    /// Push to remote.
    Push,
    /// Pull from remote.
    Pull,
    /// Materialize files.
    Materialize,
    /// View reflog.
    Log,
    /// Rollback to previous state.
    Rollback,
}

impl Op {
    /// Create an init operation.
    pub fn init(storage_dir: &str) -> Self {
        Self {
            kind: OpKind::Init,
            args: vec![storage_dir.to_string()],
        }
    }

    /// Create an add operation.
    pub fn add(files: &[&str]) -> Self {
        Self {
            kind: OpKind::Add,
            args: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Create a get operation.
    pub fn get(files: &[&str]) -> Self {
        Self {
            kind: OpKind::Get,
            args: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Create a status operation.
    pub fn status(files: &[&str]) -> Self {
        Self {
            kind: OpKind::Status,
            args: files.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Result of running an operation through an interface.
#[derive(Debug, Clone)]
pub struct RunResult {
    /// Whether the operation succeeded.
    pub success: bool,

    /// Exit code (for CLI) or equivalent status.
    pub exit_code: i32,

    /// Standard output (for CLI) or result string.
    pub stdout: String,

    /// Standard error (for CLI) or error message.
    pub stderr: String,

    /// Error type string (for R interop).
    pub error_type: Option<String>,

    /// Workspace snapshot after the operation.
    pub snapshot: Option<WorkspaceSnapshot>,
}

impl RunResult {
    /// Create a successful result.
    pub fn success(snapshot: WorkspaceSnapshot) -> Self {
        Self {
            success: true,
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            error_type: None,
            snapshot: Some(snapshot),
        }
    }

    /// Create a failed result.
    pub fn failure(exit_code: i32, stderr: String, error_type: Option<String>) -> Self {
        Self {
            success: false,
            exit_code,
            stdout: String::new(),
            stderr,
            error_type,
            snapshot: None,
        }
    }
}

/// Trait for running operations through different interfaces.
///
/// Implementations:
/// - `CoreRunner`: Calls dvs-core directly (baseline)
/// - `CliRunner`: Runs dvs CLI binary
/// - `RRunner`: Runs R scripts (future)
/// - `ServerRunner`: Tests HTTP endpoints (future)
pub trait InterfaceRunner {
    /// Get the runner name.
    fn name(&self) -> &'static str;

    /// Run an operation and return the result.
    fn run(&self, repo: &TestRepo, op: &Op) -> RunResult;

    /// Check if this runner is available.
    fn is_available(&self) -> bool {
        true
    }
}

/// Core runner - calls dvs-core directly.
///
/// This is the baseline implementation that all other runners
/// should match.
pub struct CoreRunner;

impl CoreRunner {
    /// Create a new core runner.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CoreRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl InterfaceRunner for CoreRunner {
    fn name(&self) -> &'static str {
        "core"
    }

    fn run(&self, repo: &TestRepo, op: &Op) -> RunResult {
        let result = match op.kind {
            OpKind::Init => run_init_core(repo, &op.args),
            OpKind::Add => run_add_core(repo, &op.args),
            OpKind::Get => run_get_core(repo, &op.args),
            OpKind::Status => run_status_core(repo, &op.args),
            _ => Err(format!("Operation {:?} not implemented for core runner", op.kind)),
        };

        match result {
            Ok(()) => {
                match WorkspaceSnapshot::capture(repo) {
                    Ok(snapshot) => RunResult::success(snapshot),
                    Err(e) => RunResult::failure(1, format!("Snapshot error: {}", e), None),
                }
            }
            Err(msg) => RunResult::failure(1, msg, None),
        }
    }
}

fn run_init_core(repo: &TestRepo, args: &[String]) -> Result<(), String> {
    let storage_dir = args.first().ok_or("Missing storage_dir argument")?;
    let storage_path = if storage_dir.starts_with('/') {
        PathBuf::from(storage_dir)
    } else {
        repo.root().join(storage_dir)
    };

    dvs_core::init_with_backend(
        &repo.backend(),
        &storage_path,
        None, // permissions
        None, // group
    )
    .map_err(|e| format!("init failed: {}", e))?;

    Ok(())
}

fn run_add_core(repo: &TestRepo, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("Missing file arguments".to_string());
    }

    let paths: Vec<PathBuf> = args.iter().map(|s| repo.root().join(s)).collect();

    dvs_core::add_with_backend(
        &repo.backend(),
        &paths,
        None, // message
    )
    .map_err(|e| format!("add failed: {}", e))?;

    Ok(())
}

fn run_get_core(repo: &TestRepo, args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("Missing file arguments".to_string());
    }

    let paths: Vec<PathBuf> = args.iter().map(|s| repo.root().join(s)).collect();

    dvs_core::get_with_backend(&repo.backend(), &paths)
        .map_err(|e| format!("get failed: {}", e))?;

    Ok(())
}

fn run_status_core(repo: &TestRepo, args: &[String]) -> Result<(), String> {
    let paths: Vec<PathBuf> = if args.is_empty() {
        Vec::new()
    } else {
        args.iter().map(|s| repo.root().join(s)).collect()
    };

    dvs_core::status_with_backend(&repo.backend(), &paths)
        .map_err(|e| format!("status failed: {}", e))?;

    Ok(())
}

// ============================================================================
// CLI Runner (requires `cli-runner` feature)
// ============================================================================

/// CLI runner - runs the `dvs` binary.
///
/// This runner executes the actual CLI binary and captures its output.
/// Use this to verify CLI behavior matches core behavior.
#[cfg(feature = "cli-runner")]
pub struct CliRunner {
    /// Path to the dvs binary.
    binary_path: PathBuf,
}

#[cfg(feature = "cli-runner")]
impl CliRunner {
    /// Create a new CLI runner using the default binary location.
    ///
    /// Looks for the binary in target/debug/dvs-cli relative to CARGO_MANIFEST_DIR,
    /// or uses the path from CARGO_BIN_EXE_dvs-cli if available.
    pub fn new() -> Self {
        let binary_path = Self::find_binary();
        Self { binary_path }
    }

    /// Create a CLI runner with a specific binary path.
    pub fn with_binary(path: PathBuf) -> Self {
        Self {
            binary_path: path,
        }
    }

    /// Find the dvs binary.
    fn find_binary() -> PathBuf {
        // Use assert_cmd's cargo_bin to find/build the binary
        // This handles workspace builds correctly
        match assert_cmd::cargo::cargo_bin("dvs") {
            path if path.exists() => path,
            _ => {
                // Fallback: try CARGO_BIN_EXE_dvs environment variable
                if let Ok(bin_path) = std::env::var("CARGO_BIN_EXE_dvs") {
                    return PathBuf::from(bin_path);
                }

                // Otherwise, try to find it relative to the workspace
                // CARGO_MANIFEST_DIR points to dvs-testkit, so go up one level
                if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
                    let workspace_root = PathBuf::from(manifest_dir).parent().map(|p| p.to_path_buf());
                    if let Some(root) = workspace_root {
                        // Try debug build first
                        let debug_path = root.join("target/debug/dvs");
                        if debug_path.exists() {
                            return debug_path;
                        }
                        // Try release build
                        let release_path = root.join("target/release/dvs");
                        if release_path.exists() {
                            return release_path;
                        }
                    }
                }

                // Fallback: assume it's in PATH
                PathBuf::from("dvs")
            }
        }
    }

    /// Get the command to run.
    fn command(&self) -> std::process::Command {
        std::process::Command::new(&self.binary_path)
    }
}

#[cfg(feature = "cli-runner")]
impl Default for CliRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "cli-runner")]
impl InterfaceRunner for CliRunner {
    fn name(&self) -> &'static str {
        "cli"
    }

    fn run(&self, repo: &TestRepo, op: &Op) -> RunResult {
        let mut cmd = self.command();

        // Set working directory to repo root
        cmd.current_dir(repo.root());

        // Build command arguments
        match op.kind {
            OpKind::Init => {
                cmd.arg("init");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Add => {
                cmd.arg("add");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Get => {
                cmd.arg("get");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Status => {
                cmd.arg("status");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Push => {
                cmd.arg("push");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Pull => {
                cmd.arg("pull");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Materialize => {
                cmd.arg("materialize");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Log => {
                cmd.arg("log");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
            OpKind::Rollback => {
                cmd.arg("rollback");
                for arg in &op.args {
                    cmd.arg(arg);
                }
            }
        }

        // Run the command
        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                return RunResult::failure(
                    -1,
                    format!("Failed to execute command: {}", e),
                    None,
                );
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);
        let success = output.status.success();

        // Capture snapshot after operation
        let snapshot = if success {
            WorkspaceSnapshot::capture(repo).ok()
        } else {
            // Still try to capture snapshot on failure for comparison
            WorkspaceSnapshot::capture(repo).ok()
        };

        RunResult {
            success,
            exit_code,
            stdout,
            stderr,
            error_type: None, // CLI doesn't expose error types directly
            snapshot,
        }
    }

    fn is_available(&self) -> bool {
        // Check if the binary exists
        self.binary_path.exists()
    }
}

// ============================================================================
// Conformance testing utilities
// ============================================================================

/// Run a scenario through multiple runners and compare results.
pub fn run_conformance_test<R1: InterfaceRunner, R2: InterfaceRunner>(
    baseline: &R1,
    runner: &R2,
    setup_repo: impl Fn() -> Result<TestRepo, crate::repo::TestRepoError>,
    ops: &[Op],
) -> ConformanceResult {
    // Create two repos with identical setup
    let repo1 = match setup_repo() {
        Ok(r) => r,
        Err(e) => return ConformanceResult::SetupError(format!("Baseline repo: {}", e)),
    };
    let repo2 = match setup_repo() {
        Ok(r) => r,
        Err(e) => return ConformanceResult::SetupError(format!("Runner repo: {}", e)),
    };

    let mut baseline_results = Vec::new();
    let mut runner_results = Vec::new();

    // Run all operations through both runners
    for op in ops {
        baseline_results.push(baseline.run(&repo1, op));
        runner_results.push(runner.run(&repo2, op));
    }

    // Compare final snapshots
    let baseline_snapshot = baseline_results.last().and_then(|r| r.snapshot.clone());
    let runner_snapshot = runner_results.last().and_then(|r| r.snapshot.clone());

    match (baseline_snapshot, runner_snapshot) {
        (Some(bs), Some(rs)) => {
            let diff = crate::diff::SnapshotDiff::compare(&bs, &rs);
            if diff.is_empty() {
                ConformanceResult::Pass
            } else {
                ConformanceResult::Mismatch {
                    baseline_name: baseline.name().to_string(),
                    runner_name: runner.name().to_string(),
                    diff,
                }
            }
        }
        (None, Some(_)) => ConformanceResult::BaselineError("No baseline snapshot".to_string()),
        (Some(_), None) => ConformanceResult::RunnerError("No runner snapshot".to_string()),
        (None, None) => ConformanceResult::Pass, // Both failed, which is consistent
    }
}

/// Result of a conformance test.
#[derive(Debug)]
pub enum ConformanceResult {
    /// Test passed - results match.
    Pass,
    /// Setup error.
    SetupError(String),
    /// Baseline runner error.
    BaselineError(String),
    /// Test runner error.
    RunnerError(String),
    /// Results don't match.
    Mismatch {
        baseline_name: String,
        runner_name: String,
        diff: crate::diff::SnapshotDiff,
    },
}

impl ConformanceResult {
    /// Check if the test passed.
    pub fn passed(&self) -> bool {
        matches!(self, ConformanceResult::Pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_op_constructors() {
        let init = Op::init("/storage");
        assert_eq!(init.kind, OpKind::Init);
        assert_eq!(init.args, vec!["/storage"]);

        let add = Op::add(&["file1.txt", "file2.csv"]);
        assert_eq!(add.kind, OpKind::Add);
        assert_eq!(add.args.len(), 2);
    }

    #[test]
    fn test_core_runner_init() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        let op = Op::init(".dvs-storage");
        let result = runner.run(&repo, &op);

        assert!(result.success, "Init failed: {}", result.stderr);
        assert!(result.snapshot.is_some());
        assert!(result.snapshot.unwrap().is_initialized());
    }

    #[test]
    fn test_core_runner_add() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // Initialize first
        let init = Op::init(".dvs-storage");
        runner.run(&repo, &init);

        // Create a file
        repo.write_file("data.csv", b"a,b,c\n1,2,3\n").unwrap();

        // Add it
        let add = Op::add(&["data.csv"]);
        let result = runner.run(&repo, &add);

        assert!(result.success, "Add failed: {}", result.stderr);

        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.tracked_count(), 1);
        assert!(snapshot.is_tracked(&PathBuf::from("data.csv")));
    }

    #[test]
    fn test_conformance_result() {
        let result = ConformanceResult::Pass;
        assert!(result.passed());

        let result = ConformanceResult::SetupError("test".to_string());
        assert!(!result.passed());
    }
}

// ============================================================================
// Server Runner (requires `server-runner` feature)
// ============================================================================

/// Test server wrapper for testing HTTP CAS endpoints.
///
/// This starts a dvs-server in the background on a random port
/// and provides utilities for testing object storage.
#[cfg(feature = "server-runner")]
pub struct TestServer {
    /// The URL of the running server.
    pub url: String,
    /// The port the server is listening on.
    pub port: u16,
    /// Server handle for non-blocking request processing.
    handle: std::sync::Arc<dvs_server::ServerHandle>,
    /// Background thread for serving requests.
    _server_thread: Option<std::thread::JoinHandle<()>>,
    /// Storage directory.
    _storage_dir: tempfile::TempDir,
    /// Shutdown flag.
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(feature = "server-runner")]
impl TestServer {
    /// Start a new test server on a random available port.
    pub fn start() -> Result<Self, String> {
        use dvs_server::{ServerConfig, AuthConfig};
        use std::sync::atomic::AtomicBool;

        // Create temp storage directory
        let storage_dir = tempfile::TempDir::new()
            .map_err(|e| format!("Failed to create temp dir: {}", e))?;

        // Find an available port
        let port = Self::find_available_port()?;

        // Create server config
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port,
            storage_root: storage_dir.path().to_path_buf(),
            auth: AuthConfig::default(), // Auth disabled for tests
            max_upload_size: 100 * 1024 * 1024,
            cors_enabled: false,
            cors_origins: vec![],
            log_level: "warn".to_string(),
        };

        // Start server in background mode
        let (url, handle) = dvs_server::start_server_background(config)
            .map_err(|e| format!("Failed to start server: {}", e))?;

        let handle = std::sync::Arc::new(handle);
        let shutdown = std::sync::Arc::new(AtomicBool::new(false));

        // Spawn background thread to process requests
        let handle_clone = handle.clone();
        let shutdown_clone = shutdown.clone();
        let server_thread = std::thread::spawn(move || {
            while !shutdown_clone.load(std::sync::atomic::Ordering::Relaxed) {
                // Process any pending requests
                let _ = handle_clone.handle_one();
                // Small sleep to prevent busy-waiting
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });

        // Wait for server to be ready
        Self::wait_for_server(&url)?;

        Ok(Self {
            url,
            port,
            handle,
            _server_thread: Some(server_thread),
            _storage_dir: storage_dir,
            shutdown,
        })
    }

    /// Find an available port.
    fn find_available_port() -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind: {}", e))?;
        Ok(listener.local_addr()
            .map_err(|e| format!("Failed to get addr: {}", e))?
            .port())
    }

    /// Wait for server to be ready.
    fn wait_for_server(url: &str) -> Result<(), String> {
        let health_url = format!("{}/health", url);
        let client = reqwest::blocking::Client::new();

        for _ in 0..50 {
            match client.get(&health_url).send() {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }

        Err("Server did not become ready in time".to_string())
    }

    /// Get the base URL for object operations.
    pub fn objects_url(&self) -> String {
        format!("{}/objects", self.url)
    }

    /// Check if an object exists on the server.
    pub fn object_exists(&self, algo: &str, hash: &str) -> Result<bool, String> {
        let url = format!("{}/{}/{}", self.objects_url(), algo, hash);
        let client = reqwest::blocking::Client::new();
        let resp = client.head(&url).send()
            .map_err(|e| format!("HEAD request failed: {}", e))?;
        Ok(resp.status().is_success())
    }

    /// Get an object from the server.
    pub fn get_object(&self, algo: &str, hash: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/{}/{}", self.objects_url(), algo, hash);
        let client = reqwest::blocking::Client::new();
        let resp = client.get(&url).send()
            .map_err(|e| format!("GET request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("GET returned {}", resp.status()));
        }

        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read body: {}", e))
    }

    /// Put an object to the server.
    pub fn put_object(&self, algo: &str, hash: &str, data: &[u8]) -> Result<bool, String> {
        let url = format!("{}/{}/{}", self.objects_url(), algo, hash);
        let client = reqwest::blocking::Client::new();
        let resp = client.put(&url)
            .body(data.to_vec())
            .send()
            .map_err(|e| format!("PUT request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("PUT returned {}", resp.status()));
        }

        // 201 = created new, 200 = already existed
        Ok(resp.status() == reqwest::StatusCode::CREATED)
    }
}

#[cfg(feature = "server-runner")]
impl Drop for TestServer {
    fn drop(&mut self) {
        // Signal shutdown
        self.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        // Give thread time to exit
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Server runner - tests HTTP CAS endpoints.
///
/// This runner tests that objects stored via HTTP can be retrieved
/// correctly. It's used to verify the server's storage layer.
///
/// Note: This runner tests object storage, not full DVS operations.
/// For init/add/get/status, use CoreRunner or CliRunner.
#[cfg(feature = "server-runner")]
pub struct ServerRunner {
    server: std::sync::Arc<TestServer>,
}

#[cfg(feature = "server-runner")]
impl ServerRunner {
    /// Create a new server runner.
    pub fn new() -> Result<Self, String> {
        let server = TestServer::start()?;
        Ok(Self {
            server: std::sync::Arc::new(server),
        })
    }

    /// Get the server URL.
    pub fn url(&self) -> &str {
        &self.server.url
    }

    /// Get a reference to the test server.
    pub fn server(&self) -> &TestServer {
        &self.server
    }
}

#[cfg(feature = "server-runner")]
impl Default for ServerRunner {
    fn default() -> Self {
        Self::new().expect("Failed to start test server")
    }
}

#[cfg(feature = "server-runner")]
impl InterfaceRunner for ServerRunner {
    fn name(&self) -> &'static str {
        "server"
    }

    fn run(&self, repo: &TestRepo, op: &Op) -> RunResult {
        // Server runner only supports push/pull operations
        // that interact with the CAS endpoints
        match op.kind {
            OpKind::Push => {
                // For push, we need to:
                // 1. Read objects from local storage
                // 2. Push them to the server
                // This requires the repo to be initialized with a config
                run_push_server(repo, &op.args, &self.server)
            }
            OpKind::Pull => {
                // For pull, we need to:
                // 1. Fetch objects from server
                // 2. Store them locally
                run_pull_server(repo, &op.args, &self.server)
            }
            _ => {
                // For other operations, delegate to core runner
                // since the server only provides object storage
                let core = CoreRunner::new();
                core.run(repo, op)
            }
        }
    }

    fn is_available(&self) -> bool {
        // Server is started in new(), so always available if created
        true
    }
}

#[cfg(feature = "server-runner")]
fn run_push_server(repo: &TestRepo, _args: &[String], server: &TestServer) -> RunResult {
    use dvs_core::Manifest;

    // Load manifest to find objects to push
    let dvs_dir = repo.root().join(".dvs");
    let manifest_path = dvs_dir.join("manifest.json");

    if !manifest_path.exists() {
        return RunResult::failure(1, "No manifest found - run init and add first".to_string(), None);
    }

    let manifest = match Manifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => return RunResult::failure(1, format!("Failed to load manifest: {}", e), None),
    };

    // Load config
    let config = match dvs_core::Config::load_from_dir(repo.root()) {
        Ok(c) => c,
        Err(e) => return RunResult::failure(1, format!("Failed to load config: {}", e), None),
    };

    let local_store = dvs_core::LocalStore::new(config.storage_dir.clone());

    // Push each unique object
    let mut pushed = 0;
    let mut skipped = 0;
    for oid in manifest.unique_oids() {
        // Check if already on server
        match server.object_exists(oid.algo.prefix(), &oid.hex) {
            Ok(true) => {
                skipped += 1;
                continue;
            }
            Ok(false) => {}
            Err(e) => return RunResult::failure(1, format!("Failed to check object {}: {}", oid, e), None),
        }

        // Read object from local storage to temp file
        let obj_path = local_store.object_path(&oid);
        if !obj_path.exists() {
            return RunResult::failure(1, format!("Object not found locally: {}", oid), None);
        }

        let data = match std::fs::read(&obj_path) {
            Ok(d) => d,
            Err(e) => return RunResult::failure(1, format!("Failed to read object {}: {}", oid, e), None),
        };

        // Push to server
        match server.put_object(oid.algo.prefix(), &oid.hex, &data) {
            Ok(_) => pushed += 1,
            Err(e) => return RunResult::failure(1, format!("Failed to push {}: {}", oid, e), None),
        }
    }

    // Return success with snapshot
    match WorkspaceSnapshot::capture(repo) {
        Ok(snapshot) => {
            let mut result = RunResult::success(snapshot);
            result.stdout = format!("Pushed {} objects ({} already existed)", pushed, skipped);
            result
        }
        Err(e) => RunResult::failure(1, format!("Snapshot error: {}", e), None),
    }
}

#[cfg(feature = "server-runner")]
fn run_pull_server(repo: &TestRepo, _args: &[String], server: &TestServer) -> RunResult {
    use dvs_core::{Manifest, ObjectStore as _};

    // Load manifest to find objects to pull
    let dvs_dir = repo.root().join(".dvs");
    let manifest_path = dvs_dir.join("manifest.json");

    if !manifest_path.exists() {
        return RunResult::failure(1, "No manifest found".to_string(), None);
    }

    let manifest = match Manifest::load(&manifest_path) {
        Ok(m) => m,
        Err(e) => return RunResult::failure(1, format!("Failed to load manifest: {}", e), None),
    };

    // Load config
    let config = match dvs_core::Config::load_from_dir(repo.root()) {
        Ok(c) => c,
        Err(e) => return RunResult::failure(1, format!("Failed to load config: {}", e), None),
    };

    let local_store = dvs_core::LocalStore::new(config.storage_dir.clone());

    // Pull each unique object
    let mut pulled = 0;
    let mut skipped = 0;
    for oid in manifest.unique_oids() {
        // Check if already exists locally
        if local_store.has(&oid).unwrap_or(false) {
            skipped += 1;
            continue;
        }

        // Fetch from server
        let data = match server.get_object(oid.algo.prefix(), &oid.hex) {
            Ok(d) => d,
            Err(e) => return RunResult::failure(1, format!("Failed to pull {}: {}", oid, e), None),
        };

        // Write to temp file then copy to local store
        let obj_path = local_store.object_path(&oid);
        if let Some(parent) = obj_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return RunResult::failure(1, format!("Failed to create dir: {}", e), None);
            }
        }

        if let Err(e) = std::fs::write(&obj_path, &data) {
            return RunResult::failure(1, format!("Failed to store {}: {}", oid, e), None);
        }

        pulled += 1;
    }

    // Return success with snapshot
    match WorkspaceSnapshot::capture(repo) {
        Ok(snapshot) => {
            let mut result = RunResult::success(snapshot);
            result.stdout = format!("Pulled {} objects ({} already existed)", pulled, skipped);
            result
        }
        Err(e) => RunResult::failure(1, format!("Snapshot error: {}", e), None),
    }
}

#[cfg(all(test, feature = "cli-runner"))]
mod cli_tests {
    use super::*;

    /// Helper to skip test if CLI binary is not available.
    /// Returns the runner if available, None if should skip.
    fn runner_if_available() -> Option<CliRunner> {
        let runner = CliRunner::new();
        if runner.is_available() {
            Some(runner)
        } else {
            eprintln!("Skipping CLI test: dvs binary not built. Run `cargo build -p dvs-cli` first.");
            None
        }
    }

    #[test]
    fn test_cli_runner_is_available() {
        let runner = CliRunner::new();
        // Note: This test may fail if the CLI isn't built yet.
        // Run `cargo build -p dvs-cli` first, or use `cargo test --workspace`.
        if !runner.is_available() {
            eprintln!("CLI binary not available - skipping test (build with `cargo build -p dvs-cli`)");
            return;
        }
        assert!(runner.is_available());
    }

    #[test]
    fn test_cli_runner_init() {
        let runner = match runner_if_available() {
            Some(r) => r,
            None => return,
        };
        let repo = TestRepo::new().unwrap();

        let op = Op::init(".dvs-storage");
        let result = runner.run(&repo, &op);

        assert!(result.success, "CLI init failed: {}\nstdout: {}", result.stderr, result.stdout);
        assert!(result.snapshot.is_some());
        assert!(result.snapshot.unwrap().is_initialized());
    }

    #[test]
    fn test_cli_runner_add() {
        let runner = match runner_if_available() {
            Some(r) => r,
            None => return,
        };
        let repo = TestRepo::new().unwrap();

        // Initialize first
        let init = Op::init(".dvs-storage");
        let result = runner.run(&repo, &init);
        assert!(result.success, "CLI init failed: {}", result.stderr);

        // Create a file
        repo.write_file("data.csv", b"a,b,c\n1,2,3\n").unwrap();

        // Add it
        let add = Op::add(&["data.csv"]);
        let result = runner.run(&repo, &add);

        assert!(result.success, "CLI add failed: {}\nstdout: {}", result.stderr, result.stdout);

        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.tracked_count(), 1);
        assert!(snapshot.is_tracked(&PathBuf::from("data.csv")));
    }

    #[test]
    fn test_conformance_core_vs_cli() {
        let cli = match runner_if_available() {
            Some(r) => r,
            None => return,
        };
        let core = CoreRunner::new();

        // Simple init + add scenario
        let result = run_conformance_test(
            &core,
            &cli,
            || {
                let repo = TestRepo::new()?;
                repo.write_file("test.txt", b"hello world")?;
                Ok(repo)
            },
            &[Op::init(".dvs-storage"), Op::add(&["test.txt"])],
        );

        assert!(
            result.passed(),
            "Conformance test failed: {:?}",
            result
        );
    }
}

#[cfg(all(test, feature = "server-runner"))]
mod server_tests {
    use super::*;
    use dvs_core::ObjectStore;

    #[test]
    fn test_server_start_stop() {
        let server = TestServer::start().expect("Failed to start server");
        assert!(!server.url.is_empty());
        assert!(server.port > 0);
        // Server will be stopped on drop
    }

    #[test]
    fn test_server_put_get_object() {
        let server = TestServer::start().expect("Failed to start server");

        // Create a test object
        let data = b"hello world test data";
        let hash = blake3::hash(data).to_hex().to_string();

        // Initially should not exist
        assert!(!server.object_exists("blake3", &hash).unwrap());

        // Put the object
        let created = server.put_object("blake3", &hash, data).unwrap();
        assert!(created, "Should report as newly created");

        // Now should exist
        assert!(server.object_exists("blake3", &hash).unwrap());

        // Get should return the same data
        let retrieved = server.get_object("blake3", &hash).unwrap();
        assert_eq!(retrieved, data);

        // Put again should report as already existing
        let created = server.put_object("blake3", &hash, data).unwrap();
        assert!(!created, "Should report as already existing");
    }

    #[test]
    fn test_server_runner_delegates_init_add() {
        // ServerRunner delegates init/add/get/status to CoreRunner
        let runner = ServerRunner::new().expect("Failed to create server runner");
        let repo = TestRepo::new().expect("Failed to create test repo");

        // Initialize
        let init = Op::init(".dvs-storage");
        let result = runner.run(&repo, &init);
        assert!(result.success, "Init failed: {}", result.stderr);

        // Create and add a file
        repo.write_file("data.txt", b"test content for server").unwrap();
        let add = Op::add(&["data.txt"]);
        let result = runner.run(&repo, &add);
        assert!(result.success, "Add failed: {}", result.stderr);

        // Verify file was tracked
        let snapshot = result.snapshot.unwrap();
        assert!(snapshot.is_tracked(&PathBuf::from("data.txt")));
    }

    #[test]
    fn test_server_object_roundtrip() {
        // Test that objects can be pushed to and pulled from the server
        let server = TestServer::start().expect("Failed to start server");

        // Create multiple test objects
        let test_objects = vec![
            (b"hello world".as_slice(), "blake3"),
            (b"test data 123".as_slice(), "blake3"),
            (b"another object".as_slice(), "blake3"),
        ];

        for (data, algo) in &test_objects {
            let hash = blake3::hash(data).to_hex().to_string();

            // Put object
            server.put_object(algo, &hash, data).expect("Put failed");

            // Verify it exists
            assert!(server.object_exists(algo, &hash).unwrap());

            // Get and verify content matches
            let retrieved = server.get_object(algo, &hash).expect("Get failed");
            assert_eq!(&retrieved[..], *data);
        }
    }
}
