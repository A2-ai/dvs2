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

use crate::backends::server::{
    invalid_response, join_url, response_error, status_error, unreachable,
};
use crate::error::{Error, ErrorCode};
use crate::utils::http_agent;

type TokenStore = HashMap<String, String>;

#[cfg(not(test))]
fn get_token_store_path() -> Result<PathBuf> {
    use etcetera::BaseStrategy;
    let d = etcetera::choose_base_strategy()?.config_dir();
    Ok(d.join("dvs").join("credentials.json"))
}

/// Unit tests never touch the real user's credentials.
#[cfg(test)]
fn get_token_store_path() -> Result<PathBuf> {
    Ok(std::env::temp_dir()
        .join(format!(
            "dvs-unit-tests-no-credentials-{}",
            std::process::id()
        ))
        .join("credentials.json"))
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

/// Reads the token store from `path`; none yet is an empty store. An
/// unreadable store is an error, never deleted: the user fixes or removes it.
fn read_store(path: &Path) -> Result<TokenStore> {
    let unreadable = |reason: String| {
        Error::new(
            ErrorCode::CredentialsUnreadable,
            format!(
                "{} {reason}; fix or remove it, then run `dvs login`",
                path.display()
            ),
        )
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TokenStore::new()),
        Err(e) => return Err(unreadable(format!("could not be read ({e})")).into()),
    };
    serde_json::from_reader(file)
        .map_err(|e| unreadable(format!("is not a valid credentials store ({e})")).into())
}

pub fn store_token(url: &Url, token: &str) -> Result<()> {
    let path = get_token_store_path()?;
    let mut store = read_store(&path)?;
    store.insert(url.to_string(), token.to_string());
    let json = serde_json::to_string_pretty(&store)?;
    write_store(&path, &json)?;

    Ok(())
}

pub fn get_token(url: &Url) -> Result<Option<String>> {
    let path = get_token_store_path().map_err(|e| {
        Error::new(
            ErrorCode::AuthRequired,
            format!("Could not find the credentials store ({e}). Log in first"),
        )
    })?;
    Ok(read_store(&path)?.get(url.as_str()).cloned())
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
        return Err(response_error(base_url, &mut resp, "device login"));
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
        404 => bail!("Unknown device code"),
        code @ (405 | 500..) => Err(status_error(base_url, code, "", "device login")),
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
    let mut resp = http_agent()
        .delete(url.as_str())
        .header("authorization", format!("Bearer {token}"))
        .call()
        .map_err(|e| unreachable(base_url, e))?;
    if !resp.status().is_success() {
        return Err(response_error(base_url, &mut resp, "revoking the token"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unparsable_store_is_an_error_and_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        fs::write(&path, "not json").unwrap();
        let error = read_store(&path).unwrap_err();
        assert_eq!(
            crate::backend_error_code(&error),
            Some(ErrorCode::CredentialsUnreadable)
        );
        assert!(path.exists());
    }
}
