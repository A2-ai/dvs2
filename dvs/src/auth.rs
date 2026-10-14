//! Auth for dvs-server

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use anyhow::{Context, Result};
use etcetera::BaseStrategy;
use fs_err as fs;
use url::Url;

type TokenStore = HashMap<String, String>;

/// Serializes read-modify-write of the store within this process; [`lock_store`]
/// adds a lock between processes. Each write replaces the file whole, so a reader
/// never sees half of one.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Held while a writer reads, changes and replaces the store: this process's lock,
/// then an exclusive lock on `credentials.json.lock` beside the store, which another
/// process (the CLI, an R session) takes too. Both are released on drop.
struct StoreLock {
    _file: std::fs::File,
    _process: MutexGuard<'static, ()>,
}

fn lock_store(path: &Path) -> Result<StoreLock> {
    let process = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = path.parent().expect("the store is in a directory");
    create_private_dir(dir)?;
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("could not open {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("could not lock {}", lock_path.display()))?;
    Ok(StoreLock {
        _file: file,
        _process: process,
    })
}

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
    let _lock = lock_store(path)?;
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
    let _lock = lock_store(path)?;
    let mut store = read_store(path, false)?;
    if store.remove(url.as_str()).is_none() {
        return Ok(false);
    }
    write_store(path, &store)?;
    Ok(true)
}

/// Forget the token for `url` only if it is still `token`: a token stored since (a
/// newer login) is kept.
fn delete_token_if_at(path: &Path, url: &Url, token: &str) -> Result<bool> {
    let _lock = lock_store(path)?;
    let mut store = read_store(path, false)?;
    if store.get(url.as_str()).map(String::as_str) != Some(token) {
        return Ok(false);
    }
    store.remove(url.as_str());
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

/// Forget the token for `url` if it is still `token`, the one being revoked or that
/// the server rejected; a token stored since then, by a newer login, is kept. True
/// when it was forgotten.
pub fn delete_token_if(url: &Url, token: &str) -> Result<bool> {
    delete_token_if_at(&get_token_store_path()?, url, token)
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

    /// Other processes (this test binary run again) store tokens while this one
    /// stores and conditionally deletes its own: no write is lost.
    #[test]
    fn racing_processes_keep_every_write() {
        const WRITES: usize = 40;
        if let Ok(store) = std::env::var("DVS_AUTH_TEST_STORE") {
            let i: usize = std::env::var("DVS_AUTH_TEST_WRITER")
                .unwrap()
                .parse()
                .unwrap();
            for n in 0..WRITES {
                store_token_at(Path::new(&store), &url(i), &format!("t-{i}-{n}")).unwrap();
            }
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        let writers: Vec<_> = (1..=4)
            .map(|i| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "auth::tests::racing_processes_keep_every_write"])
                    .env("DVS_AUTH_TEST_STORE", &path)
                    .env("DVS_AUTH_TEST_WRITER", i.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for n in 0..WRITES {
            let token = format!("t-0-{n}");
            store_token_at(&path, &url(0), &token).unwrap();
            assert!(delete_token_if_at(&path, &url(0), &token).unwrap());
        }
        for mut w in writers {
            assert!(w.wait().unwrap().success());
        }
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), None);
        for i in 1..=4 {
            assert_eq!(
                get_token_at(&path, &url(i)).unwrap(),
                Some(format!("t-{i}-{}", WRITES - 1)),
                "writer {i}"
            );
        }
    }

    #[test]
    fn a_conditional_delete_keeps_a_newer_token() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        store_token_at(&path, &url(0), "old").unwrap();
        store_token_at(&path, &url(1), "other").unwrap();
        store_token_at(&path, &url(0), "new").unwrap();
        assert!(!delete_token_if_at(&path, &url(0), "old").unwrap());
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), Some("new".into()));
        assert!(delete_token_if_at(&path, &url(0), "new").unwrap());
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), None);
        assert!(!delete_token_if_at(&path, &url(0), "new").unwrap());
        assert_eq!(get_token_at(&path, &url(1)).unwrap(), Some("other".into()));
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
