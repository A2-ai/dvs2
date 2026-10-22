mod common;

use common::parse_stream;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

fn cli(cwd: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dvs"));
    command.current_dir(cwd).env_remove("DVS_GET_ON_CONFLICT");
    command
}

#[test]
fn startup_and_usage_errors_are_complete_event_streams() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec!["--emit-events", "status"],
        vec!["--emit-events", "status", "--unknown-flag"],
        vec!["--emit-events", "--json", "status"],
    ] {
        let output = cli(temp.path()).args(args).output().unwrap();
        assert!(!output.status.success());
        let events = parse_stream(&output.stdout, output.status.code().unwrap());
        assert!(events.last().unwrap()["result"]["error"].is_object());
    }
}

#[test]
fn terminal_result_matches_json_for_status_and_dry_run() {
    let temp = tempfile::tempdir().unwrap();
    let storage = temp.path().join("storage");
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let init = cli(&project)
        .args(["init", "local"])
        .arg(&storage)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    std::fs::write(project.join("file with space.txt"), b"sample bytes").unwrap();
    assert!(
        cli(&project)
            .args(["add", "file with space.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::remove_file(project.join("file with space.txt")).unwrap();
    for args in [
        vec!["status"],
        vec!["get", "--dry-run", "file with space.txt"],
    ] {
        let json_output = cli(&project).arg("--json").args(&args).output().unwrap();
        // Two runs agree on everything but their timing (C2)
        let expected = common::without_timing(serde_json::from_slice(&json_output.stdout).unwrap());
        let streamed = cli(&project)
            .arg("--emit-events")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(streamed.status.code(), json_output.status.code());
        let events = parse_stream(&streamed.stdout, streamed.status.code().unwrap());
        assert_eq!(
            common::without_timing(events.last().unwrap()["result"].clone()),
            expected
        );
        let rows: Vec<_> = events
            .iter()
            .filter(|event| event["type"] == "file_result")
            .map(|event| common::without_timing(event["result"].clone()))
            .collect();
        assert_eq!(rows, expected["files"].as_array().unwrap().clone());
        assert!(
            !events
                .iter()
                .any(|event| event["type"].as_str().unwrap().starts_with("transfer_"))
        );
    }
    let restored = cli(&project)
        .args(["--emit-events", "get", "file with space.txt"])
        .output()
        .unwrap();
    assert!(restored.status.success());
    let events = parse_stream(&restored.stdout, 0);
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "transfer_started"
                && event["path"] == "file with space.txt")
    );
    assert!(events.iter().any(|event| event["type"] == "file_result"));
    assert_eq!(
        std::fs::read(project.join("file with space.txt")).unwrap(),
        b"sample bytes"
    );
}

#[test]
fn status_events_report_unsynced_files() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(
        cli(&project)
            .args(["init", "local"])
            .arg(temp.path().join("storage"))
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("tracked.txt"), "first version").unwrap();
    assert!(
        cli(&project)
            .args(["add", "tracked.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("tracked.txt"), "second version").unwrap();
    assert!(
        cli(&project)
            .args(["add", "tracked.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("tracked.txt"), "first version").unwrap();

    let output = cli(&project)
        .args(["--emit-events", "status", "--unsynced"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let events = parse_stream(&output.stdout, 0);
    let rows: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "file_result")
        .map(|event| event["result"].clone())
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "unsynced");
    assert_eq!(events.last().unwrap()["result"]["files"], json!(rows));
}

#[cfg(unix)]
#[test]
fn status_streams_unsynced_rows_without_history_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let storage = temp.path().join("storage");
    std::fs::create_dir(&project).unwrap();
    assert!(
        cli(&project)
            .args(["init", "local"])
            .arg(&storage)
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("a-current.txt"), "current").unwrap();
    assert!(
        cli(&project)
            .args(["add", "a-current.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("z-stale.txt"), "first version").unwrap();
    assert!(
        cli(&project)
            .args(["add", "z-stale.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("z-stale.txt"), "second version").unwrap();
    assert!(
        cli(&project)
            .args(["add", "z-stale.txt"])
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("z-stale.txt"), "first version").unwrap();

    std::fs::remove_file(storage.join("audit.log.jsonl")).unwrap();
    let output = cli(&project)
        .args(["--emit-events", "status"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let events = parse_stream(&output.stdout, 0);
    let rows: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "file_result")
        .map(|event| event["result"].clone())
        .collect();
    assert!(
        rows.iter()
            .any(|row| { row["path"] == "z-stale.txt" && row["status"] == "unsynced" })
    );
    assert_eq!(
        events.last().unwrap()["result"]["files"]
            .as_array()
            .unwrap()
            .len(),
        rows.len()
    );
}

#[test]
fn partial_add_conflicts_and_read_commands_preserve_authoritative_results() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    assert!(
        cli(&project)
            .args(["init", "local"])
            .arg(temp.path().join("storage"))
            .output()
            .unwrap()
            .status
            .success()
    );
    std::fs::write(project.join("good.txt"), "recorded").unwrap();
    std::fs::write(project.join("bad.txt"), "recorded").unwrap();
    std::fs::create_dir(project.join(".dvs/bad.txt.dvs")).unwrap();
    let partial = cli(&project)
        .args(["--emit-events", "add", "good.txt", "bad.txt"])
        .output()
        .unwrap();
    assert_eq!(partial.status.code(), Some(1));
    let events = parse_stream(&partial.stdout, 1);
    assert_eq!(events.last().unwrap()["result"]["outcome"], "partial");
    assert_eq!(events.last().unwrap()["result"]["counts"]["failed"], 1);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "file_result")
            .count(),
        2
    );
    std::fs::write(project.join("good.txt"), "local edits").unwrap();
    let conflict = cli(&project)
        .args(["--emit-events", "get", "good.txt"])
        .output()
        .unwrap();
    assert_eq!(conflict.status.code(), Some(3));
    let events = parse_stream(&conflict.stdout, 3);
    let rows: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "file_result")
        .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["result"]["code"], "local_changes");
    assert_eq!(
        rows[0]["result"],
        events.last().unwrap()["result"]["files"][0]
    );
    assert_eq!(
        std::fs::read_to_string(project.join("good.txt")).unwrap(),
        "local edits"
    );
    let missing_hash = "0".repeat(64);
    for args in [
        vec!["availability", missing_hash.as_str()],
        vec!["history", "good.txt"],
    ] {
        let output = cli(&project)
            .arg("--emit-events")
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let events = parse_stream(&output.stdout, 0);
        assert_eq!(events.last().unwrap()["result"]["command"], args[0]);
        assert!(events.iter().any(|event| event["type"] == "file_result"));
        // Streamed rows carry every row's error fields, history's too (§9 rec 1)
        for event in events.iter().filter(|event| event["type"] == "file_result") {
            for field in ["code", "message", "hint"] {
                assert_eq!(event["result"].get(field), Some(&Value::Null), "{event}");
            }
        }
    }
    std::fs::write(project.join("preview.txt"), "preview").unwrap();
    let preview = cli(&project)
        .args(["--emit-events", "add", "--dry-run", "preview.txt"])
        .output()
        .unwrap();
    assert!(preview.status.success());
    let events = parse_stream(&preview.stdout, 0);
    assert!(
        !events
            .iter()
            .any(|event| event["type"].as_str().unwrap().starts_with("transfer_"))
    );
    assert!(!project.join(".dvs/preview.txt.dvs").exists());
}

#[test]
fn login_instructions_are_flushed_before_polling_finishes_and_secrets_are_not_emitted() {
    let temp = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (release, released) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut content_length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            let response = if index == 0 {
                assert!(first.contains("/api/oauth/device/authorize"));
                json!({"device_code": "SECRET_DEVICE_CODE", "user_code": "PUBLIC_CODE"})
            } else {
                assert!(first.contains("/api/oauth/token"));
                released.recv_timeout(Duration::from_secs(10)).unwrap();
                json!({"access_token": "SECRET_ACCESS_TOKEN"})
            }
            .to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let mut child = cli(temp.path())
        .args(["--emit-events", "login", &url])
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("APPDATA", temp.path().join("config"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut captured = Vec::new();
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            send.send(line.clone()).unwrap();
            captured.extend_from_slice(line.as_bytes());
            captured.push(b'\n');
        }
        captured
    });
    loop {
        let line = receive
            .recv_timeout(Duration::from_secs(10))
            .expect("authorization event must arrive while server is waiting");
        let event: Value = serde_json::from_str(&line).unwrap();
        if event["type"] == "waiting_for_auth" {
            assert_eq!(event["user_code"], "PUBLIC_CODE");
            assert!(child.try_wait().unwrap().is_none());
            break;
        }
    }
    release.send(()).unwrap();
    let output = child.wait_with_output().unwrap();
    server.join().unwrap();
    let captured = reader.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_stream(&captured, 0);
    assert!(!String::from_utf8_lossy(&captured).contains("SECRET_"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("SECRET_"));
    assert!(temp.path().join("config/dvs/credentials.json").is_file());
}
