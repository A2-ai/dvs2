//! Calls to dvs-server that move a project's history: R18's timeouts and retries, shared by
//! `dvs-migrate` and `dvs migrate server --import-history` (C4).

use std::io::Read;
use std::time::Duration;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;
use ureq::Body;
use ureq::http::Response;
use url::Url;
use uuid::Uuid;

use crate::backends::server::{invalid_response, join_url, status_error, unreachable};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Waiting for a response, and reading it. Sending has no limit: a large blob streams.
const READ_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The waits before each retry of a call that failed in transit or with a 5xx (R18)
const BACKOFF_SECS: [u64; 5] = [1, 2, 4, 8, 16];
/// How many times a call is retried before it gives up
pub const RETRIES: usize = BACKOFF_SECS.len();

/// dvs2's TLS and user agent, with timeouts for long runs.
/// `crate::http_agent` is shared by all of dvs, so its timeouts stay as they are.
pub fn agent() -> ureq::Agent {
    crate::http_agent_config()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_response(Some(READ_TIMEOUT))
        .timeout_recv_body(Some(READ_TIMEOUT))
        .build()
        .new_agent()
}

/// The context of a call whose retries ran out
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
    /// Failed in transit, timed out, or got a 5xx: safe to send again
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

/// Runs a call until it is done or its retries have run out, waiting between tries
#[derive(Debug, Clone)]
pub struct Retrying {
    backoff: Vec<Duration>,
}

impl Default for Retrying {
    /// R18's waits
    fn default() -> Self {
        Self {
            backoff: BACKOFF_SECS.map(Duration::from_secs).to_vec(),
        }
    }
}

impl Retrying {
    /// R18's retries with no wait between them, so tests need not sleep
    pub fn without_waits() -> Self {
        Self {
            backoff: vec![Duration::ZERO; RETRIES],
        }
    }

    /// `once` until it is done, or its retries have run out: then the last try's error,
    /// with `GaveUp` as its context
    fn run<T>(&self, call: &str, mut once: impl FnMut() -> Result<Try<T>>) -> Result<T> {
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
}

/// A response's status and its whole body
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or_default().trim()
    }

    /// A body that arrived but does not parse is not tried again
    pub fn json<R: DeserializeOwned>(&self, url: &Url) -> Result<R> {
        serde_json::from_slice(&self.body).map_err(|e| invalid_response(url, e))
    }
}

/// One try's answer from the server at `url`, its body read in full: a 5xx, or a failure in
/// transit while sending or reading, is another try. Any other status is the caller's to word.
fn answer(url: &Url, call: &str, sent: Result<Response<Body>, ureq::Error>) -> Result<Try<Answer>> {
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
        Err(e) if in_transit(&e) => Try::Again(unreachable(url, e)),
        Err(e) => return Err(unreachable(url, e)),
        Ok(answer) if answer.status >= 500 => {
            Try::Again(status_error(url, answer.status, answer.text(), call))
        }
        Ok(answer) => Try::Done(answer),
    })
}

/// How the server took a blob a run sent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    /// 201 if the server stored it now, 200 if it had it already
    Uploaded { created: bool },
    /// 400: the bytes do not hash to the claimed hash
    Corrupt(String),
    /// Any other refusal of the bytes themselves, as 413 over the size limit
    Refused(u16, String),
    /// The local copy could not be read
    Unreadable(String),
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

/// One project's migration endpoints, called with a bearer token: a migrate key, or a
/// server admin's own login (log import A1). Every call is retried as R18 says.
pub struct Endpoints {
    url: Url,
    project: String,
    token: String,
    agent: ureq::Agent,
    retrying: Retrying,
}

impl Endpoints {
    pub fn new(url: Url, project: String, token: String, retrying: Retrying) -> Self {
        Self {
            url,
            project,
            token,
            agent: agent(),
            retrying,
        }
    }

    /// The server's URL
    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn project(&self) -> &str {
        &self.project
    }

    fn endpoint(&self, call: &str) -> Result<Url> {
        join_url(&self.url, &format!("api/{}/migration/{call}", self.project))
    }

    /// The answer to a JSON call. Any status but a 5xx is the caller's to word.
    pub fn post(&self, call: &str, body: &Value) -> Result<Answer> {
        let url = self.endpoint(call)?;
        self.retrying.run(call, || {
            let sent = self
                .agent
                .post(url.as_str())
                .header("authorization", format!("Bearer {}", self.token))
                .send_json(body);
            answer(&self.url, call, sent)
        })
    }

    /// How the server took a blob, or that the local copy could not be read, and the bytes
    /// the server took. `open` gives the uncompressed bytes, again for each try; `refused`
    /// words a status the upload does not take.
    pub fn upload(
        &self,
        run_id: Uuid,
        hash: &str,
        open: &dyn Fn() -> Result<Box<dyn Read + Send>>,
        refused: impl Fn(&Answer) -> anyhow::Error,
    ) -> Result<(After, u64)> {
        let mut url = self.endpoint(&format!("blobs/{hash}"))?;
        url.query_pairs_mut()
            .append_pair("run_id", &run_id.to_string());
        let answer = self.retrying.run("upload", || {
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
                .header("authorization", format!("Bearer {}", self.token))
                .header("content-type", "application/octet-stream")
                .send(ureq::SendBody::from_reader(&mut bytes));
            if let (Err(_), Some(local)) = (&sent, bytes.error) {
                return Ok(Try::Done(Err(After::Unreadable(local))));
            }
            let read = bytes.read;
            Ok(match answer(&self.url, "upload", sent)? {
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
            400 if answer.text().starts_with("content hash mismatch") => {
                After::Corrupt(answer.text().to_string())
            }
            413 => After::Refused(413, answer.text().to_string()),
            _ => return Err(refused(&answer)),
        };
        let sent = if matches!(after, After::Uploaded { .. }) {
            sent
        } else {
            0
        };
        Ok((after, sent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A server that answers one request per connection with each of `statuses` in turn;
    /// status 0 closes the connection unanswered. Returns its URL.
    fn serve(statuses: Vec<u16>) -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for status in statuses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                reader.read_exact(&mut vec![0; length]).unwrap();
                if status != 0 {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok"
                    );
                }
            }
        });
        url.parse().unwrap()
    }

    /// Posts to `url` through `retrying`, counting the tries
    fn call(url: &Url, retrying: &Retrying) -> (Result<Answer>, usize) {
        let agent = agent();
        let mut tries = 0;
        let answer = retrying.run("lookup", || {
            tries += 1;
            answer(url, "lookup", agent.post(url.as_str()).send("{}"))
        });
        (answer, tries)
    }

    #[test]
    fn the_agent_and_backoff_are_r18s() {
        let timeouts = agent().config().timeouts();
        assert_eq!(timeouts.connect, Some(Duration::from_secs(30)));
        assert_eq!(timeouts.recv_response, Some(Duration::from_secs(300)));
        assert_eq!(timeouts.recv_body, Some(Duration::from_secs(300)));
        assert_eq!(
            Retrying::default().backoff,
            [1, 2, 4, 8, 16].map(Duration::from_secs)
        );
    }

    #[test]
    fn a_dropped_connection_or_5xx_is_retried_until_it_succeeds() {
        let url = serve(vec![0, 503, 0, 502, 200]);
        let (answer, tries) = call(&url, &Retrying::without_waits());
        let answer = answer.unwrap();
        assert_eq!((answer.status, answer.text(), tries), (200, "ok", 5));
    }

    #[test]
    fn retries_run_out_after_five() {
        let url = serve(vec![500; RETRIES + 1]);
        let (answer, tries) = call(&url, &Retrying::without_waits());
        let error = answer.unwrap_err();
        assert_eq!(error.downcast_ref::<GaveUp>().unwrap().tries, 6);
        assert!(format!("{error:#}").contains("(500)"), "{error:#}");
        assert_eq!(tries, 6);
    }

    #[test]
    fn a_4xx_is_not_retried() {
        let url = serve(vec![404, 200]);
        let (answer, tries) = call(&url, &Retrying::without_waits());
        assert_eq!((answer.unwrap().status, tries), (404, 1));
    }
}
