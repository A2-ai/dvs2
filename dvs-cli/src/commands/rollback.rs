//! `dvs rollback` command implementation.

use serde::Serialize;

use super::Result;
use crate::output::Output;
use dvs_core::RollbackTarget;

/// JSON output for rollback command.
#[derive(Serialize)]
struct RollbackOutput {
    success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_state: Option<String>,
    to_state: String,
    restored_files: Vec<String>,
    removed_files: Vec<String>,
}

/// Run the rollback command.
pub fn run(output: &Output, target: String, force: bool, materialize: bool) -> Result<()> {
    let target = RollbackTarget::parse(&target);

    if !output.is_json() {
        output.println(&format!(
            "Rolling back to {}...",
            match &target {
                RollbackTarget::StateId(id) => format!("state {}", &id[..8.min(id.len())]),
                RollbackTarget::Index(i) => format!("@{{{}}}", i),
            }
        ));
    }

    let result = dvs_core::rollback(target, force, materialize)?;

    if output.is_json() {
        output.json(&RollbackOutput {
            success: result.success,
            from_state: result.from_state.as_ref().cloned(),
            to_state: result.to_state.clone(),
            restored_files: result
                .restored_files
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            removed_files: result
                .removed_files
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        });
    } else if result.success {
        let to_short = &result.to_state[..8.min(result.to_state.len())];

        if !result.restored_files.is_empty() {
            output.println(&format!(
                "Restored {} file(s):",
                result.restored_files.len()
            ));
            for path in &result.restored_files {
                output.println(&format!("  + {}", path.display()));
            }
        }

        if !result.removed_files.is_empty() {
            output.println(&format!("Removed {} file(s):", result.removed_files.len()));
            for path in &result.removed_files {
                output.println(&format!("  - {}", path.display()));
            }
        }

        if result.restored_files.is_empty() && result.removed_files.is_empty() {
            output.println("Already at target state.");
        } else {
            output.println(&format!("Rolled back to state {}.", to_short));
        }
    }

    Ok(())
}
