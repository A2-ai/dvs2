use anyhow::bail;
use fs_err as fs;
use serde::de::DeserializeOwned;
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
use crate::error::{BackendError, ErrorCode, backend_error_code};
use crate::utils::http_agent;
use crate::{Compression, Hashes, ProjectPath, RetrieveRequest, StoreRequest, StoreResult};

const NOT_ALLOWED: &str =
    "You are not allowed to access this project, make sure you are logged in with the right user.";
const LOGIN_EXPIRED: &str = "Your login is no longer valid, run `dvs login`";

/// Join `relative` under the server URL, keeping any proxy path prefix:
/// `Url::join` drops it for a leading-slash path or a base without a
/// trailing slash.
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

/// The shared code for an HTTP status, `None` for endpoint-specific ones.
///
/// dvs-server answers a missing project with 404 and a body starting with
/// `unknown project`; any other 404 (or a 405) is a route this server does
/// not have yet.
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
/// It also remembers its reader's first error: once ureq or `io::copy` has
/// failed, that is the only way to tell a local read error from a network one.
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

    fn take_error(&mut self) -> Option<std::io::Error> {
        self.error.take()
    }
}

impl<R: Read> Read for CountingReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(e) => {
                if self.error.is_none() && e.kind() != std::io::ErrorKind::Interrupted {
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

    /// The server rejected the stored token: forget it so the next `init` asks to log in
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

    fn status_error(&self, status: u16, body: &str, what: &str) -> anyhow::Error {
        match http_status_code(status, body) {
            Some(ErrorCode::AuthExpired) => self.login_expired(),
            Some(code) => self.shared_status_error(code, status),
            None => anyhow::anyhow!("{what} failed ({status}): {body}"),
        }
    }

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

    fn response_error(&self, resp: &mut Response<Body>, what: &str) -> anyhow::Error {
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        self.status_error(resp.status().as_u16(), &body, what)
    }

    /// A read path's JSON answer; any other status per [`Self::read_status`].
    fn read_json<T: DeserializeOwned>(&self, mut response: Response<Body>) -> anyhow::Result<T> {
        let status = response.status().as_u16();
        if status != 200 {
            let body = response.body_mut().read_to_string().unwrap_or_default();
            self.read_status(status, &body)?;
        }
        response
            .body_mut()
            .read_json()
            .map_err(|e| self.invalid_response(e))
    }

    fn read_url(&self, endpoint: &str) -> anyhow::Result<Url> {
        join_url(&self.url, &format!("api/{}/{}", self.name, endpoint))
    }

    /// Like [`Self::bearer`] but never modifies stored credentials, whatever
    /// the policy.
    fn read_bearer(&self) -> anyhow::Result<String> {
        if let Some(Some(token)) = self.access_token.get() {
            return Ok(format!("Bearer {token}"));
        }
        let token = crate::auth::get_token_readonly(&self.url)
            .map_err(|e| {
                if backend_error_code(&e).is_some() {
                    e
                } else {
                    self.coded(
                        ErrorCode::CredentialsUnreadable,
                        format!("Cannot read credentials: {e}"),
                    )
                }
            })?
            .ok_or_else(|| self.coded(ErrorCode::AuthRequired, "Log in with `dvs login`"))?;
        let _ = self.access_token.set(Some(token.clone()));
        Ok(format!("Bearer {token}"))
    }

    /// Read paths never repair credentials, and any status outside the
    /// shared table is `server_unavailable`: a read failure must never be
    /// mistaken for missing content.
    fn read_status(&self, status: u16, body: &str) -> anyhow::Result<()> {
        match status {
            200 => Ok(()),
            code => Err(match http_status_code(code, body) {
                Some(shared) => self.shared_status_error(shared, code),
                None => self.failing(code),
            }),
        }
    }

    /// Bounded access check that never modifies stored credentials.
    pub fn access_readonly(&self) -> anyhow::Result<AccessResponse> {
        let response = http_agent()
            .get(self.read_url("access")?.as_str())
            .header("authorization", self.read_bearer()?)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .call()
            .map_err(|e| self.unreachable(e))?;
        self.read_json(response)
    }

    /// Query content only after establishing project access. A blob 404 then
    /// means absent content rather than an inaccessible or nonexistent project
    /// (a HEAD answer has no body to tell them apart).
    pub fn exists_readonly(&self, hashes: &Hashes) -> anyhow::Result<bool> {
        let response = http_agent()
            .head(
                self.read_url(&format!("blobs/{}", hashes.get_blake3()))?
                    .as_str(),
            )
            .header("authorization", self.read_bearer()?)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .call()
            .map_err(|e| self.unreachable(e))?;
        match response.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            code => {
                self.read_status(code, "")?;
                unreachable!()
            }
        }
    }

    pub fn history_readonly(&self, files: &[PathBuf]) -> anyhow::Result<Vec<AuditEntry>> {
        let paths = files
            .iter()
            .map(|p| ProjectPath::from_path(p))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!(e))?;
        let response = http_agent()
            .post(self.read_url("history")?.as_str())
            .header("authorization", self.read_bearer()?)
            .config()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .send_json(&HistoryPayload { paths })
            .map_err(|e| self.unreachable(e))?;
        self.read_json(response)
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
                // On a blob, 404 is "no such content" unless the project is missing
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// One-shot HTTP server answering the first request with `status` and `body`.
    fn stub(status: u16, body: &'static str) -> (Url, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            // Read the whole request so the client never sees a reset mid-send
            let mut request = Vec::new();
            let mut buffer = [0u8; 8192];
            loop {
                let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let _ = write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        (url, handle)
    }

    fn server_at(url: Url) -> ServerBackend {
        let server = ServerBackend::new("project".into(), Some("group".into()), url);
        server.access_token.set(Some("test-token".into())).unwrap();
        server
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
    fn server_urls_keep_the_path_prefix() {
        for base in ["https://h/dvs", "https://h/dvs/"] {
            let url = Url::parse(base).unwrap();
            let server = ServerBackend::new("project".into(), None, url.clone());
            assert_eq!(
                server.init_url().unwrap().as_str(),
                "https://h/dvs/api/init"
            );
            assert_eq!(
                server.access_url().unwrap().as_str(),
                "https://h/dvs/api/project/access"
            );
            assert_eq!(
                crate::auth::get_client_approve_url(&url, "ABCD")
                    .unwrap()
                    .as_str(),
                "https://h/dvs/device/approve?user_code=ABCD"
            );
        }
    }

    #[test]
    fn failures_have_the_shared_codes() {
        for (status, body, expected) in [
            (401, "", ErrorCode::AuthExpired),
            (403, "", ErrorCode::AccessDenied),
            (
                404,
                "unknown project \"project\"",
                ErrorCode::ProjectNotFound,
            ),
            (404, "", ErrorCode::ServerTooOld),
            (405, "", ErrorCode::ServerTooOld),
            (503, "", ErrorCode::ServerUnavailable),
            (200, "not json", ErrorCode::ServerUnavailable),
        ] {
            let (url, handle) = stub(status, body);
            let error = server_at(url).manifest(&manifest()).err().unwrap();
            assert_eq!(
                backend_error_code(&error),
                Some(expected),
                "{status} {body}"
            );
            handle.join().unwrap();
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        let error = server_at(closed).access().err().unwrap();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::ServerUnavailable)
        );

        let no_token = ServerBackend::new("p".into(), None, Url::parse("https://h/").unwrap());
        let error = no_token.access().err().unwrap();
        assert_eq!(backend_error_code(&error), Some(ErrorCode::AuthRequired));
    }
}
