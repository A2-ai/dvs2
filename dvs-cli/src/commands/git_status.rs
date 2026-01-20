//! DVS git-status command.
//!
//! Runs `git status` followed by `dvs status`.

use std::process::Command;

use super::Result;
use crate::output::Output;

/// Run the git-status command.
///
/// This command:
/// 1. Runs `git status` with any provided args
/// 2. Runs `dvs status`
/// 3. Returns non-zero if either command fails
pub fn run(output: &Output, git_args: Vec<String>) -> Result<()> {
    let mut any_failed = false;

    // Run git status
    let git_result = Command::new("git").arg("status").args(&git_args).status();

    match git_result {
        Ok(status) => {
            if !status.success() {
                any_failed = true;
            }
        }
        Err(e) => {
            output.error(&format!("Failed to run git status: {}", e));
            any_failed = true;
        }
    }

    // Print separator
    if !output.is_quiet() {
        output.println("");
        output.println("--- DVS Status ---");
    }

    // Run dvs status (internal)
    match super::status::run(output, vec![]) {
        Ok(()) => {}
        Err(e) => {
            output.error(&format!("DVS status error: {}", e));
            any_failed = true;
        }
    }

    if any_failed {
        Err(super::CliError::InvalidArg(
            "One or more status commands failed".to_string(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would need a git repo setup
    // For now, just test that the module compiles
}
