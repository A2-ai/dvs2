//! The `dvs` library's server backend against the fake server, for calls the CLI does
//! not make. One test: it sets the process environment.

#[allow(dead_code)]
mod fake;

use std::sync::atomic::Ordering;

use dvs::backends::server::ServerBackend;
use dvs::remote::api_error;
use fake::{Fake, PROJECT, TOKEN};
use url::Url;

fn backend(fake: &Fake) -> ServerBackend {
    ServerBackend::new(PROJECT.into(), None, Url::parse(&fake.url).unwrap())
}

#[test]
fn versions_pages_and_content_checks() {
    let fake = Fake::start();
    let config = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary, so no other thread reads the environment
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", config.path());
        std::env::set_var("DVS_TOKEN", TOKEN);
    }

    // Versions: path, limit and cursor go in the query
    let b = backend(&fake);
    let first = b.versions("data/a b.csv", None, Some(1)).unwrap();
    assert_eq!(first.items.len(), 1);
    let cursor = first.next_cursor.unwrap();
    let second = b.versions("data/a b.csv", Some(&cursor), Some(1)).unwrap();
    assert!(second.next_cursor.is_none());
    assert_eq!(second.items[0].hash, "a".repeat(64));
    let req = fake
        .requests()
        .into_iter()
        .rev()
        .find(|r| r.is("GET", "/versions"))
        .unwrap();
    assert_eq!(req.query["path"], "data/a b.csv");
    assert_eq!(req.query["limit"], "1");
    assert_eq!(req.query["cursor"], cursor);

    // A HEAD the server refuses: 403 is about access
    fake.faults.head_status.store(403, Ordering::SeqCst);
    let hashes = dvs::Hashes {
        blake3: "c".repeat(64),
        md5: None,
    };
    let e = b.exists(&hashes).unwrap_err();
    assert_eq!(api_error(&e).unwrap().code, "insufficient_role");
    fake.faults.head_status.store(0, Ordering::SeqCst);

    // A rejected stored token, seen by a HEAD: an auth error, and the token is kept,
    // because a HEAD has no body to say why
    let url = Url::parse(&fake.url).unwrap();
    dvs::auth::store_token(&url, "stale").unwrap();
    unsafe { std::env::remove_var("DVS_TOKEN") };
    let e = backend(&fake).exists(&hashes).unwrap_err();
    let api = api_error(&e).unwrap();
    assert_eq!(api.code, "unauthenticated");
    assert!(api.message.contains("dvs login"), "{api:?}");
    assert_eq!(
        dvs::auth::get_token(&url).unwrap().as_deref(),
        Some("stale")
    );
}
