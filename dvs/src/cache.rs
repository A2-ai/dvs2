use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use crate::gitignore::add_to_gitignore;
use crate::hashes::Hashes;
use crate::paths::DvsPaths;
use anyhow::{Result, bail};
use fs_err as fs;

/// Filesystem stat used as cache key: mtime + size
#[derive(Debug, Clone, PartialEq)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub size: u64,
}

impl FileStat {
    pub fn from_path(path: &Path) -> Result<Self> {
        let meta = fs::metadata(path)?;
        let mtime = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Ok(Self {
            mtime_ns: mtime.as_nanos() as i64,
            size: meta.len(),
        })
    }
}

// Each thread lazily opens its own read-only connection.
// Keyed by db_path so different HashCache instances (e.g. in tests) get fresh connections.
thread_local! {
    static TL_READER: RefCell<Option<(PathBuf, sqlite::Connection)>> = const { RefCell::new(None) };
}

/// SQLite-backed hash cache with lock-free reads.
///
/// Uses WAL mode with separate per-thread read-only connections (via thread_local)
/// and a single Mutex-protected writer connection for inserts.
/// This allows truly concurrent reads from rayon threads without serialization.
pub struct HashCache {
    db_path: PathBuf,
    writer: Mutex<sqlite::ConnectionThreadSafe>,
}

impl HashCache {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Use a temporary connection to create the schema
        let setup_conn = sqlite::Connection::open(db_path)?;
        setup_conn.execute(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS hash_cache (
                 path      TEXT PRIMARY KEY,
                 mtime_ns  INTEGER NOT NULL,
                 size      INTEGER NOT NULL,
                 blake3    TEXT NOT NULL,
                 md5       TEXT
             );",
        )?;
        drop(setup_conn);

        // Open the thread-safe writer connection
        let mut writer = sqlite::Connection::open_thread_safe(db_path)?;
        writer.execute("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        writer.set_busy_timeout(5000)?;

        Ok(Self {
            db_path: db_path.to_path_buf(),
            writer: Mutex::new(writer),
        })
    }

    /// Run a closure with this thread's read-only connection, creating it if needed.
    fn with_reader<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&sqlite::Connection) -> Result<T>,
    {
        TL_READER.with(|cell| {
            let mut slot = cell.borrow_mut();
            let needs_open = match slot.as_ref() {
                Some((path, _)) => path != &self.db_path,
                None => true,
            };
            if needs_open {
                let conn = sqlite::Connection::open_with_flags(
                    &self.db_path,
                    sqlite::OpenFlags::new().with_read_only(),
                )?;
                *slot = Some((self.db_path.clone(), conn));
            }
            f(&slot.as_ref().unwrap().1)
        })
    }

    pub fn lookup(&self, relative_path: &str, stat: &FileStat) -> Result<Option<Hashes>> {
        self.with_reader(|conn| {
            let mut stmt = conn.prepare(
                "SELECT blake3, md5 FROM hash_cache WHERE path = ? AND mtime_ns = ? AND size = ?",
            )?;
            stmt.bind((1, relative_path))?;
            stmt.bind((2, stat.mtime_ns))?;
            stmt.bind((3, stat.size as i64))?;

            match stmt.next()? {
                sqlite::State::Row => {
                    let blake3 = stmt.read::<String, _>(0)?;
                    let md5: Option<String> = match stmt.read::<sqlite::Value, _>(1)? {
                        sqlite::Value::String(s) => Some(s),
                        _ => None,
                    };
                    Ok(Some(Hashes { blake3, md5 }))
                }
                sqlite::State::Done => Ok(None),
            }
        })
    }

    pub fn insert(&self, relative_path: &str, stat: &FileStat, hashes: &Hashes) -> Result<()> {
        let writer = self.writer.lock().unwrap();
        let mut stmt = writer.prepare(
            "INSERT INTO hash_cache (path, mtime_ns, size, blake3, md5)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(path) DO UPDATE SET mtime_ns=excluded.mtime_ns, size=excluded.size, blake3=excluded.blake3, md5=excluded.md5",
        )?;
        stmt.bind((1, relative_path))?;
        stmt.bind((2, stat.mtime_ns))?;
        stmt.bind((3, stat.size as i64))?;
        stmt.bind((4, hashes.blake3.as_str()))?;
        match &hashes.md5 {
            Some(md5) => stmt.bind((5, md5.as_str()))?,
            None => stmt.bind((5, ()))?,
        }
        stmt.next()?;
        Ok(())
    }
}

/// Compute hashes for a file, using the cache when possible.
///
/// Returns `(Hashes, file_size)`.
pub fn hashes_for_file(
    full_path: &Path,
    relative_path: &str,
    cache: Option<&HashCache>,
) -> Result<(Hashes, u64)> {
    let stat = FileStat::from_path(full_path)?;

    // Try cache lookup (no mutex — each thread has its own reader)
    if let Some(c) = cache {
        match c.lookup(relative_path, &stat) {
            Ok(Some(hashes)) => {
                log::debug!("Cache hit for {relative_path}");
                return Ok((hashes, stat.size));
            }
            Ok(None) => {
                log::debug!("Cache miss for {relative_path}");
            }
            Err(e) => {
                log::warn!("Cache lookup failed for {relative_path}: {e}");
            }
        }
    }

    // Cache miss or no cache — stream-hash the file
    let (hashes, size) = Hashes::compute_from_path(full_path, &[])?;

    // Store in cache (brief mutex lock for writer only)
    if let Some(c) = cache {
        if let Err(e) = c.insert(relative_path, &stat, &hashes) {
            log::warn!("Cache store failed for {relative_path}: {e}");
        }
    }

    Ok((hashes, size))
}

/// Try to open the hash cache, handling corruption gracefully.
///
/// On failure, deletes the DB files and retries once.
/// Also ensures the cache directory is gitignored.
pub fn open_cache(paths: &DvsPaths) -> Result<HashCache> {
    let cache_dir = paths.cache_folder();
    let db_path = cache_dir.join("dvs.db");
    let is_new = !cache_dir.exists();

    let cache = match HashCache::open(&db_path) {
        Ok(cache) => cache,
        Err(e) => {
            log::warn!("Failed to open hash cache: {e}; deleting and retrying");
            fs::remove_dir_all(&cache_dir)?;
            match HashCache::open(&db_path) {
                Ok(cache) => cache,
                Err(e) => bail!("Failed to open hash cache: {e}"),
            }
        }
    };

    if is_new {
        let relative = cache_dir
            .strip_prefix(paths.repo_root())
            .unwrap_or(&cache_dir)
            .to_path_buf();
        add_to_gitignore(paths.repo_root(), &[relative])?;
    }

    Ok(cache)
}

/// Open cache, ignoring errors since the cache is optional and shouldn't block operation
pub(crate) fn try_open_cache(paths: &DvsPaths) -> Option<HashCache> {
    match open_cache(paths) {
        Ok(c) => Some(c),
        Err(e) => {
            log::warn!("Failed to open hash cache: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_cache() -> (TempDir, HashCache) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("cache.db");
        let cache = HashCache::open(&db_path).unwrap();
        (tmp, cache)
    }

    fn sample_hashes() -> Hashes {
        Hashes {
            blake3: "abc123".to_string(),
            md5: None,
        }
    }

    fn sample_stat() -> FileStat {
        FileStat {
            mtime_ns: 1_700_000_000_000_000_000,
            size: 1024,
        }
    }

    #[test]
    fn roundtrip_store_and_lookup() {
        let (_tmp, cache) = setup_cache();
        let stat = sample_stat();
        let hashes = sample_hashes();

        cache.insert("file.txt", &stat, &hashes).unwrap();
        let result = cache.lookup("file.txt", &stat).unwrap();
        assert_eq!(result, Some(hashes));
    }

    #[test]
    fn lookup_returns_none_on_mtime_change() {
        let (_tmp, cache) = setup_cache();
        let stat = sample_stat();
        let hashes = sample_hashes();

        cache.insert("file.txt", &stat, &hashes).unwrap();

        let changed_stat = FileStat {
            mtime_ns: stat.mtime_ns + 1,
            ..stat
        };
        let result = cache.lookup("file.txt", &changed_stat).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn lookup_returns_none_on_size_change() {
        let (_tmp, cache) = setup_cache();
        let stat = sample_stat();
        let hashes = sample_hashes();

        cache.insert("file.txt", &stat, &hashes).unwrap();

        let changed_stat = FileStat {
            size: stat.size + 1,
            ..stat
        };
        let result = cache.lookup("file.txt", &changed_stat).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn store_overwrites_existing_entry() {
        let (_tmp, cache) = setup_cache();
        let stat = sample_stat();
        let hashes1 = sample_hashes();
        let hashes2 = Hashes {
            blake3: "new_blake3".to_string(),
            md5: None,
        };

        cache.insert("file.txt", &stat, &hashes1).unwrap();
        cache.insert("file.txt", &stat, &hashes2).unwrap();

        let result = cache.lookup("file.txt", &stat).unwrap();
        assert_eq!(result, Some(hashes2));
    }

    #[test]
    fn lookup_missing_path_returns_none() {
        let (_tmp, cache) = setup_cache();
        let stat = sample_stat();

        let result = cache.lookup("nonexistent.txt", &stat).unwrap();
        assert_eq!(result, None);
    }
}
