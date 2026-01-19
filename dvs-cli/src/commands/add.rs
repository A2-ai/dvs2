//! DVS add command.

use std::path::PathBuf;

use crate::output::Output;
use crate::paths;
use super::Result;

/// Run the add command.
pub fn run(
    output: &Output,
    files: Vec<PathBuf>,
    message: Option<String>,
) -> Result<()> {
    // Resolve all file paths
    let resolved_files: Vec<PathBuf> = files
        .iter()
        .map(|f| paths::resolve_path(f))
        .collect::<Result<Vec<_>>>()?;

    // Call dvs-core add
    let results = dvs_core::add(&resolved_files, message.as_deref())?;

    // Output results
    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;

    for result in &results {
        match result.outcome {
            dvs_core::Outcome::Copied => {
                success_count += 1;
                output.success(&format!("Added: {}", result.relative_path.display()));
            }
            dvs_core::Outcome::Present => {
                skip_count += 1;
                output.info(&format!("Already tracked: {}", result.relative_path.display()));
            }
            dvs_core::Outcome::Error => {
                error_count += 1;
                let msg = result.error_message.as_deref().unwrap_or("unknown error");
                output.error(&format!("Error: {} - {}", result.relative_path.display(), msg));
            }
        }
    }

    // Summary
    if !output.is_quiet() {
        output.info(&format!(
            "Summary: {} added, {} already tracked, {} errors",
            success_count, skip_count, error_count
        ));
    }

    if error_count > 0 {
        return Err(super::CliError::InvalidArg(format!(
            "{} file(s) failed to add",
            error_count
        )));
    }

    Ok(())
}
