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
//! | `ServerRunner` | ✅ Implemented | Tests HTTP CAS endpoints (feature: `server-runner`) |
//! | `RRunner` | ❌ Not implemented | Will run R scripts via dvsR |
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

mod diff;
mod integration;
mod repo;
mod runner;
mod scenario;
mod snapshot;

pub use diff::{Mismatch, SnapshotDiff};
pub use repo::TestRepo;
pub use runner::{
    run_conformance_test, ConformanceResult, CoreRunner, InterfaceRunner, Op, OpKind, RunResult,
};
pub use scenario::{Expectation, Scenario, Step};
pub use snapshot::{FileSnapshot, ObjectPresence, WorkspaceSnapshot};

#[cfg(feature = "cli-runner")]
pub use runner::CliRunner;

#[cfg(feature = "server-runner")]
pub use runner::{ServerRunner, TestServer};

/// Re-export dvs_core for convenience in tests.
pub use dvs_core;
