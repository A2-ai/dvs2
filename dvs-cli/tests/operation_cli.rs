mod common;

use common::{Sandbox, document};
use serde_json::{Value, json};

#[test]
fn every_json_result_carries_its_timing() {
    let sandbox = Sandbox::new();
    sandbox.init_local();
    sandbox.write("data.txt", "data");
    for args in [vec!["add", "data.txt"], vec!["add", "missing.txt"]] {
        let result = document(&sandbox.dvs().arg("--json").args(&args).output().unwrap());
        for key in ["started_at", "finished_at"] {
            // 2026-10-21T14:30:00.123Z
            let time = result[key].as_str().unwrap();
            assert!(time.len() == 24 && time.ends_with('Z'), "{result}");
        }
        assert!(result["finished_at"].as_str() >= result["started_at"].as_str());
        assert!(result["duration_ms"].is_u64(), "{result}");
    }
}

fn bytes(total: u64, to_transfer: u64, transferred: u64) -> Value {
    json!({
        "total": total,
        "to_transfer": to_transfer,
        "already_stored": total - to_transfer,
        "transferred": transferred,
    })
}

#[test]
fn add_reports_byte_totals() {
    let sandbox = Sandbox::new();
    sandbox.init_local();
    sandbox.write("a.bin", vec![1u8; 1000]);
    sandbox.write("b.bin", vec![2u8; 300]);
    let preview = document(&sandbox.run(&["--json", "add", "--dry-run", "a.bin", "b.bin"]));
    assert_eq!(preview["bytes"], bytes(1300, 1300, 0));
    let add = document(&sandbox.run(&["--json", "add", "a.bin", "b.bin"]));
    assert_eq!(add["bytes"], bytes(1300, 1300, 1300));
    let again = document(&sandbox.run(&["--json", "add", "--dry-run", "a.bin", "b.bin"]));
    assert_eq!(again["bytes"], bytes(1300, 0, 0));
}

#[test]
fn get_reports_byte_totals() {
    let sandbox = Sandbox::new();
    sandbox.init_local();
    sandbox.write("a.bin", vec![1u8; 1000]);
    sandbox.write("b.bin", vec![2u8; 300]);
    assert!(sandbox.run(&["add", "a.bin", "b.bin"]).status.success());
    std::fs::remove_file(sandbox.project().join("a.bin")).unwrap();
    let preview = document(&sandbox.run(&["--json", "get", "--dry-run", "a.bin", "b.bin"]));
    assert_eq!(preview["bytes"], bytes(1300, 1000, 0));
    let get = document(&sandbox.run(&["--json", "get", "a.bin", "b.bin"]));
    assert_eq!(get["bytes"], bytes(1300, 1000, 1000));
}
