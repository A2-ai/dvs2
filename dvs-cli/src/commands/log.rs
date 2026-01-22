//! `dvs log` command implementation.

use serde::Serialize;
use tabled::Tabled;

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

/// Table row for log output.
#[derive(Tabled)]
struct LogTableRow {
    #[tabled(rename = "Index")]
    index: String,
    #[tabled(rename = "Op")]
    op: String,
    #[tabled(rename = "State")]
    state: String,
    #[tabled(rename = "Timestamp")]
    timestamp: String,
    #[tabled(rename = "Message")]
    message: String,
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

    // Collect entries for JSON/table output
    let mut json_entries = Vec::new();
    let mut table_rows = Vec::new();

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

        table_rows.push(LogTableRow {
            index: format!("@{{{}}}", entry.index),
            op: e.op.to_string(),
            state: state_short.to_string(),
            timestamp: ts.clone(),
            message: e.message.clone().unwrap_or_default(),
        });

        // Human-readable output (skip for table format)
        if !output.is_json() && !output.is_table() {
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

    // Output based on format
    if output.is_json() {
        output.json(&LogOutput {
            entries: json_entries,
        });
    } else if output.is_table() {
        output.table(&table_rows);
    }

    Ok(())
}
