//! The v1 server API from the client side: transport, errors, discovery and login.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, OnceLock};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use url::Url;

use crate::api::v1::{
    DeviceAuthorization, DeviceError, DeviceToken, DeviceTokenRequest, ServerInfo,
};
use crate::auth::{get_token, store_token};
use crate::utils::http_agent;

/// The API version this client speaks.
pub const API_VERSION: u32 = 1;

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
        write!(f, "no response from the server: {}", self.0)
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

    fn transport(e: ureq::Error) -> anyhow::Error {
        NoResponse(e).into()
    }

    pub fn get(&self, base: &Url, url: &Url) -> Result<Response> {
        http_agent()
            .get(url.as_str())
            .header("authorization", self.bearer(base)?)
            .call()
            .map_err(Self::transport)
    }

    pub fn head(&self, base: &Url, url: &Url) -> Result<Response> {
        http_agent()
            .head(url.as_str())
            .header("authorization", self.bearer(base)?)
            .call()
            .map_err(Self::transport)
    }

    pub fn post_json<B: Serialize>(&self, base: &Url, url: &Url, body: &B) -> Result<Response> {
        http_agent()
            .post(url.as_str())
            .header("authorization", self.bearer(base)?)
            .send_json(body)
            .map_err(Self::transport)
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
            .map_err(Self::transport)
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
        .map_err(Client::transport)?;
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

/// Device login (RFC 8628): print where to approve, poll at the server's pace, store
/// the token. `on_code` shows the URL and code to the user.
pub fn login(base: &Url, on_code: impl FnOnce(&DeviceAuthorization)) -> Result<DeviceToken> {
    server_info(base)?;
    let resp = http_agent()
        .post(Client::url(base, "/api/v1/auth/device").as_str())
        .send_empty()
        .map_err(Client::transport)?;
    let start: DeviceAuthorization = match resp.status().as_u16() {
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
    let mut start = start;
    // A relative URI is relative to the server the client used
    if start.verification_uri_complete.starts_with('/') {
        start.verification_uri_complete =
            Client::url(base, &start.verification_uri_complete).to_string();
        start.verification_uri = Client::url(base, &start.verification_uri).to_string();
    }
    on_code(&start);

    let mut interval = Duration::from_secs(start.interval.max(1));
    let deadline = std::time::Instant::now() + Duration::from_secs(start.expires_in);
    loop {
        sleep(interval);
        if std::time::Instant::now() > deadline {
            bail!("the code expired before it was approved; run `dvs login` again");
        }
        let mut resp = http_agent()
            .post(Client::url(base, "/api/v1/auth/token").as_str())
            .send_json(DeviceTokenRequest {
                device_code: start.device_code.clone(),
            })
            .map_err(Client::transport)?;
        if resp.status().as_u16() == 200 {
            let token: DeviceToken = resp.body_mut().read_json()?;
            store_token(base, &token.access_token)?;
            return Ok(token);
        }
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        let err: DeviceError = match serde_json::from_str(&body) {
            Ok(e) => e,
            Err(_) => bail!("unexpected answer while waiting for approval: {body}"),
        };
        match err.error.as_str() {
            "authorization_pending" => {}
            // RFC 8628 §3.5: add 5 seconds
            "slow_down" => interval += Duration::from_secs(5),
            "expired_token" => {
                bail!("the code expired before it was approved; run `dvs login` again")
            }
            "access_denied" => bail!("the login was denied in the browser"),
            _ => bail!(
                "login failed: {}",
                err.error_description.unwrap_or(err.error)
            ),
        }
    }
}

/// Revoke `token` on the server. Ok(false) when the server already did not know it.
pub fn revoke(base: &Url, token: &str) -> Result<bool> {
    let resp = http_agent()
        .delete(Client::url(base, "/api/v1/auth/token").as_str())
        .header("authorization", format!("Bearer {token}"))
        .call()
        .map_err(Client::transport)?;
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
