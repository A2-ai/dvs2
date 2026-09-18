//! Auth for dvs-server

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use anyhow::{Result, bail};
use etcetera::BaseStrategy;
use fs_err as fs;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::utils::http_agent;

type TokenStore = HashMap<String, String>;

fn get_token_store_path() -> Result<PathBuf> {
    let d = etcetera::choose_base_strategy()?.config_dir();
    Ok(d.join("dvs").join("credentials.json"))
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

#[derive(Serialize, Deserialize)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
}

pub fn get_client_approve_url(base_url: &Url, user_code: &str) -> Result<Url> {
    let mut u = base_url.join("/device/approve")?;
    u.set_query(Some(&format!("user_code={user_code}")));
    Ok(u)
}

pub fn initiate_device_approval(base_url: &Url) -> Result<DeviceAuthorizationResponse> {
    let url = base_url.join("/api/oauth/device/authorize")?;
    let mut resp = http_agent().post(url.as_str()).send_empty()?;
    if !resp.status().is_success() {
        bail!("Server error: {}", resp.status());
    }
    let r = resp.body_mut().read_json::<DeviceAuthorizationResponse>()?;
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
    let url = base_url.join("/api/oauth/token")?;
    let mut resp = http_agent().post(url.as_str()).send_json(TokenPayload {
        device_code: device_code.to_string(),
    })?;
    match resp.status().as_u16() {
        200 => {
            let r = resp.body_mut().read_json::<TokenResponse>()?;
            Ok(Some(r))
        }
        400 => bail!("User code expired"),
        401 => Ok(None),
        404 => bail!("Unknown device code"),
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
