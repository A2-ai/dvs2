//! Auth for dvs-server

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::backends::server::{http_status_code, join_url};
use crate::error::{BackendError, ErrorCode};
use crate::utils::http_agent;

type TokenStore = HashMap<String, String>;

#[cfg(not(test))]
fn get_token_store_path() -> Result<PathBuf> {
    use etcetera::BaseStrategy;
    let d = etcetera::choose_base_strategy()?.config_dir();
    Ok(d.join("dvs").join("credentials.json"))
}

/// Unit tests never see the real user's credentials: each test thread reads
/// and writes only the store it pointed at, or one that does not exist.
#[cfg(test)]
fn get_token_store_path() -> Result<PathBuf> {
    test_store::path()
}

/// Per-thread credentials path for unit tests in this crate.
#[cfg(test)]
pub(crate) mod test_store {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    thread_local! {
        static PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    }

    /// Point this thread's credentials store at `path`.
    pub(crate) fn set(path: &Path) {
        PATH.with(|p| *p.borrow_mut() = Some(path.to_path_buf()));
    }

    pub(crate) fn path() -> anyhow::Result<PathBuf> {
        Ok(PATH.with(|p| p.borrow().clone()).unwrap_or_else(|| {
            std::env::temp_dir()
                .join(format!(
                    "dvs-unit-tests-no-credentials-{}",
                    std::process::id()
                ))
                .join("credentials.json")
        }))
    }
}

fn create_private_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir)?;
    Ok(())
}

fn write_store(path: &Path, contents: &str) -> Result<()> {
    create_private_dir(path.parent().unwrap())?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use fs_err::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(contents.as_bytes())?;
    Ok(())
}

/// Reads the token store from `path`. If the file is corrupted, it is deleted
/// and an empty store is returned so users can log in again.
fn read_store(path: &Path) -> Result<TokenStore> {
    match serde_json::from_reader(fs::File::open(path)?) {
        Ok(store) => Ok(store),
        Err(e) => {
            log::warn!(
                "Credentials store at {} is corrupted ({e}), deleting it",
                path.display()
            );
            fs::remove_file(path)?;
            Ok(TokenStore::new())
        }
    }
}

pub fn store_token(url: &Url, token: &str) -> Result<()> {
    let path = get_token_store_path()?;
    let mut store = if path.exists() {
        read_store(&path)?
    } else {
        TokenStore::new()
    };
    store.insert(url.to_string(), token.to_string());
    let json = serde_json::to_string_pretty(&store)?;
    write_store(&path, &json)?;

    Ok(())
}

pub fn get_token(url: &Url) -> Result<Option<String>> {
    let path = get_token_store_path()?;
    if path.exists() {
        let store = read_store(&path)?;
        Ok(store.get(url.as_str()).cloned())
    } else {
        Ok(None)
    }
}

pub fn delete_token(url: &Url) -> Result<bool> {
    let path = get_token_store_path()?;
    if path.exists() {
        let mut store = read_store(&path)?;
        let old = store.remove(url.as_str());
        let json = serde_json::to_string_pretty(&store)?;
        fs::write(path, json)?;
        Ok(old.is_some())
    } else {
        Ok(false)
    }
}

/// A transport failure talking to the auth endpoints.
fn unreachable(base_url: &Url, e: ureq::Error) -> anyhow::Error {
    BackendError::new(
        ErrorCode::ServerUnavailable,
        format!("cannot reach the dvs server at {base_url}: {e}"),
    )
    .into()
}

/// A 2xx from an auth endpoint whose body is not what it promises.
fn invalid_response(base_url: &Url, e: ureq::Error) -> anyhow::Error {
    BackendError::new(
        ErrorCode::ServerUnavailable,
        format!("Invalid response from the dvs server at {base_url}: {e}"),
    )
    .into()
}

/// An auth endpoint answered a status it has no meaning of its own for: the
/// shared code if [`http_status_code`] has one, else a plain server error.
fn status_error(base_url: &Url, status: u16) -> anyhow::Error {
    let Some(code) = http_status_code(status, "") else {
        return anyhow::anyhow!("Server error: {status}");
    };
    let message = match code {
        ErrorCode::ServerTooOld => format!(
            "the dvs server at {base_url} does not support this request ({status}); it is older than this dvs"
        ),
        ErrorCode::ServerUnavailable => {
            format!("the dvs server at {base_url} is not working ({status})")
        }
        _ => format!("Server error: {status}"),
    };
    BackendError::new(code, message).into()
}

#[derive(Serialize, Deserialize)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
}

pub fn get_client_approve_url(base_url: &Url, user_code: &str) -> Result<Url> {
    let mut u = join_url(base_url, "device/approve")?;
    u.set_query(Some(&format!("user_code={user_code}")));
    Ok(u)
}

pub fn initiate_device_approval(base_url: &Url) -> Result<DeviceAuthorizationResponse> {
    let url = join_url(base_url, "api/oauth/device/authorize")?;
    let mut resp = http_agent()
        .post(url.as_str())
        .send_empty()
        .map_err(|e| unreachable(base_url, e))?;
    if !resp.status().is_success() {
        return Err(status_error(base_url, resp.status().as_u16()));
    }
    let r = resp
        .body_mut()
        .read_json::<DeviceAuthorizationResponse>()
        .map_err(|e| invalid_response(base_url, e))?;
    Ok(r)
}

#[derive(Serialize, Deserialize)]
pub struct TokenPayload {
    pub device_code: String,
}

#[derive(Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
}

pub fn query_for_token(base_url: &Url, device_code: &str) -> Result<Option<TokenResponse>> {
    let url = join_url(base_url, "api/oauth/token")?;
    let mut resp = http_agent()
        .post(url.as_str())
        .send_json(TokenPayload {
            device_code: device_code.to_string(),
        })
        .map_err(|e| unreachable(base_url, e))?;
    match resp.status().as_u16() {
        200 => {
            let r = resp
                .body_mut()
                .read_json::<TokenResponse>()
                .map_err(|e| invalid_response(base_url, e))?;
            Ok(Some(r))
        }
        400 => bail!("User code expired"),
        401 => Ok(None),
        // The endpoint's own 404: the server forgot this device code
        404 => bail!("Unknown device code"),
        code @ (405 | 500..) => Err(status_error(base_url, code)),
        _ => bail!("Unknown server error: {}", resp.status()),
    }
}

pub fn poll_for_token(base_url: &Url, device_code: &str) -> Result<TokenResponse> {
    let mut num_retries = 0;

    loop {
        if num_retries > 120 {
            bail!("Token will have expired");
        }

        match query_for_token(base_url, device_code) {
            Ok(None) => {
                sleep(Duration::from_secs(5));
                num_retries += 1;
            }
            Ok(Some(t)) => {
                return Ok(t);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Ask the server to revoke `token` so it stops working everywhere, not just on this machine.
pub fn revoke_token(base_url: &Url, token: &str) -> Result<()> {
    let url = join_url(base_url, "api/oauth/token")?;
    let resp = http_agent()
        .delete(url.as_str())
        .header("authorization", format!("Bearer {token}"))
        .call()
        .map_err(|e| unreachable(base_url, e))?;
    if !resp.status().is_success() {
        return Err(status_error(base_url, resp.status().as_u16()));
    }
    Ok(())
}

#[cfg(test)]
mod readonly_tests {
    use super::*;
    use crate::backends::server::test_http::stub;
    use crate::error::backend_error_code;

    fn url() -> Url {
        Url::parse("https://example.test/").unwrap()
    }

    #[test]
    fn login_and_logout_still_write_the_store() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("dvs").join("credentials.json");
        test_store::set(&path);
        store_token(&url(), "token").unwrap();
        assert_eq!(get_token(&url()).unwrap().as_deref(), Some("token"));
        assert!(delete_token(&url()).unwrap());
        assert_eq!(get_token(&url()).unwrap(), None);
    }

    #[test]
    fn the_approve_url_keeps_the_proxy_prefix() {
        for base in ["https://h/dvs", "https://h/dvs/"] {
            let url = get_client_approve_url(&Url::parse(base).unwrap(), "ABCD").unwrap();
            assert_eq!(url.as_str(), "https://h/dvs/device/approve?user_code=ABCD");
        }
    }

    #[test]
    fn login_and_logout_requests_keep_the_proxy_prefix() {
        for prefix in ["/dvs", "/dvs/"] {
            let (base, handle) = stub(prefix, 200, r#"{"device_code":"d","user_code":"u"}"#);
            initiate_device_approval(&base).unwrap();
            assert!(
                handle
                    .join()
                    .unwrap()
                    .starts_with("POST /dvs/api/oauth/device/authorize ")
            );

            let (base, handle) = stub(prefix, 200, r#"{"access_token":"t"}"#);
            assert!(query_for_token(&base, "d").unwrap().is_some());
            assert!(
                handle
                    .join()
                    .unwrap()
                    .starts_with("POST /dvs/api/oauth/token ")
            );

            let (base, handle) = stub(prefix, 204, "");
            revoke_token(&base, "t").unwrap();
            assert!(
                handle
                    .join()
                    .unwrap()
                    .starts_with("DELETE /dvs/api/oauth/token ")
            );
        }
    }

    #[test]
    fn a_non_base_server_url_is_an_error_not_a_panic() {
        for base in ["foo:bar", "mailto:x@y"] {
            let url = Url::parse(base).unwrap();
            assert!(get_client_approve_url(&url, "ABCD").is_err(), "{base}");
            assert!(initiate_device_approval(&url).is_err(), "{base}");
            assert!(query_for_token(&url, "d").is_err(), "{base}");
            assert!(revoke_token(&url, "t").is_err(), "{base}");
        }
    }

    #[test]
    fn auth_endpoint_failures_are_coded() {
        for (status, expected) in [
            (404, ErrorCode::ServerTooOld),
            (405, ErrorCode::ServerTooOld),
            (503, ErrorCode::ServerUnavailable),
        ] {
            let (base, handle) = stub("/", status, "");
            let error = initiate_device_approval(&base).err().unwrap();
            assert_eq!(backend_error_code(&error), Some(expected), "{status}");
            handle.join().unwrap();
        }
        // The token endpoint's own 404 keeps its meaning
        let (base, handle) = stub("/", 404, "unknown device code");
        let error = query_for_token(&base, "d").err().unwrap();
        assert_eq!(backend_error_code(&error), None);
        handle.join().unwrap();

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        let error = revoke_token(&base, "t").unwrap_err();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::ServerUnavailable)
        );
    }

    #[test]
    fn an_invalid_body_on_a_2xx_is_server_unavailable() {
        let (base, handle) = stub("/", 200, "this is not json");
        let error = initiate_device_approval(&base).err().unwrap();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::ServerUnavailable)
        );
        handle.join().unwrap();
        let (base, handle) = stub("/", 200, "this is not json");
        let error = query_for_token(&base, "d").err().unwrap();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::ServerUnavailable)
        );
        handle.join().unwrap();
    }
}
