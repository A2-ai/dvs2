//! Rendering `dvs get` results: human text, the JSON envelope, and exit codes.

use dvs::config::{OnConflict, SettingSource};
use dvs::{
    Action, Before, GetCode, GetOutcome, GetReport, GetRow, GetVerdict, StoredCopy, format_size,
    shell_path,
};
use serde_json::json;

/// Exit codes for `get`. Usage and configuration errors exit 2 (clap and
/// `main`).
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CONFLICTS: i32 = 3;

pub fn exit_code(report: &GetReport) -> i32 {
    match report.verdict() {
        GetVerdict::Done => EXIT_OK,
        GetVerdict::Failed => EXIT_FAILED,
        GetVerdict::HeldBack => EXIT_CONFLICTS,
    }
}

/// Failures with one cause shared by every affected file (no access, server
/// down): their message and hint are printed once, not per file.
fn is_shared(code: Option<GetCode>) -> bool {
    matches!(
        code,
        Some(GetCode::AccessDenied | GetCode::ServerUnavailable)
    )
}

fn label(action: Action) -> &'static str {
    match action {
        Action::Restored => "restored",
        Action::Replaced => "replaced",
        Action::Unchanged => "unchanged",
        Action::Skipped => "skipped",
        Action::NotRun => "not run",
        Action::Failed => "failed",
        Action::WouldRestore => "would restore",
        Action::WouldReplace => "would replace",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The non-zero counts, in a fixed order: "restored 2, 3 already current".
fn counts_phrase(report: &GetReport) -> String {
    let c = &report.counts;
    let parts = [
        (c.restored, "restored {}"),
        (c.replaced, "replaced {}"),
        (c.would_restore, "would restore {}"),
        (c.would_replace, "would replace {}"),
        (c.unchanged, "{} already current"),
        (c.skipped, "skipped {}"),
        (c.not_run, "{} held back"),
        (c.failed, "{} failed"),
    ];
    let out: Vec<String> = parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, f)| f.replace("{}", &n.to_string()))
        .collect();
    if out.is_empty() {
        "nothing to do".to_string()
    } else {
        out.join(", ")
    }
}

/// Why conflicts stopped things, separating edits from files with no record.
fn conflict_reasons(report: &GetReport) -> Vec<String> {
    let count = |b: Before| {
        report
            .files
            .iter()
            .filter(|r| r.is_conflict() && r.before == b)
            .count()
    };
    let (modified, unknown) = (count(Before::Modified), count(Before::Unknown));
    let mut reasons = Vec::new();
    if modified > 0 {
        reasons.push(format!(
            "{} local changes",
            plural(modified, "file has", "files have")
        ));
    }
    if unknown > 0 {
        reasons.push(format!(
            "{} from the tracked version and dvs has no record of writing {}",
            plural(unknown, "file differs", "files differ"),
            if unknown == 1 { "it" } else { "them" }
        ));
    }
    reasons
}

fn summary(report: &GetReport) -> String {
    let c = &report.counts;
    let what = match report.outcome {
        GetOutcome::Complete
            if c.restored + c.replaced + c.would_restore + c.would_replace == 0 =>
        {
            format!("nothing to do, {} already current", c.unchanged)
        }
        GetOutcome::Complete => counts_phrase(report),
        GetOutcome::Aborted => {
            let mut reasons = conflict_reasons(report);
            if c.failed > 0 {
                reasons.push(format!(
                    "{} cannot be retrieved",
                    plural(c.failed, "file", "files")
                ));
            }
            let dry = if report.dry_run { "would be " } else { "" };
            format!("aborted, nothing {dry}changed: {}", reasons.join("; "))
        }
        GetOutcome::Partial | GetOutcome::Failed => {
            format!("{}: {}", report.outcome.as_str(), counts_phrase(report))
        }
    };
    format!(
        "dvs get{}: {what} (on_conflict = {}, {})",
        if report.dry_run { " (dry run)" } else { "" },
        report.on_conflict,
        report.source
    )
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
        .filter(|r| r.action != Action::Unchanged)
        .map(|r| r.local_path.to_string_lossy().chars().count())
        .max()
        .unwrap_or(0);
    for r in report
        .files
        .iter()
        .filter(|r| r.action != Action::Unchanged)
    {
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
    let summary = summary(report);
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
        "problem": report.problem,
        "files": report.files,
    })
}

/// The envelope for a get that could not start (not in a project, invalid
/// configuration): zero rows, the reason in `problem`, and no policy.
pub fn problem_json(problem: &str, dry_run: bool) -> serde_json::Value {
    let report = GetReport::problem(problem, dry_run, OnConflict::Abort, SettingSource::Default);
    let mut v = to_json(&report);
    v["on_conflict"] = serde_json::Value::Null;
    v["source"] = serde_json::Value::Null;
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs::GetCounts;

    #[test]
    fn aborted_summary_names_files_that_cannot_be_retrieved() {
        let mut report = GetReport::problem("", false, OnConflict::Abort, SettingSource::Default);
        report.problem = None;
        report.outcome = GetOutcome::Aborted;
        report.counts = GetCounts {
            failed: 1,
            ..GetCounts::default()
        };
        assert!(summary(&report).contains("1 file cannot be retrieved"));
        report.counts.failed = 2;
        assert!(summary(&report).contains("2 files cannot be retrieved"));
    }
}
