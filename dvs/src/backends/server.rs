use anyhow::bail;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;
use tempfile::NamedTempFile;
use url::Url;

use crate::audit::AuditEntry;

/// The server could not be reached or is failing, as opposed to refusing access.
#[derive(Debug)]
pub struct Unreachable(pub String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

/// The server refused: not logged in, login expired, or no access.
#[derive(Debug)]
pub struct Denied(pub String);

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Denied {}

const NOT_ALLOWED: &str =
    "You are not allowed to access this project, make sure you are logged in with the right user.";
use crate::auth::{delete_token, get_token};
use crate::utils::http_agent;
use crate::{Compression, Hashes, ProjectPath, RetrieveRequest, StoreRequest, StoreResult};

/// Join `relative` (no leading slash, e.g. `api/init`) onto the server URL
/// as a path *under* it. Every server and auth URL goes through here so a
/// proxy prefix survives: `https://h/dvs` and `https://h/dvs/` both give
/// `https://h/dvs/api/init`. `Url::join` alone drops the prefix for a
/// leading-slash path, and drops the last segment when the base has no
/// trailing slash.
///
/// A URL that cannot be a base (`mailto:x@y`, `foo:bar`) has no path to put
/// anything under: that is an error for the caller to report, never a panic.
pub(crate) fn join_url(base: &Url, relative: &str) -> anyhow::Result<Url> {
    debug_assert!(!relative.starts_with('/'), "{relative} must be relative");
    if base.cannot_be_a_base() {
        bail!("{base} cannot be used as a dvs server URL");
    }
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(relative)
        .map_err(|e| anyhow::anyhow!("{base} cannot be used as a dvs server URL: {e}"))
}

/// Wraps a reader and reports bytes as they're read, driving the `on_bytes`
/// progress callback. Used to stream an upload body while updating a progress bar.
struct CountingReader<'a, R> {
    inner: R,
    on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
}

impl<'a, R> CountingReader<'a, R> {
    fn new(inner: R, on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>) -> Self {
        Self { inner, on_bytes }
    }
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            if let Some(cb) = self.on_bytes {
                cb(n as u64);
            }
        }
        Ok(n)
    }
}

#[derive(Serialize, Deserialize)]
pub struct InitPayload {
    pub name: String,
    pub group: String,
    pub compression: Compression,
}

#[derive(Serialize, Deserialize)]
pub struct UploadResponse {
    pub stored_size: u64,
    pub compression: Compression,
}

#[derive(Serialize, Deserialize)]
pub struct HistoryPayload {
    #[serde(default)]
    pub paths: Vec<ProjectPath>,
}

#[derive(Serialize, Deserialize)]
pub struct UploadManifestEntry {
    pub path: ProjectPath,
    pub size: u64,
    pub hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct UploadManifest {
    pub operation_id: uuid::Uuid,
    pub message: Option<String>,
    pub entries: Vec<UploadManifestEntry>,
    /// git current status at the time of `dvs add`. The file may or may not
    /// be in this branch/commit.
    /// This is more to know: what was the person working on when they pushed
    #[serde(default)]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub git_commit: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct UploadManifestResponse {
    /// List of hashes we don't know about -> they need to be uploaded
    pub missing: Vec<String>,
    pub compression: Compression,
}

#[derive(Serialize, Deserialize)]
pub struct AccessUser {
    pub username: String,
    pub email: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct AccessResponse {
    pub access: String,
    pub user: AccessUser,
    pub groups: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ServerBackend {
    pub name: String,
    pub url: Url,
    /// This is used for init but never after since people can edit groups in the
    /// UI so that field would be dead either way
    #[serde(skip)]
    pub group: Option<String>,
    #[serde(skip)]
    access_token: OnceLock<Option<String>>,
}

impl ServerBackend {
    pub fn new(name: String, group: Option<String>, url: Url) -> Self {
        Self {
            name,
            group,
            url,
            access_token: OnceLock::new(),
        }
    }

    fn init_url(&self) -> anyhow::Result<Url> {
        join_url(&self.url, "api/init")
    }

    fn project_url(&self) -> anyhow::Result<Url> {
        join_url(&self.url, &format!("api/project/{}", self.name))
    }

    fn blob_url(&self, hash: &str) -> anyhow::Result<Url> {
        self.read_url(&format!("blobs/{hash}"))
    }

    fn manifest_url(&self) -> anyhow::Result<Url> {
        self.read_url("manifest")
    }

    fn access_url(&self) -> anyhow::Result<Url> {
        self.read_url("access")
    }

    fn history_url(&self) -> anyhow::Result<Url> {
        self.read_url("history")
    }

    fn read_url(&self, endpoint: &str) -> anyhow::Result<Url> {
        join_url(&self.url, &format!("api/{}/{}", self.name, endpoint))
    }

    fn bearer(&self) -> anyhow::Result<String> {
        let token = self.access_token.get_or_init(|| {
            get_token(&self.url).unwrap_or_else(|e| {
                log::warn!("Could not read the credentials store: {e}");
                None
            })
        });
        match token {
            Some(token) => Ok(format!("Bearer {token}")),
            None => {
                bail!("Could not find access token. Log in first")
            }
        }
    }

    /// The server rejected the stored token: forget it so the next `init` asks to log in
    fn login_expired(&self) -> anyhow::Error {
        if let Err(e) = delete_token(&self.url) {
            log::warn!("Could not remove the expired token: {e}");
        }
        Denied("Your login is no longer valid, run `dvs login`".into()).into()
    }

    fn unreachable(&self, e: ureq::Error) -> anyhow::Error {
        Unreachable(format!("cannot reach the dvs server at {}: {e}", self.url)).into()
    }

    fn failing(&self, code: u16) -> anyhow::Error {
        Unreachable(format!(
            "the dvs server at {} is not working ({code})",
            self.url
        ))
        .into()
    }

    pub fn init(&self, compression: Compression) -> anyhow::Result<bool> {
        let Some(group) = self.group.clone() else {
            bail!("A group is required to create a project on the server");
        };
        let url = self.init_url()?;
        let payload = InitPayload {
            name: self.name.clone(),
            group,
            compression,
        };
        let mut resp = http_agent()
            .post(url.as_str())
            .header("authorization", self.bearer()?)
            .send_json(&payload)?;
        match resp.status().as_u16() {
            201 => Ok(false), // created
            200 => Ok(true),  // already existed (group+compression matched)
            401 => Err(self.login_expired()),
            409 => {
                bail!(
                    "Server initialization failed: the project already exists with a different group or compression"
                )
            }
            code => bail!(
                "server init failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }

    pub fn attach(&self) -> anyhow::Result<()> {
        let url = self.project_url()?;
        let mut resp = http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer()?)
            .call()?;

        match resp.status().as_u16() {
            200 => Ok(()),
            401 => Err(self.login_expired()),
            404 => {
                bail!("Project {} not found on server", self.name)
            }
            code => bail!(
                "couldn't get project information ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }

    /// What the current user can do on the project. `None` when they are not a member.
    pub fn access(&self) -> anyhow::Result<Option<AccessResponse>> {
        let mut resp = http_agent()
            .get(self.access_url()?.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| self.unreachable(e))?;

        match resp.status().as_u16() {
            200 => Ok(Some(resp.body_mut().read_json()?)),
            401 => Err(self.login_expired()),
            403 => Ok(None),
            404 => bail!("The project {} does not exist on the server", self.name),
            code @ 500.. => Err(self.failing(code)),
            code => bail!(
                "access check failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }

    pub fn check_access(&self) -> anyhow::Result<()> {
        if self.access()?.is_none() {
            return Err(Denied(NOT_ALLOWED.into()).into());
        }
        Ok(())
    }

    /// Declares what an add is about to store. The server records every entry and
    /// answers with the hashes it does not have, which are the only ones to `store`.
    pub fn manifest(&self, manifest: &UploadManifest) -> anyhow::Result<UploadManifestResponse> {
        let mut resp = http_agent()
            .post(self.manifest_url()?.as_str())
            .header("authorization", self.bearer()?)
            .send_json(manifest)?;

        match resp.status().as_u16() {
            200 => Ok(resp.body_mut().read_json()?),
            401 => Err(self.login_expired()),
            403 => bail!(
                "You are not allowed to access this project, make sure you are logged in with the right user."
            ),
            code => bail!(
                "manifest failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }

    pub fn store(&self, req: StoreRequest<'_>) -> anyhow::Result<StoreResult> {
        let mut file = CountingReader::new(fs::File::open(req.source)?, req.on_bytes);

        let mut url = self.blob_url(req.hashes.get_blake3())?;
        url.query_pairs_mut()
            .append_pair("operation_id", &req.operation_id.to_string());
        let mut resp = http_agent()
            .put(url.as_str())
            .header("authorization", self.bearer()?)
            .header("content-type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(&mut file))?;

        match resp.status().as_u16() {
            status @ (200 | 201) => {
                let uploaded: UploadResponse = resp.body_mut().read_json()?;
                Ok(StoreResult {
                    stored_size: uploaded.stored_size,
                    compression: uploaded.compression,
                    created: status == 201,
                })
            }
            401 => Err(self.login_expired()),
            403 => bail!(
                "You are not allowed to access this project, make sure you are logged in with the right user."
            ),
            code => bail!(
                "upload failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }

    pub fn retrieve(&self, req: RetrieveRequest<'_>) -> anyhow::Result<bool> {
        let mut url = self.blob_url(req.hashes.get_blake3())?;
        url.query_pairs_mut()
            .append_pair("path", req.path.as_str())
            .append_pair("operation_id", &req.operation_id.to_string());

        let mut resp = http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| self.unreachable(e))?;

        match resp.status().as_u16() {
            200 => {}
            401 => return Err(self.login_expired()),
            403 => return Err(Denied(NOT_ALLOWED.into()).into()),
            404 => return Ok(false),
            code @ 500.. => return Err(self.failing(code)),
            code => bail!(
                "download failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }

        let tmp = NamedTempFile::new()?.into_temp_path();
        {
            let mut reader = CountingReader::new(resp.body_mut().as_reader(), req.on_bytes);
            let mut out = fs::File::create(&tmp)?;
            std::io::copy(&mut reader, &mut out)?;
        }

        if let Some(parent) = req.target.parent() {
            fs::create_dir_all(parent)?;
        }
        req.compression.decompress(&tmp, req.target, None)?;
        Ok(true)
    }

    pub fn exists(&self, hashes: &Hashes) -> anyhow::Result<bool> {
        let url = self.blob_url(hashes.get_blake3())?;

        match http_agent()
            .head(url.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| self.unreachable(e))?
            .status()
            .as_u16()
        {
            200 => Ok(true),
            401 => Err(self.login_expired()),
            403 => Err(Denied(NOT_ALLOWED.into()).into()),
            404 => Ok(false),
            code @ 500.. => Err(self.failing(code)),
            _ => bail!("Error while trying to see if a file exists"),
        }
    }

    pub fn remove(&self, _: &Hashes) -> anyhow::Result<()> {
        // We don't rollback with the server
        Ok(())
    }

    pub fn get_audit_entries(&self, files: &[PathBuf]) -> anyhow::Result<Vec<AuditEntry>> {
        let url = self.history_url()?;

        let mut paths = Vec::with_capacity(files.len());
        for p in files {
            paths.push(ProjectPath::from_path(p).map_err(|e| anyhow::anyhow!(e))?);
        }

        let payload = HistoryPayload { paths };

        let mut resp = http_agent()
            .post(url.as_str())
            .header("authorization", self.bearer()?)
            .send_json(&payload)?;

        match resp.status().as_u16() {
            200 => Ok(resp.body_mut().read_json::<Vec<AuditEntry>>()?),
            401 => Err(self.login_expired()),
            403 => bail!(
                "You are not allowed to access this project, make sure you are logged in with the right user."
            ),
            code => bail!(
                "history failed ({code}): {}",
                resp.body_mut().read_to_string().unwrap_or_default()
            ),
        }
    }
}

/// HTTP stubs for unit tests that talk to a server (here and in `auth`).
#[cfg(test)]
pub(crate) mod test_http {
    use std::io::{Read, Write};
    use url::Url;

    /// One-shot HTTP stub at `http://addr<prefix>`: answers the first
    /// request with `status` and `body`, and hands back the request line
    /// (`METHOD /path ...`).
    pub(crate) fn stub(
        prefix: &str,
        status: u16,
        body: &'static str,
    ) -> (Url, std::thread::JoinHandle<String>) {
        respond(
            prefix,
            format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
    }

    /// One-shot stub that answers the first request with the raw bytes of
    /// `response`, for answers `stub` cannot express (a truncated body).
    pub(crate) fn respond(
        prefix: &str,
        response: String,
    ) -> (Url, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            // Read the whole request so the client never sees a reset
            // mid-send: up to the end of a Content-Length or chunked body.
            // A client that gives up mid-request (a failing upload source)
            // ends the request early; that is not the stub's failure.
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let head = text[..end].to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map(|v| v.trim().parse::<usize>().unwrap());
                    let done = match length {
                        Some(length) => request.len() >= end + 4 + length,
                        None if head.contains("transfer-encoding: chunked") => {
                            text.ends_with("0\r\n\r\n")
                        }
                        None => true,
                    };
                    if done {
                        break;
                    }
                }
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let _ = stream.write_all(response.as_bytes());
            String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        });
        (
            Url::parse(&format!("http://{address}{prefix}")).unwrap(),
            handle,
        )
    }
}

#[cfg(test)]
mod readonly_tests {
    use super::*;

    /// One-shot HTTP stub under the `/proxy` prefix, see [`test_http::stub`].
    fn stub(status: u16, body: &'static str) -> (Url, std::thread::JoinHandle<String>) {
        test_http::stub("/proxy", status, body)
    }

    fn server_at(url: Url) -> ServerBackend {
        let server = ServerBackend::new("project".into(), Some("group".into()), url);
        server.access_token.set(Some("test-token".into())).unwrap();
        server
    }

    fn hashes() -> Hashes {
        Hashes {
            blake3: "a".repeat(64),
            md5: None,
        }
    }

    /// `s.store` of `source` as the project file `a`.
    fn store_from(s: &ServerBackend, source: &std::path::Path) -> anyhow::Result<StoreResult> {
        let hashes = hashes();
        let path = ProjectPath::from_path(std::path::Path::new("a")).unwrap();
        s.store(StoreRequest::new_local(
            &hashes,
            source,
            path,
            Compression::Zstd,
        ))
    }

    /// `s.retrieve` of the project file `a` into `target`.
    fn retrieve_to(s: &ServerBackend, target: &std::path::Path) -> anyhow::Result<bool> {
        let hashes = hashes();
        let path = ProjectPath::from_path(std::path::Path::new("a")).unwrap();
        s.retrieve(RetrieveRequest::new_local(
            &hashes,
            target,
            path,
            Compression::Zstd,
        ))
    }

    fn manifest() -> UploadManifest {
        UploadManifest {
            operation_id: uuid::Uuid::nil(),
            message: None,
            entries: Vec::new(),
            git_branch: None,
            git_commit: None,
        }
    }

    #[test]
    fn every_server_url_keeps_the_proxy_prefix_with_or_without_trailing_slash() {
        for base in ["https://h/dvs", "https://h/dvs/"] {
            let server = ServerBackend::new("project".into(), None, Url::parse(base).unwrap());
            let hash = "a".repeat(64);
            for (url, expected) in [
                (
                    server.init_url().unwrap(),
                    "https://h/dvs/api/init".to_string(),
                ),
                (
                    server.project_url().unwrap(),
                    "https://h/dvs/api/project/project".into(),
                ),
                (
                    server.blob_url(&hash).unwrap(),
                    format!("https://h/dvs/api/project/blobs/{hash}"),
                ),
                (
                    server.manifest_url().unwrap(),
                    "https://h/dvs/api/project/manifest".into(),
                ),
                (
                    server.access_url().unwrap(),
                    "https://h/dvs/api/project/access".into(),
                ),
                (
                    server.history_url().unwrap(),
                    "https://h/dvs/api/project/history".into(),
                ),
                (
                    server.read_url("history").unwrap(),
                    "https://h/dvs/api/project/history".into(),
                ),
            ] {
                assert_eq!(url.as_str(), expected, "base {base}");
            }
        }
        // And a bare host still lands at the root
        let server = ServerBackend::new("p".into(), None, Url::parse("https://h").unwrap());
        assert_eq!(server.init_url().unwrap().as_str(), "https://h/api/init");
    }

    #[test]
    fn requests_go_under_the_proxy_prefix() {
        let (url, handle) = stub(200, "{\"missing\":[],\"compression\":\"zstd\"}");
        server_at(url).manifest(&manifest()).unwrap();
        assert!(
            handle
                .join()
                .unwrap()
                .starts_with("POST /proxy/api/project/manifest ")
        );
        let (url, handle) = stub(201, "");
        assert!(!server_at(url).init(Compression::Zstd).unwrap());
        assert!(handle.join().unwrap().starts_with("POST /proxy/api/init "));
    }

    #[test]
    fn a_non_base_server_url_is_an_error_not_a_panic() {
        for base in ["foo:bar", "mailto:x@y"] {
            let url = Url::parse(base).unwrap();
            assert!(join_url(&url, "api/init").is_err(), "{base}");
            let server = server_at(url);
            let hashes = hashes();
            assert!(server.init(Compression::Zstd).is_err(), "{base}");
            assert!(server.attach().is_err(), "{base}");
            assert!(server.access().is_err(), "{base}");
            assert!(server.manifest(&manifest()).is_err(), "{base}");
            assert!(server.exists(&hashes).is_err(), "{base}");
            assert!(server.get_audit_entries(&[]).is_err(), "{base}");
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::write(&source, b"content").unwrap();
            assert!(store_from(&server, &source).is_err(), "{base}");
            let target = temp.path().join("out");
            assert!(retrieve_to(&server, &target).is_err(), "{base}");
        }
    }
}
