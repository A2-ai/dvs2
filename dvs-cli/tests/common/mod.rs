//! Shared helpers for the CLI integration tests: running `dvs` with a
//! private credentials directory, reading event streams, and a small
//! in-process stand-in for dvs-server.
//!
//! Every run points `XDG_CONFIG_HOME`/`APPDATA` at a directory the test owns,
//! so no test ever reads or writes the real user's credentials.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::{Value, json};

/// Generous: only reached when something is broken.
pub const WAIT: Duration = Duration::from_secs(60);

/// A test's own project directory and credentials directory.
pub struct Sandbox {
    pub temp: tempfile::TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let sandbox = Self {
            temp: tempfile::tempdir().unwrap(),
        };
        std::fs::create_dir(sandbox.project()).unwrap();
        sandbox
    }

    pub fn project(&self) -> PathBuf {
        self.temp.path().join("project")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.temp.path().join("config")
    }

    pub fn credentials_path(&self) -> PathBuf {
        self.config_dir().join("dvs").join("credentials.json")
    }

    /// `dvs` in the project, with this sandbox's credentials only.
    pub fn dvs(&self) -> Command {
        self.dvs_in(&self.project())
    }

    pub fn dvs_in(&self, cwd: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dvs"));
        command
            .current_dir(cwd)
            .env("XDG_CONFIG_HOME", self.config_dir())
            .env("APPDATA", self.config_dir())
            .env_remove("DVS_GET_ON_CONFLICT")
            .env_remove("DVS_NUM_THREADS");
        command
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.dvs().args(args).output().unwrap()
    }

    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
        let path = self.project().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    pub fn read(&self, relative: &str) -> Vec<u8> {
        std::fs::read(self.project().join(relative)).unwrap()
    }

    /// A local-storage project.
    pub fn init_local(&self) {
        let storage = self.temp.path().join("storage");
        let output = self
            .dvs()
            .args(["init", "local"])
            .arg(&storage)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
    }

    /// A project on `url`, as `dvs attach` leaves it (no network needed).
    pub fn init_server(&self, url: &str) {
        std::fs::create_dir_all(self.project().join(".dvs")).unwrap();
        std::fs::write(
            self.project().join("dvs.toml"),
            format!("[backend]\ntype = 'server'\nname = '{PROJECT}'\nurl = '{url}'\n"),
        )
        .unwrap();
    }

    /// Store `token` for `url`, as `dvs login` would.
    pub fn login(&self, url: &str, token: &str) {
        let mut store = serde_json::Map::new();
        store.insert(url.to_string(), json!(token));
        self.write_credentials(&Value::Object(store).to_string());
    }

    pub fn write_credentials(&self, contents: &str) {
        std::fs::create_dir_all(self.credentials_path().parent().unwrap()).unwrap();
        std::fs::write(self.credentials_path(), contents).unwrap();
    }

    pub fn credentials(&self) -> Option<Vec<u8>> {
        std::fs::read(self.credentials_path()).ok()
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The single `--json` document on stdout.
pub fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {} / {}", stdout(output), stderr(output)))
}

/// Every line of an event stream, checked for the stream's invariants.
pub fn parse_stream(bytes: &[u8], expected_exit: i32) -> Vec<Value> {
    let events: Vec<Value> = String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| serde_json::from_str(line).expect("every stdout line must be JSON"))
        .collect();
    assert_eq!(events.first().unwrap()["type"], "operation_started");
    assert_eq!(events.last().unwrap()["type"], "operation_finished");
    assert_eq!(events.last().unwrap()["exit_code"], expected_exit);
    for (seq, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], seq);
        assert_eq!(event["v"], 1);
        assert_eq!(event["operation_id"], events[0]["operation_id"]);
    }
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "operation_finished")
            .count(),
        1
    );
    events
}

pub fn of_type<'a>(events: &'a [Value], kind: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["type"] == kind)
        .collect()
}

pub fn file_results(events: &[Value]) -> Vec<Value> {
    of_type(events, "file_result")
        .into_iter()
        .map(|event| event["result"].clone())
        .collect()
}

/// A result with its run-specific timing removed, for comparing two runs.
pub fn without_timing(mut value: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for key in ["started_at", "finished_at", "duration_ms"] {
                    map.remove(key);
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

/// Rows in path order, as final results hold them.
pub fn by_path(mut rows: Vec<Value>) -> Vec<Value> {
    rows.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    rows
}

/// The project name every stub serves.
pub const PROJECT: &str = "proj";

/// One request as the stub saw it.
#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Full path, prefix included, without the query
    pub path: String,
    pub query: String,
    pub body: Vec<u8>,
}

type Matcher = Box<dyn Fn(&Request) -> bool + Send>;

struct Rule {
    matches: Matcher,
    status: u16,
    body: String,
}

struct Hold {
    matches: Matcher,
    arrived: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

/// A request the stub is holding until the test lets it go.
pub struct Held {
    arrived: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

impl Held {
    /// Wait until the held request has reached the stub.
    pub fn wait(&self) {
        self.arrived
            .recv_timeout(WAIT)
            .expect("the held request never arrived");
    }

    /// Let the held request finish.
    pub fn release(&self) {
        let _ = self.release.send(());
    }
}

#[derive(Default)]
struct State {
    blobs: HashMap<String, Vec<u8>>,
    requests: Vec<Request>,
    rules: Vec<Rule>,
    holds: Vec<Hold>,
}

/// A stand-in for dvs-server on 127.0.0.1, serving one project ([`PROJECT`])
/// under an optional path prefix. Blobs are stored uncompressed (the project
/// compression is `none`), so a `get` downloads what an `add` uploaded.
pub struct Stub {
    pub url: String,
    pub prefix: String,
    state: Arc<Mutex<State>>,
}

impl Stub {
    pub fn start() -> Self {
        Self::with_prefix("")
    }

    /// Served under `prefix` (e.g. `/dvs`); `url` has a trailing slash.
    pub fn with_prefix(prefix: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}{prefix}/", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let prefix = prefix.to_string();
        let served = (state.clone(), prefix.clone());
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { continue };
                let (state, prefix) = served.clone();
                std::thread::spawn(move || {
                    let _ = handle(socket, &state, &prefix);
                });
            }
        });
        Self { url, prefix, state }
    }

    /// Answer every request `matches` selects with `status` and `body`.
    pub fn fail(
        &self,
        matches: impl Fn(&Request) -> bool + Send + 'static,
        status: u16,
        body: &str,
    ) {
        self.state.lock().unwrap().rules.push(Rule {
            matches: Box::new(matches),
            status,
            body: body.to_string(),
        });
    }

    /// Hold the first request `matches` selects (after reading its body)
    /// until [`Held::release`].
    pub fn hold(&self, matches: impl Fn(&Request) -> bool + Send + 'static) -> Held {
        let (arrived_tx, arrived_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        self.state.lock().unwrap().holds.push(Hold {
            matches: Box::new(matches),
            arrived: arrived_tx,
            release: release_rx,
        });
        Held {
            arrived: arrived_rx,
            release: release_tx,
        }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn has_blob(&self, hash: &str) -> bool {
        self.state.lock().unwrap().blobs.contains_key(hash)
    }

    pub fn blob_count(&self) -> usize {
        self.state.lock().unwrap().blobs.len()
    }
}

/// Requests for a project route (`<prefix>/api/proj/<endpoint>...`).
pub fn route(method: &'static str, endpoint: &str) -> impl Fn(&Request) -> bool + Send + 'static {
    let needle = format!("/api/{PROJECT}/{endpoint}");
    move |request: &Request| request.method == method && request.path.contains(&needle)
}

fn handle(socket: TcpStream, state: &Mutex<State>, prefix: &str) -> std::io::Result<()> {
    socket.set_read_timeout(Some(WAIT))?;
    let mut reader = BufReader::new(socket.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.clone(), String::new()),
    };
    let mut content_length = 0;
    let mut chunked = false;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" || header.is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }
    let body = if chunked {
        read_chunked(&mut reader)?
    } else {
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body)?;
        body
    };
    let request = Request {
        method,
        path,
        query,
        body,
    };

    let hold = {
        let mut state = state.lock().unwrap();
        state.requests.push(request.clone());
        let index = state.holds.iter().position(|h| (h.matches)(&request));
        index.map(|i| state.holds.remove(i))
    };
    if let Some(hold) = hold {
        let _ = hold.arrived.send(());
        let _ = hold.release.recv_timeout(WAIT);
    }

    let (status, body) = respond(&request, state, prefix);
    let mut socket = socket;
    let reason = match status {
        200 => "OK",
        201 => "Created",
        _ => "Status",
    };
    let length = if request.method == "HEAD" {
        0
    } else {
        body.len()
    };
    write!(
        socket,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    )?;
    if request.method != "HEAD" {
        socket.write_all(&body)?;
    }
    socket.flush()
}

fn read_chunked(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let mut size = String::new();
        reader.read_line(&mut size)?;
        let size =
            usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
        if size == 0 {
            // Trailers end with an empty line
            loop {
                let mut trailer = String::new();
                reader.read_line(&mut trailer)?;
                if trailer == "\r\n" || trailer.is_empty() {
                    return Ok(body);
                }
            }
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..])?;
        let mut crlf = [0; 2];
        reader.read_exact(&mut crlf)?;
    }
}

fn respond(request: &Request, state: &Mutex<State>, prefix: &str) -> (u16, Vec<u8>) {
    let mut state = state.lock().unwrap();
    if let Some(rule) = state.rules.iter().find(|rule| (rule.matches)(request)) {
        return (rule.status, rule.body.clone().into_bytes());
    }
    let Some(route) = request.path.strip_prefix(prefix) else {
        return (404, Vec::new());
    };
    let project = format!("/api/{PROJECT}/");
    let json = |value: Value| value.to_string().into_bytes();
    match (request.method.as_str(), route) {
        ("POST", "/api/init") => (201, Vec::new()),
        ("GET", r) if r == format!("/api/project/{PROJECT}") => (200, json(json!({}))),
        ("POST", "/api/oauth/device/authorize") => (
            200,
            json(json!({"device_code": "DEVICE", "user_code": "USER-CODE"})),
        ),
        ("POST", "/api/oauth/token") => (200, json(json!({"access_token": "stub-token"}))),
        ("DELETE", "/api/oauth/token") => (200, Vec::new()),
        (method, r) if r.starts_with(&project) => match (method, &r[project.len()..]) {
            ("GET", "access") => (
                200,
                json(json!({
                    "access": "write",
                    "user": {"username": "tester", "email": null},
                    "groups": ["team"],
                })),
            ),
            ("POST", "manifest") => {
                let manifest: Value = serde_json::from_slice(&request.body).unwrap_or_default();
                let missing: Vec<_> = manifest["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry["hash"].as_str())
                    .filter(|hash| !state.blobs.contains_key(*hash))
                    .map(str::to_string)
                    .collect();
                (
                    200,
                    json(json!({"missing": missing, "compression": "none"})),
                )
            }
            ("POST", "history") => (200, json(json!([]))),
            (method, blob) if blob.starts_with("blobs/") => {
                let hash = blob["blobs/".len()..].to_string();
                match method {
                    "PUT" => {
                        let size = request.body.len();
                        let created = state.blobs.insert(hash, request.body.clone()).is_none();
                        (
                            if created { 201 } else { 200 },
                            json(json!({"stored_size": size, "compression": "none"})),
                        )
                    }
                    "HEAD" if state.blobs.contains_key(&hash) => (200, Vec::new()),
                    "GET" => match state.blobs.get(&hash) {
                        Some(bytes) => (200, bytes.clone()),
                        None => (404, Vec::new()),
                    },
                    _ => (404, Vec::new()),
                }
            }
            _ => (404, Vec::new()),
        },
        _ => (404, Vec::new()),
    }
}

/// Paths under `dir` (relative), recursively, skipping `.git`.
pub fn tree(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if relative == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                out.push(relative);
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}
