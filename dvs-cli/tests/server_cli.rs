//! The CLI against a stand-in dvs-server: error codes (C4), credentials in
//! machine mode (C5), URL path prefixes (C6) and byte totals (C8).
mod common;

use common::{PROJECT, Sandbox, Stub, document};

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
