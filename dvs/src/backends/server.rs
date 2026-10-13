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
    BatchKind, BatchView, CompleteBatch, CreateProject, GitContext, GrantRequest, Manifest,
    ManifestEntry, ManifestResult, MeView, OpenBatch, ProjectView, ServerInfo, StoredBlob,
};
use crate::audit::AuditEntry;
use crate::auth::delete_token;
use crate::backends::{
    Batch, ManifestItem, ManifestLimits, ManifestReply, PresentSize, RetrieveRequest, Stored,
};
use crate::remote::{ApiError, Client, NoResponse, Response, TokenSource, check, read_json};
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
        if n > 0
            && let Some(cb) = self.on_bytes
        {
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
        if matches!(api.code.as_str(), "unauthenticated" | "token_expired")
            && self.token_source() == Some(TokenSource::Stored)
        {
            if let Err(e) = delete_token(&self.url) {
                log::warn!("Could not remove the rejected token: {e}");
            }
            let mut api = api.clone();
            api.message = format!("{}; run `dvs login`", api.message.trim_end_matches('.'));
            return api.into();
        }
        e
    }

    fn json<T: serde::de::DeserializeOwned>(&self, resp: Result<Response>) -> Result<T> {
        resp.and_then(read_json).map_err(|e| self.fail(e))
    }

    /// Create the project, with `group` as its admin.
    pub fn init(&self, compression: Compression) -> Result<bool> {
        let Some(group) = self.group.clone() else {
            bail!("A group is required to create a project on the server");
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
        let resp = self.client.post_json(
            &self.url,
            &self.project_url(&["batches", &id, "manifest"]),
            &body,
        );
        let result: ManifestResult = self.json(resp)?;
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

    /// Send one content's bytes. A request that gets no response at all is sent once
    /// more: a lost response is safe to retry, because the server checks and discards
    /// bytes it already has.
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
        let resp = loop {
            attempt += 1;
            let mut body = CountingReader::new(fs::File::open(source)?, on_bytes);
            match self.client.put_reader(&self.url, &url, &mut body) {
                Err(e) if e.downcast_ref::<NoResponse>().is_some() && attempt < 2 => {
                    log::warn!(
                        "No response to the upload of {}, retrying: {e}",
                        source.display()
                    );
                }
                Err(e) if e.downcast_ref::<NoResponse>().is_some() => {
                    return Err(e.context(
                        "no response after a retry: the bytes may have been uploaded; run `dvs add` again",
                    ));
                }
                other => break other,
            }
        };
        let stored: StoredBlob = self.json(resp)?;
        Ok(Stored {
            stored_size: stored.stored_size,
            compression: parse_compression(&stored.compression)?,
        })
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
            401 => Err(self.fail(
                ApiError::new(401, "unauthenticated", "the server rejected your token").into(),
            )),
            status => bail!("the server answered {status} to a content check"),
        }
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
