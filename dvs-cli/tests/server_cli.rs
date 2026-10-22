//! The CLI against a stand-in dvs-server: error codes (C4), credentials in
//! machine mode (C5), URL path prefixes (C6) and byte totals (C8).
mod common;

use common::{PROJECT, Sandbox, Stub, document, route};

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
