//! DVS materialize command.

use std::path::PathBuf;

use crate::output::Output;
use crate::paths;
use super::Result;

/// Run the materialize command.
pub fn run(
    output: &Output,
    files: Vec<PathBuf>,
) -> Result<()> {
    let summary = if files.is_empty() {
        // Materialize all files from manifest
        dvs_core::materialize()?
    } else {
        // Materialize specific files
        let resolved_files: Vec<PathBuf> = files
            .iter()
            .map(|f| paths::resolve_path(f))
            .collect::<Result<Vec<_>>>()?;
        dvs_core::materialize_files(&resolved_files)?
    };

    // Output results
    for result in &summary.results {
        if result.is_error() {
            let msg = result.error.as_deref().unwrap_or("unknown error");
            output.error(&format!("Error materializing {}: {}", result.path.display(), msg));
        } else if result.materialized {
            output.success(&format!("Materialized: {}", result.path.display()));
        } else {
            output.info(&format!("Up to date: {}", result.path.display()));
        }
    }

    // Summary
    if !output.is_quiet() {
        output.info(&format!(
            "Summary: {} materialized, {} up to date, {} failed",
            summary.materialized, summary.up_to_date, summary.failed
        ));
    }

    if summary.failed > 0 {
        return Err(super::CliError::InvalidArg(format!(
            "{} file(s) failed to materialize",
            summary.failed
        )));
    }

    Ok(())
}
