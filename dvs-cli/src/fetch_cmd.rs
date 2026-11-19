use dvs::{Action, FetchReport, FetchRow};
use serde_json::json;

fn label(action: Action) -> &'static str {
    match action {
        Action::Restored => "fetched",
        other => crate::get_cmd::label(other),
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

pub fn print_human(report: &FetchReport) {
    for r in &report.files {
        if r.row.satisfied {
            println!("{}", line(r));
        } else {
            eprintln!("{}", line(r));
        }
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
        "counts": report.counts,
        "error": null,
        "files": report.files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs::{Before, GetCode, GetRow};
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
