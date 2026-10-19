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
}
#[test]
fn offline_info_discovers_from_subdirectories() {
    let (root, _storage) = project();
    let child = root.path().join("child");
    std::fs::create_dir(&child).unwrap();
    let output = run(&child, &["info", "--offline", "--json"]);
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
    assert_eq!(value["offline"], true);
    assert_eq!(value["protocol"]["versions"], serde_json::json!([1]));
}
#[test]
fn offline_server_info_does_not_connect_and_auth_json_does_not_start() {
    let root = TempDir::new().unwrap();
    std::fs::write(
        root.path().join("dvs.toml"),
        "[backend]\ntype = 'server'\nname = 'example'\nurl = 'http://127.0.0.1:1/'\n",
    )
    .unwrap();
    let output = run(root.path(), &["info", "--offline", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(document(&output)["logged_in"].is_null());
    assert!(document(&output)["access"].is_null());
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
