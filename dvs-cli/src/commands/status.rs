//! DVS status command.

use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// Run the status command.
pub fn run(output: &Output, files: Vec<PathBuf>) -> Result<()> {
    // Resolve file paths (empty means all tracked files)
    let resolved_files: Vec<PathBuf> = if files.is_empty() {
        Vec::new()
    } else {
        files
            .iter()
            .map(|f| paths::resolve_path(f))
            .collect::<Result<Vec<_>>>()?
    };

    // Call dvs-core status
    let results = dvs_core::status(&resolved_files)?;

    // Output results grouped by status
    let mut current_count = 0;
    let mut absent_count = 0;
    let mut unsynced_count = 0;
    let mut error_count = 0;

    for result in &results {
        match result.status {
            dvs_core::FileStatus::Current => {
                current_count += 1;
                if !output.is_quiet() {
                    output.println(&format!("  current: {}", result.relative_path.display()));
                }
            }
            dvs_core::FileStatus::Unsynced => {
                unsynced_count += 1;
                output.warn(&format!(" unsynced: {}", result.relative_path.display()));
            }
            dvs_core::FileStatus::Absent => {
                absent_count += 1;
                output.warn(&format!("   absent: {}", result.relative_path.display()));
            }
            dvs_core::FileStatus::Error => {
                error_count += 1;
                let msg = result.error_message.as_deref().unwrap_or("unknown error");
                output.error(&format!(
                    "    error: {} - {}",
                    result.relative_path.display(),
                    msg
                ));
            }
        }
    }

    // Summary
    output.info(&format!(
        "Status: {} current, {} unsynced, {} absent, {} errors",
        current_count, unsynced_count, absent_count, error_count
    ));

    Ok(())
}
