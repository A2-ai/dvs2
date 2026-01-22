//! DVS get command.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for get command.
#[derive(Serialize)]
struct GetOutput {
    files: Vec<GetFileEntry>,
    summary: GetSummary,
}

#[derive(Serialize)]
struct GetFileEntry {
    path: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct GetSummary {
    retrieved: usize,
    up_to_date: usize,
    errors: usize,
}

/// Run the get command.
pub fn run(output: &Output, files: Vec<PathBuf>, batch: bool) -> Result<()> {
    // Collect files from args or stdin (batch mode)
    let files = paths::collect_files(files, batch)?;

    if files.is_empty() {
        return Err(super::CliError::InvalidArg(
            "No files specified. Provide files as arguments or use --batch to read from stdin."
                .to_string(),
        ));
    }

    // Resolve all file paths
    let resolved_files: Vec<PathBuf> = files
        .iter()
        .map(|f| paths::resolve_path(f))
        .collect::<Result<Vec<_>>>()?;

    // Call dvs-core get
    let results = dvs_core::get(&resolved_files)?;

    // Collect results
    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;
    let mut file_entries = Vec::new();

    // Create progress bar for processing results
    let pb = output.file_progress(results.len() as u64);

    for result in &results {
        pb.inc(1);
        let (outcome_str, error) = match result.outcome {
            dvs_core::Outcome::Copied => {
                success_count += 1;
                ("retrieved", None)
            }
            dvs_core::Outcome::Present => {
                skip_count += 1;
                ("up_to_date", None)
            }
            dvs_core::Outcome::Error => {
                error_count += 1;
                ("error", result.error_message.clone())
            }
        };

        file_entries.push(GetFileEntry {
            path: result.relative_path.display().to_string(),
            outcome: outcome_str.to_string(),
            error,
        });

        // Human-readable output
        if !output.is_json() {
            match result.outcome {
                dvs_core::Outcome::Copied => {
                    output.success(&format!("Retrieved: {}", result.relative_path.display()));
                }
                dvs_core::Outcome::Present => {
                    output.info(&format!("Up to date: {}", result.relative_path.display()));
                }
                dvs_core::Outcome::Error => {
                    let msg = result.error_message.as_deref().unwrap_or("unknown error");
                    output.error(&format!(
                        "Error: {} - {}",
                        result.relative_path.display(),
                        msg
                    ));
                }
            }
        }
    }

    pb.finish_and_clear();

    // Output
    if output.is_json() {
        let json_output = GetOutput {
            files: file_entries,
            summary: GetSummary {
                retrieved: success_count,
                up_to_date: skip_count,
                errors: error_count,
            },
        };
        output.json(&json_output);
    } else if !output.is_quiet() {
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
