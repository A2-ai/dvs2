//! The CLI against a stand-in dvs-server: error codes (C4), credentials in
//! machine mode (C5), URL path prefixes (C6) and byte totals (C8).
mod common;

use common::{PROJECT, Sandbox, Stub, document, route, stderr};
use serde_json::Value;

/// A logged-in server project.
fn server_project(stub: &Stub) -> Sandbox {
    let sandbox = Sandbox::new();
    sandbox.init_server(&stub.url);
    sandbox.login(&stub.url, "good-token");
    sandbox
}

/// `names` added to the stub, then deleted locally, ready for `get`.
fn added(sandbox: &Sandbox, names: &[&str]) {
    for name in names {
        sandbox.write(name, format!("contents of {name}"));
    }
    let output = sandbox
        .dvs()
        .args(["--json", "add"])
        .args(names)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", document(&output));
    for name in names {
        std::fs::remove_file(sandbox.project().join(name)).unwrap();
    }
}

/// C6: a server behind a path prefix, with and without the trailing slash,
/// from login to upload to download.
#[test]
fn a_server_url_path_prefix_is_kept_on_every_request() {
    for trailing in ["/", ""] {
        let stub = Stub::with_prefix("/dvs");
        let url = format!("{}{trailing}", stub.url.trim_end_matches('/'));
        let sandbox = Sandbox::new();
        sandbox.init_server(&url);
        // Device login: the stub approves at once, so no one has to
        let login = sandbox.run(&["login", &url]);
        assert!(login.status.success(), "{}", common::stderr(&login));
        added(&sandbox, &["data.txt"]);
        let get = sandbox.run(&["--json", "get", "data.txt"]);
        assert!(get.status.success(), "{}", document(&get));
        assert_eq!(sandbox.read("data.txt"), b"contents of data.txt");
        let requests = stub.requests();
        let paths: Vec<_> = requests.iter().map(|r| r.path.as_str()).collect();
        assert!(
            paths.iter().all(|p| p.starts_with("/dvs/api/")),
            "{paths:?}"
        );
        for expected in [
            "/dvs/api/oauth/device/authorize".to_string(),
            "/dvs/api/oauth/token".to_string(),
            format!("/dvs/api/{PROJECT}/access"),
            format!("/dvs/api/{PROJECT}/manifest"),
        ] {
            assert!(paths.contains(&expected.as_str()), "{expected}: {paths:?}");
        }
        assert!(
            requests
                .iter()
                .any(|r| r.method == "GET" && r.path.starts_with("/dvs/api/proj/blobs/"))
        );
    }
}

/// The access check answers `status` with `body`.
fn access_fails(stub: &Stub, status: u16, body: &str) {
    stub.fail(route("GET", "access"), status, body);
}

/// C4: one shared vocabulary at the top level.
#[test]
fn backend_failures_have_one_code_at_the_top_level() {
    // (code, how the access check fails, logged in)
    type Case = (&'static str, Option<(u16, &'static str)>, bool);
    let cases: [Case; 6] = [
        ("auth_required", None, false),
        ("auth_expired", Some((401, "")), true),
        ("access_denied", Some((403, "")), true),
        (
            "project_not_found",
            Some((404, "unknown project \"proj\"")),
            true,
        ),
        ("server_too_old", Some((404, "")), true),
        ("server_unavailable", Some((503, "")), true),
    ];
    for (code, failure, logged_in) in cases {
        let stub = Stub::start();
        let sandbox = Sandbox::new();
        sandbox.init_server(&stub.url);
        if logged_in {
            sandbox.login(&stub.url, "good-token");
        }
        if let Some((status, body)) = failure {
            access_fails(&stub, status, body);
        }
        sandbox.write("data.txt", "data");
        let add = sandbox.run(&["--json", "add", "data.txt"]);
        assert_eq!(add.status.code(), Some(1), "{code}");
        assert_eq!(document(&add)["error"]["code"], code, "{}", document(&add));
    }
    // A server that cannot be reached at all
    let sandbox = Sandbox::new();
    sandbox.init_server("http://127.0.0.1:1/");
    sandbox.login("http://127.0.0.1:1/", "good-token");
    sandbox.write("data.txt", "data");
    let add = sandbox.run(&["--json", "add", "data.txt"]);
    assert_eq!(document(&add)["error"]["code"], "server_unavailable");
}

/// C4: per-file failures carry the shared codes in get rows, and a failed
/// access check gives every file its shared code, not one catch-all.
#[test]
fn per_file_backend_failures_have_the_shared_codes() {
    for (status, body, code) in [
        (401, "", "auth_expired"),
        (403, "", "access_denied"),
        (404, "unknown project \"proj\"", "project_not_found"),
        (405, "", "server_too_old"),
        (500, "", "server_unavailable"),
    ] {
        let stub = Stub::start();
        let sandbox = server_project(&stub);
        added(&sandbox, &["down.txt"]);
        stub.fail(route("GET", "blobs/"), status, body);
        let get = sandbox.run(&["--json", "get", "down.txt"]);
        assert_eq!(get.status.code(), Some(1));
        let result = document(&get);
        assert_eq!(result["files"][0]["code"], code, "{result}");

        let stub = Stub::start();
        let sandbox = server_project(&stub);
        added(&sandbox, &["down.txt"]);
        access_fails(&stub, status, body);
        let get = sandbox.run(&["--json", "get", "down.txt"]);
        assert_eq!(get.status.code(), Some(1));
        let result = document(&get);
        assert_eq!(result["files"][0]["code"], code, "{result}");
    }
}

/// C4, K25: get keeps `bad_metadata` for an unreadable record.
#[test]
fn conflicted_records_keep_bad_metadata_in_get() {
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    added(&sandbox, &["merged.txt"]);
    sandbox.write(
        ".dvs/merged.txt.dvs",
        "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> theirs\n",
    );
    let get = document(&sandbox.run(&["--json", "get", "merged.txt"]));
    assert_eq!(get["files"][0]["code"], "bad_metadata");
}

/// C5: machine mode leaves a rejected token in place and reports it.
#[test]
fn machine_mode_keeps_a_rejected_token() {
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    added(&sandbox, &["tracked.txt"]);
    access_fails(&stub, 401, "");
    sandbox.write("data.txt", "data");
    let before = sandbox.credentials().unwrap();
    for args in [
        vec!["--json", "add", "data.txt"],
        vec!["--json", "add", "--dry-run", "data.txt"],
        vec!["--json", "get", "tracked.txt"],
        vec!["--json", "info"],
    ] {
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let result = document(&output);
        let code = result["error"]["code"]
            .as_str()
            .or(result["files"][0]["code"].as_str());
        assert_eq!(code, Some("auth_expired"), "{result}");
        assert_eq!(sandbox.credentials().unwrap(), before, "{args:?}");
    }
    // A 401 on a file's upload too
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    stub.fail(route("PUT", "blobs/"), 401, "");
    sandbox.write("data.txt", "data");
    let before = sandbox.credentials().unwrap();
    let output = sandbox.run(&["--json", "add", "data.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(document(&output)["outcome"], "failed");
    assert_eq!(sandbox.credentials().unwrap(), before);
}

/// C5: human mode still forgets a rejected token and says to log in.
#[test]
fn human_mode_still_forgets_a_rejected_token() {
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    access_fails(&stub, 401, "");
    sandbox.write("data.txt", "data");
    let output = sandbox.run(&["add", "data.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("dvs login"), "{}", stderr(&output));
    let store: Value = serde_json::from_slice(&sandbox.credentials().unwrap()).unwrap();
    assert!(store.get(&stub.url).is_none(), "{store}");
}

/// C5: an unparsable credentials store is reported and kept in machine
/// mode, and deleted (as before) in human mode.
#[test]
fn machine_mode_keeps_an_unreadable_credentials_store() {
    let stub = Stub::start();
    let sandbox = Sandbox::new();
    sandbox.init_server(&stub.url);
    sandbox.write_credentials("{ not json");
    sandbox.write("data.txt", "data");
    for args in [
        vec!["--json", "add", "data.txt"],
        vec!["--json", "add", "--dry-run", "data.txt"],
        vec!["--json", "info"],
    ] {
        let output = sandbox.run(&args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        let result = document(&output);
        assert_eq!(
            result["error"]["code"], "credentials_unreadable",
            "{result}"
        );
        assert_eq!(sandbox.credentials().unwrap(), b"{ not json");
    }
    let output = sandbox.run(&["add", "data.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(sandbox.credentials().is_none());
}

/// C4, C5: attach goes through the same codes and credential policy.
#[test]
fn attach_reports_shared_codes_and_keeps_credentials_in_machine_mode() {
    for (status, body, code) in [
        (401, "", "auth_expired"),
        (404, "unknown project \"proj\"", "project_not_found"),
    ] {
        let stub = Stub::start();
        let sandbox = Sandbox::new();
        sandbox.login(&stub.url, "good-token");
        stub.fail(
            move |request| request.path.ends_with(&format!("/api/project/{PROJECT}")),
            status,
            body,
        );
        let before = sandbox.credentials().unwrap();
        let output = sandbox.run(&["--json", "attach", PROJECT, &stub.url]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(document(&output)["error"]["code"], code);
        assert_eq!(sandbox.credentials().unwrap(), before);
        assert!(!sandbox.project().join("dvs.toml").exists());
    }
}

/// C8 with a server: what is already stored is not to transfer, and only
/// what moved is transferred.
#[test]
fn add_bytes_split_stored_from_uploaded_content() {
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    sandbox.write("old.txt", "already stored");
    assert!(sandbox.run(&["--json", "add", "old.txt"]).status.success());
    sandbox.write("copy.txt", "already stored");
    sandbox.write("new.txt", "brand new");
    for dry_run in [true, false] {
        let mut args = vec!["--json", "add", "copy.txt", "new.txt"];
        if dry_run {
            args.insert(2, "--dry-run");
        }
        let result = document(&sandbox.run(&args));
        assert_eq!(
            result["bytes"],
            serde_json::json!({
                "total": 23,
                "to_transfer": 9,
                "already_stored": 14,
                "transferred": if dry_run { 0 } else { 9 },
            }),
            "{result}"
        );
    }
    // get: what is already in place is not to transfer
    std::fs::remove_file(sandbox.project().join("new.txt")).unwrap();
    for dry_run in [true, false] {
        let mut args = vec!["--json", "get", "copy.txt", "new.txt"];
        if dry_run {
            args.insert(2, "--dry-run");
        }
        let result = document(&sandbox.run(&args));
        assert_eq!(
            result["bytes"],
            serde_json::json!({
                "total": 23,
                "to_transfer": 9,
                "already_stored": 14,
                "transferred": if dry_run { 0 } else { 9 },
            }),
            "{result}"
        );
    }
}

/// C4, §9 rec 3, K26: info reports what the server said with the same codes, as
/// a `connection_error: {code, message}` object.
#[test]
fn info_reports_connection_codes() {
    for (failure, code) in [
        ((503, ""), "server_unavailable"),
        ((404, "unknown project \"proj\""), "project_not_found"),
        ((404, ""), "server_too_old"),
    ] {
        let stub = Stub::start();
        let sandbox = server_project(&stub);
        access_fails(&stub, failure.0, failure.1);
        let info = sandbox.run(&["--json", "info"]);
        assert!(info.status.success(), "{}", stderr(&info));
        let info = document(&info);
        assert_eq!(info["connection"], "unavailable");
        assert_eq!(info["connection_error"]["code"], code, "{info}");
        assert!(info["connection_error"]["message"].is_string());
        assert!(info.get("connection_code").is_none());
        assert!(info["access"].is_null());
    }
    // A rejected login is an error envelope (K24)
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    access_fails(&stub, 401, "");
    let info = sandbox.run(&["--json", "info"]);
    assert_eq!(info.status.code(), Some(1));
    assert_eq!(document(&info)["error"]["code"], "auth_expired");
    // A 403 is an answer, no membership, like human mode's "no access" (K26)
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    access_fails(&stub, 403, "");
    let info = sandbox.run(&["--json", "info"]);
    assert_eq!(info.status.code(), Some(0), "{}", stderr(&info));
    let info = document(&info);
    assert!(info["error"].is_null(), "{info}");
    assert_eq!(info["outcome"], "complete");
    assert_eq!(info["logged_in"], true);
    assert_eq!(info["connection"], "available");
    assert!(info["connection_error"].is_null(), "{info}");
    assert_eq!(info["access"], "none");
    let human = sandbox.run(&["info"]);
    assert!(human.status.success(), "{}", stderr(&human));
    assert!(String::from_utf8_lossy(&human.stdout).contains("(no access)"));
    let stub = Stub::start();
    let sandbox = server_project(&stub);
    let info = document(&sandbox.run(&["--json", "info"]));
    assert_eq!(info["connection"], "available");
    assert!(info["connection_error"].is_null());
    assert_eq!(info["access"], "write");
    // Not logged in is an answer, not an error, and not "none" (§9 rec 3)
    let sandbox = Sandbox::new();
    sandbox.init_server(&stub.url);
    let info = document(&sandbox.run(&["--json", "info"]));
    assert_eq!(info["logged_in"], false);
    assert_eq!(info["access"], "not_logged_in");
    assert!(info["connection_error"].is_null());
}
