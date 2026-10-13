//! Auth for dvs-server

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use etcetera::BaseStrategy;
use fs_err as fs;
use url::Url;

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
