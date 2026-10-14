//! Auth for dvs-server

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use etcetera::BaseStrategy;
use fs_err as fs;
use url::Url;

use crate::errors::{DvsError, ErrorCode};

type TokenStore = HashMap<String, String>;

/// Serializes read-modify-write of the store within this process; [`lock_store`]
/// adds a lock between processes. Each write replaces the file whole, so a reader
/// never sees half of one.
static STORE_LOCK: Mutex<()> = Mutex::new(());

/// How long a writer waits for the store's locks before giving up with
/// `credentials_busy`, and how often it tries meanwhile.
const LOCK_WAIT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(10)
};
const LOCK_RETRY: Duration = Duration::from_millis(50);

type Check = Box<dyn Fn() -> Result<()>>;

thread_local! {
    static CHECK: RefCell<Option<Check>> = const { RefCell::new(None) };
}

/// Run `work` with `check` consulted by every change to the credentials store made
/// on this thread: while waiting for the store's locks, and again once they are held,
/// just before writing. When it returns an error the change is not made and that
/// error is returned. For a caller that may stop waiting (an interrupted R session),
/// or that runs work it may abandon, which must then change nothing.
pub fn with_check<T>(check: impl Fn() -> Result<()> + 'static, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Check>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CHECK.with(|c| *c.borrow_mut() = previous);
        }
    }
    let previous = CHECK.with(|c| c.borrow_mut().replace(Box::new(check)));
    let _restore = Restore(previous);
    work()
}

fn run_check() -> Result<()> {
    CHECK.with(|c| c.borrow().as_ref().map_or(Ok(()), |check| check()))
}

fn busy() -> anyhow::Error {
    DvsError::new(
        ErrorCode::CredentialsBusy,
        "another dvs process is updating the credentials; try again",
    )
    .into()
}

/// Wait for `try_once` to succeed, checking [`with_check`]'s check between tries, for
/// at most [`LOCK_WAIT`] in all (from `start`).
fn wait_for<T>(start: Instant, mut try_once: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    loop {
        if let Some(held) = try_once()? {
            return Ok(held);
        }
        run_check()?;
        if start.elapsed() >= LOCK_WAIT {
            return Err(busy());
        }
        std::thread::sleep(LOCK_RETRY);
    }
}

/// Held while a writer reads, changes and replaces the store: this process's lock,
/// then an exclusive lock on `credentials.json.lock` beside the store, which another
/// process (the CLI, an R session) takes too. Both are released on drop.
struct StoreLock {
    _file: std::fs::File,
    _process: MutexGuard<'static, ()>,
}

/// Take the store's locks without blocking for long: an error with the code
/// `credentials_busy` after [`LOCK_WAIT`], or the check's error. The check runs
/// once more with both held, so a caller that stopped meanwhile writes nothing.
fn lock_store(path: &Path) -> Result<StoreLock> {
    let start = Instant::now();
    let process = wait_for(start, || match STORE_LOCK.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(TryLockError::Poisoned(p)) => Ok(Some(p.into_inner())),
        Err(TryLockError::WouldBlock) => Ok(None),
    })?;
    let dir = path.parent().expect("the store is in a directory");
    create_private_dir(dir)?;
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    let file = open_private(&lock_path)?;
    wait_for(start, || match file.try_lock() {
        Ok(()) => Ok(Some(())),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("could not lock {}", lock_path.display()))
        }
    })?;
    run_check()?;
    Ok(StoreLock {
        _file: file,
        _process: process,
    })
}

/// The lock file, readable and writable by its owner only: anyone who can open it can
/// lock it, and so hold up every change to the store. One made by an older dvs with
/// wider permissions is narrowed; one that is someone else's is refused.
fn open_private(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Through the open file, so it is the file just opened that is narrowed; only
        // its owner may do that
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| {
                format!(
                    "{} is not yours, or its permissions cannot be narrowed to yours only; \
                     remove it",
                    path.display()
                )
            })?;
    }
    Ok(file)
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

    /// These tests share the store's in-process lock, which one of them holds while
    /// it waits; one at a time, none waits on another's.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_path(store: &Path) -> PathBuf {
        let mut p = store.as_os_str().to_owned();
        p.push(".lock");
        PathBuf::from(p)
    }

    /// Hold the store's lock between processes, as another dvs process would.
    fn hold_lock(store: &Path) -> std::fs::File {
        std::fs::create_dir_all(store.parent().unwrap()).unwrap();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(lock_path(store))
            .unwrap();
        file.lock().unwrap();
        file
    }

    fn url(i: usize) -> Url {
        Url::parse(&format!("https://server-{i}.example/")).unwrap()
    }

    #[test]
    fn concurrent_deletes_keep_the_other_servers() {
        let _serial = serial();
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
        let _serial = serial();
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
    fn a_held_lock_gives_up_with_credentials_busy() {
        let _serial = serial();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        store_token_at(&path, &url(0), "old").unwrap();
        let held = hold_lock(&path);
        let start = Instant::now();
        let e = store_token_at(&path, &url(0), "new").unwrap_err();
        assert_eq!(crate::error_code(&e), Some("credentials_busy"));
        assert!(start.elapsed() >= LOCK_WAIT);
        assert!(delete_token_if_at(&path, &url(0), "old").is_err());
        drop(held);
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), Some("old".into()));
        store_token_at(&path, &url(0), "new").unwrap();
    }

    #[test]
    fn a_check_stops_a_wait_and_a_write() {
        let _serial = serial();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        store_token_at(&path, &url(0), "old").unwrap();

        // Stopping while another process holds the lock: well before LOCK_WAIT
        let held = hold_lock(&path);
        let calls = std::rc::Rc::new(std::cell::Cell::new(0));
        let counted = calls.clone();
        let start = Instant::now();
        let e = with_check(
            move || {
                counted.set(counted.get() + 1);
                if counted.get() >= 3 {
                    anyhow::bail!("stopped")
                }
                Ok(())
            },
            || store_token_at(&path, &url(0), "new"),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "stopped");
        assert!(start.elapsed() < LOCK_WAIT / 2);
        drop(held);

        // Stopped by the time the locks are held: nothing is written
        let e = with_check(
            || anyhow::bail!("stopped"),
            || delete_token_if_at(&path, &url(0), "old"),
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "stopped");
        assert_eq!(get_token_at(&path, &url(0)).unwrap(), Some("old".into()));
        // The check ends with the work
        assert!(delete_token_if_at(&path, &url(0), "old").unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn the_lock_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let _serial = serial();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("dvs/credentials.json");
        store_token_at(&path, &url(0), "t").unwrap();
        assert_eq!(mode(&lock_path(&path)), 0o600);

        // One an older dvs left open to everyone is narrowed
        std::fs::set_permissions(lock_path(&path), std::fs::Permissions::from_mode(0o644)).unwrap();
        delete_token_if_at(&path, &url(0), "t").unwrap();
        assert_eq!(mode(&lock_path(&path)), 0o600);
    }

    #[test]
    fn a_conditional_delete_keeps_a_newer_token() {
        let _serial = serial();
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
        let _serial = serial();
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
