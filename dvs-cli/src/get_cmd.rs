//! Rendering `dvs get` results: human text and the JSON envelope.

use dvs::{
    Action, Before, ErrorCode, GetCode, GetOutcome, GetReport, GetRow, StoredCopy, format_size,
    shell_path,
};
use serde_json::json;

/// Failures with one cause shared by every affected file (no access, server
/// down): their message and hint are printed once, not per file.
fn is_shared(code: Option<GetCode>) -> bool {
    matches!(
        code,
        Some(GetCode::Shared(
            ErrorCode::AccessDenied | ErrorCode::ServerUnavailable
        ))
    )
}

fn label(action: Action) -> &'static str {
    match action {
        Action::Restored => "restored",
        Action::Replaced => "replaced",
        Action::Unchanged => "unchanged",
        Action::Skipped => "skipped",
        Action::NotRun => "not run",
        Action::Cancelled => "cancelled",
        Action::Failed => "failed",
        Action::WouldRestore => "would restore",
        Action::WouldReplace => "would replace",
    }
}

/// Rows listed one per line: every file that changed or needs attention.
/// Cancelled files are only counted, as add and status do.
fn is_listed(row: &GetRow) -> bool {
    !matches!(row.action, Action::Unchanged | Action::Cancelled)
}

/// Print the report for people: one line per file that changed or needs
/// attention, a summary, and what to do next.
pub fn print_human(report: &GetReport) {
    if let Some(problem) = &report.problem {
        eprintln!("dvs get: {problem}");
        return;
    }
    let width = report
        .files
        .iter()
        .filter(|r| is_listed(r))
        .map(|r| r.local_path.to_string_lossy().chars().count())
        .max()
        .unwrap_or(0);
    let listed = report.files.iter().any(is_listed);
    for r in report.files.iter().filter(|r| is_listed(r)) {
        let detail = match (r.satisfied, &r.message) {
            _ if is_shared(r.code) => String::new(),
            (true, None) => r.size.map(format_size).unwrap_or_default(),
            (_, Some(m)) => m.clone(),
            (false, None) => String::new(),
        };
        let extra = match (r.is_conflict(), r.stored_copy) {
            (true, Some(StoredCopy::Present)) => " (a copy of your version appears to be stored)",
            _ => "",
        };
        let line = format!(
            "{:<13} {:<width$}  {detail}{extra}",
            label(r.action),
            r.local_path.display()
        );
        if r.satisfied {
            println!("{}", line.trim_end());
        } else {
            eprintln!("{}", line.trim_end());
        }
    }
    // Set apart from the file lines, like add's
    let summary = format!("{}{}", if listed { "\n" } else { "" }, report.summary());
    if report.outcome == GetOutcome::Complete {
        println!("{summary}");
    } else {
        eprintln!("{summary}");
    }

    let mut shown: Vec<GetCode> = Vec::new();
    for r in report.files.iter().filter(|r| is_shared(r.code)) {
        let code = r.code.expect("shared rows have a code");
        if shown.contains(&code) {
            continue;
        }
        shown.push(code);
        if let Some(m) = &r.message {
            eprintln!("  {m}");
        }
        if let Some(h) = &r.hint {
            eprintln!("  {h}");
        }
    }

    // Next steps for conflicts, in one place rather than per row.
    let conflicts: Vec<&GetRow> = report.files.iter().filter(|r| r.is_conflict()).collect();
    if !conflicts.is_empty() {
        let paths = conflicts
            .iter()
            .map(|r| shell_path(&r.local_path))
            .collect::<Vec<_>>()
            .join(" ");
        if conflicts.iter().any(|r| r.before == Before::Unknown) {
            eprintln!(
                "  no record usually means a clone from before dvs recorded what it writes, or a cleared cache"
            );
        }
        eprintln!("  keep your version:         dvs add {paths}   (makes it the tracked version)");
        let held_back = report
            .files
            .iter()
            .any(|r| r.code == Some(GetCode::BatchAborted));
        if held_back && report.counts.failed == 0 {
            eprintln!("  get the other files:       rerun with --on-conflict skip");
        }
        eprintln!(
            "  take the tracked version:  dvs get --force {paths}   (replaces yours; copy it first to keep both)"
        );
    }
    for r in report.files.iter().filter(|r| {
        !r.satisfied
            && !r.is_conflict()
            && r.code != Some(GetCode::BatchAborted)
            && !is_shared(r.code)
    }) {
        if let Some(h) = &r.hint {
            eprintln!("  {}: {h}", r.local_path.display());
        }
    }
}

pub fn to_json(report: &GetReport) -> serde_json::Value {
    json!({
        "v": 1,
        "command": "get",
        "outcome": report.outcome,
        "dry_run": report.dry_run,
        "on_conflict": report.on_conflict,
        "source": report.source,
        "counts": report.counts,
        "bytes": report.bytes,
        "error": report
            .problem
            .as_ref()
            .map(|p| crate::output::error_json(p.code, p.message.as_str())),
        "files": report.files,
    })
}
