//! Shared helpers for the CLI integration tests. Every run points
//! `XDG_CONFIG_HOME`/`APPDATA` at a directory the test owns, so no test ever
//! reads or writes the real user's credentials.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{Value, json};

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

    pub fn dvs(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dvs"));
        command
            .current_dir(self.project())
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

pub fn document(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| panic!("{e}: {} / {}", stdout(output), stderr(output)))
}

pub const PROJECT: &str = "proj";

/// A stand-in dvs-server on 127.0.0.1 that answers every request with one status.
pub struct Stub {
    pub url: String,
}

impl Stub {
    pub fn start(status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else { continue };
                // Requests without a body (or rejected before it) are enough here
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                    line.clear();
                }
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status} Status\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            }
        });
        Self { url }
    }
}
