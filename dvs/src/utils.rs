use anyhow::{Result, bail};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const DEFAULT_THREADS_PER_CPU: usize = 4;
/// Maximum thread count threshold when `DVS_NUM_THREADS` is unset
const DEFAULT_MAX_THREADS: usize = 16;
/// Maximum thread count threshold if `DVS_NUM_THREADS` is set
const ENV_MAX_THREADS: usize = 32;

/// Global thread count override. 0 = unset (use environment variable or default).
static NUM_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Set the number of threads for DVS parallel operations.
/// Pass 0 to clear (revert to environment variable / automatic detection).
pub fn set_num_threads(n: usize) {
    NUM_THREADS.store(n, Ordering::Relaxed);
}

/// Returns the configured thread count, or `None` if unset.
pub(crate) fn get_num_threads() -> Option<usize> {
    match NUM_THREADS.load(Ordering::Relaxed) {
        0 => None,
        n => Some(n),
    }
}

/// Which tier of the priority chain produced the thread count.
#[derive(Debug, Clone, Copy)]
enum ThreadSource {
    Override,
    Environment,
    Default,
}

impl std::fmt::Display for ThreadSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Override => "override",
            Self::Environment => "environment",
            Self::Default => "default",
        })
    }
}

const KB: u64 = 1_024;
const MB: u64 = 1_024 * 1_024;
const GB: u64 = 1_024 * 1_024 * 1_024;
const TB: u64 = 1_024 * 1_024 * 1_024 * 1_024;

/// A size as the server's pages show it: bytes below 1 KiB, then binary units with one
/// decimal (`40.5 KiB`, `1.2 GiB`), the units `--min-size` and `--max-size` take.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < KB {
        return format!("{bytes} B");
    }
    let mut n = bytes as f64 / KB as f64;
    let mut unit = 0;
    // 1023.96 KiB would round to "1024.0 KiB"
    while n >= 1023.95 && unit < UNITS.len() - 1 {
        n /= KB as f64;
        unit += 1;
    }
    format!("{n:.1} {}", UNITS[unit])
}

/// A time as the server's pages show it: `2026-10-13 14:05 UTC`.
pub fn format_time(t: &jiff::Timestamp) -> String {
    t.strftime("%Y-%m-%d %H:%M UTC").to_string()
}

/// The first 12 characters of a content hash, as shown everywhere; the full hash is in
/// `--json`.
pub fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// The first 10 characters of a commit.
pub fn short_commit(commit: &str) -> &str {
    commit.get(..10).unwrap_or(commit)
}

/// `from` for "since": `7d`, `12h` or `2w` ago, as RFC 3339; a date (`2026-10-01`) or a
/// timestamp is passed through for the server to read.
pub fn since(value: &str) -> Result<String> {
    let value = value.trim();
    let hours_per = |unit: char| match unit {
        'h' => Some(1),
        'd' => Some(24),
        'w' => Some(24 * 7),
        _ => None,
    };
    if let Some(unit) = value.chars().last().filter(|c| c.is_ascii_alphabetic())
        && let Ok(n) = value[..value.len() - 1].parse::<i64>()
    {
        let Some(per) = hours_per(unit.to_ascii_lowercase()) else {
            bail!("{value:?}: use a number of hours, days or weeks, such as 12h, 7d or 2w");
        };
        if n < 0 {
            bail!("{value:?}: the number cannot be negative");
        }
        let ago = jiff::SignedDuration::from_hours(n.saturating_mul(per));
        let t = jiff::Timestamp::now()
            .checked_sub(ago)
            .map_err(|_| anyhow::anyhow!("{value:?} is too long ago"))?;
        return Ok(t.round(jiff::Unit::Second).unwrap_or(t).to_string());
    }
    if value.is_empty() {
        bail!("give a time, such as 7d, 12h, 2w or 2026-10-01");
    }
    Ok(value.to_string())
}

/// Takes a human formatted byte size and convert it to the number of bytes
pub fn parse_size(size: &str) -> Result<u64> {
    let s = size.trim();

    if s == "0" {
        return Ok(0);
    }

    let num_end = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(num_end);
    let num_str = num_str.trim();
    let unit_str = unit_str.trim().to_ascii_uppercase();

    if num_str.is_empty() {
        bail!("Invalid size string: {s:?} (no number found)");
    }

    let num: f64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid number in size string: {num_str:?}"))?;

    let multiplier = match unit_str.as_str() {
        "" | "B" => 1u64,
        "K" | "KB" | "KIB" => KB,
        "M" | "MB" | "MIB" => MB,
        "G" | "GB" | "GIB" => GB,
        "T" | "TB" | "TIB" => TB,
        _ => bail!("Unknown size unit: {unit_str:?}"),
    };

    Ok((num * multiplier as f64) as u64)
}

/// Creates a rayon thread pool with a thread count resolved from a 3-tier
/// priority chain, each clamped to `work_items`:
///
/// 1. **Override** — `set_num_threads(n)` (capped at `ENV_MAX_THREADS`)
/// 2. **Environment** — `DVS_NUM_THREADS` environment variable, must parse to
///    `usize > 0` (capped at `ENV_MAX_THREADS`)
/// 3. **Default** — `available_cpus * DEFAULT_THREADS_PER_CPU`
///    (capped at `DEFAULT_MAX_THREADS`)
pub fn get_threadpool(work_items: usize) -> Result<rayon::ThreadPool> {
    debug_assert_ne!(
        work_items, 0,
        "the thread pool should not be instantiated when there are no work items to process"
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(thread_count(work_items))
        .build()?;
    Ok(pool)
}

/// The thread count set by `set_num_threads` or `DVS_NUM_THREADS`, capped at
/// `ENV_MAX_THREADS`; None when neither is set.
pub(crate) fn requested_threads() -> Option<usize> {
    get_num_threads()
        .or_else(|| {
            std::env::var("DVS_NUM_THREADS")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&n| n > 0)
        })
        .map(|n| n.min(ENV_MAX_THREADS))
}

/// Threads for `work_items` items, from the priority chain of [`get_threadpool`].
pub(crate) fn thread_count(work_items: usize) -> usize {
    let work_limit = work_items.max(1);
    let available_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    let source = if get_num_threads().is_some() {
        ThreadSource::Override
    } else if requested_threads().is_some() {
        ThreadSource::Environment
    } else {
        ThreadSource::Default
    };
    let num_threads = match requested_threads() {
        Some(n) => n.min(work_limit),
        None => available_cpus
            .saturating_mul(DEFAULT_THREADS_PER_CPU)
            .min(DEFAULT_MAX_THREADS)
            .min(work_limit),
    };
    log::debug!("thread pool: {num_threads} threads (source: {source}, work_items={work_items})",);
    num_threads
}

fn insecure_tls_requested(value: Option<&str>) -> bool {
    value.is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

/// How long a request may wait for the response headers, and for each read of a body,
/// unless `DVS_HTTP_TIMEOUT` says otherwise
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(120);

/// The HTTP timeout from `DVS_HTTP_TIMEOUT` (whole seconds; 0 turns it off), or the default
/// when it is unset or not a number.
fn http_timeout(value: Option<&str>) -> Option<Duration> {
    let Some(value) = value else {
        return Some(DEFAULT_HTTP_TIMEOUT);
    };
    match value.trim().parse::<u64>() {
        Ok(0) => None,
        Ok(secs) => Some(Duration::from_secs(secs)),
        Err(_) => {
            log::warn!(
                "DVS_HTTP_TIMEOUT={value:?} is not a whole number of seconds; using {}s",
                DEFAULT_HTTP_TIMEOUT.as_secs()
            );
            Some(DEFAULT_HTTP_TIMEOUT)
        }
    }
}

/// An agent that gives up on a server that stops answering: when the response headers do not
/// come within `timeout`, a body read waits that long for a byte, or a write waits that long
/// for the server to take one. A large body that keeps moving, either way, may take as long
/// as it needs.
fn build_http_agent(insecure: bool, timeout: Option<Duration>) -> ureq::Agent {
    use ureq::unversioned::resolver::DefaultResolver;
    use ureq::unversioned::transport::{Connector, DefaultConnector};

    let config = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .disable_verification(insecure)
                .build(),
        )
        .http_status_as_error(false)
        .user_agent(crate::remote::USER_AGENT.as_str())
        .timeout_connect(Some(Duration::from_secs(5)))
        .max_idle_connections(ENV_MAX_THREADS)
        .max_idle_connections_per_host(ENV_MAX_THREADS)
        .build();
    match timeout {
        // ureq's own timeouts will not do: each one is for a whole phase (the headers' one
        // even keeps counting while the body is read), so it would cut a large transfer short.
        Some(timeout) => ureq::Agent::with_parts(
            config,
            DefaultConnector::default().chain(IdleTimeout(timeout)),
            DefaultResolver::default(),
        ),
        None => config.new_agent(),
    }
}

/// Wraps each connection so that no wait to read or write a byte lasts longer than its
/// duration. Each wait starts over once bytes move.
#[derive(Debug)]
struct IdleTimeout(Duration);

impl ureq::unversioned::transport::Connector<Box<dyn ureq::unversioned::transport::Transport>>
    for IdleTimeout
{
    type Out = IdleTransport;

    fn connect(
        &self,
        _details: &ureq::unversioned::transport::ConnectionDetails,
        chained: Option<Box<dyn ureq::unversioned::transport::Transport>>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(|inner| IdleTransport {
            inner,
            idle: self.0,
        }))
    }
}

#[derive(Debug)]
struct IdleTransport {
    inner: Box<dyn ureq::unversioned::transport::Transport>,
    idle: Duration,
}

impl IdleTransport {
    /// The wait ureq asks for, but no longer than `idle`
    fn limit(
        &self,
        mut timeout: ureq::unversioned::transport::NextTimeout,
    ) -> ureq::unversioned::transport::NextTimeout {
        use ureq::unversioned::transport::time::Duration as Wait;
        let longer = match timeout.after {
            Wait::Exact(after) => after > self.idle,
            Wait::NotHappening => true,
        };
        if longer {
            timeout.after = Wait::Exact(self.idle);
        }
        timeout
    }
}

impl ureq::unversioned::transport::Transport for IdleTransport {
    fn buffers(&mut self) -> &mut dyn ureq::unversioned::transport::Buffers {
        self.inner.buffers()
    }

    fn transmit_output(
        &mut self,
        amount: usize,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<(), ureq::Error> {
        // The socket's write timeout holds for each write call, which a partial write ends:
        // a write that stops moving fails, a slow one carries on.
        let timeout = self.limit(timeout);
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(
        &mut self,
        timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<bool, ureq::Error> {
        let timeout = self.limit(timeout);
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}

pub fn http_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let insecure = insecure_tls_requested(std::env::var("DVS_INSECURE").ok().as_deref());
        if insecure {
            log::warn!("TLS certificate verification is disabled via DVS_INSECURE");
        }
        let timeout = http_timeout(std::env::var("DVS_HTTP_TIMEOUT").ok().as_deref());
        build_http_agent(insecure, timeout)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insecure_tls_opt_in_is_explicit() {
        for value in ["1", "true", "TRUE", "TrUe"] {
            assert!(insecure_tls_requested(Some(value)));
        }
        for value in ["0", "false", "yes", "on", "", " true", "true "] {
            assert!(!insecure_tls_requested(Some(value)));
        }
        assert!(!insecure_tls_requested(None));
    }
    #[test]
    fn http_timeout_comes_from_the_environment() {
        assert_eq!(http_timeout(None), Some(DEFAULT_HTTP_TIMEOUT));
        assert_eq!(http_timeout(Some("30")), Some(Duration::from_secs(30)));
        assert_eq!(http_timeout(Some(" 600 ")), Some(Duration::from_secs(600)));
        assert_eq!(http_timeout(Some("0")), None);
        assert_eq!(http_timeout(Some("2m")), Some(DEFAULT_HTTP_TIMEOUT));
        assert_eq!(http_timeout(Some("-1")), Some(DEFAULT_HTTP_TIMEOUT));
    }

    /// Serves one request: waits `before_headers`, sends the headers, then the body's
    /// chunks with `between` before each one.
    fn slow_server(before_headers: Duration, chunks: usize, between: Duration) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 2 {
                line.clear();
            }
            let mut stream = stream;
            std::thread::sleep(before_headers);
            let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {chunks}\r\n\r\n");
            if stream.write_all(head.as_bytes()).is_err() {
                return;
            }
            for _ in 0..chunks {
                std::thread::sleep(between);
                if stream.write_all(b"x").and_then(|_| stream.flush()).is_err() {
                    return;
                }
            }
        });
        url
    }

    fn fetch(agent: &ureq::Agent, url: &str) -> std::result::Result<String, String> {
        let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|e| e.to_string())
    }

    #[test]
    fn a_slow_server_times_out_but_a_slow_body_does_not() {
        let agent = build_http_agent(false, Some(Duration::from_millis(600)));
        let short = Duration::from_millis(250);
        let long = Duration::from_millis(1500);

        // Headers that never come in time
        let err = fetch(&agent, &slow_server(long, 1, Duration::ZERO)).unwrap_err();
        assert!(err.contains("timeout") || err.contains("Timeout"), "{err}");

        // A body that keeps coming, for longer than the timeout in all
        assert_eq!(
            fetch(&agent, &slow_server(Duration::ZERO, 8, short)).unwrap(),
            "x".repeat(8)
        );

        // Headers that take a while, then a body that keeps coming
        assert_eq!(
            fetch(&agent, &slow_server(Duration::from_millis(400), 4, short)).unwrap(),
            "x".repeat(4)
        );

        // A body that stops
        let err = fetch(&agent, &slow_server(Duration::ZERO, 2, long)).unwrap_err();
        assert!(err.contains("timeout") || err.contains("Timeout"), "{err}");
    }

    /// Takes one upload: reads the headers, then the body in `step`-byte reads, with `between`
    /// before each until it has `slow_for` bytes, but stops for good after `stop_after` bytes.
    /// Answers 200 once it has it all.
    fn slow_reader(step: usize, between: Duration, slow_for: usize, stop_after: usize) -> String {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (mut line, mut length) = (String::new(), 0);
            while reader.read_line(&mut line).unwrap_or(0) > 2 {
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                line.clear();
            }
            let mut buf = vec![0; step];
            let mut read = 0;
            while read < length {
                if read >= stop_after {
                    // Keep the connection open without reading
                    std::thread::sleep(Duration::from_secs(30));
                    return;
                }
                if read < slow_for {
                    std::thread::sleep(between);
                }
                let want = step.min(length - read);
                match reader.read(&mut buf[..want]) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => read += n,
                }
            }
            let mut stream = stream;
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        });
        url
    }

    fn upload(agent: &ureq::Agent, url: &str, size: usize) -> std::result::Result<(), String> {
        // Run it aside, so a hang fails the test rather than stalling it
        let (agent, url) = (agent.clone(), url.to_string());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let body = vec![b'x'; size];
            let result = agent
                .put(&url)
                .send(&body[..])
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        rx.recv_timeout(Duration::from_secs(20))
            .expect("the upload hung")
    }

    #[test]
    fn a_stalled_upload_times_out_but_a_slow_one_does_not() {
        let agent = build_http_agent(false, Some(Duration::from_millis(600)));
        let mb = 1 << 20;

        // Far more than the socket buffers hold, and the server stops taking it
        let server = slow_reader(mb, Duration::ZERO, 0, mb);
        let err = upload(&agent, &server, 64 * mb).unwrap_err();
        assert!(err.contains("timeout") || err.contains("Timeout"), "{err}");

        // Taken slowly but steadily, for longer than the timeout in all, then quickly: the
        // buffered rest goes before the answer is awaited, which a slow end would hold up
        let server = slow_reader(mb / 2, Duration::from_millis(150), 8 * mb, usize::MAX);
        upload(&agent, &server, 32 * mb).unwrap();
    }

    #[test]
    fn sizes_match_the_server() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(41_463), "40.5 KiB");
        assert_eq!(format_size(1_048_524), "1023.9 KiB");
        assert_eq!(format_size(1_048_535), "1.0 MiB");
        assert_eq!(format_size(5 * GB / 2), "2.5 GiB");
        assert_eq!(format_size(3 * TB), "3.0 TiB");
    }

    #[test]
    fn times_hashes_and_commits_are_short() {
        let t: jiff::Timestamp = "2026-10-13T14:05:59Z".parse().unwrap();
        assert_eq!(format_time(&t), "2026-10-13 14:05 UTC");
        assert_eq!(short_hash(&"a".repeat(64)), "a".repeat(12));
        assert_eq!(short_hash("abc"), "abc");
        assert_eq!(short_commit(&"b".repeat(40)), "b".repeat(10));
    }

    #[test]
    fn since_is_a_time_ago() {
        let week: jiff::Timestamp = since("7d").unwrap().parse().unwrap();
        let ago = jiff::Timestamp::now().duration_since(week);
        assert!((ago.as_secs() - 7 * 86_400).abs() < 5, "{ago:?}");
        assert!(since("12h").is_ok() && since("2W").is_ok());
        assert_eq!(since("2026-10-01").unwrap(), "2026-10-01");
        assert!(since("3y").is_err());
        assert!(since("").is_err());
    }

    #[test]
    fn parse_size_basic() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("500MB").unwrap(), 500 * MB);
        assert_eq!(parse_size("500 MB").unwrap(), 500 * MB);
        assert_eq!(parse_size("1GB").unwrap(), GB);
        assert_eq!(parse_size("1 gb").unwrap(), GB);
        assert_eq!(parse_size("2TB").unwrap(), 2 * TB);
        assert_eq!(parse_size("100KB").unwrap(), 100 * KB);
        assert_eq!(parse_size("1024B").unwrap(), 1024);
        assert_eq!(parse_size("1.5GB").unwrap(), (1.5 * GB as f64) as u64);
        assert!(parse_size("").is_err());
        assert!(parse_size("MB").is_err());
        assert!(parse_size("500XB").is_err());
    }
}
