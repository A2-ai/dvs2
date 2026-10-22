use anyhow::bail;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::sync::OnceLock;
use tempfile::NamedTempFile;
use ureq::Body;
use ureq::http::Response;
use url::Url;

use crate::audit::AuditEntry;
use crate::auth::{CredentialPolicy, delete_token, get_token_with};
use crate::error::{BackendError, ErrorCode};
use crate::utils::http_agent;
use crate::{Compression, Hashes, ProjectPath, RetrieveRequest, StoreRequest, StoreResult};

const NOT_ALLOWED: &str =
    "You are not allowed to access this project, make sure you are logged in with the right user.";
const LOGIN_EXPIRED: &str = "Your login is no longer valid, run `dvs login`";

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

/// The shared code for an HTTP status that has no meaning of its own on the
/// endpoint that returned it. `None` for statuses outside the shared table
/// (400, 409, 423, ...), which keep their endpoint-specific message.
///
/// The 404 rule: dvs-server answers a missing project on every project
/// route (`/api/{name}/...`, `/api/project/{name}`) with 404 and the body
/// `unknown project "<name>"` (its `resolve_project` extractor; the wording
/// has not changed since that extractor was written). Any other 404 is the
/// router's empty "no such route", and a 405 is "no such method on this
/// route": both mean the server predates the endpoint, `server_too_old`.
/// Endpoints where 404 itself means "no such content" (blob HEAD/GET) or
/// "unknown device code" (oauth token) check that before asking here.
pub(crate) fn http_status_code(status: u16, body: &str) -> Option<ErrorCode> {
    match status {
        401 => Some(ErrorCode::AuthExpired),
        403 => Some(ErrorCode::AccessDenied),
        404 if body.trim_start().starts_with("unknown project") => Some(ErrorCode::ProjectNotFound),
        404 | 405 => Some(ErrorCode::ServerTooOld),
        500.. => Some(ErrorCode::ServerUnavailable),
        _ => None,
    }
}

/// Wraps a reader and reports bytes as they're read, driving the `on_bytes`
/// progress callback. Used to stream an upload body while updating a progress bar.
///
/// It also remembers the first error its reader returned. Once ureq or
/// `io::copy` has failed, that is the only way to tell which side broke: the
/// local file (an upload source) or the network (a download body). The two
/// mean different things to callers: `io_error` vs `server_unavailable`.
struct CountingReader<'a, R> {
    inner: R,
    on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
    error: Option<std::io::Error>,
}

impl<'a, R> CountingReader<'a, R> {
    fn new(inner: R, on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>) -> Self {
        Self {
            inner,
            on_bytes,
            error: None,
        }
    }

    /// The first error the wrapped reader returned, if any.
    fn take_error(&mut self) -> Option<std::io::Error> {
        self.error.take()
    }
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(e) => {
                // Interrupted is retried by the caller, so it never decides the blame
                if self.error.is_none() && e.kind() != std::io::ErrorKind::Interrupted {
                    // io::Error is not Clone: keep a copy, pass the original on
                    self.error = Some(std::io::Error::new(e.kind(), e.to_string()));
                }
                return Err(e);
            }
        };
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
    /// Whether failures may repair stored credentials. Set by the frontend,
    /// never read from `dvs.toml`.
    #[serde(skip)]
    credential_policy: CredentialPolicy,
}

impl ServerBackend {
    pub fn new(name: String, group: Option<String>, url: Url) -> Self {
        Self {
            name,
            group,
            url,
            access_token: OnceLock::new(),
            credential_policy: CredentialPolicy::default(),
        }
    }

    pub fn set_credential_policy(&mut self, policy: CredentialPolicy) {
        self.credential_policy = policy;
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

    /// The stored token, read as the credential policy allows. No token is
    /// `auth_required`. Under `Preserve` an unreadable store is
    /// `credentials_unreadable` and is left alone; under `Repair` it is
    /// repaired (deleted) by the read, or, if even that fails, treated as
    /// no token as before.
    fn bearer(&self) -> anyhow::Result<String> {
        let token = match self.access_token.get() {
            Some(token) => token,
            None => {
                let read = match get_token_with(&self.url, self.credential_policy) {
                    Ok(token) => token,
                    Err(e) if self.credential_policy == CredentialPolicy::Preserve => {
                        return Err(e);
                    }
                    Err(e) => {
                        log::warn!("Could not read the credentials store: {e}");
                        None
                    }
                };
                self.access_token.get_or_init(|| read)
            }
        };
        match token {
            Some(token) => Ok(format!("Bearer {token}")),
            None => Err(BackendError::new(
                ErrorCode::AuthRequired,
                "Could not find access token. Log in first",
            )
            .into()),
        }
    }

    fn coded(&self, code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
        BackendError::new(code, message).into()
    }

    /// The server rejected the stored token: forget it so the next `init` asks
    /// to log in, unless the credential policy says to leave it alone
    fn login_expired(&self) -> anyhow::Error {
        if self.credential_policy == CredentialPolicy::Repair {
            if let Err(e) = delete_token(&self.url) {
                log::warn!("Could not remove the expired token: {e}");
            }
        }
        self.coded(ErrorCode::AuthExpired, LOGIN_EXPIRED)
    }

    fn unreachable(&self, e: ureq::Error) -> anyhow::Error {
        self.coded(
            ErrorCode::ServerUnavailable,
            format!("cannot reach the dvs server at {}: {e}", self.url),
        )
    }

    /// A 2xx whose body is not what the endpoint promises: the server is
    /// not working, never "no content" or an uncoded failure
    fn invalid_response(&self, e: impl std::fmt::Display) -> anyhow::Error {
        self.coded(
            ErrorCode::ServerUnavailable,
            format!("Invalid server response: {e}"),
        )
    }

    fn failing(&self, code: u16) -> anyhow::Error {
        self.coded(
            ErrorCode::ServerUnavailable,
            format!("the dvs server at {} is not working ({code})", self.url),
        )
    }

    /// The error for an HTTP status `what` (e.g. "upload") did not expect,
    /// per [`http_status_code`]. A 401 goes through [`Self::login_expired`];
    /// statuses outside the shared table keep the server's own message.
    fn status_error(&self, status: u16, body: &str, what: &str) -> anyhow::Error {
        match http_status_code(status, body) {
            Some(ErrorCode::AuthExpired) => self.login_expired(),
            Some(code) => self.shared_status_error(code, status),
            None => anyhow::anyhow!("{what} failed ({status}): {body}"),
        }
    }

    /// The message for a shared-table status. A 401 here never touches
    /// credentials; callers that may repair them use [`Self::login_expired`].
    fn shared_status_error(&self, code: ErrorCode, status: u16) -> anyhow::Error {
        match code {
            ErrorCode::AuthExpired => self.coded(code, LOGIN_EXPIRED),
            ErrorCode::AccessDenied => self.coded(code, NOT_ALLOWED),
            ErrorCode::ProjectNotFound => self.coded(
                code,
                format!("The project {} does not exist on the server", self.name),
            ),
            ErrorCode::ServerTooOld => self.coded(
                code,
                format!(
                    "the dvs server at {} does not support this request ({status}); it is older than this dvs",
                    self.url
                ),
            ),
            _ => self.failing(status),
        }
    }

    /// [`Self::status_error`] for a response, reading its body.
    fn response_error(&self, resp: &mut Response<Body>, what: &str) -> anyhow::Error {
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        self.status_error(resp.status().as_u16(), &body, what)
    }

    fn read_url(&self, endpoint: &str) -> anyhow::Result<Url> {
        join_url(&self.url, &format!("api/{}/{}", self.name, endpoint))
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
            .send_json(&payload)
            .map_err(|e| self.unreachable(e))?;
        match resp.status().as_u16() {
            201 => Ok(false), // created
            200 => Ok(true),  // already existed (group+compression matched)
            409 => {
                bail!(
                    "Server initialization failed: the project already exists with a different group or compression"
                )
            }
            _ => Err(self.response_error(&mut resp, "server init")),
        }
    }

    pub fn attach(&self) -> anyhow::Result<()> {
        let url = self.project_url()?;
        let mut resp = http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| self.unreachable(e))?;

        match resp.status().as_u16() {
            200 => Ok(()),
            _ => Err(self.response_error(&mut resp, "getting project information")),
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
            200 => Ok(Some(
                resp.body_mut()
                    .read_json()
                    .map_err(|e| self.invalid_response(e))?,
            )),
            403 => Ok(None),
            _ => Err(self.response_error(&mut resp, "access check")),
        }
    }

    pub fn check_access(&self) -> anyhow::Result<()> {
        if self.access()?.is_none() {
            return Err(self.coded(ErrorCode::AccessDenied, NOT_ALLOWED));
        }
        Ok(())
    }

    /// Declares what an add is about to store. The server records every entry and
    /// answers with the hashes it does not have, which are the only ones to `store`.
    pub fn manifest(&self, manifest: &UploadManifest) -> anyhow::Result<UploadManifestResponse> {
        let mut resp = http_agent()
            .post(self.manifest_url()?.as_str())
            .header("authorization", self.bearer()?)
            .send_json(manifest)
            .map_err(|e| self.unreachable(e))?;

        match resp.status().as_u16() {
            200 => resp
                .body_mut()
                .read_json()
                .map_err(|e| self.invalid_response(e)),
            _ => Err(self.response_error(&mut resp, "manifest")),
        }
    }

    pub fn store(&self, req: StoreRequest<'_>) -> anyhow::Result<StoreResult> {
        let mut file = CountingReader::new(fs::File::open(req.source)?, req.on_bytes);

        let mut url = self.blob_url(req.hashes.get_blake3())?;
        url.query_pairs_mut()
            .append_pair("operation_id", &req.operation_id.to_string());
        let sent = http_agent()
            .put(url.as_str())
            .header("authorization", self.bearer()?)
            .header("content-type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(&mut file));
        let mut resp = match sent {
            Ok(resp) => resp,
            // The source file failed mid-upload: a local problem, not the server's
            Err(e) => match file.take_error() {
                Some(local) => return Err(local.into()),
                None => return Err(self.unreachable(e)),
            },
        };

        match resp.status().as_u16() {
            status @ (200 | 201) => {
                let uploaded: UploadResponse = resp
                    .body_mut()
                    .read_json()
                    .map_err(|e| self.invalid_response(e))?;
                Ok(StoreResult {
                    stored_size: uploaded.stored_size,
                    compression: uploaded.compression,
                    created: status == 201,
                })
            }
            _ => Err(self.response_error(&mut resp, "upload")),
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
            code => {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                // On a blob, 404 is "no such content" unless it is the
                // project that is missing
                if code == 404 && http_status_code(code, &body) != Some(ErrorCode::ProjectNotFound)
                {
                    return Ok(false);
                }
                return Err(self.status_error(code, &body, "download"));
            }
        }

        let tmp = NamedTempFile::new()?.into_temp_path();
        {
            let mut reader = CountingReader::new(resp.body_mut().as_reader(), req.on_bytes);
            let mut out = fs::File::create(&tmp)?;
            self.download(&mut reader, &mut out)?;
        }

        if let Some(parent) = req.target.parent() {
            fs::create_dir_all(parent)?;
        }
        req.compression.decompress(&tmp, req.target, None)?;
        Ok(true)
    }

    /// Copy a download body to a local file. The body failing is the
    /// server's (`server_unavailable`); the local write failing is an
    /// uncoded io error, which callers report as `io_error`.
    fn download<R: Read>(
        &self,
        body: &mut CountingReader<'_, R>,
        out: &mut impl std::io::Write,
    ) -> anyhow::Result<u64> {
        std::io::copy(body, out).map_err(|e| match body.take_error() {
            Some(network) => self.coded(
                ErrorCode::ServerUnavailable,
                format!("the download from {} was cut off: {network}", self.url),
            ),
            None => e.into(),
        })
    }

    /// A HEAD answer has no body, so a blob 404 is always "absent" here;
    /// callers establish project access first ([`Self::check_access`]).
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
            404 => Ok(false),
            code => Err(self.status_error(code, "", "checking whether a file exists")),
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
            .send_json(&payload)
            .map_err(|e| self.unreachable(e))?;

        match resp.status().as_u16() {
            200 => resp
                .body_mut()
                .read_json::<Vec<AuditEntry>>()
                .map_err(|e| self.invalid_response(e)),
            _ => Err(self.response_error(&mut resp, "history")),
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
    use crate::auth::test_store;
    use crate::error::backend_error_code;

    /// One-shot HTTP stub under the `/proxy` prefix, see [`test_http::stub`].
    fn stub(status: u16, body: &'static str) -> (Url, std::thread::JoinHandle<String>) {
        test_http::stub("/proxy", status, body)
    }

    /// A URL nothing listens on: every request is a transport error.
    fn closed_url() -> Url {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        Url::parse(&format!("http://{address}/proxy")).unwrap()
    }

    fn server_at(url: Url) -> ServerBackend {
        let server = ServerBackend::new("project".into(), Some("group".into()), url);
        server.access_token.set(Some("test-token".into())).unwrap();
        server
    }

    /// A server whose token comes from a credentials store in `dir`, which
    /// holds a token for that server.
    fn server_with_store(url: Url, dir: &std::path::Path) -> (ServerBackend, PathBuf) {
        let path = dir.join("credentials.json");
        test_store::set(&path);
        let json = serde_json::json!({ url.as_str(): "test-token" }).to_string();
        fs::write(&path, json).unwrap();
        (
            ServerBackend::new("project".into(), Some("group".into()), url),
            path,
        )
    }

    /// An endpoint call under test, reduced to the code it failed with.
    type Call = fn(&ServerBackend) -> ErrorCode;
    /// Same, for calls that need a local file.
    type CallWithSource = fn(&ServerBackend, &std::path::Path) -> ErrorCode;

    fn code<T>(result: anyhow::Result<T>) -> ErrorCode {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(e) => backend_error_code(&e).unwrap_or_else(|| panic!("not coded: {e:#}")),
        }
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
    fn the_status_table_follows_the_server_api() {
        assert_eq!(http_status_code(401, ""), Some(ErrorCode::AuthExpired));
        assert_eq!(http_status_code(403, ""), Some(ErrorCode::AccessDenied));
        assert_eq!(
            http_status_code(404, "unknown project \"p\""),
            Some(ErrorCode::ProjectNotFound)
        );
        assert_eq!(http_status_code(404, ""), Some(ErrorCode::ServerTooOld));
        assert_eq!(http_status_code(405, ""), Some(ErrorCode::ServerTooOld));
        for status in [500, 502, 503] {
            assert_eq!(
                http_status_code(status, ""),
                Some(ErrorCode::ServerUnavailable)
            );
        }
        for status in [400, 409, 423] {
            assert_eq!(http_status_code(status, "x"), None);
        }
    }

    /// A real HTTP response must retain 404/403/503 semantics through ureq.
    #[test]
    fn head_responses_preserve_absent_denied_and_unavailable() {
        for (status, expected) in [
            (200, None),
            (404, None),
            (403, Some(ErrorCode::AccessDenied)),
            (503, Some(ErrorCode::ServerUnavailable)),
        ] {
            let (url, handle) = stub(status, "");
            let result = server_at(url).exists(&hashes());
            match expected {
                None => assert_eq!(result.unwrap(), status == 200),
                Some(expected) => assert_eq!(code(result), expected),
            }
            assert!(
                handle
                    .join()
                    .unwrap()
                    .starts_with("HEAD /proxy/api/project/blobs/")
            );
        }
    }

    #[test]
    fn no_token_is_auth_required() {
        let temp = tempfile::tempdir().unwrap();
        test_store::set(&temp.path().join("credentials.json"));
        let server = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        assert_eq!(code(server.access()), ErrorCode::AuthRequired);
        assert!(!temp.path().join("credentials.json").exists());
    }

    #[test]
    fn a_401_under_preserve_is_auth_expired_and_keeps_the_token() {
        let temp = tempfile::tempdir().unwrap();
        let (url, handle) = stub(401, "invalid or expired token");
        let (mut server, path) = server_with_store(url, temp.path());
        server.set_credential_policy(CredentialPolicy::Preserve);
        let before = fs::read(&path).unwrap();
        assert_eq!(code(server.access()), ErrorCode::AuthExpired);
        assert_eq!(fs::read(&path).unwrap(), before);
        handle.join().unwrap();
    }

    #[test]
    fn a_401_under_repair_is_auth_expired_and_forgets_the_token() {
        let temp = tempfile::tempdir().unwrap();
        let (url, handle) = stub(401, "invalid or expired token");
        let (server, _path) = server_with_store(url.clone(), temp.path());
        assert_eq!(code(server.access()), ErrorCode::AuthExpired);
        assert_eq!(crate::auth::get_token(&url).unwrap(), None);
        handle.join().unwrap();
    }

    #[test]
    fn a_corrupt_store_under_preserve_is_credentials_unreadable_and_kept() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        test_store::set(&path);
        let original = b"{ not json";
        fs::write(&path, original).unwrap();
        let mut server = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        server.set_credential_policy(CredentialPolicy::Preserve);
        assert_eq!(code(server.access()), ErrorCode::CredentialsUnreadable);
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn a_corrupt_store_under_repair_is_deleted_as_before() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        test_store::set(&path);
        fs::write(&path, b"{ not json").unwrap();
        let server = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        assert_eq!(code(server.access()), ErrorCode::AuthRequired);
        assert!(!path.exists());
    }

    #[test]
    fn a_403_is_access_denied_on_every_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"content").unwrap();
        let calls: Vec<(&str, CallWithSource)> = vec![
            ("access", |s, _| code(s.check_access())),
            ("manifest", |s, _| code(s.manifest(&manifest()))),
            ("history", |s, _| code(s.get_audit_entries(&[]))),
            ("exists", |s, _| code(s.exists(&hashes()))),
            ("store", |s, source| code(store_from(s, source))),
            ("retrieve", |s, source| {
                code(retrieve_to(s, &source.with_extension("out")))
            }),
        ];
        for (name, call) in calls {
            let (url, handle) = stub(403, "forbidden");
            assert_eq!(
                call(&server_at(url), &source),
                ErrorCode::AccessDenied,
                "{name}"
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn transport_errors_are_server_unavailable_on_every_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"content").unwrap();
        let server = server_at(closed_url());
        let hashes = hashes();
        for (name, result) in [
            ("init", code(server.init(Compression::Zstd))),
            ("attach", code(server.attach())),
            ("access", code(server.access())),
            ("manifest", code(server.manifest(&manifest()))),
            ("history", code(server.get_audit_entries(&[]))),
            ("exists", code(server.exists(&hashes))),
            ("store", code(store_from(&server, &source))),
            (
                "retrieve",
                code(retrieve_to(&server, &temp.path().join("out"))),
            ),
        ] {
            assert_eq!(result, ErrorCode::ServerUnavailable, "{name}");
        }
    }

    #[test]
    fn a_5xx_is_server_unavailable() {
        for status in [500, 503] {
            let (url, handle) = stub(status, "internal server error");
            assert_eq!(
                code(server_at(url).manifest(&manifest())),
                ErrorCode::ServerUnavailable
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn a_missing_endpoint_is_server_too_old() {
        for status in [404, 405] {
            let (url, handle) = stub(status, "");
            assert_eq!(
                code(server_at(url).access()),
                ErrorCode::ServerTooOld,
                "{status}"
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn an_unknown_project_is_project_not_found() {
        const BODY: &str = "unknown project \"project\"";
        let calls: Vec<(&str, Call)> = vec![
            ("attach", |s| code(s.attach())),
            ("access", |s| code(s.access())),
            ("manifest", |s| code(s.manifest(&manifest()))),
            ("retrieve", |s| {
                let temp = tempfile::tempdir().unwrap();
                code(retrieve_to(s, &temp.path().join("out")))
            }),
        ];
        for (name, call) in calls {
            let (url, handle) = stub(404, BODY);
            assert_eq!(call(&server_at(url)), ErrorCode::ProjectNotFound, "{name}");
            handle.join().unwrap();
        }
    }

    #[test]
    fn a_blob_404_is_still_absent_content() {
        let temp = tempfile::tempdir().unwrap();
        let (url, handle) = stub(404, "Not found");
        let found = retrieve_to(&server_at(url), &temp.path().join("out")).unwrap();
        assert!(!found);
        handle.join().unwrap();
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

    #[test]
    fn an_invalid_body_on_a_2xx_is_server_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::write(&source, b"content").unwrap();
        let calls: Vec<(&str, u16, CallWithSource)> = vec![
            ("access", 200, |s, _| code(s.access())),
            ("manifest", 200, |s, _| code(s.manifest(&manifest()))),
            ("history", 200, |s, _| code(s.get_audit_entries(&[]))),
            ("store", 201, |s, source| code(store_from(s, source))),
        ];
        for (name, status, call) in calls {
            let (url, handle) = stub(status, "this is not json");
            assert_eq!(
                call(&server_at(url), &source),
                ErrorCode::ServerUnavailable,
                "{name}"
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn a_source_that_fails_mid_upload_is_a_local_io_error() {
        let temp = tempfile::tempdir().unwrap();
        // A directory opens fine on unix but every read fails (EISDIR)
        let source = temp.path().join("a-directory");
        fs::create_dir(&source).unwrap();
        let (url, handle) = stub(201, r#"{"stored_size":0,"compression":"zstd"}"#);
        let error = store_from(&server_at(url), &source).err().unwrap();
        assert_eq!(backend_error_code(&error), None, "{error:#}");
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "{error:#}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn a_download_cut_off_by_the_network_is_server_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        let (url, handle) = test_http::respond(
            "/proxy",
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".into(),
        );
        let result = retrieve_to(&server_at(url), &temp.path().join("out"));
        assert_eq!(code(result), ErrorCode::ServerUnavailable);
        handle.join().unwrap();
    }

    #[test]
    fn a_local_write_failure_during_download_is_an_io_error() {
        struct Full;
        impl std::io::Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk full"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let server = server_at(Url::parse("https://h/").unwrap());
        let mut body = CountingReader::new(&b"downloaded bytes"[..], None);
        let error = server.download(&mut body, &mut Full).unwrap_err();
        assert_eq!(backend_error_code(&error), None, "{error:#}");
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "{error:#}"
        );
    }

    #[test]
    fn no_config_dir_under_preserve_is_auth_required() {
        test_store::no_config_dir();
        let mut server = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        server.set_credential_policy(CredentialPolicy::Preserve);
        assert_eq!(code(server.access()), ErrorCode::AuthRequired);
        // Human mode reads it as no token, as before
        let server = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        assert_eq!(code(server.access()), ErrorCode::AuthRequired);
    }
}
