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

/// What dvs may do to stored credentials that turn out to be unusable.
///
/// Only `dvs login` and `dvs logout` change credentials on purpose; this
/// governs every other path (server calls, `info`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CredentialPolicy {
    /// Human mode: forget a token the server rejects (401) and delete a
    /// credentials store that cannot be parsed, so `dvs login` starts clean.
    #[default]
    Repair,
    /// Machine mode (`--json`, `--emit-events`): never modify credentials.
    /// A 401 is reported as `auth_expired` and an unparsable store as
    /// `credentials_unreadable`, both left in place.
    Preserve,
}

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
        static NO_CONFIG_DIR: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Point this thread's credentials store at `path`.
    pub(crate) fn set(path: &Path) {
        PATH.with(|p| *p.borrow_mut() = Some(path.to_path_buf()));
        NO_CONFIG_DIR.with(|n| *n.borrow_mut() = false);
    }

    /// Make this thread behave as if there were no config directory, as
    /// when etcetera cannot find a home directory.
    pub(crate) fn no_config_dir() {
        NO_CONFIG_DIR.with(|n| *n.borrow_mut() = true);
    }

    pub(crate) fn path() -> anyhow::Result<PathBuf> {
        if NO_CONFIG_DIR.with(|n| *n.borrow()) {
            anyhow::bail!("could not determine the home directory");
        }
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

/// Reads the token store at `path`; a missing file is an empty store.
///
/// A store that cannot be parsed is deleted under `Repair`, and an empty
/// store returned so users can log in again. Under `Preserve` it is left in
/// place and reported as `credentials_unreadable`, as is one that cannot be
/// opened at all.
///
/// Under `Repair` a store whose path cannot even be looked at (a parent
/// that is unreadable, or is a file) is an empty store too, as it was when
/// every reader checked `path.exists()` first.
fn read_store(path: &Path, policy: CredentialPolicy) -> Result<TokenStore> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TokenStore::new()),
        Err(_) if policy == CredentialPolicy::Repair && !path.exists() => {
            return Ok(TokenStore::new());
        }
        Err(e) => {
            return Err(match policy {
                CredentialPolicy::Repair => e.into(),
                CredentialPolicy::Preserve => BackendError::new(
                    ErrorCode::CredentialsUnreadable,
                    format!("{} could not be read ({e})", path.display()),
                )
                .into(),
            });
        }
    };
    match serde_json::from_reader(file) {
        Ok(store) => Ok(store),
        Err(e) => match policy {
            CredentialPolicy::Repair => {
                log::warn!(
                    "Credentials store at {} is corrupted ({e}), deleting it",
                    path.display()
                );
                fs::remove_file(path)?;
                Ok(TokenStore::new())
            }
            CredentialPolicy::Preserve => Err(BackendError::new(
                ErrorCode::CredentialsUnreadable,
                format!("{} is not a valid credentials store ({e})", path.display()),
            )
            .into()),
        },
    }
}

pub fn store_token(url: &Url, token: &str) -> Result<()> {
    let path = get_token_store_path()?;
    let mut store = read_store(&path, CredentialPolicy::Repair)?;
    store.insert(url.to_string(), token.to_string());
    let json = serde_json::to_string_pretty(&store)?;
    write_store(&path, &json)?;

    Ok(())
}

/// Read the token for `url`, deleting a corrupt store (`Repair`).
pub fn get_token(url: &Url) -> Result<Option<String>> {
    get_token_with(url, CredentialPolicy::Repair)
}

/// Read the token for `url` as `policy` allows: under `Repair` a corrupt
/// store is deleted (and reads as no token), under `Preserve` it is an error
/// coded `credentials_unreadable` and the file is left alone.
///
/// With no config directory there is nowhere a login could have been
/// stored: under `Preserve` that is `auth_required`, so machine mode always
/// gets a code; `Repair` keeps the plain error it always returned.
pub fn get_token_with(url: &Url, policy: CredentialPolicy) -> Result<Option<String>> {
    let path = match get_token_store_path() {
        Ok(path) => path,
        Err(e) if policy == CredentialPolicy::Preserve => {
            return Err(BackendError::new(
                ErrorCode::AuthRequired,
                format!("Could not find the credentials store ({e}). Log in first"),
            )
            .into());
        }
        Err(e) => return Err(e),
    };
    token_from_store(&path, url, policy)
}

fn token_from_store(path: &Path, url: &Url, policy: CredentialPolicy) -> Result<Option<String>> {
    Ok(read_store(path, policy)?.get(url.as_str()).cloned())
}

pub fn delete_token(url: &Url) -> Result<bool> {
    let path = get_token_store_path()?;
    if path.exists() {
        let mut store = read_store(&path, CredentialPolicy::Repair)?;
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
    fn corrupt_credentials_are_reported_and_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        let original = b"corrupt credentials must remain available for diagnosis";
        fs::write(&path, original).unwrap();
        let error = token_from_store(&path, &url(), CredentialPolicy::Preserve).unwrap_err();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::CredentialsUnreadable)
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn corrupt_credentials_are_deleted_under_repair() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(
            token_from_store(&path, &url(), CredentialPolicy::Repair).unwrap(),
            None
        );
        assert!(!path.exists());
    }

    #[test]
    fn get_token_with_preserve_keeps_a_corrupt_store() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        test_store::set(&path);
        fs::write(&path, b"{ not json").unwrap();
        let read = get_token_with(&url(), CredentialPolicy::Preserve);
        assert_eq!(
            backend_error_code(&read.unwrap_err()),
            Some(ErrorCode::CredentialsUnreadable)
        );
        assert_eq!(fs::read(&path).unwrap(), b"{ not json");
    }

    #[test]
    fn missing_credentials_are_not_created() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing").join("credentials.json");
        for policy in [CredentialPolicy::Preserve, CredentialPolicy::Repair] {
            assert_eq!(token_from_store(&path, &url(), policy).unwrap(), None);
        }
        assert!(!path.parent().unwrap().exists());
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

    #[test]
    fn a_store_under_a_file_reads_as_no_token_under_repair() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("not-a-directory");
        fs::write(&file, b"").unwrap();
        // Opening it fails with ENOTDIR, not NotFound
        let path = file.join("credentials.json");
        assert_eq!(
            token_from_store(&path, &url(), CredentialPolicy::Repair).unwrap(),
            None
        );
        test_store::set(&path);
        assert_eq!(get_token(&url()).unwrap(), None);
        // Machine mode still says something is wrong with the store
        let error = token_from_store(&path, &url(), CredentialPolicy::Preserve).unwrap_err();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::CredentialsUnreadable)
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_store_in_an_unreadable_directory_reads_as_no_token_under_repair() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("locked");
        fs::create_dir(&dir).unwrap();
        let path = dir.join("credentials.json");
        fs::write(&path, br#"{"https://example.test/":"token"}"#).unwrap();
        fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let read = token_from_store(&path, &url(), CredentialPolicy::Repair);
        fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        // EACCES on the directory; as root it is simply readable
        assert!(matches!(read.unwrap().as_deref(), None | Some("token")));
        assert!(path.exists());
    }

    #[test]
    fn no_config_dir_is_auth_required_under_preserve() {
        test_store::no_config_dir();
        let read = get_token_with(&url(), CredentialPolicy::Preserve);
        assert_eq!(
            backend_error_code(&read.unwrap_err()),
            Some(ErrorCode::AuthRequired)
        );
        // Repair keeps the plain error it always had
        assert_eq!(backend_error_code(&get_token(&url()).unwrap_err()), None);
    }
}
