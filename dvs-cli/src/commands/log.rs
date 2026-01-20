//! `dvs log` command implementation.

use super::Result;
use crate::output::Output;
use dvs_core::types::ReflogEntry;

/// Run the log command.
pub fn run(output: &Output, limit: Option<usize>) -> Result<()> {
    let entries = dvs_core::log(limit)?;

    if entries.is_empty() {
        output.println("No reflog entries.");
        return Ok(());
    }

    for entry in entries {
        let e = &entry.entry;

        // Format: @{N} <op> <state> <timestamp> [message]
        let state_short = ReflogEntry::parse_state_id(&e.new)
            .map(|s| &s[..8.min(s.len())])
            .unwrap_or(&e.new[..8.min(e.new.len())]);

        let ts = e.ts.format("%Y-%m-%d %H:%M:%S");

        let line = if let Some(ref msg) = e.message {
            format!(
                "@{{{}}}: {} {} {} - {}",
                entry.index, e.op, state_short, ts, msg
            )
        } else {
            format!("@{{{}}}: {} {} {}", entry.index, e.op, state_short, ts)
        };

        output.println(&line);

        // Show affected paths if any
        if !e.paths.is_empty() {
            for path in &e.paths {
                output.println(&format!("    {}", path.display()));
            }
        }
    }

    Ok(())
}
