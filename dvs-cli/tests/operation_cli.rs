//! Operation reporting on a local project: timing (C2) and byte totals (C8).
mod common;

use common::{Sandbox, document};
use serde_json::{Value, json};

fn local_project() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.init_local();
    sandbox
}

fn is_rfc3339_ms(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    // 2026-10-21T14:30:00.123Z
    text.len() == 24 && text.ends_with('Z') && &text[19..20] == "." && &text[10..11] == "T"
}

fn assert_timed(result: &Value) {
    assert!(is_rfc3339_ms(&result["started_at"]), "{result}");
    assert!(is_rfc3339_ms(&result["finished_at"]), "{result}");
    assert!(result["finished_at"].as_str() >= result["started_at"].as_str());
    assert!(result["duration_ms"].is_u64(), "{result}");
}

/// C2, K6: every command's JSON result is timed, dry runs and errors too.
#[test]
fn every_json_result_carries_its_timing() {
    let sandbox = Sandbox::new();
    let init = sandbox
        .dvs()
        .args(["--json", "init", "local"])
        .arg(sandbox.temp.path().join("storage"))
        .output()
        .unwrap();
    assert_timed(&document(&init));
    sandbox.write("data.txt", "data");
    let hash = "0".repeat(64);
    for args in [
        vec!["add", "--dry-run", "data.txt"],
        vec!["add", "data.txt"],
        vec!["status"],
        vec!["get", "--dry-run", "data.txt"],
        vec!["get", "data.txt"],
        vec!["info"],
        vec!["history"],
        vec!["availability", &hash],
        vec!["logout"],
        vec!["add", "missing.txt"],
        vec!["get", "--on-conflict", "bogus"],
    ] {
        let output = sandbox.dvs().arg("--json").args(&args).output().unwrap();
        assert_timed(&document(&output));
    }
    let outside = tempfile::tempdir().unwrap();
    assert_timed(&document(
        &sandbox
            .dvs_in(outside.path())
            .args(["--json", "status"])
            .output()
            .unwrap(),
    ));
}

fn sizes(result: &Value, keep: impl Fn(&Value) -> bool) -> u64 {
    result["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| keep(row))
        .filter_map(|row| row["size"].as_u64())
        .sum()
}

/// C8: add and get byte totals, dry run and real, match their rows.
#[test]
fn add_and_get_report_byte_totals_matching_their_rows() {
    let sandbox = local_project();
    sandbox.write("a.bin", vec![1u8; 1000]);
    sandbox.write("b.bin", vec![2u8; 300]);
    let preview = document(&sandbox.run(&["--json", "add", "--dry-run", "a.bin", "b.bin"]));
    let total = sizes(&preview, |_| true);
    assert_eq!(total, 1300);
    assert_eq!(
        preview["bytes"],
        json!({"total": 1300, "to_transfer": 1300, "already_stored": 0, "transferred": 0})
    );
    let add = document(&sandbox.run(&["--json", "add", "a.bin", "b.bin"]));
    assert_eq!(
        add["bytes"],
        json!({"total": 1300, "to_transfer": 1300, "already_stored": 0, "transferred": 1300})
    );
    // Nothing left to store
    let again = document(&sandbox.run(&["--json", "add", "--dry-run", "a.bin", "b.bin"]));
    assert_eq!(again["bytes"]["already_stored"], 1300);
    assert_eq!(again["bytes"]["to_transfer"], 0);

    std::fs::remove_file(sandbox.project().join("a.bin")).unwrap();
    for (args, transferred) in [
        (vec!["--json", "get", "--dry-run", "a.bin", "b.bin"], 0),
        (vec!["--json", "get", "a.bin", "b.bin"], 1000),
    ] {
        let get = document(&sandbox.run(&args));
        let missing = sizes(&get, |row| row["before"] != "current");
        let current = sizes(&get, |row| row["before"] == "current");
        assert_eq!(
            get["bytes"],
            json!({
                "total": missing + current,
                "to_transfer": missing,
                "already_stored": current,
                "transferred": transferred,
            }),
            "{get}"
        );
        assert_eq!(missing, 1000);
    }
    // A get that cannot start still has the shape
    let nothing = document(&sandbox.run(&["--json", "get", "nothing-here.bin"]));
    assert!(nothing["bytes"].is_object(), "{nothing}");
    let refused = document(&sandbox.run(&["--json", "add", "nothing-here.bin"]));
    assert_eq!(
        refused["bytes"],
        json!({"total": 0, "to_transfer": 0, "already_stored": 0, "transferred": 0})
    );
}
