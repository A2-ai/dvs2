//! dvs-server's migration endpoints, called with a migrate key.

use std::io::Read;
use std::time::Duration;

use anyhow::Result;
use dvs::ErrorCode;
use dvs::backends::server::{coded, invalid_response, join_url, status_error, unreachable};
use dvs::migration::{
    LookupRequest, LookupResponse, ManifestEntry, ManifestRequest, ManifestResponse,
    StartRunRequest, StartRunResponse, Totals, WalkKind,
};
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

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Waiting for a response, and reading it. Sending has no limit: a large blob streams.
const READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The waits before each retry of a call that failed in transit or with a 5xx (R18)
const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 16];
/// How many times a call is retried before it gives up
pub const RETRIES: usize = BACKOFF_SECS.len();

pub struct Client {
    url: Url,
    project: String,
    key: String,
    agent: ureq::Agent,
    backoff: Vec<Duration>,
}

/// A refusal the admin answers with a flag, not a failure: exit status 2
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// The context of a call whose retries ran out: the run stays open (R35)
#[derive(Debug)]
pub struct GaveUp {
    tries: usize,
}

impl std::fmt::Display for GaveUp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gave up after {} tries", self.tries)
    }
}

/// What one try of a call came to
enum Try<T> {
    Done(T),
    /// Failed in transit, timed out, or got a 5xx: safe to send again (D10, I6)
    Again(anyhow::Error),
}

/// Whether a ureq error is the network's, which another try may not meet
fn in_transit(e: &ureq::Error) -> bool {
    matches!(
        e,
        ureq::Error::Io(_)
            | ureq::Error::Timeout(_)
            | ureq::Error::ConnectionFailed
            | ureq::Error::HostNotFound
            | ureq::Error::Protocol(_)
    )
}

/// dvs-migrate's own agent: dvs2's TLS and user agent, with timeouts for long runs.
/// `dvs::http_agent` is shared by all of dvs, so its timeouts stay as they are.
fn agent() -> ureq::Agent {
    dvs::http_agent_config()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(READ_TIMEOUT))
        .timeout_recv_body(Some(READ_TIMEOUT))
        .build()
        .new_agent()
}

/// Keeps the reader's first error: once ureq has failed, that is the only way to tell a
/// local read error from a network one.
struct Reading<R> {
    inner: R,
    error: Option<String>,
    /// Bytes read so far: what this try sent
    read: u64,
}

impl<R: Read> Read for Reading<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf).inspect_err(|e| {
            if self.error.is_none() && e.kind() != std::io::ErrorKind::Interrupted {
                self.error = Some(e.to_string());
            }
        })?;
        self.read += n as u64;
        Ok(n)
    }
}

impl Client {
    pub fn new(url: Url, project: String, key: String) -> Self {
        Self {
            url,
            project,
            key,
            agent: agent(),
            backoff: BACKOFF_SECS.map(Duration::from_secs).to_vec(),
        }
    }

    /// `once` until it is done, or its retries have run out
    fn retrying<T>(&self, call: &str, mut once: impl FnMut() -> Result<Try<T>>) -> Result<T> {
        let mut waits = self.backoff.iter();
        let mut tries = 0;
        loop {
            tries += 1;
            match once()? {
                Try::Done(done) => return Ok(done),
                Try::Again(error) => match waits.next() {
                    Some(wait) => {
                        eprintln!(
                            "warning: {call} failed: {error:#}; trying again in {} s",
                            wait.as_secs()
                        );
                        std::thread::sleep(*wait);
                    }
                    None => return Err(error.context(GaveUp { tries })),
                },
            }
        }
    }

    fn endpoint(&self, call: &str) -> Result<Url> {
        join_url(&self.url, &format!("api/{}/migration/{call}", self.project))
    }

    /// A migrate key's refusals, which dvs2's helpers would word for a login
    fn refused(&self, answer: &Answer, what: &str) -> anyhow::Error {
        let status = answer.status;
        let body = answer.text();
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

    /// One try's answer, its body read in full: a 5xx, or a failure in transit while
    /// sending or reading, is another try (R18)
    fn answer(&self, call: &str, sent: Result<Response<Body>, ureq::Error>) -> Result<Try<Answer>> {
        let read = sent.and_then(|mut resp| {
            let body = resp
                .body_mut()
                .with_config()
                .limit(u64::MAX)
                .read_to_vec()?;
            Ok(Answer {
                status: resp.status().as_u16(),
                body,
            })
        });
        Ok(match read {
            Err(e) if in_transit(&e) => Try::Again(unreachable(&self.url, e)),
            Err(e) => return Err(unreachable(&self.url, e)),
            Ok(answer) if answer.status >= 500 => Try::Again(self.refused(&answer, call)),
            Ok(answer) => Try::Done(answer),
        })
    }

    /// The answer to a JSON call, retried as R18 says. Any status but a 5xx is returned
    /// for the caller to word.
    fn send(&self, call: &str, body: &impl Serialize) -> Result<Answer> {
        let mut body = serde_json::to_value(body)?;
        strip_nul(&mut body);
        let url = self.endpoint(call)?;
        self.retrying(call, || {
            let sent = self
                .agent
                .post(url.as_str())
                .header("authorization", format!("Bearer {}", self.key))
                .send_json(&body);
            self.answer(call, sent)
        })
    }

    fn post<R: DeserializeOwned>(&self, call: &str, body: &impl Serialize, ok: u16) -> Result<R> {
        let answer = self.send(call, body)?;
        if answer.status != ok {
            return Err(self.refused(&answer, call));
        }
        answer.json(&self.url)
    }

    pub fn lookup(&self, req: &LookupRequest) -> Result<LookupResponse> {
        self.post("lookup", req, 200)
    }

    /// Opens a new run; other runs in progress never stop it (R35)
    pub fn start_run(
        &self,
        walk: WalkKind,
        totals: Totals,
        other_source: bool,
    ) -> Result<StartRunResponse> {
        let req = StartRunRequest {
            client_version: format!("dvs-migrate {}", env!("CARGO_PKG_VERSION")),
            walk,
            totals,
            other_source,
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

    /// How the server took a blob, or that the local copy could not be read, and the bytes
    /// the server took. `open` gives the uncompressed bytes, again for each try.
    pub fn upload(
        &self,
        run: Uuid,
        hash: &str,
        open: &dyn Fn() -> Result<Box<dyn Read + Send>>,
    ) -> Result<(After, u64)> {
        let mut url = self.endpoint(&format!("blobs/{hash}"))?;
        url.query_pairs_mut()
            .append_pair("run_id", &run.to_string());
        let answer = self.retrying("upload", || {
            let mut bytes = match open() {
                Ok(inner) => Reading {
                    inner,
                    error: None,
                    read: 0,
                },
                Err(e) => return Ok(Try::Done(Err(After::Unreadable(format!("{e:#}"))))),
            };
            let sent = self
                .agent
                .put(url.as_str())
                .header("authorization", format!("Bearer {}", self.key))
                .header("content-type", "application/octet-stream")
                .send(ureq::SendBody::from_reader(&mut bytes));
            if let (Err(_), Some(local)) = (&sent, bytes.error) {
                return Ok(Try::Done(Err(After::Unreadable(local))));
            }
            let read = bytes.read;
            Ok(match self.answer("upload", sent)? {
                Try::Done(answer) => Try::Done(Ok((answer, read))),
                Try::Again(e) => Try::Again(e),
            })
        })?;
        let (answer, sent) = match answer {
            Ok(answer) => answer,
            Err(unreadable) => return Ok((unreadable, 0)),
        };
        let after = match answer.status {
            200 | 201 => After::Uploaded {
                created: answer.status == 201,
            },
            400 => {
                let body = answer.text();
                if body.starts_with("content hash mismatch") {
                    After::Corrupt(body.to_string())
                } else {
                    return Err(coded(
                        ErrorCode::OperationFailed,
                        format!("upload refused (400): {body}"),
                    ));
                }
            }
            413 => After::Refused(413, answer.text().to_string()),
            _ => return Err(self.refused(&answer, "upload")),
        };
        let sent = if matches!(after, After::Uploaded { .. }) {
            sent
        } else {
            0
        };
        Ok((after, sent))
    }

    /// A 200 is stored, also when a retry finds the run already reported (R33)
    pub fn report(&self, report: Value) -> Result<Value> {
        self.post("report", &report, 200)
    }
}

/// A response's status and its whole body
struct Answer {
    status: u16,
    body: Vec<u8>,
}

impl Answer {
    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or_default().trim()
    }

    /// A body that arrived but does not parse is not tried again
    fn json<R: DeserializeOwned>(&self, url: &Url) -> Result<R> {
        serde_json::from_slice(&self.body).map_err(|e| invalid_response(url, e))
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
pub(crate) mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A body `serve` cuts short: it promises more than it sends, then closes
    const CUT_SHORT: &str = "\0cut short";

    /// A server that answers one request per connection with each of `responses` in turn,
    /// then stops listening; status 0 closes the connection unanswered. Returns its URL
    /// and the requests' first line and body.
    pub(crate) fn serve(responses: Vec<(u16, &'static str)>) -> (Url, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut first, mut length, mut chunked) = (String::new(), 0, false);
                reader.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let lower = line.to_ascii_lowercase();
                    if let Some(n) = lower.strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    chunked |= lower.starts_with("transfer-encoding: chunked");
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut request = Vec::new();
                if chunked {
                    while !request.ends_with(b"0\r\n\r\n") {
                        reader.read_until(b'\n', &mut request).unwrap();
                    }
                } else {
                    request.resize(length, 0);
                    reader.read_exact(&mut request).unwrap();
                }
                log.lock().unwrap().push(format!(
                    "{}\n{}",
                    first.trim(),
                    String::from_utf8_lossy(&request)
                ));
                if status == 0 {
                    continue;
                }
                let (body, promised) = match body.strip_prefix(CUT_SHORT) {
                    Some(body) => (body, body.len() + 100),
                    None => (body, body.len()),
                };
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {promised}\r\nconnection: close\r\n\r\n{body}"
                );
            }
        });
        (url.parse().unwrap(), seen)
    }

    /// The JSON body of a request `serve` saw
    fn body(request: &str) -> Value {
        serde_json::from_str(request.split_once('\n').unwrap().1).unwrap()
    }

    pub(crate) fn client(url: Url) -> Client {
        let mut client = Client::new(url, "proj".into(), "dvsm_key".into());
        client.backoff = vec![Duration::ZERO; BACKOFF_SECS.len()];
        client
    }

    const LOOKUP_OK: &str = r#"{"have":[],"compression":"zstd","known_commits":2}"#;

    /// An error as a call whose retries ran out returns it
    pub(crate) fn gave_up() -> anyhow::Error {
        let (url, _) = serve(vec![(500, "down"); RETRIES + 1]);
        client(url).lookup(&LookupRequest::default()).unwrap_err()
    }

    #[test]
    fn the_backoff_doubles_from_one_second_five_times() {
        assert_eq!(BACKOFF_SECS, [1, 2, 4, 8, 16]);
        let client = Client::new("http://x/".parse().unwrap(), "p".into(), "k".into());
        assert_eq!(client.backoff, BACKOFF_SECS.map(Duration::from_secs));
    }

    #[test]
    fn a_body_cut_short_is_retried_and_one_that_does_not_parse_is_not() {
        let cut = format!("{CUT_SHORT}{}", &LOOKUP_OK[..10]).leak();
        let (url, seen) = serve(vec![(200, cut), (200, LOOKUP_OK)]);
        let found = client(url).lookup(&LookupRequest::default()).unwrap();
        assert_eq!(found.known_commits, 2);
        assert_eq!(seen.lock().unwrap().len(), 2);

        let (url, seen) = serve(vec![(200, "{not json"), (200, LOOKUP_OK)]);
        let error = client(url).lookup(&LookupRequest::default()).unwrap_err();
        assert!(
            format!("{error:#}").contains("Invalid response"),
            "{error:#}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_5xx_is_retried_until_it_succeeds() {
        let (url, seen) = serve(vec![(503, "busy"), (502, "busy"), (200, LOOKUP_OK)]);
        let req = LookupRequest {
            commits: vec!["c1".into(), "c2".into()],
            ..LookupRequest::default()
        };
        let found = client(url).lookup(&req).unwrap();
        assert_eq!(found.known_commits, 2);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(body(&seen[2])["commits"], serde_json::json!(["c1", "c2"]));
    }

    #[test]
    fn retries_run_out_after_five() {
        let (url, seen) = serve(vec![(500, "down"); 6]);
        let error = client(url).lookup(&LookupRequest::default()).unwrap_err();
        assert!(error.downcast_ref::<GaveUp>().is_some(), "{error:#}");
        assert_eq!(seen.lock().unwrap().len(), 6);
    }

    #[test]
    fn a_dropped_connection_is_retried_and_a_4xx_is_not() {
        let (url, seen) = serve(vec![(0, ""), (200, LOOKUP_OK)]);
        client(url).lookup(&LookupRequest::default()).unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);

        let (url, seen) = serve(vec![(400, "bad"), (200, LOOKUP_OK)]);
        assert!(client(url).lookup(&LookupRequest::default()).is_err());
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn an_upload_is_read_again_for_each_try() {
        let (url, seen) = serve(vec![(503, "busy"), (201, r#"{"stored_size":5}"#)]);
        let opened = std::cell::Cell::new(0);
        let open = || -> Result<Box<dyn Read + Send>> {
            opened.set(opened.get() + 1);
            Ok(Box::new(&b"hello"[..]))
        };
        let after = client(url)
            .upload(Uuid::nil(), &"ab".repeat(32), &open)
            .unwrap();
        assert_eq!(after, (After::Uploaded { created: true }, 5));
        assert_eq!(opened.get(), 2);
        assert!(seen.lock().unwrap()[1].starts_with("PUT /api/proj/migration/blobs/"));
    }

    #[test]
    fn start_run_sends_other_source_and_expects_201() {
        let started = r#"{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"t",
            "compression":"none","in_progress":[]}"#;
        let (url, seen) = serve(vec![(201, started)]);
        let run = client(url)
            .start_run(WalkKind::Full, Totals::default(), true)
            .unwrap();
        assert!(run.in_progress.is_empty());
        assert_eq!(body(&seen.lock().unwrap()[0])["other_source"], true);

        let (url, _) = serve(vec![(200, started)]);
        let error = client(url)
            .start_run(WalkKind::Full, Totals::default(), false)
            .unwrap_err();
        assert!(
            format!("{error:#}").starts_with("runs failed (200)"),
            "{error:#}"
        );
    }

    #[test]
    fn a_report_answered_200_is_stored_also_on_a_retry() {
        let stored =
            r#"{"run_id":"00000000-0000-0000-0000-000000000000","entries":4000,"problems":12}"#;
        let (url, seen) = serve(vec![(0, ""), (200, stored)]);
        let counts = client(url)
            .report(serde_json::json!({"command": "run"}))
            .unwrap();
        assert_eq!(counts["entries"], 4000);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

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
