use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dvs"))
        .current_dir(root)
        .args(args)
        .env("XDG_CONFIG_HOME", root.join("test-config"))
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
        result["problem"]
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
    assert_eq!(document(&output)["access"], "none");
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
    assert!(value["connection_error"].is_string());
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
