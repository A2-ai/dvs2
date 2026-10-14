//! A fake dvs server speaking just enough of API v1 for the CLI tests: one project,
//! blobs in memory, every request recorded, and switches for the failures the client
//! must survive.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

pub const PROJECT: &str = "demo";
pub const TOKEN: &str = "good-token";
const NOW: &str = "2026-10-12T00:00:00Z";

/// One request as the server saw it.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    pub fn is(&self, method: &str, suffix: &str) -> bool {
        self.method == method && self.path.ends_with(suffix)
    }
}

/// Knobs a test turns before running the CLI.
#[derive(Default)]
pub struct Faults {
    /// Entries per manifest call
    pub max_manifest_entries: AtomicUsize,
    /// Answer this manifest call (1-based) with an error
    pub fail_manifest_call: AtomicUsize,
    /// Close this many `PUT`s without answering
    pub drop_puts: AtomicUsize,
    pub fail_complete: AtomicBool,
    /// Serve downloads zstd-compressed, saying so in the header
    pub serve_zstd: AtomicBool,
    /// The project does not exist
    pub no_project: AtomicBool,
    /// Creating the project finds one you can read
    pub project_exists: AtomicBool,
    /// The largest blob taken, in bytes; 0 for no limit
    pub max_blob_size: AtomicUsize,
    /// Refuse the manifest entry with this path as `invalid_entry`, naming its index
    pub reject_entry_path: Mutex<Option<String>>,
    /// After this manifest call (1-based), another client publishes `publish_bytes`
    pub publish_after_manifest_call: AtomicUsize,
    pub publish_bytes: Mutex<Vec<u8>>,
    /// Close this many manifest calls without answering
    pub drop_manifests: AtomicUsize,
    /// On the first `PUT`, lock the project and answer 423 without reading the body
    pub lock_on_put: AtomicBool,
    /// Store this many `PUT`s, then cut their response off after the headers
    pub truncate_put_responses: AtomicUsize,
    /// Answer `HEAD`s with this status instead
    pub head_status: AtomicUsize,
    /// How the device login's token polls are answered, in order: `pending`,
    /// `slow_down`, `denied`; once empty, approved
    pub login_answers: Mutex<Vec<&'static str>>,
}

#[derive(Default)]
struct State {
    requests: Vec<Request>,
    blobs: HashMap<String, Vec<u8>>,
    batches: usize,
    manifest_calls: usize,
    locked: bool,
}

pub struct Fake {
    pub url: String,
    pub faults: Arc<Faults>,
    state: Arc<Mutex<State>>,
}

impl Fake {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let faults = Arc::new(Faults {
            max_manifest_entries: AtomicUsize::new(1000),
            max_blob_size: AtomicUsize::new(1 << 20),
            ..Default::default()
        });
        let state = Arc::new(Mutex::new(State::default()));
        let (f, s) = (faults.clone(), state.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (f, s) = (f.clone(), s.clone());
                std::thread::spawn(move || serve(stream, &f, &s));
            }
        });
        Self { url, faults, state }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn count(&self, method: &str, suffix: &str) -> usize {
        self.requests()
            .iter()
            .filter(|r| r.is(method, suffix))
            .count()
    }
}

/// Read a request; with `skip_body`, only its line and headers.
fn read_request(stream: &mut TcpStream, skip_body: bool) -> Option<Request> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        reader.read_line(&mut h).ok()?;
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        let (k, v) = h.split_once(':')?;
        headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
    let mut body = Vec::new();
    let skip_body = skip_body && method == "PUT";
    if skip_body {
    } else if let Some(len) = headers.get("content-length") {
        body.resize(len.parse().ok()?, 0);
        reader.read_exact(&mut body).ok()?;
    } else if headers
        .get("transfer-encoding")
        .is_some_and(|t| t.contains("chunked"))
    {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size).ok()?;
            let size = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            let mut chunk = vec![0; size + 2];
            reader.read_exact(&mut chunk).ok()?;
            if size == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..size]);
        }
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q),
        None => (target.clone(), ""),
    };
    let query = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();
    Some(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

enum Reply {
    Json(u16, Value),
    /// The status and headers of this JSON, then only part of its body
    Truncated(u16, Value),
    Bytes(Vec<(&'static str, String)>, Vec<u8>),
    Status(u16),
    /// Close without answering
    Drop,
}

fn error(status: u16, code: &str, message: &str) -> Reply {
    Reply::Json(
        status,
        json!({"error": {"code": code, "message": message, "request_id": "req-1"}}),
    )
}

fn project(locked: bool) -> Value {
    json!({
        "name": PROJECT, "compression": "none", "locked": locked, "version": 1,
        "created_at": NOW, "created_by": "alice", "your_role": "write",
        "grants": [], "repo_commit_url": null,
    })
}

fn serve(mut stream: TcpStream, faults: &Faults, state: &Mutex<State>) {
    let lock_on_put = faults.lock_on_put.load(Ordering::SeqCst);
    let Some(req) = read_request(&mut stream, lock_on_put) else {
        return;
    };
    state.lock().unwrap().requests.push(req.clone());
    let reply = if lock_on_put && req.method == "PUT" {
        faults.lock_on_put.store(false, Ordering::SeqCst);
        state.lock().unwrap().locked = true;
        error(423, "project_locked", "the project is locked")
    } else {
        handle(&req, faults, state)
    };
    let mut cut = false;
    let (status, headers, body): (u16, Vec<(&str, String)>, Vec<u8>) = match reply {
        Reply::Drop => return,
        Reply::Status(s) => (s, vec![], vec![]),
        Reply::Json(s, v) => (
            s,
            vec![("content-type", "application/json".into())],
            serde_json::to_vec(&v).unwrap(),
        ),
        Reply::Truncated(s, v) => {
            cut = true;
            (
                s,
                vec![("content-type", "application/json".into())],
                serde_json::to_vec(&v).unwrap(),
            )
        }
        Reply::Bytes(h, b) => (200, h, b),
    };
    let mut out = format!("HTTP/1.1 {status} X\r\nconnection: close\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    let _ = stream.write_all(out.as_bytes());
    if req.method != "HEAD" {
        let _ = stream.write_all(if cut { &body[..body.len() / 2] } else { &body });
    }
}

fn handle(req: &Request, faults: &Faults, state: &Mutex<State>) -> Reply {
    let path = req.path.as_str();
    if path == "/api/server" {
        return Reply::Json(
            200,
            json!({
                "server_version": "0.5.0", "api_versions": [1], "min_client_version": "0.5.0",
                "features": [],
                "limits": {
                    "max_blob_size": faults.max_blob_size.load(Ordering::SeqCst), "max_page_size": 100, "max_body_bytes": 10_485_760,
                    "max_manifest_entries": faults.max_manifest_entries.load(Ordering::SeqCst),
                    "max_path_bytes": 4096,
                },
                "auth": {"device_flow": true},
            }),
        );
    }
    if req.method == "POST" && path == "/api/v1/auth/device" {
        return Reply::Json(
            200,
            json!({
                "device_code": "device-code", "user_code": "ABCD-EFGH",
                "verification_uri": "/device", "verification_uri_complete": "/device?code=ABCD-EFGH",
                "expires_in": 600, "interval": 1,
            }),
        );
    }
    if req.method == "POST" && path == "/api/v1/auth/token" {
        let mut answers = faults.login_answers.lock().unwrap();
        let answer = (!answers.is_empty()).then(|| answers.remove(0));
        return match answer {
            None => Reply::Json(
                200,
                json!({"access_token": TOKEN, "token_type": "Bearer",
                       "expires_at": "2026-11-12T00:00:00Z"}),
            ),
            Some("pending") => Reply::Json(400, json!({"error": "authorization_pending"})),
            Some("slow_down") => Reply::Json(400, json!({"error": "slow_down"})),
            Some(_) => Reply::Json(400, json!({"error": "access_denied"})),
        };
    }
    if req.headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {TOKEN}")) {
        return if req.method == "HEAD" {
            Reply::Status(401)
        } else {
            error(401, "unauthenticated", "the token was not recognised")
        };
    }
    let segments: Vec<&str> = path.trim_start_matches("/api/v1/").split('/').collect();
    match (req.method.as_str(), segments.as_slice()) {
        ("DELETE", ["auth", "token"]) => Reply::Status(204),
        ("GET", ["me"]) => Reply::Json(
            200,
            json!({
                "sub": "alice-sub", "username": "alice", "email": "alice@example.com",
                "groups": ["analysts"], "global_admin": false,
                "token": {"id": "00000000-0000-0000-0000-000000000001", "source": "login",
                          "created_at": NOW, "expires_at": "2026-11-12T00:00:00Z"},
            }),
        ),
        ("POST", ["projects"]) => {
            if faults.project_exists.load(Ordering::SeqCst) {
                Reply::Json(
                    409,
                    json!({"error": {"code": "project_exists", "message": "taken",
                                     "project": {"name": PROJECT, "your_role": "read"}}}),
                )
            } else {
                Reply::Json(201, project(false))
            }
        }
        (_, ["projects", name, ..])
            if *name != PROJECT || faults.no_project.load(Ordering::SeqCst) =>
        {
            error(
                404,
                "project_not_found",
                "no such project, or you cannot read it",
            )
        }
        ("GET", ["projects", _]) => Reply::Json(200, project(state.lock().unwrap().locked)),
        ("GET", ["projects", _, "versions"]) => {
            let path = req.query.get("path").cloned().unwrap_or_default();
            let version = |hash: &str| {
                json!({"hash": hash, "size": 5, "stored_size": 5, "first_uploaded_at": NOW,
                       "last_uploaded_at": NOW, "placements": 1, "downloads": 0,
                       "yanked": false, "other_paths": 0})
            };
            match req.query.get("cursor").map(String::as_str) {
                None => Reply::Json(
                    200,
                    json!({"items": [version(&"b".repeat(64))], "next_cursor": format!("after-{path}")}),
                ),
                Some(_) => Reply::Json(
                    200,
                    json!({"items": [version(&"a".repeat(64))], "next_cursor": null}),
                ),
            }
        }
        ("POST", ["projects", _, "batches"]) => {
            let mut s = state.lock().unwrap();
            s.batches += 1;
            let kind = req.json()["kind"].clone();
            Reply::Json(
                201,
                json!({"id": format!("00000000-0000-0000-0000-{:012}", s.batches), "kind": kind,
                       "created_at": NOW, "expires_at": NOW}),
            )
        }
        ("POST", ["projects", _, "batches", _, "manifest"]) => {
            let mut s = state.lock().unwrap();
            s.manifest_calls += 1;
            if faults.drop_manifests.load(Ordering::SeqCst) > 0 {
                faults.drop_manifests.fetch_sub(1, Ordering::SeqCst);
                return Reply::Drop;
            }
            if s.manifest_calls == faults.fail_manifest_call.load(Ordering::SeqCst) {
                return error(409, "batch_closed", "the batch is closed");
            }
            let entries = req.json()["entries"].as_array().unwrap().clone();
            assert!(entries.len() <= faults.max_manifest_entries.load(Ordering::SeqCst));
            let max_size = faults.max_blob_size.load(Ordering::SeqCst) as u64;
            let rejected = faults.reject_entry_path.lock().unwrap().clone();
            for (i, e) in entries.iter().enumerate() {
                let bad = if max_size > 0 && e["size"].as_u64().unwrap() > max_size {
                    Some("size")
                } else if rejected.as_deref() == e["path"].as_str() {
                    Some("path")
                } else {
                    None
                };
                if let Some(field) = bad {
                    return Reply::Json(
                        422,
                        json!({"error": {"code": "invalid_entry", "message": "refused",
                                         "index": i, "field": field}}),
                    );
                }
            }
            let mut missing = BTreeSet::new();
            let mut present = Vec::new();
            for e in entries {
                let hash = e["hash"].as_str().unwrap().to_string();
                match s.blobs.get(&hash) {
                    Some(b) => {
                        present.push(json!({"hash": hash, "size": b.len(), "stored_size": b.len()}))
                    }
                    None => {
                        missing.insert(hash);
                    }
                }
            }
            if s.manifest_calls == faults.publish_after_manifest_call.load(Ordering::SeqCst) {
                let bytes = faults.publish_bytes.lock().unwrap().clone();
                s.blobs
                    .insert(blake3::hash(&bytes).to_hex().to_string(), bytes);
            }
            Reply::Json(
                200,
                json!({"missing": missing, "present": present, "compression": "none"}),
            )
        }
        ("PUT", ["projects", _, "blobs", hash]) => {
            if faults.drop_puts.load(Ordering::SeqCst) > 0 {
                faults.drop_puts.fetch_sub(1, Ordering::SeqCst);
                return Reply::Drop;
            }
            assert!(req.query.contains_key("batch"));
            let max_size = faults.max_blob_size.load(Ordering::SeqCst);
            if max_size > 0 && req.body.len() > max_size {
                return error(413, "payload_too_large", "the blob is too large");
            }
            let actual = blake3::hash(&req.body).to_hex().to_string();
            if actual != *hash {
                return error(422, "hash_mismatch", "the bytes do not match the hash");
            }
            let size = req.body.len();
            let created = state
                .lock()
                .unwrap()
                .blobs
                .insert(hash.to_string(), req.body.clone())
                .is_none();
            let body = json!({"size": size, "stored_size": size, "compression": "none"});
            let status = if created { 201 } else { 200 };
            if faults.truncate_put_responses.load(Ordering::SeqCst) > 0 {
                faults.truncate_put_responses.fetch_sub(1, Ordering::SeqCst);
                return Reply::Truncated(status, body);
            }
            Reply::Json(status, body)
        }
        ("POST", ["projects", _, "batches", id, "complete"]) => {
            if faults.fail_complete.load(Ordering::SeqCst) {
                return error(500, "internal", "something broke");
            }
            Reply::Json(200, json!({"id": id, "completed_at": NOW, "abandoned": 0}))
        }
        ("HEAD", ["projects", _, "blobs", hash]) => {
            match faults.head_status.load(Ordering::SeqCst) {
                0 => {}
                status => return Reply::Status(status as u16),
            }
            if state.lock().unwrap().blobs.contains_key(*hash) {
                Reply::Status(200)
            } else {
                Reply::Status(404)
            }
        }
        ("GET", ["projects", _, "blobs", hash]) => {
            assert!(req.query.contains_key("batch") && req.query.contains_key("path"));
            let Some(bytes) = state.lock().unwrap().blobs.get(*hash).cloned() else {
                return error(404, "content_not_found", "no such content");
            };
            if faults.serve_zstd.load(Ordering::SeqCst) {
                let compressed = zstd::encode_all(bytes.as_slice(), 0).unwrap();
                Reply::Bytes(vec![("x-dvs-compression", "zstd".into())], compressed)
            } else {
                Reply::Bytes(vec![("x-dvs-compression", "none".into())], bytes)
            }
        }
        _ => error(404, "not_found", "no such route"),
    }
}

/// A Git checkout and an isolated configuration directory to run `dvs` in.
pub struct Checkout {
    pub tmp: tempfile::TempDir,
}

impl Checkout {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("repo")).unwrap();
        let git = std::process::Command::new("git")
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(tmp.path().join("repo"))
            .status()
            .unwrap();
        assert!(git.success());
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        Self { tmp }
    }

    pub fn repo(&self) -> std::path::PathBuf {
        self.tmp.path().join("repo")
    }

    pub fn write(&self, name: &str, content: &[u8]) {
        let path = self.repo().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub fn credentials(&self) -> std::path::PathBuf {
        self.tmp.path().join("config/dvs/credentials.json")
    }

    pub fn store_token(&self, url: &str, token: &str) {
        let path = self.credentials();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json!({ url: token }).to_string()).unwrap();
    }

    /// Run `dvs` with `DVS_TOKEN` set to `token`, or unset.
    pub fn dvs(&self, token: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_dvs"));
        cmd.current_dir(self.repo())
            .args(args)
            .env("XDG_CONFIG_HOME", self.tmp.path().join("config"))
            .env("HOME", self.tmp.path())
            .env_remove("DVS_TOKEN")
            .env_remove("RUST_LOG");
        if let Some(t) = token {
            cmd.env("DVS_TOKEN", t);
        }
        let out = cmd.output().unwrap();
        Output {
            code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    pub fn dvs_ok(&self, args: &[&str]) -> Output {
        let out = self.dvs(Some(TOKEN), args);
        assert_eq!(out.code, 0, "{out:?}");
        out
    }
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn json(&self) -> Value {
        serde_json::from_str(self.stdout.trim()).unwrap_or_else(|e| panic!("{e}: {self:?}"))
    }

    /// `path → outcome` from `--json` add output.
    pub fn outcomes(&self) -> Vec<(String, String)> {
        self.json()
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["path"].as_str().unwrap().to_string(),
                    r["outcome"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect()
    }
}
