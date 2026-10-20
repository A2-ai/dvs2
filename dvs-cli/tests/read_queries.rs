use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dvs"))
        .current_dir(root)
        .args(args)
        .env_remove("DVS_GET_ON_CONFLICT")
        .output()
        .unwrap()
}
fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn project() -> (TempDir, TempDir) {
    let root = TempDir::new().unwrap();
    let storage = TempDir::new().unwrap();
    let init = run(
        root.path(),
        &[
            "init",
            "local",
            storage.path().join("objects").to_str().unwrap(),
            "--json",
        ],
    );
    assert!(init.status.success(), "{}", document(&init));
    (root, storage)
}
fn add(root: &Path, path: &str, contents: &str) -> String {
    std::fs::write(root.join(path), contents).unwrap();
    let output = run(root, &["add", path, "--json"]);
    assert!(output.status.success(), "{}", document(&output));
    document(&output)["files"][0]["hash"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn status_uses_local_baseline_and_reports_unsynced_without_one() {
    let (root, _storage) = project();
    add(root.path(), "data.txt", "first version");
    add(root.path(), "data.txt", "second version");
    add(root.path(), "zz-current.txt", "current version");
    // Simulate metadata arriving through Git while the local data remains at
    // the version DVS last installed. The cache baseline distinguishes stale
    // data from a local edit without consulting history.
    std::fs::copy(
        root.path().join(".dvs/zz-current.txt.dvs"),
        root.path().join(".dvs/data.txt.dvs"),
    )
    .unwrap();
    let stale = document(&run(root.path(), &["status", "--json"]));
    let stale_row = stale["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == "data.txt")
        .unwrap();
    assert_eq!(stale_row["status"], "stale");
    assert_eq!(stale_row["working"]["state"], "present");
    assert!(stale_row.get("state").is_none());
    let human_stale = run(root.path(), &["status"]);
    let human_stale = String::from_utf8_lossy(&human_stale.stdout);
    assert!(human_stale.contains("stale"), "{human_stale}");
    let stale_only = run(root.path(), &["status", "--stale"]);
    assert!(stale_only.status.success());
    let stale_only = String::from_utf8_lossy(&stale_only.stdout);
    assert!(stale_only.contains("data.txt"), "{stale_only}");
    assert!(!stale_only.contains("zz-current.txt"), "{stale_only}");

    std::fs::write(root.path().join("data.txt"), "new unrecorded edit").unwrap();
    let modified = run(root.path(), &["status", "--json"]);
    assert!(modified.status.success(), "{}", document(&modified));
    assert_eq!(document(&modified)["files"][0]["status"], "modified");
    let human_modified = run(root.path(), &["status"]);
    let human_modified = String::from_utf8_lossy(&human_modified.stdout);
    assert!(human_modified.contains("modified"), "{human_modified}");
    let modified_only = run(root.path(), &["status", "--modified"]);
    assert!(modified_only.status.success());
    let modified_only = String::from_utf8_lossy(&modified_only.stdout);
    assert!(modified_only.contains("data.txt"), "{modified_only}");
    assert!(!modified_only.contains("zz-current.txt"), "{modified_only}");
    let unsynced = run(root.path(), &["status", "--unsynced", "--json"]);
    assert!(unsynced.status.success());
    assert!(document(&unsynced)["files"].as_array().unwrap().is_empty());

    std::fs::remove_dir_all(root.path().join(".dvs/.cache")).unwrap();
    let unsynced = run(root.path(), &["status", "--json"]);
    assert!(unsynced.status.success(), "{}", document(&unsynced));
    let unsynced = document(&unsynced);
    assert!(
        unsynced["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| { row["path"] == "data.txt" && row["status"] == "unsynced" })
    );
}

#[test]
fn availability_checks_batches_and_optional_path_history() {
    let (root, storage) = project();
    let hash = add(root.path(), "data.txt", "recorded");
    let other_hash = add(root.path(), "other.txt", "other recorded content");
    let absent = "0".repeat(64);
    let output = run(
        root.path(),
        &[
            "availability",
            &hash.to_uppercase(),
            &absent,
            &hash,
            "--json",
        ],
    );
    assert!(output.status.success());
    let value = document(&output);
    assert_eq!(value["files"][0]["hash"], hash);
    assert_eq!(value["files"][0]["state"], "present");
    assert_eq!(value["files"][1]["state"], "absent");
    assert_eq!(value["files"][2]["state"], "present");
    let path_query = run(
        root.path(),
        &[
            "availability",
            &hash,
            &other_hash,
            "--path",
            "data.txt",
            "--json",
        ],
    );
    assert!(path_query.status.success(), "{}", document(&path_query));
    let path_query = document(&path_query);
    assert_eq!(path_query["files"][0]["state"], "present");
    assert_eq!(path_query["files"][0]["in_history"], true);
    assert_eq!(path_query["files"][1]["state"], "present");
    assert_eq!(path_query["files"][1]["in_history"], false);
    std::fs::remove_file(storage.path().join("objects/audit.log.jsonl")).unwrap();
    let history_unavailable = run(
        root.path(),
        &["availability", &hash, "--path", "data.txt", "--json"],
    );
    assert_eq!(history_unavailable.status.code(), Some(1));
    let history_unavailable = document(&history_unavailable);
    assert_eq!(history_unavailable["files"][0]["state"], "present");
    assert!(history_unavailable["files"][0]["in_history"].is_null());
    std::fs::remove_dir_all(storage.path().join("objects")).unwrap();
    let unknown = run(root.path(), &["availability", &hash, "--json"]);
    assert_eq!(unknown.status.code(), Some(1));
    assert_eq!(document(&unknown)["files"][0]["state"], "unknown");
    let invalid = run(root.path(), &["availability", &hash, "invalid", "--json"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(document(&invalid)["error"]["code"], "invalid_arguments");
    assert!(document(&invalid)["files"].as_array().unwrap().is_empty());
}

#[test]
fn history_supports_absent_files_subdirectories_and_newline_paths() {
    let (root, _storage) = project();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let filename = "line\nbreak.txt";
    add(root.path(), &format!("data/{filename}"), "recorded");
    std::fs::remove_file(root.path().join("data").join(filename)).unwrap();
    let output = run(&root.path().join("data"), &["history", filename, "--json"]);
    assert!(output.status.success(), "{}", document(&output));
    let value = document(&output);
    assert_eq!(value["files"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["files"][0]["action"]["add"]["file"]["path"],
        format!("data/{filename}")
    );
    let all = document(&run(root.path(), &["history", "--json"]));
    assert!(all["files"].as_array().unwrap().len() >= 2);
    let invalid = run(root.path(), &["history", "../outside", "--json"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(document(&invalid)["error"]["code"], "invalid_arguments");
}

#[test]
fn removed_cache_and_stdin_modes_are_not_accepted() {
    let (root, _storage) = project();
    let hash = "a".repeat(64);
    for args in [
        vec!["--json", "status", "--cache-only"],
        vec!["--json", "availability", &hash, "--stdin"],
        vec!["--json", "history", "--stdin"],
    ] {
        let output = run(root.path(), &args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(document(&output)["error"]["code"], "invalid_arguments");
    }
}

#[test]
fn rich_status_keeps_metadata_errors_in_the_result() {
    let (root, _storage) = project();
    add(root.path(), "data.txt", "recorded");
    std::fs::write(
        root.path().join(".dvs/data.txt.dvs"),
        "<<<<<<< merge conflict",
    )
    .unwrap();
    let output = run(root.path(), &["status", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let value = document(&output);
    assert_eq!(value["files"][0]["code"], "bad_metadata");
    assert_eq!(value["outcome"], "failed");
}
