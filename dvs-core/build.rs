//! Build script for dvs-core.
//!
//! Sets compile-time environment variables for version tracking:
//! - `DVS_VERSION`: Package version (from Cargo.toml or env)
//! - `DVS_COMMIT_SHA`: Git commit hash (from `git rev-parse HEAD` or env)
//! - `DVS_VERSION_STRING`: Combined version string for display (e.g., "0.1.0 (abc12345)")

use std::process::Command;

fn main() {
    // Tell cargo to rerun if git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/HEAD");

    // Set DVS_VERSION (use env var if set, otherwise Cargo.toml version)
    let version = std::env::var("DVS_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=DVS_VERSION={}", version);

    // Set DVS_COMMIT_SHA (use env var if set, otherwise try git)
    let commit = std::env::var("DVS_COMMIT_SHA").ok().or_else(get_git_commit);
    if let Some(ref sha) = commit {
        println!("cargo:rustc-env=DVS_COMMIT_SHA={}", sha);
    }

    // Build a combined version string for display
    let version_string = match commit {
        Some(sha) => format!("{} ({})", version, sha),
        None => version,
    };
    println!("cargo:rustc-env=DVS_VERSION_STRING={}", version_string);
}

/// Try to get the current git commit SHA.
fn get_git_commit() -> Option<String> {
    // Try from workspace root first (../ from dvs-core)
    let output = Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()?;

    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }

    None
}
