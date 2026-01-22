//! DVS pull command.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for pull command.
#[derive(Serialize)]
struct PullOutput {
    objects: Vec<PullObjectEntry>,
    summary: PullSummary,
}

#[derive(Serialize)]
struct PullObjectEntry {
    oid: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct PullSummary {
    downloaded: usize,
    already_cached: usize,
    failed: usize,
}

/// Run the pull command.
pub fn run(
    output: &Output,
    remote: Option<String>,
    files: Vec<PathBuf>,
    batch: bool,
) -> Result<()> {
    // Collect files from args or stdin (batch mode)
    let files = paths::collect_files(files, batch)?;

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

    // Collect results for JSON
    let mut object_entries = Vec::new();

    // Create progress bar for processing results
    let pb = output.file_progress(summary.results.len() as u64);

    for result in &summary.results {
        pb.inc(1);
        let (outcome, error) = if result.is_error() {
            ("error", result.error.clone())
        } else if result.downloaded {
            ("downloaded", None)
        } else {
            ("already_cached", None)
        };

        object_entries.push(PullObjectEntry {
            oid: result.oid.to_string(),
            outcome: outcome.to_string(),
            error,
        });

        // Human-readable output
        if !output.is_json() {
            if result.is_error() {
                let msg = result.error.as_deref().unwrap_or("unknown error");
                output.error(&format!("Error pulling {}: {}", result.oid, msg));
            } else if result.downloaded {
                output.success(&format!("Downloaded: {}", result.oid));
            } else {
                output.info(&format!("Already cached: {}", result.oid));
            }
        }
    }

    pb.finish_and_clear();

    // Output
    if output.is_json() {
        let json_output = PullOutput {
            objects: object_entries,
            summary: PullSummary {
                downloaded: summary.downloaded,
                already_cached: summary.cached,
                failed: summary.failed,
            },
        };
        output.json(&json_output);
    } else if !output.is_quiet() {
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
