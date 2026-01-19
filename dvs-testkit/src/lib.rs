//! DVS Test Kit - Conformance testing utilities.
//!
//! This crate provides utilities for testing DVS across multiple interfaces
//! (CLI, R, server, daemon) to ensure they all produce the same consequences.
//!
//! # Key Types
//!
//! - [`TestRepo`]: Creates temporary test repositories with git init and storage
//! - [`WorkspaceSnapshot`]: Captures DVS workspace state for comparison
//! - [`InterfaceRunner`]: Trait for running operations through different interfaces
//! - [`Scenario`]: Defines test setup, operations, and expected outcomes
//!
//! # Available Runners
//!
//! | Runner | Status | Description |
//! |--------|--------|-------------|
//! | `CoreRunner` | ✅ Implemented | Baseline - calls dvs-core directly |
//! | `CliRunner` | ✅ Implemented | Runs `dvs` CLI binary (feature: `cli-runner`) |
//! | `RRunner` | ❌ Not implemented | Will run R scripts via dvsR |
//! | `ServerRunner` | ❌ Not implemented | Will test HTTP endpoints via dvs-server |
//! | `DaemonRunner` | ❌ Not implemented | Will test daemon IPC via dvs-daemon |
//!
//! # Example
//!
//! ```no_run
//! use dvs_testkit::{TestRepo, WorkspaceSnapshot};
//!
//! let repo = TestRepo::new().unwrap();
//! repo.write_file("data.csv", b"a,b,c\n1,2,3\n").unwrap();
//!
//! // Run dvs init through core
//! dvs_core::init_with_backend(
//!     &repo.backend(),
//!     repo.storage_dir(),
//!     None,
//!     None,
//! ).unwrap();
//!
//! // Capture snapshot
//! let snapshot = WorkspaceSnapshot::capture(&repo).unwrap();
//! ```

mod repo;
mod snapshot;
mod runner;
mod scenario;
mod diff;

pub use repo::TestRepo;
pub use snapshot::{WorkspaceSnapshot, FileSnapshot, ObjectPresence};
pub use runner::{InterfaceRunner, RunResult, Op, OpKind, CoreRunner, ConformanceResult, run_conformance_test};
pub use scenario::{Scenario, Step, Expectation};
pub use diff::{SnapshotDiff, Mismatch};

#[cfg(feature = "cli-runner")]
pub use runner::CliRunner;

/// Re-export dvs_core for convenience in tests.
pub use dvs_core;
