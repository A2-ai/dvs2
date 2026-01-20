//! `dvs merge-repo` command implementation.

use serde::Serialize;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;
use dvs_core::{ConflictMode, MergeOptions, MergeResult};

/// JSON output for merge-repo command.
#[derive(Serialize)]
struct MergeRepoOutput {
    dry_run: bool,
    files_merged: usize,
    files_skipped: usize,
    objects_copied: usize,
    objects_existed: usize,
    merged_paths: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<String>,
}

/// Options for the merge-repo command.
pub struct MergeRepoOptions {
    /// Path to source DVS repository.
    pub source: PathBuf,
    /// Optional prefix for imported files.
    pub prefix: Option<PathBuf>,
    /// Conflict handling mode.
    pub conflict_mode: ConflictMode,
    /// Verify object hashes during copy.
    pub verify: bool,
    /// Show what would be merged without making changes.
    pub dry_run: bool,
}

/// Run the merge-repo command.
pub fn run(output: &Output, options: MergeRepoOptions) -> Result<()> {
    let merge_options = MergeOptions {
        prefix: options.prefix.clone(),
        conflict_mode: options.conflict_mode,
        verify: options.verify,
        dry_run: options.dry_run,
    };

    if !output.is_json() {
        if options.dry_run {
            output.println("Dry run - no changes will be made:");
        }
        output.println(&format!("Merging from {}...", options.source.display()));
    }

    let result = dvs_core::merge_repo(&options.source, merge_options)?;

    if output.is_json() {
        output.json(&MergeRepoOutput {
            dry_run: options.dry_run,
            files_merged: result.files_merged,
            files_skipped: result.files_skipped,
            objects_copied: result.objects_copied,
            objects_existed: result.objects_existed,
            merged_paths: result
                .merged_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            conflicts: result
                .conflicts
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        });
    } else {
        display_result(output, &result, options.dry_run);
    }

    Ok(())
}

fn display_result(output: &Output, result: &MergeResult, dry_run: bool) {
    let action = if dry_run { "Would merge" } else { "Merged" };

    if result.files_merged > 0 {
        output.println(&format!("{} {} file(s):", action, result.files_merged));
        for path in &result.merged_paths {
            output.println(&format!("  + {}", path.display()));
        }
    }

    if result.files_skipped > 0 {
        output.println(&format!(
            "Skipped {} file(s) (conflicts)",
            result.files_skipped
        ));
    }

    if !dry_run && (result.objects_copied > 0 || result.objects_existed > 0) {
        output.println(&format!(
            "Objects: {} copied, {} already existed",
            result.objects_copied, result.objects_existed
        ));
    }

    if result.files_merged == 0 && result.files_skipped == 0 {
        output.println("No files to merge.");
    } else if !dry_run && result.files_merged > 0 {
        output.println("Merge complete.");
    }
}
