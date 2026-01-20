//! `dvs rollback` command implementation.

use super::Result;
use crate::output::Output;
use dvs_core::RollbackTarget;

/// Run the rollback command.
pub fn run(output: &Output, target: String, force: bool, materialize: bool) -> Result<()> {
    let target = RollbackTarget::parse(&target);

    output.println(&format!(
        "Rolling back to {}...",
        match &target {
            RollbackTarget::StateId(id) => format!("state {}", &id[..8.min(id.len())]),
            RollbackTarget::Index(i) => format!("@{{{}}}", i),
        }
    ));

    let result = dvs_core::rollback(target, force, materialize)?;

    if result.success {
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
