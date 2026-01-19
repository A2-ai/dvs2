//! DVS push command.

use std::path::PathBuf;

use crate::output::Output;
use crate::paths;
use super::Result;

/// Run the push command.
pub fn run(
    output: &Output,
    remote: Option<String>,
    files: Vec<PathBuf>,
) -> Result<()> {
    let summary = if files.is_empty() {
        // Push all tracked objects
        dvs_core::push(remote.as_deref())?
    } else {
        // Push specific files
        let resolved_files: Vec<PathBuf> = files
            .iter()
            .map(|f| paths::resolve_path(f))
            .collect::<Result<Vec<_>>>()?;
        dvs_core::push_files(&resolved_files, remote.as_deref())?
    };

    // Output results
    for result in &summary.results {
        if result.is_error() {
            let msg = result.error.as_deref().unwrap_or("unknown error");
            output.error(&format!("Error pushing {}: {}", result.oid, msg));
        } else if result.uploaded {
            output.success(&format!("Uploaded: {}", result.oid));
        } else {
            output.info(&format!("Already present: {}", result.oid));
        }
    }

    // Summary
    if !output.is_quiet() {
        output.info(&format!(
            "Summary: {} uploaded, {} already present, {} failed",
            summary.uploaded, summary.present, summary.failed
        ));
    }

    if summary.failed > 0 {
        return Err(super::CliError::InvalidArg(format!(
            "{} object(s) failed to push",
            summary.failed
        )));
    }

    Ok(())
}
