use dvs::{Action, Before, FetchReport, FetchRow, GetOutcome};
use serde_json::json;

fn label(action: Action) -> &'static str {
    match action {
        Action::Restored => "fetched",
        Action::WouldRestore => "would fetch",
        other => crate::output::action_label(other),
    }
}

fn line(r: &FetchRow) -> String {
    let row = &r.row;
    let detail = match (&r.written_to, &r.hash, &row.message) {
        (_, _, Some(m)) if !row.satisfied => m.clone(),
        (Some(_), Some(hash), _) => format!(
            "from {} @ {}",
            row.local_path.display(),
            hash.get(..8).unwrap_or(hash)
        ),
        (_, _, m) => m.clone().unwrap_or_default(),
    };
    let shown = r.written_to.as_ref().unwrap_or(&row.local_path);
    format!("{}  {}  {detail}", label(row.action), shown.display())
        .trim_end()
        .to_string()
}

fn summary(report: &FetchReport) -> String {
    let c = &report.counts;
    let parts = [
        (c.restored, "fetched {}"),
        (c.replaced, "replaced {}"),
        (c.unchanged, "{} already there"),
        (c.skipped, "skipped {}"),
        (c.not_run, "{} held back"),
        (c.cancelled, "cancelled {}"),
        (c.failed, "{} failed"),
    ];
    let phrase = parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, f)| f.replace("{}", &n.to_string()))
        .collect::<Vec<_>>()
        .join(", ");
    let changed = |r: &&FetchRow| r.row.is_conflict() && r.row.before == Before::Modified;
    let mut out = match report.outcome {
        GetOutcome::Complete if c.restored + c.replaced == 0 => {
            format!("nothing to do, {} already there", c.unchanged)
        }
        GetOutcome::Complete => phrase,
        GetOutcome::Aborted => {
            let mut reasons = Vec::new();
            let n = report.files.iter().filter(changed).count();
            match n {
                0 => {}
                1 => reasons.push("a scratchpad copy was changed".to_string()),
                n => reasons.push(format!("{n} scratchpad copies were changed")),
            }
            match c.failed {
                0 => {}
                1 => reasons.push("1 file cannot be retrieved".to_string()),
                n => reasons.push(format!("{n} files cannot be retrieved")),
            }
            format!("aborted, nothing changed: {}", reasons.join("; "))
        }
        _ => format!("{}: {phrase}", report.outcome.as_str()),
    };
    if let Some(first) = out.get(..1) {
        out.replace_range(..1, &first.to_uppercase());
    }
    if report
        .files
        .iter()
        .any(|r| r.row.before == Before::Modified)
    {
        out.push_str(&format!(
            "\non_conflict = {} ({})",
            report.on_conflict, report.source
        ));
    }
    out
}

pub fn print_human(report: &FetchReport) {
    if let Some(problem) = &report.problem {
        eprintln!("dvs fetch: {problem}");
        return;
    }
    for r in &report.files {
        if r.row.satisfied {
            println!("{}", line(r));
        } else {
            eprintln!("{}", line(r));
        }
    }
    let summary = format!("\n{}", summary(report));
    if report.outcome == GetOutcome::Complete {
        println!("{summary}");
    } else {
        eprintln!("{summary}");
    }
    for r in report.files.iter().filter(|r| !r.row.satisfied) {
        if let Some(h) = &r.row.hint {
            eprintln!("  {}: {h}", r.row.local_path.display());
        }
    }
}

pub fn to_json(report: &FetchReport) -> serde_json::Value {
    json!({
        "v": 1,
        "command": "fetch",
        "outcome": report.outcome,
        "dry_run": false,
        "on_conflict": report.on_conflict,
        "source": report.source,
        "counts": report.counts,
        "error": report
            .problem
            .as_ref()
            .map(|p| crate::output::error_json(p.code, p.message.as_str())),
        "files": report.files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs::{GetCode, GetRow};
    use std::path::PathBuf;

    fn row(action: Action, written_to: Option<&str>, message: Option<&str>) -> FetchRow {
        FetchRow {
            row: GetRow {
                path: PathBuf::from("data/a.csv"),
                local_path: PathBuf::from("data/a.csv"),
                before: Before::Absent,
                action,
                satisfied: action == Action::Restored,
                size: Some(3),
                target_hash: None,
                local_hash: None,
                stored_copy: None,
                code: (action == Action::Failed)
                    .then_some(GetCode::Shared(dvs::ErrorCode::InvalidArguments)),
                message: message.map(str::to_string),
                hint: None,
                duration_ms: None,
            },
            written_to: written_to.map(PathBuf::from),
            hash: written_to.map(|_| "a1b2c3d4e5f6".to_string()),
        }
    }

    #[test]
    fn lines_name_the_scratch_copy_and_its_source() {
        assert_eq!(
            line(&row(
                Action::Restored,
                Some(".dvs-scratchpad/a1b2c3d4/data/a.csv"),
                None
            )),
            "fetched  .dvs-scratchpad/a1b2c3d4/data/a.csv  from data/a.csv @ a1b2c3d4"
        );
        assert_eq!(
            line(&row(Action::Failed, None, Some("not tracked by dvs"))),
            "failed  data/a.csv  not tracked by dvs"
        );
    }
}
