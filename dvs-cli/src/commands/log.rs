//! `dvs log` command implementation.

use serde::Serialize;

use super::Result;
use crate::output::Output;
use dvs_core::types::ReflogEntry;

/// JSON output for log command.
#[derive(Serialize)]
struct LogOutput {
    entries: Vec<LogEntryJson>,
}

#[derive(Serialize)]
struct LogEntryJson {
    index: usize,
    op: String,
    state: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    paths: Vec<String>,
}

/// Run the log command.
pub fn run(output: &Output, limit: Option<usize>) -> Result<()> {
    let entries = dvs_core::log(limit)?;

    if entries.is_empty() {
        if output.is_json() {
            output.json(&LogOutput { entries: vec![] });
        } else {
            output.println("No reflog entries.");
        }
        return Ok(());
    }

    // Collect entries for JSON output
    let mut json_entries = Vec::new();

    for entry in &entries {
        let e = &entry.entry;

        // Format: @{N} <op> <state> <timestamp> [message]
        let state_short = ReflogEntry::parse_state_id(&e.new)
            .map(|s| &s[..8.min(s.len())])
            .unwrap_or(&e.new[..8.min(e.new.len())]);

        let ts = e.ts.format("%Y-%m-%d %H:%M:%S").to_string();

        json_entries.push(LogEntryJson {
            index: entry.index,
            op: e.op.to_string(),
            state: state_short.to_string(),
            timestamp: ts.clone(),
            message: e.message.clone(),
            paths: e.paths.iter().map(|p| p.display().to_string()).collect(),
        });

        // Human-readable output
        if !output.is_json() {
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
    }

    // JSON output
    if output.is_json() {
        output.json(&LogOutput {
            entries: json_entries,
        });
    }

    Ok(())
}
