//! DVS add command.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for add command.
#[derive(Serialize)]
struct AddOutput {
    files: Vec<AddFileEntry>,
    summary: AddSummary,
}

#[derive(Serialize)]
struct AddFileEntry {
    path: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct AddSummary {
    added: usize,
    already_tracked: usize,
    errors: usize,
}

/// Run the add command.
pub fn run(
    output: &Output,
    files: Vec<PathBuf>,
    message: Option<String>,
    metadata_format: Option<String>,
    batch: bool,
) -> Result<()> {
    // Collect files from args or stdin (batch mode)
    let files = paths::collect_files(files, batch)?;

    if files.is_empty() {
        return Err(super::CliError::InvalidArg(
            "No files specified. Provide files as arguments or use --batch to read from stdin."
                .to_string(),
        ));
    }

    // Parse and validate metadata format if provided
    let format_override = if let Some(ref fmt) = metadata_format {
        match dvs_core::MetadataFormat::from_str(fmt) {
            Some(format) => Some(format),
            None => {
                return Err(super::CliError::InvalidArg(format!(
                    "Invalid metadata format '{}'. Use 'json' or 'toml'.",
                    fmt
                )));
            }
        }
    } else {
        None
    };

    // Resolve all file paths
    let resolved_files: Vec<PathBuf> = files
        .iter()
        .map(|f| paths::resolve_path(f))
        .collect::<Result<Vec<_>>>()?;

    // Call dvs-core add with format override if specified
    let results = dvs_core::add_with_format(&resolved_files, message.as_deref(), format_override)?;

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
                ("added", None)
            }
            dvs_core::Outcome::Present => {
                skip_count += 1;
                ("already_tracked", None)
            }
            dvs_core::Outcome::Error => {
                error_count += 1;
                ("error", result.error_message.clone())
            }
        };

        file_entries.push(AddFileEntry {
            path: result.relative_path.display().to_string(),
            outcome: outcome_str.to_string(),
            error,
        });

        // Human-readable output
        if !output.is_json() {
            match result.outcome {
                dvs_core::Outcome::Copied => {
                    output.success(&format!("Added: {}", result.relative_path.display()));
                }
                dvs_core::Outcome::Present => {
                    output.info(&format!(
                        "Already tracked: {}",
                        result.relative_path.display()
                    ));
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
        let json_output = AddOutput {
            files: file_entries,
            summary: AddSummary {
                added: success_count,
                already_tracked: skip_count,
                errors: error_count,
            },
        };
        output.json(&json_output);
    } else if !output.is_quiet() {
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
