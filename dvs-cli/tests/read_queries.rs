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
fn status_reports_differences_as_unsynced_without_consulting_history() {
    let (root, _storage) = project();
    add(root.path(), "data.txt", "first version");
    add(root.path(), "data.txt", "second version");
    add(root.path(), "zz-current.txt", "current version");
    std::fs::write(root.path().join("data.txt"), "first version").unwrap();
    let status = run(root.path(), &["status", "--json"]);
    assert!(status.status.success(), "{}", document(&status));
    let status = document(&status);
    assert_eq!(status["files"][0]["status"], "unsynced");
    assert_eq!(status["files"][0]["state"], "modified");
    assert!(status["files"][0].get("before").is_none());
    let human = run(root.path(), &["status"]);
    let human = String::from_utf8_lossy(&human.stdout);
    assert!(human.contains("unsynced"), "{human}");
    let unsynced_only = run(root.path(), &["status", "--unsynced"]);
    assert!(unsynced_only.status.success());
    let unsynced_only = String::from_utf8_lossy(&unsynced_only.stdout);
    assert!(unsynced_only.contains("data.txt"), "{unsynced_only}");
    assert!(!unsynced_only.contains("zz-current.txt"), "{unsynced_only}");

    std::fs::write(root.path().join("data.txt"), "new unrecorded edit").unwrap();
    let modified = run(root.path(), &["status", "--json"]);
    assert!(modified.status.success(), "{}", document(&modified));
    assert_eq!(document(&modified)["files"][0]["status"], "unsynced");
    let human_modified = run(root.path(), &["status"]);
    let human_modified = String::from_utf8_lossy(&human_modified.stdout);
    assert!(human_modified.contains("unsynced"), "{human_modified}");
    let modified_only = run(root.path(), &["status", "--unsynced"]);
    assert!(modified_only.status.success());
    let modified_only = String::from_utf8_lossy(&modified_only.stdout);
    assert!(modified_only.contains("data.txt"), "{modified_only}");
    assert!(!modified_only.contains("zz-current.txt"), "{modified_only}");
    std::fs::remove_file(_storage.path().join("objects/audit.log.jsonl")).unwrap();
    let unknown = run(root.path(), &["status", "--json"]);
    assert!(unknown.status.success(), "{}", document(&unknown));
    assert_eq!(document(&unknown)["files"][0]["status"], "unsynced");
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
