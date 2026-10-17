//! Rendering `dvs get` results: human text, the JSON envelope, and exit codes.

use std::path::Path;

use dvs::{Action, GetCode, GetOutcome, GetReport, GetRow, StoredCopy, format_size};
use serde_json::json;

/// Exit codes for `get`. Usage and configuration errors exit 2 (clap and
/// `main`), before any of this runs.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_CONFLICTS: i32 = 3;

pub fn exit_code(report: &GetReport) -> i32 {
    if report.has_failures() {
        EXIT_FAILED
    } else if report.has_conflicts() {
        EXIT_CONFLICTS
    } else {
        EXIT_OK
    }
}

/// Quote a path for a shell command line when it needs it.
fn shell_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        s.into_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// The next command to run for a row, in CLI syntax.
pub fn hint(row: &GetRow) -> Option<String> {
    let p = shell_path(&row.local_path);
    match row.code? {
        GetCode::LocalChanges | GetCode::UnknownLocal => Some(match row.stored_copy {
            Some(StoredCopy::Present) => format!(
                "a copy of your version appears to be stored; to replace it: dvs get --force {p}"
            ),
            _ => format!("keep your version: dvs add {p}; discard it: dvs get --force {p}"),
        }),
        GetCode::NotTracked => Some("check the path; list tracked files with: dvs status".into()),
        GetCode::SourceMissing => Some(
            "the upload of this version may have been interrupted; ask whoever added it to run dvs add again"
                .into(),
        ),
        GetCode::AccessDenied => Some("check you are logged in with access: dvs info, dvs login".into()),
        GetCode::DestinationChanged => Some(format!("run again once nothing is writing {p}")),
        GetCode::FileInUse => Some("close the program using the file, then run dvs get again".into()),
        _ => None,
    }
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

/// The non-zero counts, in a fixed order: "restored 2, 3 already current".
fn counts_phrase(report: &GetReport) -> String {
    let c = &report.counts;
    let parts = [
        (c.restored, "restored {}"),
        (c.replaced, "replaced {}"),
        (c.would_restore, "would restore {}"),
        (c.would_replace, "would replace {}"),
        (c.unchanged, "{} already current"),
        (c.skipped, "skipped {} with local changes"),
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

fn summary(report: &GetReport) -> String {
    let conflicts = report
        .files
        .iter()
        .filter(|r| matches!(r.code, Some(GetCode::LocalChanges | GetCode::UnknownLocal)))
        .count();
    let what = match report.outcome {
        GetOutcome::Complete
            if report.counts.restored
                + report.counts.replaced
                + report.counts.would_restore
                + report.counts.would_replace
                == 0 =>
        {
            format!("nothing to do, {} already current", report.counts.unchanged)
        }
        GetOutcome::Complete => counts_phrase(report),
        GetOutcome::Aborted => {
            let mut reasons = Vec::new();
            if conflicts > 0 {
                reasons.push(format!(
                    "{conflicts} file{} {} local changes",
                    if conflicts == 1 { "" } else { "s" },
                    if conflicts == 1 { "has" } else { "have" }
                ));
            }
            if report.counts.failed > 0 {
                reasons.push(format!("{} cannot be retrieved", report.counts.failed));
            }
            let dry = if report.dry_run { "would be " } else { "" };
            format!("aborted, nothing {dry}changed: {}", reasons.join(", "))
        }
        GetOutcome::Partial | GetOutcome::Failed => {
            format!("{}: {}", report.outcome.as_str(), counts_phrase(report))
        }
    };
    format!(
        "dvs get: {what} (on_conflict = {}, {})",
        report.on_conflict, report.source
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
            (true, None) => r.size.map(format_size).unwrap_or_default(),
            (_, Some(m)) => m.clone(),
            (false, None) => String::new(),
        };
        let extra = match (r.code, r.stored_copy) {
            (Some(GetCode::LocalChanges | GetCode::UnknownLocal), Some(StoredCopy::Present)) => {
                " (a copy appears to be stored)"
            }
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

    // Next steps for conflicts, in one place rather than per row.
    let conflicts: Vec<&GetRow> = report
        .files
        .iter()
        .filter(|r| matches!(r.code, Some(GetCode::LocalChanges | GetCode::UnknownLocal)))
        .collect();
    if !conflicts.is_empty() {
        let paths = conflicts
            .iter()
            .map(|r| shell_path(&r.local_path))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("  keep your changes:     dvs add {paths}");
        let held_back = report
            .files
            .iter()
            .any(|r| r.code == Some(GetCode::BatchAborted));
        if held_back && report.counts.failed == 0 {
            eprintln!("  get the other files:   rerun with --on-conflict skip");
        }
        eprintln!("  discard your changes:  dvs get --force {paths}");
    }
    for r in report
        .files
        .iter()
        .filter(|r| !r.satisfied && !r.code.is_some_and(GetCode::is_conflict))
    {
        if let Some(h) = hint(r) {
            eprintln!("  {}: {h}", r.local_path.display());
        }
    }
}

pub fn to_json(report: &GetReport) -> serde_json::Value {
    let files: Vec<_> = report
        .files
        .iter()
        .map(|r| {
            let mut v = serde_json::to_value(r).expect("rows serialize");
            v["hint"] = json!(hint(r));
            v
        })
        .collect();
    json!({
        "v": 1,
        "command": "get",
        "outcome": report.outcome,
        "dry_run": report.dry_run,
        "on_conflict": report.on_conflict,
        "source": report.source,
        "counts": report.counts,
        "problem": report.problem,
        "files": files,
    })
}
