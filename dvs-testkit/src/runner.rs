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
        // First, check if we're in a cargo test environment
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

#[cfg(all(test, feature = "cli-runner"))]
mod cli_tests {
    use super::*;

    #[test]
    fn test_cli_runner_is_available() {
        let runner = CliRunner::new();
        // Should be available if cargo is available
        assert!(runner.is_available());
    }

    #[test]
    fn test_cli_runner_init() {
        let repo = TestRepo::new().unwrap();
        let runner = CliRunner::new();

        let op = Op::init(".dvs-storage");
        let result = runner.run(&repo, &op);

        assert!(result.success, "CLI init failed: {}\nstdout: {}", result.stderr, result.stdout);
        assert!(result.snapshot.is_some());
        assert!(result.snapshot.unwrap().is_initialized());
    }

    #[test]
    fn test_cli_runner_add() {
        let repo = TestRepo::new().unwrap();
        let runner = CliRunner::new();

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
        let core = CoreRunner::new();
        let cli = CliRunner::new();

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
