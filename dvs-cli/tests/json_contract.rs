use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dvs"))
        .current_dir(root)
        .args(args)
        .env("XDG_CONFIG_HOME", root.join("test-config"))
        .env("APPDATA", root.join("test-config"))
        .env_remove("DVS_GET_ON_CONFLICT")
        .output()
        .unwrap()
}
fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stdout)))
}
fn project() -> (TempDir, TempDir) {
    let root = TempDir::new().unwrap();
    let storage = TempDir::new().unwrap();
    let output = run(
        root.path(),
        &[
            "--json",
            "init",
            "local",
            storage.path().join("objects").to_str().unwrap(),
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(document(&output)["command"], "init");
    (root, storage)
}
#[test]
fn startup_and_argument_errors_are_single_typed_documents() {
    let root = TempDir::new().unwrap();
    for command in ["info", "status", "get", "logout", "login"] {
        let output = run(root.path(), &[command, "--json"]);
        assert_eq!(output.status.code(), Some(1));
        let value = document(&output);
        assert_eq!(value["v"], 1);
        assert_eq!(value["command"], command);
        assert_eq!(value["error"]["code"], "not_in_project");
        assert_eq!(value["outcome"], "failed");
    }
    for args in [
        vec!["--json", "not-a-command"],
        vec!["get", "--json", "--on-conflict", "bogus"],
        vec!["info", "--json", "--offline"],
        vec!["--json"],
    ] {
        let output = run(root.path(), &args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(document(&output)["error"]["code"], "invalid_arguments");
    }
    std::fs::write(root.path().join("dvs.toml"), "invalid = [").unwrap();
    let output = run(root.path(), &["get", "--json"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(document(&output)["error"]["code"], "invalid_configuration");
}
#[test]
fn results_wrap_files_and_preserve_get_conflicts() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("data.txt"), "recorded").unwrap();
    let add = run(root.path(), &["add", "data.txt", "--json"]);
    assert!(add.status.success());
    assert_eq!(document(&add)["counts"]["succeeded"], 1);
    let status = document(&run(root.path(), &["status", "--json"]));
    assert_eq!(status["command"], "status");
    assert_eq!(status["files"].as_array().unwrap().len(), 1);
    assert!(status["files"][0].get("state").is_none());
    assert_eq!(status["files"][0]["working"]["state"], "present");
    assert!(status["files"][0].get("before").is_none());
    std::fs::write(root.path().join("data.txt"), "local edit").unwrap();
    let get = run(root.path(), &["get", "data.txt", "--json"]);
    assert_eq!(get.status.code(), Some(3));
    let value = document(&get);
    assert_eq!(value["command"], "get");
    assert_eq!(value["outcome"], "aborted");
    assert_eq!(value["files"][0]["before"], "modified");
    assert_eq!(
        std::fs::read_to_string(root.path().join("data.txt")).unwrap(),
        "local edit"
    );
}

#[test]
fn add_file_failures_have_stable_codes_and_keep_the_message() {
    let (root, storage) = project();
    std::fs::write(root.path().join("data.txt"), "recorded").unwrap();

    // Make the configured storage path unusable after init. This reaches the
    // per-file store failure path rather than failing batch setup.
    let object_dir = storage.path().join("objects");
    std::fs::remove_dir_all(&object_dir).unwrap();
    std::fs::write(&object_dir, "blocks directory creation").unwrap();

    let output = run(root.path(), &["add", "data.txt", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let result = document(&output);
    assert_eq!(result["outcome"], "failed");
    assert_eq!(result["files"][0]["code"], "io_error");
    assert!(result["files"][0]["message"].as_str().is_some());
}
#[test]
fn partial_add_has_one_result_and_nonzero_exit() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("good.txt"), "content").unwrap();
    std::fs::write(root.path().join("bad.txt"), "content").unwrap();
    // Block one metadata destination while leaving the other writable.
    std::fs::create_dir(root.path().join(".dvs/bad.txt.dvs")).unwrap();
    let output = run(root.path(), &["add", "good.txt", "bad.txt", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let result = document(&output);
    assert_eq!(result["outcome"], "partial");
    assert_eq!(result["counts"]["failed"], 1);
    assert_eq!(result["counts"]["succeeded"], 1);
    assert!(output.stderr.is_empty());
}
#[test]
fn add_no_match_is_a_file_selection_failure_not_invalid_arguments() {
    let (root, _storage) = project();
    let json_output = run(root.path(), &["add", "missing.txt", "--json"]);
    assert_eq!(json_output.status.code(), Some(1));
    assert!(json_output.stderr.is_empty());
    let result = document(&json_output);
    assert_eq!(result["error"]["code"], "no_files_selected");
    assert_eq!(result["command"], "add");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Path not found: missing.txt")
    );

    let human_output = run(root.path(), &["add", "missing.txt"]);
    assert_eq!(human_output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&human_output.stderr).contains("Path not found: missing.txt"));

    let glob_output = run(root.path(), &["add", "--glob", "*.missing", "--json"]);
    assert_eq!(glob_output.status.code(), Some(1));
    assert_eq!(document(&glob_output)["error"]["code"], "no_files_selected");
}
#[test]
fn partial_add_keeps_final_human_diagnostic() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("bad.txt"), "content").unwrap();
    std::fs::create_dir(root.path().join(".dvs/bad.txt.dvs")).unwrap();
    let output = run(root.path(), &["add", "bad.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Error adding bad.txt:"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Some files failed to add"));
}
#[test]
fn status_failures_keep_human_diagnostic_and_json_stderr_clean() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("bad.txt"), "content").unwrap();
    let add = run(root.path(), &["add", "bad.txt"]);
    assert!(add.status.success());
    std::fs::write(root.path().join(".dvs/bad.txt.dvs"), "not metadata").unwrap();

    let json_output = run(root.path(), &["status", "--json"]);
    assert_eq!(json_output.status.code(), Some(1));
    assert!(json_output.stderr.is_empty());
    assert_eq!(document(&json_output)["counts"]["failed"], 1);

    let human_output = run(root.path(), &["status"]);
    assert_eq!(human_output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&human_output.stderr)
            .contains("Error: Some files failed to get status")
    );
}
#[test]
fn info_discovers_from_subdirectories_without_an_offline_flag() {
    let (root, _storage) = project();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let output = run(&child, &["info", "--json"]);
    assert!(output.status.success());
    let value = document(&output);
    assert_eq!(
        value["project_root"],
        root.path().canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        value["metadata_directory"],
        root.path()
            .canonicalize()
            .unwrap()
            .join(".dvs")
            .to_str()
            .unwrap()
    );
    assert_eq!(value["protocol"]["versions"], serde_json::json!([1]));
    assert!(value.get("offline").is_none());
    assert_eq!(value["connection"], "not_applicable");
    assert!(
        value["protocol"]["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|capability| capability == "connectivity_status")
    );
}
#[test]
fn server_info_reports_unknown_connection_without_credentials() {
    let root = TempDir::new().unwrap();
    std::fs::write(
        root.path().join("dvs.toml"),
        "[backend]\ntype = 'server'\nname = 'example'\nurl = 'http://127.0.0.1:1/'\n",
    )
    .unwrap();
    let output = run(root.path(), &["info", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(document(&output)["logged_in"], false);
    assert_eq!(document(&output)["connection"], "unknown");
    assert_eq!(document(&output)["access"], "not_logged_in");
    let credentials = root.path().join("test-config/dvs");
    std::fs::create_dir_all(&credentials).unwrap();
    std::fs::write(
        credentials.join("credentials.json"),
        r#"{"http://127.0.0.1:1/":"test-token"}"#,
    )
    .unwrap();
    let disconnected = run(root.path(), &["info", "--json"]);
    assert!(disconnected.status.success());
    let value = document(&disconnected);
    assert_eq!(value["connection"], "unavailable");
    assert_eq!(value["connection_error"]["code"], "server_unavailable");
    assert!(value["connection_error"]["message"].is_string());
    assert!(value["access"].is_null());
    assert!(value["groups"].is_null());
    let login = run(root.path(), &["login", "http://127.0.0.1:1/", "--json"]);
    assert_eq!(document(&login)["error"]["code"], "authentication_required");
}
#[test]
fn help_version_and_literal_flags_keep_conventional_behavior() {
    let root = TempDir::new().unwrap();
    for args in [["--json", "--help"], ["--json", "--version"]] {
        let output = run(root.path(), &args);
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("dvs"));
    }
    let output = run(root.path(), &["status", "--", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Not in a DVS repository"));
}

/// Every key of every object in a document, recursively.
fn keys(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                out.push(key.clone());
                keys(value, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keys(item, out)),
        _ => {}
    }
}

/// §9 rec 4: one exit-code table for every command, and (rec 1) one error
/// shape: no `problem` anywhere, `error` is null or `{code, message, hint}`.
#[test]
fn every_command_follows_the_exit_code_table_and_the_error_shape() {
    let (root, storage) = project();
    std::fs::write(root.path().join("data.txt"), "recorded").unwrap();
    let hash = "0".repeat(64);
    let objects = storage.path().join("objects");
    let objects = objects.to_str().unwrap();
    // (arguments, exit status): 0 complete, 1 failure/partial, 2 usage/config
    let cases: Vec<(Vec<&str>, i32)> = vec![
        (vec!["add", "data.txt"], 0),
        (vec!["add", "missing.txt"], 1),
        (vec!["add", "--bogus"], 2),
        (vec!["status"], 0),
        (vec!["status", "--bogus"], 2),
        (vec!["hash", "data.txt"], 0),
        (vec!["hash", "missing.txt"], 1),
        (vec!["get", "data.txt"], 0),
        (vec!["get", "untracked.txt"], 1),
        (vec!["availability", &hash], 0),
        (vec!["availability", "not-a-hash"], 2),
        (vec!["history"], 0),
        (vec!["history", "../outside.txt"], 2),
        (vec!["info"], 0),
        (vec!["init", "local", objects], 1),
        (vec!["attach", "proj", "http://127.0.0.1:1/"], 1),
        (vec!["attach"], 2),
        (vec!["login"], 1),
        (vec!["logout"], 1),
    ];
    for (args, expected) in cases {
        let output = run(root.path(), &[&["--json"], &args[..]].concat());
        assert_eq!(output.status.code(), Some(expected), "{args:?}");
        let value = document(&output);
        let mut all = Vec::new();
        keys(&value, &mut all);
        assert!(!all.contains(&"problem".to_string()), "{args:?}: {value}");
        if expected == 0 {
            assert!(value["error"].is_null(), "{args:?}: {value}");
        } else if value["files"].as_array().is_none_or(Vec::is_empty) {
            let error = value["error"].as_object().expect("an error object");
            let mut fields: Vec<_> = error.keys().cloned().collect();
            fields.sort();
            assert_eq!(fields, ["code", "hint", "message"], "{args:?}");
        }
    }
    // 3: get held back by local conflicts, nothing else failed
    std::fs::write(root.path().join("data.txt"), "local edit").unwrap();
    assert_eq!(
        run(root.path(), &["--json", "get", "data.txt"])
            .status
            .code(),
        Some(3)
    );
    // 2: configuration
    std::fs::write(root.path().join("dvs.toml"), "invalid = [").unwrap();
    for args in [
        vec!["add", "data.txt"],
        vec!["status"],
        vec!["hash"],
        vec!["get"],
        vec!["info"],
        vec!["history"],
        vec!["availability", &hash],
        vec!["login"],
        vec!["logout"],
    ] {
        let output = run(root.path(), &[&["--json"], &args[..]].concat());
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(document(&output)["error"]["code"], "invalid_configuration");
    }
}

/// §9 rec 1: add rows have fixed keys, success or error.
#[test]
fn add_success_and_error_rows_have_the_same_keys() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("good.txt"), "content").unwrap();
    std::fs::write(root.path().join("bad.txt"), "content").unwrap();
    std::fs::create_dir(root.path().join(".dvs/bad.txt.dvs")).unwrap();
    let result = document(&run(root.path(), &["add", "good.txt", "bad.txt", "--json"]));
    let row_keys = |row: &Value| {
        let mut keys: Vec<_> = row.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    let files = result["files"].as_array().unwrap();
    let (bad, good) = (&files[0], &files[1]);
    assert_eq!(good["path"], "good.txt");
    assert_eq!(
        row_keys(good),
        [
            "code",
            "duration_ms",
            "hash",
            "hint",
            "message",
            "outcome",
            "path",
            "size",
            "stored_size"
        ]
    );
    assert_eq!(row_keys(good), row_keys(bad));
    assert!(good["code"].is_null() && good["message"].is_null() && good["hint"].is_null());
    assert_eq!(good["outcome"], "copied");
    assert!(bad["code"].is_string(), "{bad}");
    assert!(bad["message"].is_string());
    for field in ["outcome", "hash", "size", "stored_size", "hint"] {
        assert!(bad[field].is_null(), "{field}: {bad}");
    }
}

/// §9 rec 1: status, hash and history rows carry `message` (not `error`)
/// and `hint`; status rows have the same keys whether they failed or not.
#[test]
fn status_hash_and_history_rows_use_code_message_hint() {
    let (root, _storage) = project();
    for name in ["bad.txt", "good.txt"] {
        std::fs::write(root.path().join(name), "content").unwrap();
        assert!(run(root.path(), &["add", name]).status.success());
    }
    std::fs::write(root.path().join(".dvs/bad.txt.dvs"), "not metadata").unwrap();
    let status = document(&run(root.path(), &["status", "--json"]));
    let (bad, good) = (&status["files"][0], &status["files"][1]);
    assert!(bad["code"].is_string(), "{bad}");
    assert!(bad["message"].is_string(), "{bad}");
    assert!(bad.get("error").is_none(), "{bad}");
    assert_eq!(bad.get("hint"), Some(&Value::Null), "{bad}");
    for field in ["code", "message", "hint"] {
        assert_eq!(good.get(field), Some(&Value::Null), "{field}: {good}");
    }
    let row_keys = |row: &Value| {
        let mut keys: Vec<_> = row.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(row_keys(bad), row_keys(good));
    let hash = document(&run(root.path(), &["hash", "bad.txt", "--json"]));
    let row = &hash["files"][0];
    for field in ["code", "message", "hint"] {
        assert_eq!(row.get(field), Some(&Value::Null), "{field}: {row}");
    }
    let history = document(&run(root.path(), &["history", "--json"]));
    let rows = history["files"].as_array().unwrap();
    assert!(!rows.is_empty(), "{history}");
    for row in rows {
        for field in ["code", "message", "hint"] {
            assert_eq!(row.get(field), Some(&Value::Null), "{field}: {row}");
        }
    }
}

/// K25: an unreadable `.dvs` record is `bad_metadata` in add too, never
/// `invalid_record`; availability never reads records, so a conflicted one
/// changes nothing there.
#[test]
fn an_unreadable_record_is_bad_metadata_in_add() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("data.txt"), "content").unwrap();
    let added = document(&run(root.path(), &["add", "data.txt", "--json"]));
    let hash = added["files"][0]["hash"].as_str().unwrap().to_string();
    let conflicted = "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> theirs\n";
    std::fs::write(root.path().join(".dvs/data.txt.dvs"), conflicted).unwrap();

    let output = run(root.path(), &["add", "data.txt", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let row = &document(&output)["files"][0];
    assert_eq!(row["code"], "bad_metadata", "{row}");
    assert!(row["message"].as_str().unwrap().contains("data.txt.dvs"));
    // The record is left for the user to fix
    assert_eq!(
        std::fs::read_to_string(root.path().join(".dvs/data.txt.dvs")).unwrap(),
        conflicted
    );

    let output = run(
        root.path(),
        &["availability", &hash, "--path", "data.txt", "--json"],
    );
    assert_eq!(output.status.code(), Some(0));
    let row = &document(&output)["files"][0];
    assert_eq!(row["state"], "present");
    assert!(row["code"].is_null(), "{row}");
}

/// §9 rec 1: availability rows have fixed keys with or without `--path`.
#[test]
fn availability_rows_have_fixed_keys() {
    let (root, _storage) = project();
    let hash = "0".repeat(64);
    let row_keys = |args: &[&str]| {
        let value = document(&run(root.path(), args));
        let mut keys: Vec<_> = value["files"][0]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    let plain = row_keys(&["availability", &hash, "--json"]);
    assert_eq!(
        plain,
        [
            "code",
            "hash",
            "hint",
            "history_code",
            "history_message",
            "in_history",
            "message",
            "state"
        ]
    );
    assert_eq!(
        plain,
        row_keys(&["availability", &hash, "--path", "x.txt", "--json"])
    );
}

/// §9 rec 2: history times are RFC 3339 UTC in JSON, and people see them.
#[test]
fn history_timestamps_are_rfc3339_and_shown_to_people() {
    let (root, _storage) = project();
    std::fs::write(root.path().join("data.txt"), "content").unwrap();
    assert!(run(root.path(), &["add", "data.txt"]).status.success());
    let history = document(&run(root.path(), &["history", "--json"]));
    let entries = history["files"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        let timestamp = entry["timestamp"].as_str().expect("an RFC 3339 string");
        // Same format as started_at: 2026-10-21T14:30:00.000Z
        assert_eq!(timestamp.len(), 24, "{timestamp}");
        assert!(timestamp.ends_with('Z') && timestamp.as_bytes()[10] == b'T');
        assert!(timestamp <= history["finished_at"].as_str().unwrap());
    }
    let human = run(root.path(), &["history"]);
    let text = String::from_utf8_lossy(&human.stdout);
    let line = text.lines().find(|l| l.contains("data.txt")).unwrap();
    let time = line.split_whitespace().next().unwrap();
    assert_eq!(time, entries[1]["timestamp"], "{text}");
}

/// §9 rec 5: get counts carry `total` and `succeeded` next to the detail;
/// held-back rows are in neither `succeeded` nor `failed`.
#[test]
fn get_counts_have_total_and_succeeded() {
    let (root, _storage) = project();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(root.path().join(name), name).unwrap();
    }
    assert!(
        run(root.path(), &["add", "a.txt", "b.txt", "c.txt"])
            .status
            .success()
    );
    std::fs::remove_file(root.path().join("a.txt")).unwrap();
    let dry = document(&run(root.path(), &["get", "--dry-run", "--json"]));
    assert_eq!(dry["counts"]["total"], 3, "{dry}");
    assert_eq!(dry["counts"]["succeeded"], 3);
    assert_eq!(dry["counts"]["would_restore"], 1);
    let get = document(&run(root.path(), &["get", "--json"]));
    assert_eq!(get["counts"]["total"], 3);
    assert_eq!(get["counts"]["succeeded"], 3);
    assert_eq!(get["counts"]["restored"], 1);
    assert_eq!(get["counts"]["unchanged"], 2);
    // A conflict holds the other files back: in neither succeeded nor failed
    std::fs::remove_file(root.path().join("a.txt")).unwrap();
    std::fs::write(root.path().join("b.txt"), "local edit").unwrap();
    let aborted = document(&run(root.path(), &["get", "--json"]));
    assert_eq!(aborted["outcome"], "aborted");
    assert_eq!(aborted["counts"]["total"], 3);
    assert_eq!(aborted["counts"]["succeeded"], 1);
    assert_eq!(aborted["counts"]["failed"], 0);
    assert_eq!(aborted["counts"]["not_run"], 2);
}
