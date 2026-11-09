//! dvs-server's migration endpoints, called with a migrate key.

use std::io::Read;

use anyhow::Result;
use dvs::backends::server::{coded, invalid_response, join_url, status_error, unreachable};
use dvs::migration::{
    LookupRequest, LookupResponse, ManifestEntry, ManifestRequest, ManifestResponse,
    StartRunRequest, StartRunResponse, Totals, WalkKind,
};
use dvs::{ErrorCode, http_agent};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use ureq::Body;
use ureq::http::Response;
use url::Url;
use uuid::Uuid;

use crate::commands::After;

/// The server's limit on list lengths per call
pub const MAX_ENTRIES: usize = 10_000;
/// Under the server's 64 MiB JSON body limit, with room for the envelope
pub const MAX_BODY: usize = 48 * 1024 * 1024;

pub struct Client {
    url: Url,
    project: String,
    key: String,
}

/// Keeps the reader's first error: once ureq has failed, that is the only way to tell a
/// local read error from a network one.
struct Reading<R> {
    inner: R,
    error: Option<String>,
}

impl<R: Read> Read for Reading<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf).inspect_err(|e| {
            if self.error.is_none() && e.kind() != std::io::ErrorKind::Interrupted {
                self.error = Some(e.to_string());
            }
        })
    }
}

impl Client {
    pub fn new(url: Url, project: String, key: String) -> Self {
        Self { url, project, key }
    }

    fn endpoint(&self, call: &str) -> Result<Url> {
        join_url(&self.url, &format!("api/{}/migration/{call}", self.project))
    }

    /// A migrate key's refusals, which dvs2's helpers would word for a login
    fn refused(&self, resp: &mut Response<Body>, what: &str) -> anyhow::Error {
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let body = body.trim();
        let message = format!("{what} refused ({status}): {body}");
        match status {
            401 => coded(
                ErrorCode::AuthExpired,
                format!("{message}; issue a new key on the project's Migration page"),
            ),
            403 => coded(ErrorCode::AccessDenied, message),
            404 if body.starts_with("unknown project") => coded(
                ErrorCode::ProjectNotFound,
                format!(
                    "the dvs server at {} has no project {}",
                    self.url, self.project
                ),
            ),
            404 if body.starts_with("unknown migration run") => {
                coded(ErrorCode::OperationFailed, message)
            }
            400 | 409 | 413 | 423 => coded(ErrorCode::OperationFailed, message),
            _ => status_error(&self.url, status, body, what),
        }
    }

    fn post<R: DeserializeOwned>(&self, call: &str, body: &impl Serialize, ok: u16) -> Result<R> {
        let mut body = serde_json::to_value(body)?;
        strip_nul(&mut body);
        let mut resp = http_agent()
            .post(self.endpoint(call)?.as_str())
            .header("authorization", format!("Bearer {}", self.key))
            .send_json(&body)
            .map_err(|e| unreachable(&self.url, e))?;
        if resp.status().as_u16() != ok {
            return Err(self.refused(&mut resp, call));
        }
        resp.body_mut()
            .with_config()
            .limit(u64::MAX)
            .read_json()
            .map_err(|e| invalid_response(&self.url, e))
    }

    pub fn lookup(&self, req: &LookupRequest) -> Result<LookupResponse> {
        self.post("lookup", req, 200)
    }

    pub fn start_run(&self, walk: WalkKind, totals: Totals) -> Result<StartRunResponse> {
        let req = StartRunRequest {
            client_version: format!("dvs-migrate {}", env!("CARGO_PKG_VERSION")),
            walk,
            totals,
        };
        self.post("runs", &req, 201)
    }

    pub fn manifest(&self, run_id: Uuid, entries: &[ManifestEntry]) -> Result<ManifestResponse> {
        let req = ManifestRequest {
            run_id,
            entries: entries.to_vec(),
        };
        self.post("manifest", &req, 200)
    }

    /// How the server took a blob, or that the local copy could not be read
    pub fn upload(&self, run: Uuid, hash: &str, bytes: impl Read) -> Result<After> {
        let mut url = self.endpoint(&format!("blobs/{hash}"))?;
        url.query_pairs_mut()
            .append_pair("run_id", &run.to_string());
        let mut bytes = Reading {
            inner: bytes,
            error: None,
        };
        let sent = http_agent()
            .put(url.as_str())
            .header("authorization", format!("Bearer {}", self.key))
            .header("content-type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(&mut bytes));
        let mut resp = match (sent, bytes.error) {
            (Err(_), Some(local)) => return Ok(After::Unreadable(local)),
            (sent, _) => sent.map_err(|e| unreachable(&self.url, e))?,
        };
        let status = resp.status().as_u16();
        let mut body = || resp.body_mut().read_to_string().unwrap_or_default();
        match status {
            200 | 201 => Ok(After::Uploaded {
                created: status == 201,
            }),
            400 => {
                let body = body();
                if body.starts_with("content hash mismatch") {
                    Ok(After::Corrupt(body))
                } else {
                    Err(coded(
                        ErrorCode::OperationFailed,
                        format!("upload refused (400): {body}"),
                    ))
                }
            }
            413 => Ok(After::Refused(413, body())),
            _ => Err(self.refused(&mut resp, "upload")),
        }
    }

    pub fn report(&self, report: Value) -> Result<Value> {
        self.post("report", &report, 200)
    }
}

/// The server refuses NUL in text, and no recorded value needs one
fn strip_nul(value: &mut Value) {
    match value {
        Value::String(s) if s.contains('\0') => s.retain(|c| c != '\0'),
        Value::Array(items) => items.iter_mut().for_each(strip_nul),
        Value::Object(map) => map.values_mut().for_each(strip_nul),
        _ => {}
    }
}

/// Splits `items` so each call stays under the server's list and body limits.
pub fn chunks<T: Serialize>(items: &[T]) -> Vec<&[T]> {
    let mut chunks = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (i, item) in items.iter().enumerate() {
        let size = serde_json::to_vec(item).map_or(0, |v| v.len()) + 1;
        if i > start && (i - start == MAX_ENTRIES || bytes + size > MAX_BODY) {
            chunks.push(&items[start..i]);
            (start, bytes) = (i, 0);
        }
        bytes += size;
    }
    if start < items.len() {
        chunks.push(&items[start..]);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_stay_under_the_entry_limit() {
        let items: Vec<u8> = vec![0; MAX_ENTRIES * 2 + 1];
        let sizes: Vec<usize> = chunks(&items).iter().map(|c| c.len()).collect();
        assert_eq!(sizes, vec![MAX_ENTRIES, MAX_ENTRIES, 1]);
        assert!(chunks::<u8>(&[]).is_empty());
    }

    #[test]
    fn strip_nul_reaches_every_string() {
        let mut v = serde_json::json!({"m": "a\0b", "l": [{"u": "\0"}], "n": 1});
        strip_nul(&mut v);
        assert_eq!(v, serde_json::json!({"m": "ab", "l": [{"u": ""}], "n": 1}));
    }
}
