//! DVS materialize command.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use crate::paths;

/// JSON output for materialize command.
#[derive(Serialize)]
struct MaterializeOutput {
    files: Vec<MaterializeFileEntry>,
    summary: MaterializeSummary,
}

#[derive(Serialize)]
struct MaterializeFileEntry {
    path: String,
    outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct MaterializeSummary {
    materialized: usize,
    up_to_date: usize,
    failed: usize,
}

/// Run the materialize command.
pub fn run(output: &Output, files: Vec<PathBuf>) -> Result<()> {
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

    // Collect results for JSON
    let mut file_entries = Vec::new();

    for result in &summary.results {
        let (outcome, error) = if result.is_error() {
            ("error", result.error.clone())
        } else if result.materialized {
            ("materialized", None)
        } else {
            ("up_to_date", None)
        };

        file_entries.push(MaterializeFileEntry {
            path: result.path.display().to_string(),
            outcome: outcome.to_string(),
            error,
        });

        // Human-readable output
        if !output.is_json() {
            if result.is_error() {
                let msg = result.error.as_deref().unwrap_or("unknown error");
                output.error(&format!(
                    "Error materializing {}: {}",
                    result.path.display(),
                    msg
                ));
            } else if result.materialized {
                output.success(&format!("Materialized: {}", result.path.display()));
            } else {
                output.info(&format!("Up to date: {}", result.path.display()));
            }
        }
    }

    // Output
    if output.is_json() {
        let json_output = MaterializeOutput {
            files: file_entries,
            summary: MaterializeSummary {
                materialized: summary.materialized,
                up_to_date: summary.up_to_date,
                failed: summary.failed,
            },
        };
        output.json(&json_output);
    } else if !output.is_quiet() {
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
