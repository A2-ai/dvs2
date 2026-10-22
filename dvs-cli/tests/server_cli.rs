mod common;

use common::{Sandbox, Stub, document};
use serde_json::Value;

#[test]
fn a_rejected_token_is_kept_in_machine_mode_and_forgotten_in_human_mode() {
    let stub = Stub::start(401);
    let sandbox = Sandbox::new();
    sandbox.init_server(&stub.url);
    sandbox.login(&stub.url, "good-token");
    sandbox.write("data.txt", "data");
    let before = sandbox.credentials().unwrap();
    let output = sandbox.run(&["--json", "add", "data.txt"]);
    assert_eq!(document(&output)["error"]["code"], "auth_expired");
    assert_eq!(sandbox.credentials().unwrap(), before);

    let output = sandbox.run(&["add", "data.txt"]);
    assert_eq!(output.status.code(), Some(1));
    let store: Value = serde_json::from_slice(&sandbox.credentials().unwrap()).unwrap();
    assert!(store.get(&stub.url).is_none(), "{store}");
}

#[test]
fn a_corrupt_credentials_store_is_kept_in_machine_mode_and_deleted_in_human_mode() {
    let sandbox = Sandbox::new();
    sandbox.init_server("http://127.0.0.1:1/");
    sandbox.write_credentials("{ not json");
    sandbox.write("data.txt", "data");
    let output = sandbox.run(&["--json", "add", "data.txt"]);
    assert_eq!(document(&output)["error"]["code"], "credentials_unreadable");
    assert_eq!(sandbox.credentials().unwrap(), b"{ not json");

    let output = sandbox.run(&["add", "data.txt"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(sandbox.credentials().is_none());
}
