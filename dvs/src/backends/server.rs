//! A project on a dvs server, through API v1.

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use url::Url;

use crate::api::v1::{
    AccessView, ActivityBatch, ActivityItem, BatchKind, BatchView, CompleteBatch, CreateProject,
    FileStatus, FileView, FilesSummary, GitContext, GrantRequest, GroupView, Manifest,
    ManifestEntry, ManifestResult, MeView, OpenBatch, Page, PatchProject, ProjectView, PutGrant,
    Role, ServerInfo, StatusFile, StatusRequest, StatusResult, StoredBlob, UnyankRequest,
    VersionView, YankRequest, YankView,
};
use crate::audit::AuditEntry;
use crate::auth::delete_token;
use crate::backends::{
    Batch, ManifestItem, ManifestLimits, ManifestReply, PresentSize, RetrieveRequest, Stored,
};
use crate::queries::{ActivityQuery, FilesQuery, GroupsQuery, Pairs, ProjectsQuery};
use crate::remote::{
    ApiError, Client, NoResponse, Response, TOKEN_ENV, TokenSource, check, is_no_response,
    read_json,
};
use crate::{Compression, Hashes};

/// The header naming a download's compression.
pub const COMPRESSION_HEADER: &str = "x-dvs-compression";

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
        if let Some(cb) = self.on_bytes.filter(|_| n > 0) {
            cb(n as u64);
        }
        Ok(n)
    }
}

fn parse_compression(s: &str) -> Result<Compression> {
    match s {
        "zstd" => Ok(Compression::Zstd),
        "none" => Ok(Compression::None),
        other => bail!("the server uses compression {other:?}, which this dvs cannot read"),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerBackend {
    pub name: String,
    pub url: Url,
    /// The group that gets admin when `dvs init` creates the project; not kept
    #[serde(skip)]
    pub group: Option<String>,
    #[serde(skip)]
    client: Arc<Client>,
}

impl PartialEq for ServerBackend {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.url == other.url && self.group == other.group
    }
}

impl ServerBackend {
    pub fn new(name: String, group: Option<String>, url: Url) -> Self {
        Self {
            name,
            group,
            url,
            client: Arc::default(),
        }
    }

    fn project_url(&self, rest: &[&str]) -> Url {
        let mut segments = vec!["projects", self.name.as_str()];
        segments.extend_from_slice(rest);
        Client::v1(&self.url, &segments)
    }

    /// Whether `url` reaches the same server as this backend's URL: both as requests
    /// are built, so a trailing slash, the host's case, a default port or a path after
    /// the host make no difference.
    pub fn same_server(&self, url: &Url) -> bool {
        Client::v1(&self.url, &[]) == Client::v1(url, &[])
    }

    /// The token in use and where it came from.
    pub fn token_source(&self) -> Option<TokenSource> {
        self.client.token(&self.url).map(|(_, s)| *s)
    }

    /// An error to return: a rejected stored token is forgotten, so the next command
    /// asks to log in.
    fn fail(&self, e: anyhow::Error) -> anyhow::Error {
        let Some(api) = crate::remote::api_error(&e) else {
            return e;
        };
        if !matches!(api.code.as_str(), "unauthenticated" | "token_expired") {
            return e;
        }
        let mut api = api.clone();
        let message = api.message.trim_end_matches('.').to_string();
        match self.token_source() {
            Some(TokenSource::Stored) => {
                if self.client.forget_once() {
                    if let Err(e) = delete_token(&self.url) {
                        log::warn!("Could not remove the rejected token: {e}");
                    }
                }
                if !message.contains("dvs login") {
                    api.message = format!("{message}; run `dvs login`");
                }
            }
            // `dvs login` would not help: the variable wins over a stored token
            Some(TokenSource::Environment) => {
                api.message = format!(
                    "{message}: the server rejected the token in {TOKEN_ENV}. Set it to a \
                     valid token, or unset it and run `dvs login`"
                );
            }
            None => return e,
        }
        api.into()
    }

    fn json<T: serde::de::DeserializeOwned>(&self, resp: Result<Response>) -> Result<T> {
        resp.and_then(read_json).map_err(|e| self.fail(e))
    }

    /// Create the project, with `group` as its admin.
    pub fn init(&self, compression: Compression) -> Result<bool> {
        let Some(group) = self.group.clone() else {
            bail!("An Arx group is required to create a project on the server: give --group NAME");
        };
        self.client.server_info(&self.url)?;
        let body = CreateProject {
            name: self.name.clone(),
            compression: Some(compression.to_string()),
            grants: vec![GrantRequest {
                group,
                role: "admin".to_string(),
            }],
        };
        let resp = self
            .client
            .post_json(&self.url, &Client::v1(&self.url, &["projects"]), &body);
        match self.json::<ProjectView>(resp) {
            Ok(_) => Ok(false),
            Err(e) => {
                if let Some(api) = crate::remote::api_error(&e)
                    && api.code == "project_exists"
                    && api.details.contains_key("project")
                {
                    let mut api = api.clone();
                    api.message = format!(
                        "{} exists and you can read it; to use it here, run `dvs attach {} {}`",
                        self.name, self.name, self.url
                    );
                    return Err(api.into());
                }
                Err(e)
            }
        }
    }

    /// The project as the server describes it to the caller.
    pub fn project(&self) -> Result<ProjectView> {
        self.client.server_info(&self.url)?;
        let resp = self.client.get(&self.url, &self.project_url(&[]));
        self.json(resp)
    }

    /// What the server says about itself, checked against this client.
    pub fn server_info(&self) -> Result<ServerInfo> {
        self.client.server_info(&self.url).cloned()
    }

    /// Who the token belongs to.
    pub fn me(&self) -> Result<MeView> {
        let resp = self.client.get(&self.url, &Client::v1(&self.url, &["me"]));
        self.json(resp)
    }

    pub fn check_access(&self) -> Result<()> {
        self.project().map(|_| ())
    }

    pub fn manifest_limits(&self) -> Result<ManifestLimits> {
        let info = self.client.server_info(&self.url)?;
        Ok(ManifestLimits {
            max_entries: info.limits.max_manifest_entries.max(1),
            max_body_bytes: info.limits.max_body_bytes.max(1024),
            // 0 means no limit
            max_blob_size: Some(info.limits.max_blob_size).filter(|&n| n > 0),
            max_path_bytes: Some(info.limits.max_path_bytes).filter(|&n| n > 0),
        })
    }

    pub fn open_batch(
        &self,
        kind: BatchKind,
        message: Option<&str>,
        git: &GitContext,
    ) -> Result<Batch> {
        self.client.server_info(&self.url)?;
        let body = OpenBatch {
            kind,
            message: message.map(str::to_owned),
            git: Some(git.clone()),
        };
        let resp = self
            .client
            .post_json(&self.url, &self.project_url(&["batches"]), &body);
        let view: BatchView = self.json(resp)?;
        Ok(Batch {
            id: view.id,
            kind,
            message: None,
            // The server decides; the manifest says what it is
            compression: Compression::default(),
        })
    }

    pub fn manifest(&self, batch: &Batch, items: &[ManifestItem]) -> Result<ManifestReply> {
        let body = Manifest {
            entries: items
                .iter()
                .map(|i| ManifestEntry {
                    path: i.path.as_str().to_string(),
                    hash: i.hashes.get_blake3().to_string(),
                    size: i.size,
                })
                .collect(),
        };
        let id = batch.id.to_string();
        let url = self.project_url(&["batches", &id, "manifest"]);
        // Recording a manifest is idempotent, so a lost response is sent once more
        let result: ManifestResult = match self.json(self.client.post_json(&self.url, &url, &body))
        {
            Err(e) if is_no_response(&e) => {
                log::warn!("No response to the manifest, retrying: {e:#}");
                self.json(self.client.post_json(&self.url, &url, &body))?
            }
            other => other?,
        };
        Ok(ManifestReply {
            missing: result.missing.into_iter().collect::<BTreeSet<_>>(),
            present: result
                .present
                .into_iter()
                .map(|p| {
                    (
                        p.hash,
                        PresentSize {
                            size: p.size,
                            stored_size: p.stored_size,
                        },
                    )
                })
                .collect::<HashMap<_, _>>(),
            compression: parse_compression(&result.compression)?,
        })
    }

    /// Send one content's bytes. When a request gets no response at all (or its
    /// response is cut off), the server is asked whether it now has the content: if it
    /// does, the upload is done; if the project was locked or the role lowered, that is
    /// the error; otherwise the bytes are sent once more, which is safe because the
    /// server checks and discards bytes it already has.
    pub fn put(
        &self,
        batch: &Batch,
        hashes: &Hashes,
        source: &Path,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<Stored> {
        let mut url = self.project_url(&["blobs", hashes.get_blake3()]);
        url.query_pairs_mut()
            .append_pair("batch", &batch.id.to_string());
        let mut attempt = 0;
        loop {
            attempt += 1;
            let e = match self.put_once(&url, source, on_bytes) {
                Ok(stored) => {
                    return Ok(Stored {
                        stored_size: Some(stored.stored_size),
                        compression: parse_compression(&stored.compression)?,
                    });
                }
                Err(e) if is_no_response(&e) => e,
                Err(e) => return Err(self.fail(e)),
            };
            if let Ok(true) = self.exists(hashes) {
                log::warn!(
                    "No response to the upload of {}, but the server has the content",
                    source.display()
                );
                return Ok(Stored {
                    stored_size: None,
                    compression: Compression::default(),
                });
            }
            if let Ok(project) = self.project() {
                if project.locked {
                    return Err(
                        ApiError::new(423, "project_locked", "the project is locked").into(),
                    );
                }
                if project.your_role < Role::Write {
                    return Err(ApiError::new(
                        403,
                        "insufficient_role",
                        format!(
                            "uploading needs the write role; yours is {}",
                            project.your_role
                        ),
                    )
                    .into());
                }
            }
            if attempt >= 2 {
                return Err(e.context(
                    "no response after a retry: the bytes may have been uploaded; run `dvs add` again",
                ));
            }
            log::warn!(
                "No response to the upload of {}, retrying: {e:#}",
                source.display()
            );
        }
    }

    /// One `PUT`. A transport failure while sending, or while reading a success
    /// response, is a [`NoResponse`]; an error the server answered is not.
    fn put_once(
        &self,
        url: &Url,
        source: &Path,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<StoredBlob> {
        let mut body = CountingReader::new(fs::File::open(source)?, on_bytes);
        let mut resp = self.client.put_reader(&self.url, url, &mut body)?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            return Err(ApiError::from_body(status, &body).into());
        }
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| anyhow::Error::from(NoResponse(e)))?;
        serde_json::from_str(&text).context("the server's answer to an upload")
    }

    pub fn complete(&self, batch: &Batch, counts: &CompleteBatch) -> Result<()> {
        let id = batch.id.to_string();
        let resp = self.client.post_json(
            &self.url,
            &self.project_url(&["batches", &id, "complete"]),
            counts,
        );
        resp.and_then(check).map_err(|e| self.fail(e))?;
        Ok(())
    }

    /// Download into `req.target`, decompressing as the server says it stored the
    /// bytes. False when the project has no such content.
    pub fn fetch(&self, batch: &Batch, req: RetrieveRequest<'_>) -> Result<bool> {
        let mut url = self.project_url(&["blobs", req.hashes.get_blake3()]);
        url.query_pairs_mut()
            .append_pair("batch", &batch.id.to_string())
            .append_pair("path", req.path.as_str());
        let resp = self
            .client
            .get(&self.url, &url)
            .and_then(check)
            .map_err(|e| self.fail(e));
        let mut resp = match resp {
            Ok(resp) => resp,
            Err(e)
                if crate::remote::api_error(&e)
                    .is_some_and(|api| api.code == "content_not_found") =>
            {
                return Ok(false);
            }
            Err(e) => return Err(e),
        };
        let compression = match resp.headers().get(COMPRESSION_HEADER) {
            Some(v) => parse_compression(v.to_str().unwrap_or_default())?,
            // Servers before the header: what the metadata says
            None => req.compression,
        };

        let tmp = NamedTempFile::new()?.into_temp_path();
        {
            let mut reader = CountingReader::new(resp.body_mut().as_reader(), req.on_bytes);
            let mut out = fs::File::create(&tmp)?;
            std::io::copy(&mut reader, &mut out).context("the download was cut off")?;
        }
        if let Some(parent) = req.target.parent() {
            fs::create_dir_all(parent)?;
        }
        compression.decompress(&tmp, req.target, None)?;
        Ok(true)
    }

    /// `HEAD`: whether the project has the content. Records nothing.
    pub fn exists(&self, hashes: &Hashes) -> Result<bool> {
        let url = self.project_url(&["blobs", hashes.get_blake3()]);
        let resp = self
            .client
            .head(&self.url, &url)
            .map_err(|e| self.fail(e))?;
        match resp.status().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            // A HEAD has no body to say why, so the stored token is kept
            401 => Err(ApiError::new(
                401,
                "unauthenticated",
                "the server rejected the token (HTTP 401); run `dvs login`",
            )
            .into()),
            403 => Err(ApiError::new(
                403,
                "insufficient_role",
                "the server refused the content check (HTTP 403)",
            )
            .into()),
            status => bail!("the server answered {status} to a content check"),
        }
    }

    /// The versions of `path`, newest first: one page of at most `limit` (the server's
    /// default when None), starting at `cursor`.
    pub fn versions(
        &self,
        path: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Page<VersionView>> {
        self.client.server_info(&self.url)?;
        let mut url = self.project_url(&["versions"]);
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("path", path);
            if let Some(limit) = limit {
                q.append_pair("limit", &limit.to_string());
            }
            if let Some(cursor) = cursor {
                q.append_pair("cursor", cursor);
            }
        }
        let resp = self.client.get(&self.url, &url);
        self.json(resp)
    }

    /// `url` with `pairs` as its query string.
    fn with_query(mut url: Url, pairs: &Pairs) -> Url {
        if !pairs.is_empty() {
            let mut q = url.query_pairs_mut();
            for (k, v) in pairs {
                q.append_pair(k, v);
            }
        }
        url
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, url: Url) -> Result<T> {
        self.client.server_info(&self.url)?;
        let resp = self.client.get(&self.url, &url);
        self.json(resp)
    }

    /// The caller's username on the server, for the `.dvs` files it writes.
    pub fn username(&self) -> Option<String> {
        self.me().ok().and_then(|me| me.username)
    }

    /// The projects the caller can read.
    pub fn projects(&self, query: &ProjectsQuery) -> Result<Page<ProjectView>> {
        let url = Self::with_query(Client::v1(&self.url, &["projects"]), &query.pairs());
        self.get_json(url)
    }

    /// Lock or unlock the project, with an optional reason.
    pub fn set_locked(&self, locked: bool, reason: Option<String>) -> Result<ProjectView> {
        self.client.server_info(&self.url)?;
        let body = PatchProject {
            locked: Some(locked),
            lock_reason: reason.filter(|r| !r.trim().is_empty()),
            ..Default::default()
        };
        let resp = self
            .client
            .patch_json(&self.url, &self.project_url(&[]), &body, None);
        self.json(resp)
    }

    /// Arx groups, the ones you belong to marked.
    pub fn groups(&self, query: &GroupsQuery) -> Result<Page<GroupView>> {
        let url = Self::with_query(Client::v1(&self.url, &["groups"]), &query.pairs());
        self.get_json(url)
    }

    /// The project's grants and its version, for `If-Match`.
    pub fn access(&self) -> Result<AccessView> {
        self.get_json(self.project_url(&["access"]))
    }

    /// Give `group_id` `role`, if the project is still at `version`.
    pub fn put_grant(
        &self,
        group_id: uuid::Uuid,
        role: Role,
        confirm_self_demotion: bool,
        version: Option<i64>,
    ) -> Result<AccessView> {
        self.client.server_info(&self.url)?;
        let body = PutGrant {
            role: role.as_str().to_string(),
            confirm_self_demotion,
        };
        let id = group_id.to_string();
        let resp = self.client.put_json(
            &self.url,
            &self.project_url(&["access", &id]),
            &body,
            version,
        );
        self.json(resp)
    }

    /// Remove `group_id`'s grant, if the project is still at `version`.
    pub fn delete_grant(
        &self,
        group_id: uuid::Uuid,
        confirm_self_demotion: bool,
        version: Option<i64>,
    ) -> Result<()> {
        self.client.server_info(&self.url)?;
        let id = group_id.to_string();
        let mut url = self.project_url(&["access", &id]);
        if confirm_self_demotion {
            url.query_pairs_mut()
                .append_pair("confirm_self_demotion", "true");
        }
        let resp = self.client.delete::<()>(&self.url, &url, None, version);
        resp.and_then(check).map_err(|e| self.fail(e))?;
        Ok(())
    }

    /// Every path uploaded to the project, with its latest upload.
    pub fn files(&self, query: &FilesQuery) -> Result<Page<FileView>> {
        self.get_json(Self::with_query(
            self.project_url(&["files"]),
            &query.pairs(),
        ))
    }

    /// What the files matching `query` add up to.
    pub fn files_summary(&self, query: &FilesQuery) -> Result<FilesSummary> {
        let url = Self::with_query(
            self.project_url(&["files", "summary"]),
            &query.summary_pairs(),
        );
        self.get_json(url)
    }

    /// Local files compared with the server, in requests within its limits.
    pub fn remote_status(&self, files: &[StatusFile]) -> Result<Vec<FileStatus>> {
        let info = self.client.server_info(&self.url)?;
        if info.limits.max_status_paths == 0 {
            bail!(
                "{} cannot compare files yet; it needs a newer dvs-server",
                self.url
            );
        }
        let max_paths = info.limits.max_status_paths;
        // Leave room for the envelope
        let max_bytes = info
            .limits
            .max_status_body_bytes
            .saturating_sub(64)
            .max(1024);
        let mut out = Vec::with_capacity(files.len());
        let mut start = 0;
        while start < files.len() {
            let (mut end, mut bytes) = (start, 0);
            while end < files.len() && end - start < max_paths {
                let size = serde_json::to_vec(&files[end]).map_or(max_bytes, |v| v.len()) + 1;
                if end > start && bytes + size > max_bytes {
                    break;
                }
                bytes += size;
                end += 1;
            }
            let body = StatusRequest {
                files: files[start..end].to_vec(),
            };
            let resp = self
                .client
                .post_json(&self.url, &self.project_url(&["status"]), &body);
            let result: StatusResult = self.json(resp)?;
            out.extend(result.files);
            start = end;
        }
        Ok(out)
    }

    /// Events, newest first.
    pub fn activity(&self, query: &ActivityQuery) -> Result<Page<ActivityItem>> {
        let mut query = query.clone();
        query.group_by = None;
        self.get_json(Self::with_query(
            self.project_url(&["activity"]),
            &query.pairs(),
        ))
    }

    /// Runs of `dvs add` and `dvs get`, newest first.
    pub fn activity_batches(&self, query: &ActivityQuery) -> Result<Page<ActivityBatch>> {
        let mut query = query.clone();
        query.group_by = Some("batch".into());
        self.get_json(Self::with_query(
            self.project_url(&["activity"]),
            &query.pairs(),
        ))
    }

    /// Every event matching `query` as CSV, oldest first, written to `out`.
    pub fn activity_csv(&self, query: &ActivityQuery, out: &mut dyn std::io::Write) -> Result<u64> {
        self.client.server_info(&self.url)?;
        let mut query = query.clone();
        query.group_by = None;
        query.limit = None;
        query.cursor = None;
        let mut pairs = query.pairs();
        pairs.push(("format", "csv".to_string()));
        let url = Self::with_query(self.project_url(&["activity"]), &pairs);
        let mut resp = self
            .client
            .get(&self.url, &url)
            .and_then(check)
            .map_err(|e| self.fail(e))?;
        let n = std::io::copy(&mut resp.body_mut().as_reader(), out)
            .context("the export was cut off")?;
        Ok(n)
    }

    /// The yanks in force, newest first.
    pub fn yanks(&self) -> Result<Vec<YankView>> {
        self.get_json(self.project_url(&["yanks"]))
    }

    /// Block downloads of `hash` everywhere in the project.
    pub fn yank(&self, hash: &str, reason: &str) -> Result<YankView> {
        self.client.server_info(&self.url)?;
        let body = YankRequest {
            hash: hash.to_string(),
            reason: reason.to_string(),
        };
        let resp = self
            .client
            .post_json(&self.url, &self.project_url(&["yanks"]), &body);
        self.json(resp)
    }

    /// Allow downloads of `hash` again.
    pub fn unyank(&self, hash: &str, reason: Option<String>) -> Result<YankView> {
        self.client.server_info(&self.url)?;
        let body = UnyankRequest { reason };
        let resp = self.client.delete(
            &self.url,
            &self.project_url(&["yanks", hash]),
            body.reason.is_some().then_some(&body),
            None,
        );
        self.json(resp)
    }

    pub fn get_audit_entries(&self, _files: &[PathBuf]) -> Result<Vec<AuditEntry>> {
        bail!(
            "{} is a server project: its history is on the server, in the web UI at {}. \
             `dvs_log()` will read it from R.",
            self.name,
            self.url
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_is_the_same_however_its_url_is_written() {
        let backend = ServerBackend::new(
            "p".into(),
            None,
            Url::parse("https://dvs.example.com/dvs").unwrap(),
        );
        for same in [
            "https://dvs.example.com/dvs/",
            "https://DVS.Example.com:443/dvs",
            "https://dvs.example.com",
        ] {
            assert!(backend.same_server(&Url::parse(same).unwrap()), "{same}");
        }
        for other in [
            "http://dvs.example.com/dvs",
            "https://dvs.example.com:8443/dvs",
        ] {
            assert!(!backend.same_server(&Url::parse(other).unwrap()), "{other}");
        }
    }
}
