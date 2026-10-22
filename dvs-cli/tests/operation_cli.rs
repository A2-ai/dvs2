//! Operation reporting on a local project: timing (C2), `dvs hash` (C7) and
//! byte totals (C8).
mod common;

use common::{Sandbox, by_path, document, file_results, parse_stream, stderr};
use serde_json::{Value, json};

fn local_project() -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.init_local();
    sandbox
}

fn tracked(sandbox: &Sandbox, files: &[(&str, &str)]) {
    for (name, contents) in files {
        sandbox.write(name, contents);
    }
    let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
    let output = sandbox
        .dvs()
        .args(["--json", "add"])
        .args(&names)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", document(&output));
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
        vec!["hash", "data.txt"],
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
        let result = document(&output);
        assert_timed(&result);
        if args[0] == "hash" {
            for row in result["files"].as_array().unwrap() {
                assert!(
                    row["duration_ms"].is_u64() || row["duration_ms"].is_null(),
                    "{args:?}: {row}"
                );
                assert!(row.get("duration_ms").is_some(), "{args:?}: {row}");
            }
        }
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

/// C2: every event has `ts` and `elapsed_ms`; transfers and the terminal
/// event have their durations.
#[test]
fn every_event_carries_ts_and_elapsed_ms() {
    let sandbox = local_project();
    sandbox.write("a.txt", "a".repeat(10_000));
    sandbox.write("b.txt", "b".repeat(10_000));
    let mut kinds = std::collections::BTreeSet::new();
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    for args in [
        vec!["add", "--dry-run", "a.txt", "b.txt"],
        vec!["add", "a.txt", "b.txt"],
        vec!["status"],
        vec!["hash"],
    ] {
        let output = sandbox
            .dvs()
            .arg("--emit-events")
            .args(&args)
            .output()
            .unwrap();
        let events = parse_stream(&output.stdout, 0);
        let mut last_elapsed = 0;
        for event in &events {
            kinds.insert(event["type"].as_str().unwrap().to_string());
            let ts = event["ts"].as_u64().unwrap_or_else(|| panic!("{event}"));
            assert!(ts + 5 >= before, "{event}");
            let elapsed = event["elapsed_ms"].as_u64().unwrap();
            assert!(elapsed >= last_elapsed, "{event}");
            last_elapsed = elapsed;
            if event["type"] == "transfer_finished" {
                assert!(event["duration_ms"].is_u64(), "{event}");
            }
        }
        let finished = events.last().unwrap();
        assert_timed(finished);
        assert_timed(&finished["result"]);
    }
    std::fs::remove_file(sandbox.project().join("a.txt")).unwrap();
    let output = sandbox
        .dvs()
        .args(["--emit-events", "get", "a.txt"])
        .output()
        .unwrap();
    for event in parse_stream(&output.stdout, 0) {
        kinds.insert(event["type"].as_str().unwrap().to_string());
        assert!(event["ts"].is_u64() && event["elapsed_ms"].is_u64());
    }
    for kind in [
        "operation_started",
        "transfer_started",
        "transfer_finished",
        "file_result",
        "operation_finished",
    ] {
        assert!(kinds.contains(kind), "{kind}: {kinds:?}");
    }
}

/// An event stream and its `--json` twin agree apart from timing.
#[test]
fn streamed_results_match_json_results_apart_from_timing() {
    let sandbox = local_project();
    tracked(&sandbox, &[("one.txt", "one"), ("two.txt", "two")]);
    sandbox.write("three.txt", "three");
    // Both runs below then read the hashes from the cache
    assert!(sandbox.run(&["hash", "three.txt"]).status.success());
    for args in [
        vec!["status"],
        vec!["hash", "one.txt", "two.txt", "three.txt"],
        vec!["add", "--dry-run", "three.txt", "one.txt"],
        vec!["get", "--dry-run", "one.txt"],
    ] {
        let json_output = sandbox.dvs().arg("--json").args(&args).output().unwrap();
        let expected = common::without_timing(document(&json_output));
        let streamed = sandbox
            .dvs()
            .arg("--emit-events")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(streamed.status.code(), json_output.status.code());
        let events = parse_stream(&streamed.stdout, streamed.status.code().unwrap());
        let result = common::without_timing(events.last().unwrap()["result"].clone());
        assert_eq!(result, expected, "{args:?}");
        // Each row once, as the final result holds it
        let rows = common::without_timing(json!(by_path(file_results(&events))));
        assert_eq!(rows, expected["files"], "{args:?}");
    }
}

/// C7: hashes for tracked and untracked files, from the cache or computed,
/// selected like status, never touching records or the baseline.
#[test]
fn hash_reports_hashes_for_selected_files() {
    let sandbox = local_project();
    tracked(&sandbox, &[("tracked.txt", "tracked v1")]);
    sandbox.write("untracked.txt", "untracked");
    sandbox.write("dir/a.csv", "a");
    sandbox.write("dir/nested/b.csv", "b");
    sandbox.write("dir/c.txt", "c");

    let first = document(&sandbox.run(&["--json", "hash", "untracked.txt"]));
    assert_eq!(first["command"], "hash");
    assert_eq!(first["outcome"], "complete");
    assert_eq!(first["dry_run"], false);
    assert_eq!(
        first["counts"],
        json!({"total": 1, "succeeded": 1, "failed": 0})
    );
    let row = &first["files"][0];
    assert_eq!(row["path"], "untracked.txt");
    assert_eq!(row["hash_source"], "computed");
    assert_eq!(row["size"], 9);
    assert_eq!(row["hashes"]["blake3"].as_str().unwrap().len(), 64);
    assert!(row["duration_ms"].is_u64());
    assert!(row["code"].is_null() && row["message"].is_null());
    let again = document(&sandbox.run(&["--json", "hash", "untracked.txt"]));
    assert_eq!(again["files"][0]["hash_source"], "cache");
    assert_eq!(again["files"][0]["hashes"], row["hashes"]);

    // No paths: the tracked files status would show, nothing untracked
    let all = document(&sandbox.run(&["--json", "hash"]));
    let paths: Vec<_> = all["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].clone())
        .collect();
    assert_eq!(paths, vec![json!("tracked.txt")]);
    // A directory: its direct children; -r: everything below; a glob
    let paths = |args: &[&str]| -> Vec<String> {
        let mut full = vec!["--json", "hash"];
        full.extend_from_slice(args);
        document(&sandbox.run(&full))["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(paths(&["dir"]), vec!["dir/a.csv", "dir/c.txt"]);
    assert_eq!(
        paths(&["-r", "dir"]),
        vec!["dir/a.csv", "dir/c.txt", "dir/nested/b.csv"]
    );
    assert_eq!(paths(&["dir", "-g", "*.csv"]), vec!["dir/a.csv"]);
    assert_eq!(
        paths(&["-g", "**/*.csv"]),
        vec!["dir/a.csv", "dir/nested/b.csv"]
    );

    // A missing path is an error row, never dropped
    let missing = sandbox.run(&["--json", "hash", "untracked.txt", "nope.txt"]);
    assert_eq!(missing.status.code(), Some(1));
    let result = document(&missing);
    assert_eq!(result["outcome"], "partial");
    assert_eq!(
        result["counts"],
        json!({"total": 2, "succeeded": 1, "failed": 1})
    );
    assert_eq!(result["files"][0]["path"], "nope.txt");
    assert!(result["files"][0]["code"].is_string());
    assert!(result["files"][0]["hashes"].is_null());

    // Hashing a changed tracked file leaves the record and the baseline alone
    let record = sandbox.read(".dvs/tracked.txt.dvs");
    sandbox.write("tracked.txt", "tracked v2, edited");
    let edited = document(&sandbox.run(&["--json", "hash", "tracked.txt"]));
    assert_eq!(edited["files"][0]["hash_source"], "computed");
    assert_eq!(sandbox.read(".dvs/tracked.txt.dvs"), record);
    let status = document(&sandbox.run(&["--json", "status", "tracked.txt"]));
    // Still differs from the baseline: hash did not record the edit
    assert_eq!(status["files"][0]["state"], "modified", "{status}");

    // Human output: a table on stdout
    let human = sandbox.run(&["hash", "untracked.txt"]);
    assert!(human.status.success());
    let text = common::stdout(&human);
    assert!(
        text.contains("untracked.txt") && text.contains(row["hashes"]["blake3"].as_str().unwrap())
    );
}

#[cfg(unix)]
#[test]
fn hash_reports_an_unreadable_file_as_a_row_error() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = local_project();
    sandbox.write("secret.txt", "secret");
    sandbox.write("open.txt", "open");
    let secret = sandbox.project().join("secret.txt");
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&secret).is_ok() {
        // Running as root: permissions do not stop reads
        return;
    }
    let output = sandbox.run(&["--json", "hash", "open.txt", "secret.txt"]);
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let result = document(&output);
    assert_eq!(result["files"][1]["path"], "secret.txt");
    assert_eq!(result["files"][1]["code"], "io_error");
    assert!(result["files"][1]["message"].is_string());
    assert_eq!(result["files"][0]["code"], Value::Null);
    let human = sandbox.run(&["hash", "open.txt"]);
    assert!(human.status.success(), "{}", stderr(&human));
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
