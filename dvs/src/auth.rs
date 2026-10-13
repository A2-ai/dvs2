//! Auth for dvs-server

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use etcetera::BaseStrategy;
use fs_err as fs;
use url::Url;

type TokenStore = HashMap<String, String>;

/// Serializes read-modify-write of the store within this process; each write
/// replaces the file whole, so a reader never sees half of one.
static STORE_LOCK: Mutex<()> = Mutex::new(());

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

/// Write the store to a private temporary file beside it, then rename it into place.
fn write_store(path: &Path, store: &TokenStore) -> Result<()> {
    let dir = path.parent().expect("the store is in a directory");
    create_private_dir(dir)?;
    // A new temporary file is readable by its owner only
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    tmp.write_all(serde_json::to_string_pretty(store)?.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// The store at `path`: empty when there is none. A file that is not a store is an
/// error, unless `replace_corrupt`, when it is treated as empty (so a login can
/// replace it).
fn read_store(path: &Path, replace_corrupt: bool) -> Result<TokenStore> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TokenStore::new()),
        Err(e) => return Err(e.into()),
    };
    match serde_json::from_reader(file) {
        Ok(store) => Ok(store),
        Err(e) if replace_corrupt => {
            log::warn!(
                "Credentials store at {} is corrupted ({e}); replacing it",
                path.display()
            );
            Ok(TokenStore::new())
        }
        Err(e) => Err(anyhow::anyhow!(
            "credentials store at {} is corrupted ({e}); run `dvs login` to replace it",
            path.display()
        )),
    }
}

fn store_token_at(path: &Path, url: &Url, token: &str) -> Result<()> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = read_store(path, true)?;
    store.insert(url.to_string(), token.to_string());
    write_store(path, &store)
}

fn get_token_at(path: &Path, url: &Url) -> Result<Option<String>> {
    match read_store(path, false) {
        Ok(store) => Ok(store.get(url.as_str()).cloned()),
        Err(e) => {
            log::warn!("{e:#}");
            Ok(None)
        }
    }
}

fn delete_token_at(path: &Path, url: &Url) -> Result<bool> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = read_store(path, false)?;
    if store.remove(url.as_str()).is_none() {
        return Ok(false);
    }
    write_store(path, &store)?;
    Ok(true)
}

pub fn store_token(url: &Url, token: &str) -> Result<()> {
    store_token_at(&get_token_store_path()?, url, token)
}

pub fn get_token(url: &Url) -> Result<Option<String>> {
    get_token_at(&get_token_store_path()?, url)
}

/// Forget the token for `url`, keeping every other server's. False when there was none.
pub fn delete_token(url: &Url) -> Result<bool> {
    delete_token_at(&get_token_store_path()?, url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(i: usize) -> Url {
        Url::parse(&format!("https://server-{i}.example/")).unwrap()
    }

    #[test]
    fn concurrent_deletes_keep_the_other_servers() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        for i in 0..5 {
            store_token_at(&path, &url(i), &format!("token-{i}")).unwrap();
        }
        std::thread::scope(|s| {
            for _ in 0..16 {
                s.spawn(|| delete_token_at(&path, &url(0)).unwrap());
            }
        });
        for i in 1..5 {
            assert_eq!(
                get_token_at(&path, &url(i)).unwrap(),
                Some(format!("token-{i}"))
            );
        }
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_corrupt_store_is_kept_by_delete_and_replaced_by_login() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"https://a/\": \"t").unwrap();
        assert!(delete_token_at(&path, &url(0)).is_err());
        assert!(path.is_file(), "delete never removes the store");
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), None);
        store_token_at(&path, &url(1), "fresh").unwrap();
        assert_eq!(
            get_token_at(&path, &url(1)).unwrap(),
            Some("fresh".to_string())
        );
    }
}
