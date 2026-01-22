//! DVS push command.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for push command.
#[derive(Serialize)]
struct PushOutput {
    objects: Vec<PushObjectEntry>,
    summary: PushSummary,
}

#[derive(Serialize)]
struct PushObjectEntry {
    oid: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PushSummary {
    uploaded: usize,
    already_present: usize,
    failed: usize,
}

/// Run the push command.
pub fn run(output: &Output, remote: Option<String>, files: Vec<PathBuf>, batch: bool) -> Result<()> {
    // Collect files from args or stdin (batch mode)
    let files = paths::collect_files(files, batch)?;

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

    // Collect results for JSON
    let mut object_entries = Vec::new();

    // Create progress bar for processing results
    let pb = output.file_progress(summary.results.len() as u64);

    for result in &summary.results {
        pb.inc(1);
        let (outcome, error) = if result.is_error() {
            ("error", result.error.clone())
        } else if result.uploaded {
            ("uploaded", None)
        } else {
            ("already_present", None)
        };

        object_entries.push(PushObjectEntry {
            oid: result.oid.to_string(),
            outcome: outcome.to_string(),
            error,
        });

        // Human-readable output
        if !output.is_json() {
            if result.is_error() {
                let msg = result.error.as_deref().unwrap_or("unknown error");
                output.error(&format!("Error pushing {}: {}", result.oid, msg));
            } else if result.uploaded {
                output.success(&format!("Uploaded: {}", result.oid));
            } else {
                output.info(&format!("Already present: {}", result.oid));
            }
        }
    }

    pb.finish_and_clear();

    // Output
    if output.is_json() {
        let json_output = PushOutput {
            objects: object_entries,
            summary: PushSummary {
                uploaded: summary.uploaded,
                already_present: summary.present,
                failed: summary.failed,
            },
        };
        output.json(&json_output);
    } else if !output.is_quiet() {
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
