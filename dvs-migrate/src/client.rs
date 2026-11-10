//! dvs-server's migration endpoints, called with a migrate key.

use std::io::Read;

use anyhow::Result;
use dvs::ErrorCode;
use dvs::backends::server::{coded, status_error};
use dvs::migration::{
    LookupRequest, LookupResponse, ManifestEntry, ManifestRequest, ManifestResponse,
    StartRunRequest, StartRunResponse, Totals, WalkKind,
};
use dvs::retry::{After, Answer, Endpoints, Retrying};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

/// How start run's 409 begins on a project whose history was imported from its audit log
const IMPORTED_FROM_LOG: &str = "project's history was imported from its audit log";

pub struct Client {
    endpoints: Endpoints,
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

impl Client {
    pub fn new(url: Url, project: String, key: String) -> Self {
        Self {
            endpoints: Endpoints::new(url, project, key, Retrying::default()),
        }
    }

    /// A migrate key's refusals, which dvs2's helpers would word for a login
    fn refused(&self, answer: &Answer, what: &str) -> anyhow::Error {
        let url = self.endpoints.url();
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
                    "the dvs server at {url} has no project {}",
                    self.endpoints.project()
                ),
            ),
            404 if body.starts_with("unknown migration run") => {
                coded(ErrorCode::OperationFailed, message)
            }
            // A3: the history came in another way, which --other-source answers
            409 if body.starts_with(IMPORTED_FROM_LOG) => Refused(body.to_string()).into(),
            400 | 409 | 413 | 423 => coded(ErrorCode::OperationFailed, message),
            _ => status_error(url, status, body, what),
        }
    }

    /// The answer to a JSON call, retried as R18 says. Any status but a 5xx is returned
    /// for the caller to word.
    fn send(&self, call: &str, body: &impl Serialize) -> Result<Answer> {
        let mut body = serde_json::to_value(body)?;
        strip_nul(&mut body);
        self.endpoints.post(call, &body)
    }

    fn post<R: DeserializeOwned>(&self, call: &str, body: &impl Serialize, ok: u16) -> Result<R> {
        let answer = self.send(call, body)?;
        if answer.status != ok {
            return Err(self.refused(&answer, call));
        }
        answer.json(self.endpoints.url())
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
        self.endpoints
            .upload(run, hash, open, |answer| self.refused(answer, "upload"))
    }

    /// A 200 is stored, also when a retry finds the run already reported (R33)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use dvs::retry::RETRIES;

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
        let no_wait = Retrying::without_waits();
        Client {
            endpoints: Endpoints::new(url, "proj".into(), "dvsm_key".into(), no_wait),
        }
    }

    const LOOKUP_OK: &str = r#"{"have":[],"compression":"zstd","known_commits":2}"#;

    /// An error as a call whose retries ran out returns it
    pub(crate) fn gave_up() -> anyhow::Error {
        let (url, _) = serve(vec![(500, "down"); RETRIES + 1]);
        client(url).lookup(&LookupRequest::default()).unwrap_err()
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
    fn start_run_on_a_project_imported_from_its_log_is_a_refusal() {
        let imported = "project's history was imported from its audit log by alice on \
                        2026-11-10; pass --other-source to run anyway";
        let (url, _) = serve(vec![(409, imported)]);
        let error = client(url)
            .start_run(WalkKind::Full, Totals::default(), false)
            .unwrap_err();
        assert_eq!(error.downcast_ref::<Refused>().unwrap().0, imported);

        // Any other 409 is still a failure
        let (url, _) = serve(vec![(409, "run already reported")]);
        let error = client(url)
            .start_run(WalkKind::Full, Totals::default(), false)
            .unwrap_err();
        assert!(error.downcast_ref::<Refused>().is_none(), "{error:#}");
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
    fn strip_nul_reaches_every_string() {
        let mut v = serde_json::json!({"m": "a\0b", "l": [{"u": "\0"}], "n": 1});
        strip_nul(&mut v);
        assert_eq!(v, serde_json::json!({"m": "ab", "l": [{"u": ""}], "n": 1}));
    }
}
