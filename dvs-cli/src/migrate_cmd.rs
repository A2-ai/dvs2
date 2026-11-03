//! Rendering `dvs migrate` results: human text and the JSON envelope.

use dvs::migrate::{ChangeKind, MigrateDetail, MigrateReport, MigrateRow, ProjectAction};
use dvs::{ErrorCode, Outcome, format_size, shell_path};
use serde_json::{Value, json};

use crate::output;

/// A migrate row, as streamed and in the result. The library's `error` text
/// is the row's `message`, as on every other row.
pub fn row(r: &MigrateRow) -> Value {
    let mut value = serde_json::to_value(r).expect("rows serialize");
    if let Some(error) = value.as_object_mut().and_then(|o| o.remove("error")) {
        value["message"] = error;
    }
    for key in ["outcome", "hash", "size"] {
        if value.get(key).is_none() {
            value[key] = Value::Null;
        }
    }
    if matches!(r.detail, MigrateDetail::Cancelled { .. }) {
        value["code"] = json!(ErrorCode::Cancelled);
    }
    let has_code = !matches!(r.detail, MigrateDetail::Success { .. });
    output::file_result(value, has_code)
}

pub fn to_json(report: &MigrateReport) -> Value {
    let files = report.files.iter().map(row).collect();
    let mut result = output::rows_envelope(
        "migrate",
        report.dry_run,
        files,
        report.counts,
        report.outcome,
    );
    result["bytes"] = json!(report.bytes);
    result["project"] = json!(report.project);
    result["sources"] = json!(report.sources);
    result["changes"] = json!(report.changes);
    result["warnings"] = json!(
        report
            .warnings
            .iter()
            .map(|w| json!({"code": null, "message": w}))
            .collect::<Vec<_>>()
    );
    result
}

/// One line after the rows: "Copied 2, 1 already stored, 1 missing, 1 failed".
fn summary(report: &MigrateReport) -> String {
    let stored = report.already_stored();
    let missing = report
        .files
        .iter()
        .filter(|r| matches!(r.detail, MigrateDetail::Missing { .. }))
        .count();
    let copied = report.counts.succeeded - stored - missing;
    let copy = if report.dry_run {
        format!(
            "Would copy {copied} [{}]",
            format_size(report.bytes.to_transfer)
        )
    } else {
        format!("Copied {copied}")
    };
    crate::summary_line(
        [
            (copied, copy),
            (stored, format!("{stored} already stored")),
            (missing, format!("{missing} missing")),
        ],
        &report.counts,
    )
    .unwrap_or_else(|| "No files listed".to_string())
}

/// `server`: the project name and URL of a migration to a server.
pub fn print_human(report: &MigrateReport, server: Option<&str>) {
    let dry = report.dry_run;
    if report.sources.is_empty() {
        println!("Nothing to migrate: this project lists no earlier storage to copy from");
        return;
    }
    let sources: Vec<_> = report
        .sources
        .iter()
        .map(|s| s.display().to_string())
        .collect();
    println!("From: {}", sources.join(", "));
    if let (Some(action), Some(server)) = (report.project, server) {
        let line = match (action, dry) {
            (ProjectAction::Created, false) => "Created the server project",
            (ProjectAction::Created, true) => "Would create the server project",
            (ProjectAction::Added, false) => "Added to the server project",
            (ProjectAction::Added, true) => "Would add to the server project",
        };
        println!("{line} {server}");
    }
    for r in &report.files {
        let path = r.path.display();
        match &r.detail {
            MigrateDetail::Success { outcome, size, .. } => {
                let label = match (outcome, dry) {
                    (Outcome::Copied, true) => "To copy",
                    (Outcome::Copied, false) => "Copied",
                    (Outcome::Present, _) => "Already stored",
                };
                println!("{label}: {path} [{}]", format_size(*size));
            }
            MigrateDetail::Missing {
                hash,
                size,
                message,
                ..
            } => eprintln!("Missing: {path} [{}] {hash}: {message}", format_size(*size)),
            MigrateDetail::Error { error, .. } => eprintln!("Error migrating {path}: {error}"),
            MigrateDetail::Cancelled { .. } => {}
        }
    }
    let line = summary(report);
    if report.outcome == dvs::RunOutcome::Complete {
        println!("\n{line}");
    } else {
        eprintln!("\n{line}");
    }
    for warning in &report.warnings {
        eprintln!("Warning: {warning}");
    }

    if report.changes.is_empty() {
        println!("No files changed");
        return;
    }
    println!(
        "\n{}",
        if dry {
            "Files it would change:"
        } else {
            "Changed files:"
        }
    );
    for change in &report.changes {
        let kind = match change.change {
            ChangeKind::Written => "written",
            ChangeKind::Removed => "removed",
        };
        println!("  {kind:<8} {}", change.path.display());
    }
    println!("\n{}", next_step(report));
}

/// The line after the changed files: what to do next.
fn next_step(report: &MigrateReport) -> String {
    if report.dry_run {
        "Dry run: nothing was changed".into()
    } else if report.outcome != dvs::RunOutcome::Complete {
        // Committing now would commit a half-migrated checkout
        "The checkout is partly migrated; run dvs migrate to finish it".into()
    } else {
        format!("To commit them: {}", commit_command(report))
    }
}

fn commit_command(report: &MigrateReport) -> String {
    let paths: Vec<_> = report.changes.iter().map(|c| shell_path(&c.path)).collect();
    format!("git add -- {} && git commit", paths.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs::migrate::Change;
    use dvs::{Counts, GetCode, RunOutcome};

    fn report(files: Vec<MigrateRow>, changes: Vec<Change>) -> MigrateReport {
        MigrateReport {
            sources: vec!["/data/old".into()],
            counts: Counts::new(files.len(), 0),
            files,
            changes,
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_blob_is_a_coded_row_in_a_complete_run() {
        let missing = MigrateRow {
            path: "x.csv".into(),
            detail: MigrateDetail::Missing {
                code: GetCode::SourceMissing,
                hash: "ab".into(),
                size: 2,
                message: "gone".into(),
            },
            duration_ms: None,
        };
        let json = to_json(&report(vec![missing], Vec::new()));
        assert_eq!(json["outcome"], "complete");
        assert_eq!(json["files"][0]["code"], "source_missing");
        assert_eq!(json["files"][0]["hash"], "ab");
    }

    #[test]
    fn the_commit_suggestion_names_every_changed_file() {
        let change = |path: &str, change| Change {
            path: path.into(),
            change,
        };
        let report = report(
            Vec::new(),
            vec![
                change("dvs.toml", ChangeKind::Written),
                change("my data.csv.dvs", ChangeKind::Removed),
            ],
        );
        assert_eq!(
            commit_command(&report),
            "git add -- dvs.toml 'my data.csv.dvs' && git commit"
        );
    }

    #[test]
    fn an_unfinished_run_says_to_finish_it_instead_of_committing() {
        let change = Change {
            path: "dvs.toml".into(),
            change: ChangeKind::Written,
        };
        let mut report = report(Vec::new(), vec![change]);
        assert!(next_step(&report).starts_with("To commit them: "));
        report.outcome = RunOutcome::Cancelled;
        assert_eq!(
            next_step(&report),
            "The checkout is partly migrated; run dvs migrate to finish it"
        );
    }

    #[test]
    fn a_dry_run_summary_gives_the_bytes_to_copy() {
        let copy = |path: &str| MigrateRow {
            path: path.into(),
            detail: MigrateDetail::Success {
                outcome: Outcome::Copied,
                hash: "ab".into(),
                size: 7,
            },
            duration_ms: None,
        };
        let mut report = report(vec![copy("a.csv"), copy("b.csv")], Vec::new());
        report.dry_run = true;
        report.bytes.to_transfer = 14;
        assert_eq!(summary(&report), "Would copy 2 [14 B]");
    }
}
