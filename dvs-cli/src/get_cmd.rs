//! Rendering `dvs get` results: human text and the JSON envelope.

use std::path::{Path, PathBuf};

use dvs::config::OnConflict;
use dvs::{
    Action, Before, ErrorCode, GetCode, GetOutcome, GetReport, GetRow, HintSyntax, NextStep,
    StoredCopy, format_size, shell_path,
};
use serde_json::{Value, json};

/// Hints in CLI syntax: `dvs add b.csv`, `dvs get --force b.csv`.
pub struct Cli;

impl HintSyntax for Cli {
    fn step(&self, step: &NextStep) -> String {
        let paths = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|p| shell_path(p))
                .collect::<Vec<_>>()
                .join(" ")
        };
        match step {
            NextStep::Add { paths: p } => format!("dvs add {}", paths(p)),
            NextStep::Get {
                paths: p,
                on_conflict,
            } => match on_conflict {
                Some(OnConflict::Overwrite) => format!("dvs get --force {}", paths(p)),
                Some(policy) => format!("dvs get --on-conflict {policy} {}", paths(p)),
                None => format!("dvs get {}", paths(p)),
            },
            NextStep::RerunWith {
                on_conflict,
                recursive,
            } => {
                let mut options = Vec::new();
                if let Some(policy) = on_conflict {
                    options.push(format!("--on-conflict {policy}"));
                }
                if *recursive {
                    options.push("-r".to_string());
                }
                options.join(" ")
            }
            NextStep::Retry => "dvs get".to_string(),
            NextStep::Status => "dvs status".to_string(),
            NextStep::Login => "dvs info, dvs login".to_string(),
        }
    }

    fn path(&self, path: &Path) -> String {
        shell_path(path)
    }
}

/// A get row as JSON: the row's fields, with `hint` rendered in CLI syntax and
/// its commands as data in `next_steps` (`[]` when there are none).
pub fn row_json(row: &GetRow) -> Value {
    // No `..`: a field added to GetRow does not compile here until it is
    // placed in the JSON.
    let GetRow {
        path,
        local_path,
        before,
        action,
        satisfied,
        size,
        target_hash,
        local_hash,
        stored_copy,
        code,
        message,
        hint,
        duration_ms,
    } = row;
    let next_steps: Vec<&NextStep> = hint.iter().flat_map(|h| h.steps()).collect();
    json!({
        "path": path,
        "local_path": local_path,
        "before": before,
        "action": action,
        "satisfied": satisfied,
        "size": size,
        "target_hash": target_hash,
        "local_hash": local_hash,
        "stored_copy": stored_copy,
        "code": code,
        "message": message,
        "hint": hint.as_ref().map(|h| h.render(&Cli)),
        "next_steps": next_steps,
        "duration_ms": duration_ms,
    })
}

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
            eprintln!("  {}", h.render(&Cli));
        }
    }

    // Next steps for conflicts, in one place rather than per row.
    let conflicts: Vec<&GetRow> = report.files.iter().filter(|r| r.is_conflict()).collect();
    if !conflicts.is_empty() {
        let join = |rows: &[&&GetRow]| {
            rows.iter()
                .map(|r| shell_path(&r.local_path))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let paths = join(&conflicts.iter().collect::<Vec<_>>());
        // Keeping these unseen would drop the newer tracked version
        let (both, keepable): (Vec<&&GetRow>, Vec<&&GetRow>) = conflicts
            .iter()
            .partition(|r| r.before == Before::BothModified);
        if conflicts.iter().any(|r| r.before == Before::Unknown) {
            eprintln!(
                "  no record usually means a clone from before dvs recorded what it writes, or a cleared cache"
            );
        }
        if !both.is_empty() {
            eprintln!(
                "  compare first:             {}   (the tracked version changed too: copy yours aside, \
                 take the tracked version, then dvs add yours or a merge to keep it)",
                join(&both)
            );
        }
        if !keepable.is_empty() {
            eprintln!(
                "  keep your version:         dvs add {}   (makes it the tracked version)",
                join(&keepable)
            );
        }
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
            eprintln!("  {}: {}", r.local_path.display(), h.render(&Cli));
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
        "files": report.files.iter().map(row_json).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(code: GetCode, local_path: &Path) -> GetRow {
        let mut row = GetRow {
            path: local_path.to_path_buf(),
            local_path: local_path.to_path_buf(),
            before: Before::Modified,
            action: Action::Failed,
            satisfied: false,
            size: None,
            target_hash: None,
            local_hash: None,
            stored_copy: None,
            code: Some(code),
            message: None,
            hint: None,
            duration_ms: None,
        };
        row.hint = dvs::row_hint(&row);
        row
    }

    /// The CLI text of every hint, as it was when core built the strings.
    #[test]
    fn hints_read_as_before_in_cli_syntax() {
        let choices = "keep your version: dvs add b.csv (makes it the tracked version); \
                       take the tracked version: dvs get --force b.csv (replaces yours)";
        let unknown = format!(
            "dvs has no record of writing this file (a clone from before dvs recorded this, \
             or a cleared cache); {choices}"
        );
        let cases: Vec<(GetCode, Option<&str>)> = vec![
            (GetCode::LocalChanges, Some(choices)),
            (GetCode::UnknownLocal, Some(&unknown)),
            (
                GetCode::BatchAborted,
                Some("held back by another file; to get the rest: --on-conflict skip"),
            ),
            (
                GetCode::NotTracked,
                Some("check the path; list tracked files with: dvs status"),
            ),
            (
                GetCode::SourceMissing,
                Some(
                    "the stored content of this version is missing (probably an interrupted \
                     upload); tell whoever added it",
                ),
            ),
            (
                GetCode::Shared(ErrorCode::AccessDenied),
                Some("check you are logged in with access: dvs info, dvs login"),
            ),
            (
                GetCode::Shared(ErrorCode::ServerUnavailable),
                Some("check your network or VPN and the server, then run dvs get again"),
            ),
            (
                GetCode::DestinationChanged,
                Some("run again once nothing is writing b.csv"),
            ),
            (
                GetCode::FileInUse,
                Some("close the program using the file, then run dvs get again"),
            ),
            (
                GetCode::Shared(ErrorCode::BadMetadata),
                Some(
                    "fix the .dvs file for b.csv (for example a git merge conflict), then run \
                     dvs get again",
                ),
            ),
            (GetCode::NotAFile, None),
            (GetCode::UnsafePath, None),
            (GetCode::HashMismatch, None),
            (GetCode::PermissionDenied, None),
            (GetCode::Shared(ErrorCode::IoError), None),
        ];
        for (code, expected) in cases {
            let json = row_json(&row(code, Path::new("b.csv")));
            assert_eq!(json["hint"].as_str(), expected, "{code:?}");
            assert!(json["next_steps"].is_array(), "{code:?}");
        }

        // A directory that selects nothing tracked
        let dir = tempfile::tempdir().unwrap();
        let json = row_json(&row(GetCode::NotTracked, dir.path()));
        assert_eq!(
            json["hint"],
            "nothing tracked is selected here; add -r to include subdirectories, or check the path"
        );
    }

    #[test]
    fn next_steps_are_the_hint_commands_as_data() {
        let json = row_json(&row(GetCode::LocalChanges, Path::new("my data.csv")));
        assert_eq!(
            json["hint"],
            "keep your version: dvs add 'my data.csv' (makes it the tracked version); \
             take the tracked version: dvs get --force 'my data.csv' (replaces yours)"
        );
        assert_eq!(
            json["next_steps"],
            json!([
                {"action": "add", "paths": ["my data.csv"]},
                {"action": "get", "paths": ["my data.csv"], "on_conflict": "overwrite"},
            ])
        );
        let json = row_json(&row(GetCode::NotAFile, Path::new("b.csv")));
        assert_eq!(json["next_steps"], json!([]));
        assert!(json["hint"].is_null());
    }

    /// Both sides changed: compare first, and never a bare "keep yours".
    #[test]
    fn both_modified_hint_compares_first() {
        let mut both = row(GetCode::LocalChanges, Path::new("b.csv"));
        both.before = Before::BothModified;
        both.hint = dvs::row_hint(&both);
        let json = row_json(&both);
        assert_eq!(json["before"], "both_modified");
        assert_eq!(
            json["hint"],
            "your file and the tracked version both changed since dvs last wrote it, so compare \
             them before keeping either. Copy yours aside, then take the tracked version: \
             dvs get --force b.csv. To keep yours or a merge instead, put it back, then: \
             dvs add b.csv"
        );
        assert_eq!(
            json["next_steps"],
            json!([
                {"action": "get", "paths": ["b.csv"], "on_conflict": "overwrite"},
                {"action": "add", "paths": ["b.csv"]},
            ])
        );
    }
}
