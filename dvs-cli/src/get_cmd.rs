//! Rendering `dvs get` results: human text, the JSON envelope, and exit codes.

use std::path::Path;

use dvs::config::{OnConflict, SettingSource};
use dvs::{Action, Before, GetCode, GetOutcome, GetReport, GetRow, StoredCopy, format_size};
use serde_json::json;

/// Exit codes. Every command shares one table (`output::exit_code`,
/// specs.md "Machine output"); only `get` uses 3. Usage and configuration errors exit
/// 2 (clap and `main`).
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
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

/// Failures with one cause shared by every affected file (no access, server
/// down): their message and hint are printed once, not per file.
fn is_shared(code: Option<GetCode>) -> bool {
    matches!(
        code,
        Some(GetCode::AccessDenied | GetCode::ServerUnavailable)
    )
}

fn is_conflict_row(row: &GetRow) -> bool {
    matches!(
        row.code,
        Some(GetCode::LocalChanges | GetCode::UnknownLocal)
    )
}

/// The next command to run for a row, in CLI syntax.
pub fn hint(row: &GetRow) -> Option<String> {
    let p = shell_path(&row.local_path);
    let choices = format!(
        "keep your version: dvs add {p} (makes it the tracked version); \
         take the tracked version: dvs get --force {p} (replaces yours)"
    );
    match row.code? {
        GetCode::LocalChanges => Some(choices),
        GetCode::UnknownLocal => Some(format!(
            "dvs has no record of writing this file (a clone from before dvs recorded this, \
             or a cleared cache); {choices}"
        )),
        GetCode::BatchAborted => {
            Some("held back by another file; to get the rest: --on-conflict skip".into())
        }
        GetCode::NotTracked if row.local_path.is_dir() => Some(
            "nothing tracked is selected here; add -r to include subdirectories, or check the path"
                .into(),
        ),
        GetCode::NotTracked => Some("check the path; list tracked files with: dvs status".into()),
        GetCode::SourceMissing => Some(
            "the stored content of this version is missing (probably an interrupted upload); \
             tell whoever added it"
                .into(),
        ),
        GetCode::AccessDenied => {
            Some("check you are logged in with access: dvs info, dvs login".into())
        }
        GetCode::ServerUnavailable => {
            Some("check your network or VPN and the server, then run dvs get again".into())
        }
        GetCode::DestinationChanged => Some(format!("run again once nothing is writing {p}")),
        GetCode::FileInUse => {
            Some("close the program using the file, then run dvs get again".into())
        }
        GetCode::BadMetadata => Some(format!(
            "fix the .dvs file for {p} (for example a git merge conflict), then run dvs get again"
        )),
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
            .filter(|r| is_conflict_row(r) && r.before == b)
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
                reasons.push(format!("{} cannot be retrieved", c.failed));
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
        let extra = match (is_conflict_row(r), r.stored_copy) {
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
        if let Some(h) = hint(r) {
            eprintln!("  {h}");
        }
    }

    // Next steps for conflicts, in one place rather than per row.
    let conflicts: Vec<&GetRow> = report.files.iter().filter(|r| is_conflict_row(r)).collect();
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
    for r in report
        .files
        .iter()
        .filter(|r| !r.satisfied && !r.code.is_some_and(GetCode::is_conflict) && !is_shared(r.code))
    {
        if let Some(h) = hint(r) {
            eprintln!("  {}: {h}", r.local_path.display());
        }
    }
}

/// One row as `get --json` reports it, and as `--emit-events` streams it
/// the moment it is final (C3).
pub fn row_json(r: &GetRow) -> serde_json::Value {
    let mut v = serde_json::to_value(r).expect("rows serialize");
    v["hint"] = json!(hint(r));
    // Not serialized by the library (K8): the CLI's rows carry it (C2)
    v["duration_ms"] = json!(r.duration_ms);
    v
}

/// get's detailed counts plus the `total`/`succeeded` every command has
/// (§9 rec 5). `succeeded` = rows at their target (restored, replaced,
/// unchanged; would_restore/would_replace in a dry run). `failed` stays the
/// `failed` action. Skipped and not_run (held back) rows are in neither.
/// Added here, not in `GetCounts`, so R output is unchanged (K8).
fn counts_json(report: &GetReport) -> serde_json::Value {
    let c = &report.counts;
    let mut counts = serde_json::to_value(c).expect("counts serialize");
    counts["total"] = json!(report.files.len());
    counts["succeeded"] =
        json!(c.restored + c.replaced + c.unchanged + c.would_restore + c.would_replace);
    counts
}

pub fn to_json(report: &GetReport) -> serde_json::Value {
    let files: Vec<_> = report.files.iter().map(row_json).collect();
    json!({
        "v": 1,
        "command": "get",
        "outcome": report.outcome,
        "dry_run": report.dry_run,
        "on_conflict": report.on_conflict,
        "source": report.source,
        "counts": counts_json(report),
        "bytes": report.bytes,
        // An operation-level problem ("no tracked files matched") keeps its
        // code (K2); the library's `problem` text is its message
        "error": report
            .problem
            .as_ref()
            .map(|message| crate::output::error_json("operation_failed", message.as_str())),
        "files": files,
    })
}

/// The envelope for a get that could not start (not in a project, invalid
/// configuration): zero rows, the reason in `error`, and no policy.
pub fn problem_json(problem: &str, dry_run: bool) -> serde_json::Value {
    let report = GetReport::problem(problem, dry_run, OnConflict::Abort, SettingSource::Default);
    let mut v = to_json(&report);
    v["on_conflict"] = serde_json::Value::Null;
    v["source"] = serde_json::Value::Null;
    v
}
