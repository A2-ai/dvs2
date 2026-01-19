//! DVS pull command.

use std::path::PathBuf;

use crate::output::Output;
use crate::paths;
use super::Result;

/// Run the pull command.
pub fn run(
    output: &Output,
    remote: Option<String>,
    files: Vec<PathBuf>,
) -> Result<()> {
    let summary = if files.is_empty() {
        // Pull all objects from manifest
        dvs_core::pull(remote.as_deref())?
    } else {
        // Pull specific files
        let resolved_files: Vec<PathBuf> = files
            .iter()
            .map(|f| paths::resolve_path(f))
            .collect::<Result<Vec<_>>>()?;
        dvs_core::pull_files(&resolved_files, remote.as_deref())?
    };

    // Output results
    for result in &summary.results {
        if result.is_error() {
            let msg = result.error.as_deref().unwrap_or("unknown error");
            output.error(&format!("Error pulling {}: {}", result.oid, msg));
        } else if result.downloaded {
            output.success(&format!("Downloaded: {}", result.oid));
        } else {
            output.info(&format!("Already cached: {}", result.oid));
        }
    }

    // Summary
    if !output.is_quiet() {
        output.info(&format!(
            "Summary: {} downloaded, {} already cached, {} failed",
            summary.downloaded, summary.cached, summary.failed
        ));
    }

    if summary.failed > 0 {
        return Err(super::CliError::InvalidArg(format!(
            "{} object(s) failed to pull",
            summary.failed
        )));
    }

    Ok(())
}
