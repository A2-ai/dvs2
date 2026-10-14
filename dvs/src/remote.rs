//! The v1 server API from the client side: transport, errors, discovery and login.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, OnceLock};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use url::Url;

use crate::api::v1::{
    DeviceAuthorization, DeviceError, DeviceToken, DeviceTokenRequest, ServerInfo,
};
use crate::auth::{get_token, store_token};
use crate::utils::http_agent;

/// The API version this client speaks.
pub const API_VERSION: u32 = 1;

/// Login polls in a row that may get no answer before the login fails.
const MAX_UNANSWERED_POLLS: u32 = 3;

/// Overrides the stored token for every server, e.g. in CI.
pub const TOKEN_ENV: &str = "DVS_TOKEN";

/// `dvs/<version> (<os>; <arch>)`, sent on every request.
pub static USER_AGENT: LazyLock<String> = LazyLock::new(|| {
    format!(
        "dvs/{} ({}; {})",
        crate::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH
    )
});

/// An error the server answered with, or one the client raises in the same terms.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    /// Stable, machine-readable: branch on this
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    /// Everything else in the error object
    pub details: Map<String, Value>,
}

impl ApiError {
    pub fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.to_string(),
            message: message.into(),
            request_id: None,
            details: Map::new(),
        }
    }

    /// Parse the error envelope; anything else becomes a generic error with the
    /// status and the start of the body.
    pub fn from_body(status: u16, body: &str) -> Self {
        let error = match serde_json::from_str::<Value>(body) {
            Ok(Value::Object(mut root)) => root.remove("error"),
            _ => None,
        };
        if let Some(Value::Object(mut error)) = error {
            let take = |e: &mut Map<String, Value>, k: &str| match e.remove(k) {
                Some(Value::String(s)) => Some(s),
                _ => None,
            };
            if let Some(code) = take(&mut error, "code") {
                let message = take(&mut error, "message").unwrap_or_else(|| code.clone());
                let request_id = take(&mut error, "request_id");
                return Self {
                    status,
                    code,
                    message,
                    request_id,
                    details: error,
                };
            }
        }
        let snippet: String = body.trim().chars().take(200).collect();
        let message = if snippet.is_empty() {
            format!("the server answered {status}")
        } else {
            format!("the server answered {status}: {snippet}")
        };
        Self::new(status, "error", message)
    }

    /// The token is missing, rejected or expired.
    pub fn is_auth(&self) -> bool {
        matches!(
            self.code.as_str(),
            "not_logged_in" | "unauthenticated" | "token_expired" | "account_inactive"
        )
    }

    /// The project is missing, or the caller may not do this to it.
    pub fn is_access(&self) -> bool {
        matches!(
            self.code.as_str(),
            "project_not_found" | "insufficient_role" | "not_in_group" | "global_admin_required"
        )
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

/// The API error behind an `anyhow` error, if there is one.
pub fn api_error(e: &anyhow::Error) -> Option<&ApiError> {
    e.chain().find_map(|c| c.downcast_ref::<ApiError>())
}

/// Where the token in use came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    /// `DVS_TOKEN`
    Environment,
    /// `dvs login`, in the credentials store
    Stored,
}

/// The token for `url`: `DVS_TOKEN` when set, otherwise the stored one.
pub fn token_for(url: &Url) -> Result<Option<(String, TokenSource)>> {
    if let Ok(token) = std::env::var(TOKEN_ENV) {
        let token = token.trim();
        if !token.is_empty() {
            return Ok(Some((token.to_string(), TokenSource::Environment)));
        }
    }
    Ok(get_token(url)?.map(|t| (t, TokenSource::Stored)))
}

/// A connection to one server: its base URL and the token for it, read once.
#[derive(Debug, Default)]
pub struct Client {
    token: OnceLock<Option<(String, TokenSource)>>,
    info: OnceLock<ServerInfo>,
    /// The stored token was rejected and has been forgotten
    forgotten: AtomicBool,
}

/// A transport failure: no response at all, so the request may or may not have
/// reached the server.
#[derive(Debug)]
pub struct NoResponse(pub ureq::Error);

impl fmt::Display for NoResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no response: {}", self.0)
    }
}

impl std::error::Error for NoResponse {}

/// Whether the request got no response at all.
pub fn is_no_response(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<NoResponse>().is_some())
}

pub type Response = ureq::http::Response<ureq::Body>;

/// Turn a response into its JSON body, or the error it carries.
pub fn read_json<T: DeserializeOwned>(mut resp: Response) -> Result<T> {
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        return Err(ApiError::from_body(status, &body).into());
    }
    Ok(resp.body_mut().read_json::<T>()?)
}

/// The error a non-2xx response carries; Ok for a 2xx.
pub fn check(mut resp: Response) -> Result<Response> {
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(resp);
    }
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    Err(ApiError::from_body(status, &body).into())
}

impl Client {
    /// `path` (absolute, like `/api/server`) on the server at `base`.
    pub fn url(base: &Url, path: &str) -> Url {
        base.join(path).expect("API paths are valid")
    }

    /// `/api/v1/` followed by `segments`, each percent-encoded.
    pub fn v1(base: &Url, segments: &[&str]) -> Url {
        let mut url = Self::url(base, "/api/v1");
        url.path_segments_mut()
            .expect("an http URL has a path")
            .extend(segments);
        url
    }

    /// The token for `base`, from `DVS_TOKEN` or the store.
    pub fn token(&self, base: &Url) -> Option<&(String, TokenSource)> {
        self.token
            .get_or_init(|| {
                token_for(base).unwrap_or_else(|e| {
                    log::warn!("Could not read the credentials store: {e}");
                    None
                })
            })
            .as_ref()
    }

    /// True the first time only: a rejected stored token is forgotten once per client,
    /// however many requests saw it rejected.
    pub fn forget_once(&self) -> bool {
        !self.forgotten.swap(true, Ordering::SeqCst)
    }

    pub fn bearer(&self, base: &Url) -> Result<String> {
        match self.token(base) {
            Some((token, _)) => Ok(format!("Bearer {token}")),
            None => Err(ApiError::new(
                401,
                "not_logged_in",
                format!("not logged in to {base}; run `dvs login` or set {TOKEN_ENV}"),
            )
            .into()),
        }
    }

    /// A request that got no answer, naming the server it was for.
    pub fn transport(base: &Url) -> impl Fn(ureq::Error) -> anyhow::Error + '_ {
        move |e| anyhow::Error::from(NoResponse(e)).context(format!("could not reach {base}"))
    }

    pub fn get(&self, base: &Url, url: &Url) -> Result<Response> {
        http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer(base)?)
            .call()
            .map_err(Self::transport(base))
    }

    pub fn head(&self, base: &Url, url: &Url) -> Result<Response> {
        http_agent()
            .head(url.as_str())
            .header("authorization", self.bearer(base)?)
            .call()
            .map_err(Self::transport(base))
    }

    pub fn post_json<B: Serialize>(&self, base: &Url, url: &Url, body: &B) -> Result<Response> {
        http_agent()
            .post(url.as_str())
            .header("authorization", self.bearer(base)?)
            .send_json(body)
            .map_err(Self::transport(base))
    }

    /// `PATCH` a JSON body, with `If-Match` when `version` is given.
    pub fn patch_json<B: Serialize>(
        &self,
        base: &Url,
        url: &Url,
        body: &B,
        version: Option<i64>,
    ) -> Result<Response> {
        let mut req = http_agent()
            .patch(url.as_str())
            .header("authorization", self.bearer(base)?);
        if let Some(v) = version {
            req = req.header("if-match", format!("\"{v}\""));
        }
        req.send_json(body).map_err(Self::transport(base))
    }

    /// `PUT` a JSON body, with `If-Match` when `version` is given.
    pub fn put_json<B: Serialize>(
        &self,
        base: &Url,
        url: &Url,
        body: &B,
        version: Option<i64>,
    ) -> Result<Response> {
        let mut req = http_agent()
            .put(url.as_str())
            .header("authorization", self.bearer(base)?);
        if let Some(v) = version {
            req = req.header("if-match", format!("\"{v}\""));
        }
        req.send_json(body).map_err(Self::transport(base))
    }

    /// `DELETE`, with a JSON body when one is given and `If-Match` when `version` is.
    pub fn delete<B: Serialize>(
        &self,
        base: &Url,
        url: &Url,
        body: Option<&B>,
        version: Option<i64>,
    ) -> Result<Response> {
        let mut req = http_agent()
            .delete(url.as_str())
            .header("authorization", self.bearer(base)?);
        if let Some(v) = version {
            req = req.header("if-match", format!("\"{v}\""));
        }
        match body {
            Some(body) => req.force_send_body().send_json(body),
            None => req.call(),
        }
        .map_err(Self::transport(base))
    }

    /// `PUT` a streamed body.
    pub fn put_reader(
        &self,
        base: &Url,
        url: &Url,
        body: &mut dyn std::io::Read,
    ) -> Result<Response> {
        http_agent()
            .put(url.as_str())
            .header("authorization", self.bearer(base)?)
            .header("content-type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(body))
            .map_err(Self::transport(base))
    }

    /// `GET /api/server`, once per client. Fails when the server cannot serve this
    /// client.
    pub fn server_info(&self, base: &Url) -> Result<&ServerInfo> {
        if let Some(info) = self.info.get() {
            return Ok(info);
        }
        let info = server_info(base)?;
        Ok(self.info.get_or_init(|| info))
    }
}

/// Compare dotted versions numerically; a missing part counts as 0.
fn version_at_least(have: &str, need: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (have, need) = (parse(have), parse(need));
    for i in 0..3 {
        let (h, n) = (
            have.get(i).copied().unwrap_or(0),
            need.get(i).copied().unwrap_or(0),
        );
        if h != n {
            return h > n;
        }
    }
    true
}

/// `GET /api/server`, checked: the server must speak API v1 and accept this client.
pub fn server_info(base: &Url) -> Result<ServerInfo> {
    let resp = http_agent()
        .get(Client::url(base, "/api/server").as_str())
        .call()
        .map_err(Client::transport(base))?;
    if resp.status().as_u16() == 404 {
        bail!(
            "{base} does not serve the dvs API v{API_VERSION}; is the URL right, and the server up to date?"
        );
    }
    let info: ServerInfo = read_json(resp)?;
    if !info.api_versions.contains(&API_VERSION) {
        bail!(
            "{base} speaks API versions {:?}; this dvs speaks v{API_VERSION}. Update dvs.",
            info.api_versions
        );
    }
    if !version_at_least(crate::VERSION, &info.min_client_version) {
        bail!(
            "{base} needs dvs {} or newer; this is {}. Update dvs.",
            info.min_client_version,
            crate::VERSION
        );
    }
    Ok(info)
}

/// A device login (RFC 8628) waiting for approval: what to show the person, and what
/// [`poll_login`] needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingLogin {
    /// The user code, the verification URIs (absolute), the device code
    pub authorization: DeviceAuthorization,
    /// Seconds to wait between polls; [`PendingLogin::slow_down`] raises it
    pub interval: u64,
    /// When the codes expire, approved or not
    pub expires_at: jiff::Timestamp,
}

impl PendingLogin {
    /// After [`LoginPoll::SlowDown`]: poll 5 seconds less often from now on (RFC 8628
    /// §3.5).
    pub fn slow_down(&mut self) {
        self.interval += 5;
    }

    /// Whether the codes have expired.
    pub fn is_expired(&self) -> bool {
        jiff::Timestamp::now() > self.expires_at
    }
}

/// The answer to one poll of a device login.
#[derive(Debug, Clone)]
pub enum LoginPoll {
    /// Not approved yet: poll again after `interval` seconds
    Pending,
    /// Polling too often: call [`PendingLogin::slow_down`], then poll again
    SlowDown,
    /// Approved; the token is stored for the server
    Done(DeviceToken),
}

/// Start a device login with the server at `base`: the code to show and where to
/// approve it. Then call [`poll_login`] every `interval` seconds.
pub fn start_login(base: &Url) -> Result<PendingLogin> {
    server_info(base)?;
    let resp = http_agent()
        .post(Client::url(base, "/api/v1/auth/device").as_str())
        .send_empty()
        .map_err(Client::transport(base))?;
    let mut start: DeviceAuthorization = match resp.status().as_u16() {
        200 => read_json(resp)?,
        status => {
            let mut resp = resp;
            let body = resp.body_mut().read_to_string().unwrap_or_default();
            let err: Option<DeviceError> = serde_json::from_str(&body).ok();
            bail!(
                "the server refused to start a login ({status}): {}",
                err.and_then(|e| e.error_description.or(Some(e.error)))
                    .unwrap_or(body)
            );
        }
    };
    // A relative URI is relative to the server the client used
    if start.verification_uri_complete.starts_with('/') {
        start.verification_uri_complete =
            Client::url(base, &start.verification_uri_complete).to_string();
        start.verification_uri = Client::url(base, &start.verification_uri).to_string();
    }
    let lifetime = jiff::SignedDuration::from_secs(start.expires_in.min(i64::MAX as u64) as i64);
    Ok(PendingLogin {
        interval: start.interval.max(1),
        expires_at: jiff::Timestamp::now()
            .checked_add(lifetime)
            .unwrap_or(jiff::Timestamp::MAX),
        authorization: start,
    })
}

/// One token request for a pending login. On approval the token is stored for `base`.
/// A login that expired or was denied is an error with the code `login_expired` or
/// `login_denied`; a request that got no answer is one [`is_no_response`] recognizes,
/// which a caller may poll again after.
pub fn poll_login(base: &Url, pending: &PendingLogin) -> Result<LoginPoll> {
    let expired = || {
        anyhow::Error::from(ApiError::new(
            400,
            "login_expired",
            "the code expired before it was approved; run `dvs login` again",
        ))
    };
    if pending.is_expired() {
        return Err(expired());
    }
    let mut resp = http_agent()
        .post(Client::url(base, "/api/v1/auth/token").as_str())
        .send_json(DeviceTokenRequest {
            device_code: pending.authorization.device_code.clone(),
        })
        .map_err(Client::transport(base))?;
    let status = resp.status().as_u16();
    if status == 200 {
        let token: DeviceToken = resp.body_mut().read_json()?;
        store_token(base, &token.access_token)?;
        return Ok(LoginPoll::Done(token));
    }
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let err: DeviceError = match serde_json::from_str(&body) {
        Ok(e) => e,
        Err(_) => bail!("unexpected answer while waiting for approval: {body}"),
    };
    match err.error.as_str() {
        "authorization_pending" => Ok(LoginPoll::Pending),
        "slow_down" => Ok(LoginPoll::SlowDown),
        "expired_token" => Err(expired()),
        "access_denied" => Err(ApiError::new(
            status,
            "login_denied",
            "the login was denied in the browser",
        )
        .into()),
        _ => bail!(
            "login failed: {}",
            err.error_description.unwrap_or(err.error)
        ),
    }
}

/// Device login (RFC 8628), blocking: show where to approve with `on_code`, poll at
/// the server's pace (a few polls in a row may get no answer), store the token.
pub fn login(base: &Url, on_code: impl FnOnce(&DeviceAuthorization)) -> Result<DeviceToken> {
    let mut pending = start_login(base)?;
    on_code(&pending.authorization);
    // Polls that got no answer in a row: a proxy closing an idle connection, or a
    // moment of network trouble, is tried again at the next poll
    let mut unanswered = 0;
    loop {
        sleep(Duration::from_secs(pending.interval));
        match poll_login(base, &pending) {
            Ok(LoginPoll::Done(token)) => return Ok(token),
            Ok(LoginPoll::Pending) => unanswered = 0,
            Ok(LoginPoll::SlowDown) => {
                unanswered = 0;
                pending.slow_down();
            }
            Err(e) if is_no_response(&e) && unanswered < MAX_UNANSWERED_POLLS => {
                unanswered += 1;
                log::warn!("No answer while waiting for approval, trying again: {e:#}");
            }
            Err(e) => return Err(e),
        }
    }
}

/// Revoke `token` on the server. Ok(false) when the server already did not know it.
pub fn revoke(base: &Url, token: &str) -> Result<bool> {
    let resp = http_agent()
        .delete(Client::url(base, "/api/v1/auth/token").as_str())
        .header("authorization", format!("Bearer {token}"))
        .call()
        .map_err(Client::transport(base))?;
    match resp.status().as_u16() {
        200..=299 => Ok(true),
        401 => Ok(false),
        _ => {
            check(resp)?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_parse_the_envelope_and_keep_details() {
        let e = ApiError::from_body(
            409,
            r#"{"error":{"code":"project_exists","message":"taken","request_id":"r1","project":{"your_role":"read"}}}"#,
        );
        assert_eq!(e.code, "project_exists");
        assert_eq!(e.message, "taken");
        assert_eq!(e.request_id.as_deref(), Some("r1"));
        assert_eq!(e.details["project"]["your_role"], "read");

        let e = ApiError::from_body(502, "<html>bad gateway</html>");
        assert_eq!(e.code, "error");
        assert!(e.message.contains("502"), "{}", e.message);
        let e = ApiError::from_body(500, "");
        assert_eq!(e.message, "the server answered 500");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_at_least("0.5.0", "0.5.0"));
        assert!(version_at_least("0.10.0", "0.5.0"));
        assert!(version_at_least("1.0.0", "0.5.1"));
        assert!(!version_at_least("0.4.9", "0.5.0"));
        assert!(version_at_least("0.5.0-rc.1", "0.5"));
    }

    #[test]
    fn the_user_agent_names_the_client() {
        assert!(USER_AGENT.starts_with(&format!("dvs/{} (", crate::VERSION)));
    }
}
