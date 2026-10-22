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
use crate::auth::get_token;
use crate::error::{Error, ErrorCode};
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

fn coded(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    Error::new(code, message).into()
}

pub(crate) fn unreachable(base_url: &Url, e: ureq::Error) -> anyhow::Error {
    coded(
        ErrorCode::ServerUnavailable,
        format!("cannot reach the dvs server at {base_url}: {e}"),
    )
}

pub(crate) fn invalid_response(base_url: &Url, e: impl std::fmt::Display) -> anyhow::Error {
    coded(
        ErrorCode::ServerUnavailable,
        format!("Invalid response from the dvs server at {base_url}: {e}"),
    )
}

/// The error for a status the endpoint does not handle itself.
pub(crate) fn status_error(base_url: &Url, status: u16, body: &str, what: &str) -> anyhow::Error {
    let Some(code) = http_status_code(status, body) else {
        return anyhow::anyhow!("{what} failed ({status}): {body}");
    };
    let message = match code {
        ErrorCode::AuthExpired => LOGIN_EXPIRED.to_string(),
        ErrorCode::AccessDenied => NOT_ALLOWED.to_string(),
        ErrorCode::ProjectNotFound => format!(
            "the dvs server at {base_url} does not have this project ({})",
            body.trim()
        ),
        ErrorCode::ServerTooOld => format!(
            "the dvs server at {base_url} does not support this request ({status}); it is older than this dvs"
        ),
        _ => format!("the dvs server at {base_url} is not working ({status})"),
    };
    coded(code, message)
}

pub(crate) fn response_error(
    base_url: &Url,
    resp: &mut Response<Body>,
    what: &str,
) -> anyhow::Error {
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    status_error(base_url, resp.status().as_u16(), &body, what)
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

    fn bearer(&self) -> anyhow::Result<String> {
        let token = match self.access_token.get() {
            Some(token) => token,
            None => {
                let read = get_token(&self.url)?;
                self.access_token.get_or_init(|| read)
            }
        };
        match token {
            Some(token) => Ok(format!("Bearer {token}")),
            None => Err(coded(
                ErrorCode::AuthRequired,
                "Could not find access token. Log in first",
            )),
        }
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
            .map_err(|e| unreachable(&self.url, e))?;
        match resp.status().as_u16() {
            201 => Ok(false), // created
            200 => Ok(true),  // already existed (group+compression matched)
            409 => {
                bail!(
                    "Server initialization failed: the project already exists with a different group or compression"
                )
            }
            _ => Err(response_error(&self.url, &mut resp, "server init")),
        }
    }

    pub fn attach(&self) -> anyhow::Result<()> {
        let url = self.project_url()?;
        let mut resp = http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| unreachable(&self.url, e))?;

        match resp.status().as_u16() {
            200 => Ok(()),
            _ => Err(response_error(
                &self.url,
                &mut resp,
                "getting project information",
            )),
        }
    }

    /// What the current user can do on the project. `None` when they are not a member.
    pub fn access(&self) -> anyhow::Result<Option<AccessResponse>> {
        let mut resp = http_agent()
            .get(self.access_url()?.as_str())
            .header("authorization", self.bearer()?)
            .call()
            .map_err(|e| unreachable(&self.url, e))?;

        match resp.status().as_u16() {
            200 => Ok(Some(
                resp.body_mut()
                    .read_json()
                    .map_err(|e| invalid_response(&self.url, e))?,
            )),
            403 => Ok(None),
            _ => Err(response_error(&self.url, &mut resp, "access check")),
        }
    }

    pub fn check_access(&self) -> anyhow::Result<()> {
        if self.access()?.is_none() {
            return Err(coded(ErrorCode::AccessDenied, NOT_ALLOWED));
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
            .map_err(|e| unreachable(&self.url, e))?;

        match resp.status().as_u16() {
            200 => resp
                .body_mut()
                .read_json()
                .map_err(|e| invalid_response(&self.url, e)),
            _ => Err(response_error(&self.url, &mut resp, "manifest")),
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
                None => return Err(unreachable(&self.url, e)),
            },
        };

        match resp.status().as_u16() {
            status @ (200 | 201) => {
                let uploaded: UploadResponse = resp
                    .body_mut()
                    .read_json()
                    .map_err(|e| invalid_response(&self.url, e))?;
                Ok(StoreResult {
                    stored_size: uploaded.stored_size,
                    compression: uploaded.compression,
                    created: status == 201,
                })
            }
            _ => Err(response_error(&self.url, &mut resp, "upload")),
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
            .map_err(|e| unreachable(&self.url, e))?;

        match resp.status().as_u16() {
            200 => {}
            code => {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                // On a blob, 404 is "no such content" unless the project is missing
                if code == 404 && http_status_code(code, &body) != Some(ErrorCode::ProjectNotFound)
                {
                    return Ok(false);
                }
                return Err(status_error(&self.url, code, &body, "download"));
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
            Some(network) => coded(
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
            .map_err(|e| unreachable(&self.url, e))?
            .status()
            .as_u16()
        {
            200 => Ok(true),
            404 => Ok(false),
            code => Err(status_error(
                &self.url,
                code,
                "",
                "checking whether a file exists",
            )),
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
            .map_err(|e| unreachable(&self.url, e))?;

        match resp.status().as_u16() {
            200 => resp
                .body_mut()
                .read_json::<Vec<AuditEntry>>()
                .map_err(|e| invalid_response(&self.url, e)),
            _ => Err(response_error(&self.url, &mut resp, "history")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn statuses_map_to_the_shared_codes() {
        for (status, body, expected) in [
            (401, "", Some(ErrorCode::AuthExpired)),
            (403, "", Some(ErrorCode::AccessDenied)),
            (
                404,
                "unknown project \"project\"",
                Some(ErrorCode::ProjectNotFound),
            ),
            (404, "", Some(ErrorCode::ServerTooOld)),
            (405, "", Some(ErrorCode::ServerTooOld)),
            (503, "", Some(ErrorCode::ServerUnavailable)),
            (409, "", None),
        ] {
            assert_eq!(http_status_code(status, body), expected, "{status} {body}");
        }
    }
}
