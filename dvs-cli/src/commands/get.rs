//! DVS get command.

use std::path::PathBuf;

use crate::output::Output;
use crate::paths;
use super::Result;

/// Run the get command.
pub fn run(
    output: &Output,
    files: Vec<PathBuf>,
) -> Result<()> {
    // Resolve all file paths
    let resolved_files: Vec<PathBuf> = files
        .iter()
        .map(|f| paths::resolve_path(f))
        .collect::<Result<Vec<_>>>()?;

    // Call dvs-core get
    let results = dvs_core::get(&resolved_files)?;

    // Output results
    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;

    for result in &results {
        match result.outcome {
            dvs_core::Outcome::Copied => {
                success_count += 1;
                output.success(&format!("Retrieved: {}", result.relative_path.display()));
            }
            dvs_core::Outcome::Present => {
                skip_count += 1;
                output.info(&format!("Up to date: {}", result.relative_path.display()));
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
            "Summary: {} retrieved, {} up-to-date, {} errors",
            success_count, skip_count, error_count
        ));
    }

    if error_count > 0 {
        return Err(super::CliError::InvalidArg(format!(
            "{} file(s) failed to retrieve",
            error_count
        )));
    }

    Ok(())
}
