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

    fn init_url(&self) -> Url {
        self.url.join("/api/init").unwrap()
    }

    fn project_url(&self) -> Url {
        self.url
            .join(&format!("api/project/{}", self.name,))
            .unwrap()
    }

    fn blob_url(&self, hash: &str) -> Url {
        self.url
            .join(&format!("api/{}/blobs/{hash}", self.name))
            .unwrap()
    }

    fn manifest_url(&self) -> Url {
        self.url
            .join(&format!("api/{}/manifest", self.name))
            .unwrap()
    }

    fn access_url(&self) -> Url {
        self.url
            .join(&format!("/api/{}/access", self.name))
            .unwrap()
    }

    fn history_url(&self) -> Url {
        self.url
            .join(&format!("/api/{}/history", self.name))
            .unwrap()
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
        let url = self.init_url();
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
        let url = self.project_url();
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
            .get(self.access_url().as_str())
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
            .post(self.manifest_url().as_str())
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

        let mut url = self.blob_url(req.hashes.get_blake3());
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
            401 => return Err(self.login_expired()),
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
        let mut url = self.blob_url(req.hashes.get_blake3());
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
        let url = self.blob_url(hashes.get_blake3());

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
        let url = self.history_url();

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
            401 => return Err(self.login_expired()),
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
