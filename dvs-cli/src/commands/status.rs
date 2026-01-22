//! DVS status command.

use serde::Serialize;
use std::path::PathBuf;
use tabled::Tabled;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for status command.
#[derive(Serialize)]
struct StatusOutput {
    files: Vec<FileStatusEntry>,
    summary: StatusSummary,
}

#[derive(Serialize)]
struct FileStatusEntry {
    path: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Table row for status output.
#[derive(Tabled)]
struct StatusTableRow {
    #[tabled(rename = "Path")]
    path: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Error")]
    error: String,
}

#[derive(Serialize)]
struct StatusSummary {
    current: usize,
    unsynced: usize,
    absent: usize,
    errors: usize,
}

/// Run the status command.
pub fn run(output: &Output, files: Vec<PathBuf>, batch: bool) -> Result<()> {
    // Collect files from args or stdin (batch mode)
    let files = paths::collect_files(files, batch)?;

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

    // Count results by status
    let mut current_count = 0;
    let mut absent_count = 0;
    let mut unsynced_count = 0;
    let mut error_count = 0;

    // Build file entries for JSON output
    let mut file_entries = Vec::new();

    // Create progress bar for processing results
    let pb = output.file_progress(results.len() as u64);

    for result in &results {
        pb.inc(1);
        let status_str = match result.status {
            dvs_core::FileStatus::Current => {
                current_count += 1;
                "current"
            }
            dvs_core::FileStatus::Unsynced => {
                unsynced_count += 1;
                "unsynced"
            }
            dvs_core::FileStatus::Absent => {
                absent_count += 1;
                "absent"
            }
            dvs_core::FileStatus::Error => {
                error_count += 1;
                "error"
            }
        };

        file_entries.push(FileStatusEntry {
            path: result.relative_path.display().to_string(),
            status: status_str.to_string(),
            error: result.error_message.clone(),
        });

        // Human-readable output (skip for table format since we print table at end)
        if !output.is_json() && !output.is_table() {
            match result.status {
                dvs_core::FileStatus::Current => {
                    if !output.is_quiet() {
                        output.println(&format!("  current: {}", result.relative_path.display()));
                    }
                }
                dvs_core::FileStatus::Unsynced => {
                    output.warn(&format!(" unsynced: {}", result.relative_path.display()));
                }
                dvs_core::FileStatus::Absent => {
                    output.warn(&format!("   absent: {}", result.relative_path.display()));
                }
                dvs_core::FileStatus::Error => {
                    let msg = result.error_message.as_deref().unwrap_or("unknown error");
                    output.error(&format!(
                        "    error: {} - {}",
                        result.relative_path.display(),
                        msg
                    ));
                }
            }
        }
    }

    pb.finish_and_clear();

    // JSON output
    if output.is_json() {
        let json_output = StatusOutput {
            files: file_entries,
            summary: StatusSummary {
                current: current_count,
                unsynced: unsynced_count,
                absent: absent_count,
                errors: error_count,
            },
        };
        output.json(&json_output);
    } else if output.is_table() {
        // Table output
        let table_rows: Vec<StatusTableRow> = file_entries
            .iter()
            .map(|e| StatusTableRow {
                path: e.path.clone(),
                status: e.status.clone(),
                error: e.error.clone().unwrap_or_default(),
            })
            .collect();
        output.table(&table_rows);
        output.info(&format!(
            "\nSummary: {} current, {} unsynced, {} absent, {} errors",
            current_count, unsynced_count, absent_count, error_count
        ));
    } else {
        // Human summary
        output.info(&format!(
            "Status: {} current, {} unsynced, {} absent, {} errors",
            current_count, unsynced_count, absent_count, error_count
        ));
    }

    Ok(())
}
