use std::path::Path;
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use crate::gitignore::add_to_gitignore;
use crate::hashes::Hashes;
use crate::paths::DvsPaths;
use anyhow::{Result, bail};
use fs_err as fs;
use rusqlite::Connection;

/// Bump whenever the cache schema changes; a mismatch drops and rebuilds the table.
const SCHEMA_VERSION: i64 = 1;

/// Filesystem stat used as cache key: mtime + ctime + size.
#[derive(Debug, Clone, PartialEq)]
pub struct FileStat {
    pub mtime_ns: i64,
    pub ctime_ns: i64,
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
            ctime_ns: ctime_ns(&meta),
            size: meta.len(),
        })
    }
}

#[cfg(unix)]
fn ctime_ns(meta: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    meta.ctime()
        .saturating_mul(1_000_000_000)
        .saturating_add(meta.ctime_nsec())
}

#[cfg(not(unix))]
fn ctime_ns(_meta: &std::fs::Metadata) -> i64 {
    0
}

/// SQLite-backed hash cache.
/// All errors are non-fatal — callers fall back to re-hashing.
pub struct HashCache {
    conn: Connection,
}

impl HashCache {
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;.
        conn.execute_batch(
            "PRAGMA journal_mode=TRUNCATE;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;",
        )?;

        // The cache is disposable, so on a schema-version mismatch we drop and
        // recreate the table rather than migrate it.
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            conn.execute_batch("DROP TABLE IF EXISTS hash_cache;")?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hash_cache (
                 path      TEXT PRIMARY KEY,
                 mtime_ns  INTEGER NOT NULL,
                 ctime_ns  INTEGER NOT NULL,
                 size      INTEGER NOT NULL,
                 blake3    TEXT NOT NULL,
                 md5       TEXT
             );",
        )?;
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        Ok(Self { conn })
    }

    pub fn lookup(&self, relative_path: &str, stat: &FileStat) -> Result<Option<Hashes>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT blake3, md5 FROM hash_cache \
             WHERE path = ?1 AND mtime_ns = ?2 AND ctime_ns = ?3 AND size = ?4",
        )?;
        let mut rows = stmt.query(rusqlite::params![
            relative_path,
            stat.mtime_ns,
            stat.ctime_ns,
            stat.size as i64,
        ])?;
        match rows.next()? {
            Some(row) => {
                let blake3: String = row.get(0)?;
                let md5: Option<String> = row.get(1)?;
                Ok(Some(Hashes { blake3, md5 }))
            }
            None => Ok(None),
        }
    }

    pub fn insert(&self, relative_path: &str, stat: &FileStat, hashes: &Hashes) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "INSERT INTO hash_cache (path, mtime_ns, ctime_ns, size, blake3, md5)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET mtime_ns=?2, ctime_ns=?3, size=?4, blake3=?5, md5=?6",
        )?;
        stmt.execute(rusqlite::params![
            relative_path,
            stat.mtime_ns,
            stat.ctime_ns,
            stat.size as i64,
            &hashes.blake3,
            hashes.md5,
        ])?;
        Ok(())
    }
}

/// Compute hashes for a file, using the cache when possible.
///
/// Returns `(Hashes, file_size)`.
pub fn hashes_for_file(
    full_path: &Path,
    relative_path: &str,
    cache: Option<&Mutex<HashCache>>,
) -> Result<(Hashes, u64)> {
    let stat = FileStat::from_path(full_path)?;

    // Try cache lookup (brief lock)
    if let Some(mtx) = cache {
        match mtx.lock().unwrap().lookup(relative_path, &stat) {
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

    // Store in cache (brief lock)
    if let Some(mtx) = cache {
        if let Err(e) = mtx.lock().unwrap().insert(relative_path, &stat, &hashes) {
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

/// Open a thread safe cache, ignoring errors when it encounters them since
/// the cache is optional and shouldn't block actual operation
pub(crate) fn try_open_cache(paths: &DvsPaths) -> Option<Mutex<HashCache>> {
    match open_cache(paths) {
        Ok(c) => Some(Mutex::new(c)),
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
            ctime_ns: 1_700_000_000_000_000_000,
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
