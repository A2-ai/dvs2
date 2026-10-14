//! The CLI against a fake server: the transfer protocol, its failures, tokens and
//! exit codes.

mod fake;

use std::sync::atomic::Ordering;

use fake::{Checkout, Fake, PROJECT, TOKEN};

fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// A checkout attached to the fake server's project.
fn attached(fake: &Fake) -> Checkout {
    let c = Checkout::new();
    let out = c.dvs_ok(&["attach", PROJECT, &fake.url]);
    assert!(out.stdout.contains("your role: write"), "{out:?}");
    c
}

#[test]
fn attach_then_add_and_get_round_trip() {
    let fake = Fake::start();
    let c = attached(&fake);
    // `attach` creates the metadata folder, so the first add works
    assert!(c.repo().join(".dvs").is_dir());

    c.write("a.csv", b"alpha");
    c.write("data/b.csv", b"alpha");
    c.write("c.csv", b"gamma");
    let out = c.dvs_ok(&[
        "--json",
        "add",
        "a.csv",
        "data/b.csv",
        "c.csv",
        "-m",
        "first",
    ]);
    assert_eq!(
        out.outcomes(),
        pairs(&[
            ("a.csv", "added"),
            ("c.csv", "added"),
            ("data/b.csv", "added")
        ])
    );
    // Shared content goes up once
    assert_eq!(fake.count("PUT", ""), 2);
    let open = fake
        .requests()
        .into_iter()
        .find(|r| r.is("POST", "/batches"))
        .unwrap();
    assert_eq!(open.json()["kind"], "upload");
    assert_eq!(open.json()["message"], "first");
    assert_eq!(
        open.json()["git"],
        serde_json::json!({"state": "unborn", "branch": "main"})
    );
    assert!(
        open.headers["user-agent"].starts_with("dvs/0.5.0 ("),
        "{:?}",
        open.headers
    );

    // A copy of stored content: recorded, no bytes
    c.write("d.csv", b"gamma");
    let out = c.dvs_ok(&["--json", "add", "a.csv", "d.csv"]);
    assert_eq!(
        out.outcomes(),
        pairs(&[("a.csv", "skipped"), ("d.csv", "present")])
    );
    assert_eq!(fake.count("PUT", ""), 2);

    // Nothing changed: no batch at all
    let batches = fake.count("POST", "/batches");
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert_eq!(out.outcomes(), pairs(&[("a.csv", "skipped")]));
    assert_eq!(fake.count("POST", "/batches"), batches);

    for f in ["a.csv", "data/b.csv", "c.csv", "d.csv"] {
        std::fs::remove_file(c.repo().join(f)).unwrap();
    }
    c.write("c.csv", b"local edits");
    let out = c.dvs_ok(&["--json", "get", "--glob", "**/*"]);
    assert!(
        out.json()
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["outcome"] == "copied"),
        "{out:?}"
    );
    assert_eq!(
        std::fs::read(c.repo().join("data/b.csv")).unwrap(),
        b"alpha"
    );
    assert_eq!(std::fs::read(c.repo().join("c.csv")).unwrap(), b"gamma");
    let downloads: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.is("GET", "") && r.path.contains("/blobs/"))
        .collect();
    assert_eq!(downloads.len(), 4);
    let mut paths: Vec<_> = downloads.iter().map(|r| r.query["path"].clone()).collect();
    paths.sort();
    assert_eq!(paths, ["a.csv", "c.csv", "d.csv", "data/b.csv"]);
    // One download batch for the lot, closed at the end
    let batch = &downloads[0].query["batch"];
    assert!(downloads.iter().all(|r| &r.query["batch"] == batch));
    let completes: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.is("POST", "/complete"))
        .collect();
    let last = completes.last().unwrap();
    assert!(last.path.contains(batch.as_str()));
    assert_eq!(last.json()["ok"], 4);

    // Everything is present: no download batch
    let batches = fake.count("POST", "/batches");
    c.dvs_ok(&["get", "--glob", "**/*"]);
    assert_eq!(fake.count("POST", "/batches"), batches);
}

#[test]
fn the_manifest_is_sent_in_chunks() {
    let fake = Fake::start();
    fake.faults.max_manifest_entries.store(2, Ordering::SeqCst);
    let c = attached(&fake);
    for i in 0..5 {
        c.write(&format!("f{i}.csv"), format!("content {i}").as_bytes());
    }
    let out = c.dvs_ok(&["--json", "add", "--glob", "*.csv"]);
    assert!(out.outcomes().iter().all(|(_, o)| o == "added"), "{out:?}");
    // The fake asserts each call is within the limit
    assert_eq!(fake.count("POST", "/manifest"), 3);
    assert_eq!(fake.count("POST", "/batches"), 1);
}

#[test]
fn a_failed_chunk_stops_later_chunks_and_earlier_ones_finish() {
    let fake = Fake::start();
    fake.faults.max_manifest_entries.store(2, Ordering::SeqCst);
    fake.faults.fail_manifest_call.store(2, Ordering::SeqCst);
    let c = attached(&fake);
    for i in 0..5 {
        c.write(&format!("f{i}.csv"), format!("content {i}").as_bytes());
    }
    let out = c.dvs(Some(TOKEN), &["--json", "add", "--glob", "*.csv"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.outcomes(),
        pairs(&[
            ("f0.csv", "added"),
            ("f1.csv", "added"),
            ("f2.csv", "failed"),
            ("f3.csv", "failed"),
            ("f4.csv", "not_attempted"),
        ])
    );
    let rows = out.json();
    assert_eq!(rows[2]["code"], "batch_closed");
    assert!(c.repo().join(".dvs/f1.csv.dvs").is_file());
    assert!(!c.repo().join(".dvs/f2.csv.dvs").exists());
    assert_eq!(fake.count("POST", "/manifest"), 2);
    let complete = fake
        .requests()
        .into_iter()
        .find(|r| r.is("POST", "/complete"))
        .unwrap();
    assert_eq!(
        complete.json(),
        serde_json::json!({"ok": 2, "failed": 3, "skipped": 0})
    );
}

#[test]
fn a_lost_upload_response_is_retried_once() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    fake.faults.drop_puts.store(1, Ordering::SeqCst);
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert_eq!(out.outcomes(), pairs(&[("a.csv", "added")]));
    assert_eq!(fake.count("PUT", ""), 2);

    c.write("b.csv", b"beta");
    fake.faults.drop_puts.store(2, Ordering::SeqCst);
    let out = c.dvs(Some(TOKEN), &["--json", "add", "b.csv"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.outcomes(), pairs(&[("b.csv", "failed")]));
    let error = out.json()[0]["error"].as_str().unwrap().to_string();
    assert!(error.contains("may have been uploaded"), "{error}");
    assert!(!c.repo().join(".dvs/b.csv.dvs").exists());
}

#[test]
fn a_cut_off_upload_response_is_settled_by_asking_the_server() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    fake.faults
        .truncate_put_responses
        .store(1, Ordering::SeqCst);
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert_eq!(out.outcomes(), pairs(&[("a.csv", "added")]));
    // The HEAD found the content, so it was not sent again
    assert_eq!(fake.count("PUT", ""), 1);
    assert_eq!(fake.count("HEAD", ""), 1);
    assert!(c.repo().join(".dvs/a.csv.dvs").is_file());
}

#[test]
fn an_upload_refused_unread_reports_the_lock() {
    let fake = Fake::start();
    let c = attached(&fake);
    // Large enough that the refusal arrives while the body is still being sent
    let big: Vec<u8> = (0..900_000u32).flat_map(|i| i.to_le_bytes()).collect();
    fake.faults
        .max_blob_size
        .store(big.len() * 2, Ordering::SeqCst);
    c.write("big.bin", &big);
    fake.faults.lock_on_put.store(true, Ordering::SeqCst);
    let out = c.dvs(Some(TOKEN), &["--json", "add", "big.bin"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(out.outcomes(), pairs(&[("big.bin", "failed")]));
    assert_eq!(out.json()[0]["code"], "project_locked", "{out:?}");
    // The refusal usually breaks the upload, so the client learns of the lock by asking
    assert_eq!(fake.count("PUT", ""), 1, "not retried");
    assert!(!c.repo().join(".dvs/big.bin.dvs").exists());
}

#[test]
fn content_published_between_manifest_calls_is_not_sent() {
    let fake = Fake::start();
    fake.faults.max_manifest_entries.store(1, Ordering::SeqCst);
    // Another client uploads the shared content after the first call reported it missing
    fake.faults
        .publish_after_manifest_call
        .store(1, Ordering::SeqCst);
    *fake.faults.publish_bytes.lock().unwrap() = b"shared".to_vec();
    let c = attached(&fake);
    c.write("a.csv", b"shared");
    c.write("b.csv", b"shared");
    let out = c.dvs_ok(&["--json", "add", "a.csv", "b.csv"]);
    assert_eq!(
        out.outcomes(),
        pairs(&[("a.csv", "present"), ("b.csv", "present")])
    );
    assert_eq!(fake.count("POST", "/manifest"), 2);
    assert_eq!(fake.count("PUT", ""), 0);
}

#[test]
fn a_lost_manifest_response_is_retried_once() {
    let fake = Fake::start();
    fake.faults.drop_manifests.store(1, Ordering::SeqCst);
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert_eq!(out.outcomes(), pairs(&[("a.csv", "added")]));
    assert_eq!(fake.count("POST", "/manifest"), 2);
    assert!(out.stderr.contains("retrying"), "{out:?}");
}

#[test]
fn a_file_the_server_would_refuse_fails_alone() {
    let fake = Fake::start();
    fake.faults.max_blob_size.store(10, Ordering::SeqCst);
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    c.write("big.csv", b"more than ten bytes");
    c.write("c.csv", b"gamma");
    c.write("d.csv", b"delta");
    // The client cannot know about this one: the server refuses it, and the rest are
    // sent again without it
    *fake.faults.reject_entry_path.lock().unwrap() = Some("c.csv".into());
    let out = c.dvs(
        Some(TOKEN),
        &["--json", "add", "a.csv", "big.csv", "c.csv", "d.csv"],
    );
    assert_eq!(out.code, 1, "{out:?}");
    assert_eq!(
        out.outcomes(),
        pairs(&[
            ("a.csv", "added"),
            ("big.csv", "failed"),
            ("c.csv", "failed"),
            ("d.csv", "added")
        ])
    );
    let rows = out.json();
    assert_eq!(rows[1]["code"], "invalid_entry");
    assert!(
        rows[1]["error"].as_str().unwrap().contains("larger than"),
        "{rows}"
    );
    assert_eq!(rows[2]["code"], "invalid_entry");
    // big.csv never reached the server; c.csv's call was sent again without it
    let manifests: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.is("POST", "/manifest"))
        .map(|r| r.json()["entries"].as_array().unwrap().len())
        .collect();
    assert_eq!(manifests, [3, 2]);
}

#[test]
fn a_failed_complete_is_only_a_warning() {
    let fake = Fake::start();
    fake.faults.fail_complete.store(true, Ordering::SeqCst);
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert_eq!(out.outcomes(), pairs(&[("a.csv", "added")]));
    assert!(c.repo().join(".dvs/a.csv.dvs").is_file());
    assert!(
        out.stderr.contains("closing the upload failed"),
        "the warning shows without RUST_LOG: {out:?}"
    );
}

#[test]
fn the_compression_header_decides_decompression() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha alpha alpha alpha");
    c.dvs_ok(&["add", "a.csv"]);
    let dvs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(c.repo().join(".dvs/a.csv.dvs")).unwrap()).unwrap();
    assert_eq!(dvs["compression"], "none", "the project's compression");

    fake.faults.serve_zstd.store(true, Ordering::SeqCst);
    std::fs::remove_file(c.repo().join("a.csv")).unwrap();
    c.dvs_ok(&["get", "a.csv"]);
    assert_eq!(
        std::fs::read(c.repo().join("a.csv")).unwrap(),
        b"alpha alpha alpha alpha"
    );
}

#[test]
fn dry_runs_use_head_and_open_no_batch() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    c.dvs_ok(&["add", "a.csv"]);
    c.write("b.csv", b"alpha");
    c.write("c.csv", b"gamma");
    let batches = fake.count("POST", "/batches");

    let out = c.dvs_ok(&["--json", "add", "--dry-run", "a.csv", "b.csv", "c.csv"]);
    assert_eq!(
        out.outcomes(),
        pairs(&[
            ("a.csv", "skipped"),
            ("b.csv", "present"),
            ("c.csv", "added")
        ])
    );
    std::fs::remove_file(c.repo().join("a.csv")).unwrap();
    c.dvs_ok(&["get", "--dry-run", "a.csv"]);
    assert!(!c.repo().join("a.csv").exists());
    assert_eq!(fake.count("POST", "/batches"), batches);
    assert_eq!(fake.count("HEAD", ""), 3);
    assert!(!c.repo().join(".dvs/c.csv.dvs").exists());
}

#[test]
fn dvs_token_wins_and_logout_leaves_it_alone() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.store_token(&fake.url, "stored-token");
    c.write("a.csv", b"alpha");
    // DVS_TOKEN is used, not the stored token (which the fake rejects)
    c.dvs_ok(&["add", "a.csv"]);

    let before = fake.requests().len();
    let out = c.dvs(Some(TOKEN), &["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stderr.contains("DVS_TOKEN is set"), "{out:?}");
    let revokes: Vec<_> = fake.requests()[before..]
        .iter()
        .filter(|r| r.is("DELETE", "/auth/token"))
        .cloned()
        .collect();
    assert_eq!(revokes.len(), 1);
    assert_eq!(revokes[0].headers["authorization"], "Bearer stored-token");
    let store = std::fs::read_to_string(c.credentials()).unwrap();
    assert!(!store.contains("stored-token"), "{store}");

    // Nothing stored: nothing revoked
    let out = c.dvs(Some(TOKEN), &["logout"]);
    assert!(out.stdout.contains("not logged in"), "{out:?}");
    assert_eq!(fake.count("DELETE", "/auth/token"), 1);

    // The stored token is DVS_TOKEN itself: forgotten, never revoked
    c.store_token(&fake.url, TOKEN);
    let out = c.dvs(Some(TOKEN), &["logout"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stderr.contains("not revoked"), "{out:?}");
    assert_eq!(fake.count("DELETE", "/auth/token"), 1);
    let store = std::fs::read_to_string(c.credentials()).unwrap();
    assert!(!store.contains(TOKEN), "{store}");
    // And it still works
    c.dvs_ok(&["info"]);
}

#[test]
fn a_rejected_stored_token_is_forgotten() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.store_token(&fake.url, "expired-token");
    c.write("a.csv", b"alpha");
    let out = c.dvs(None, &["add", "a.csv"]);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains("dvs login"), "{out:?}");
    let store = std::fs::read_to_string(c.credentials()).unwrap();
    assert!(!store.contains("expired-token"), "{store}");
}

#[test]
fn exit_codes() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");

    // Usage
    assert_eq!(c.dvs(Some(TOKEN), &["add", "--no-such-flag"]).code, 2);
    // Not logged in
    let out = c.dvs(None, &["add", "a.csv"]);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stderr.contains("not_logged_in"), "{out:?}");
    // No such project, or no access
    fake.faults.no_project.store(true, Ordering::SeqCst);
    assert_eq!(c.dvs(Some(TOKEN), &["add", "a.csv"]).code, 5);
    fake.faults.no_project.store(false, Ordering::SeqCst);
    // The project exists: init suggests attach
    fake.faults.project_exists.store(true, Ordering::SeqCst);
    let other = Checkout::new();
    let out = other.dvs(
        Some(TOKEN),
        &["init", "server", PROJECT, &fake.url, "--group", "analysts"],
    );
    assert_eq!(out.code, 4, "{out:?}");
    assert!(out.stderr.contains("dvs attach"), "{out:?}");
    assert!(!other.repo().join("dvs.toml").exists());
}

#[test]
fn init_creates_the_project_with_the_group_as_admin() {
    let fake = Fake::start();
    let c = Checkout::new();
    c.dvs_ok(&["init", "server", PROJECT, &fake.url, "--group", "analysts"]);
    let create = fake
        .requests()
        .into_iter()
        .find(|r| r.is("POST", "/api/v1/projects"))
        .unwrap();
    assert_eq!(
        create.json()["grants"],
        serde_json::json!([{"group": "analysts", "role": "admin"}])
    );
    assert!(c.repo().join("dvs.toml").is_file());
    assert!(c.repo().join(".dvs").is_dir());
}

#[test]
fn info_shows_who_and_what() {
    let fake = Fake::start();
    let c = attached(&fake);
    let out = c.dvs_ok(&["info"]);
    assert!(out.stdout.contains("dvs-server 0.5.0"), "{out:?}");
    assert!(out.stdout.contains("alice <alice@example.com>"), "{out:?}");
    assert!(out.stdout.contains("from DVS_TOKEN"), "{out:?}");
    assert!(out.stdout.contains("your role: write"), "{out:?}");

    let out = c.dvs(None, &["info"]);
    assert_eq!(out.code, 3, "{out:?}");
    assert!(out.stdout.contains("dvs-server 0.5.0"), "{out:?}");
}

#[test]
fn add_get_and_status_say_what_happened() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    let out = c.dvs_ok(&["add", "a.csv"]);
    assert!(out.stdout.starts_with("Added: a.csv [5 B]"), "{out:?}");
    assert!(out.stderr.contains("commit the new .dvs files"), "{out:?}");

    // Unchanged: said so, and a message for it is not dropped silently
    let out = c.dvs_ok(&["add", "a.csv", "-m", "again"]);
    assert!(
        out.stdout.starts_with("Unchanged: a.csv [5 B] as "),
        "{out:?}"
    );
    assert!(out.stderr.contains("message was not recorded"), "{out:?}");
    assert!(!out.stderr.contains("commit the new"), "{out:?}");
    // The hint is for people only
    let out = c.dvs_ok(&["--json", "add", "a.csv"]);
    assert!(out.stderr.is_empty(), "{out:?}");

    // dvs.toml and the metadata folder are Git's
    let out = c.dvs(Some(TOKEN), &["add", "dvs.toml"]);
    assert_ne!(out.code, 0, "{out:?}");
    assert!(out.stderr.contains("commit it to Git"), "{out:?}");

    let out = c.dvs_ok(&["get", "a.csv"]);
    assert_eq!(
        out.stdout.trim(),
        "Nothing to get: 1 file already matches its .dvs file"
    );
    std::fs::remove_file(c.repo().join("a.csv")).unwrap();
    let out = c.dvs_ok(&["get", "--dry-run", "a.csv"]);
    assert!(out.stdout.starts_with("Would get: a.csv [5 B]"), "{out:?}");
    assert!(!c.repo().join("a.csv").exists());

    c.write("notes.txt", b"n");
    let out = c.dvs_ok(&["status", "notes.txt"]);
    assert!(out.stdout.contains("notes.txt is not tracked"), "{out:?}");
    let out = c.dvs_ok(&["status", "missing.txt"]);
    assert!(out.stdout.contains("missing.txt does not exist"), "{out:?}");

    // A filter that hides a tracked file says so, rather than "not tracked"
    c.dvs_ok(&["get", "a.csv"]);
    let out = c.dvs_ok(&["status", "a.csv", "--unsynced"]);
    assert_eq!(out.stdout.trim(), "a.csv is tracked, but not unsynced");
    let out = c.dvs_ok(&["status", ".", "--absent", "--unsynced"]);
    assert!(
        out.stdout
            .contains("No tracked files in . are absent or unsynced"),
        "{out:?}"
    );

    // --json errors are JSON, usage errors too
    let out = c.dvs(None, &["--json", "add", "notes.txt"]);
    let error: serde_json::Value = serde_json::from_str(out.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "not_logged_in", "{out:?}");
    let out = c.dvs(None, &["--json", "status", "--no-such-flag"]);
    assert_eq!(out.code, 2, "{out:?}");
    let error: serde_json::Value = serde_json::from_str(out.stderr.trim()).unwrap();
    assert_eq!(error["error"]["code"], "usage", "{out:?}");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--no-such-flag"),
        "{out:?}"
    );
    let out = c.dvs(None, &["--json", "--help"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert!(out.stdout.contains("Exit codes:"), "{out:?}");
    // Without --json, clap's own text
    let out = c.dvs(None, &["status", "--no-such-flag"]);
    assert_eq!(out.code, 2, "{out:?}");
    assert!(out.stderr.starts_with("error:"), "{out:?}");
}

#[test]
fn a_yank_by_an_unreadable_dvs_file_stops() {
    let fake = Fake::start();
    let c = attached(&fake);
    c.write("a.csv", b"alpha");
    c.dvs_ok(&["add", "a.csv"]);
    c.write(
        ".dvs/a.csv.dvs",
        b"<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> other\n",
    );
    let out = c.dvs(Some(TOKEN), &["yank", "a.csv", "--reason", "x"]);
    assert_eq!(out.code, 1, "{out:?}");
    assert!(out.stderr.contains("could not read"), "{out:?}");
    assert_eq!(fake.count("POST", "/yanks"), 0);
}
