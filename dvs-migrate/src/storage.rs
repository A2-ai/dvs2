//! Local storage, read-only: where a hash's content is, its bytes, and whether a store
//! could still take adds.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;
use dvs::Compression;
use dvs::backends::local::stored_compression;
use dvs::legacy::{blob_hash, blob_path, blob_paths, find_blob};

/// A blob in a local store and how it is stored
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub path: PathBuf,
    pub compression: Compression,
    pub stored_size: u64,
}

/// The first of `stores` holding `hash`, and the path looked at: the hit, else the first
/// store's. Its compression is read from the blob, whatever the sidecar or store says.
pub fn find(hash: &str, size: u64, stores: &[&str]) -> (Option<Blob>, Option<String>) {
    let first = stores.first().map(|store| {
        blob_path(Path::new(store), hash)
            .to_string_lossy()
            .into_owned()
    });
    let Some((path, compression)) = find_blob(stores, hash, size) else {
        return (None, first);
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return (None, first);
    };
    // A size of 0 may be unknown (devious v0 wrote none): with the blob's own size the
    // check falls through to hashing, which tells raw zstd-looking content from compressed
    let compression = match compression {
        Ok(Compression::Zstd) if size == 0 => stored_compression(&path, meta.len(), hash),
        compression => compression,
    };
    let looked_at = path.to_string_lossy().into_owned();
    let blob = Blob {
        path,
        // Unreadable here is unreadable at upload too, where it is reported
        compression: compression.unwrap_or(Compression::None),
        stored_size: meta.len(),
    };
    (Some(blob), Some(looked_at))
}

/// A blob file in a store, as listed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// Store-relative, with `/` separators
    pub path: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// What a store holds, from file names and sizes only: no content is read
#[derive(Debug, Default)]
pub struct Listing {
    /// Every blob file by hash, in the order `find_blob` tries them: a hash in more than
    /// one layout has the copy `find_blob` finds first
    pub blobs: BTreeMap<String, Vec<Stored>>,
    /// Every blob file, a hash's other layouts included
    pub bytes: u64,
    /// Store-relative paths of everything else, folders below the blob depth included
    pub other_files: Vec<String>,
    /// What could not be read and was skipped, for stderr
    pub warnings: Vec<String>,
}

impl Listing {
    /// The size of the copy of `hash` that `find_blob` finds first
    pub fn size(&self, hash: &str) -> Option<u64> {
        Some(self.blobs.get(hash)?.first()?.size)
    }
}

/// Lists a store's blobs in every layout `find_blob` reads. Blobs are at depth 1
/// (`<hash>.dvsfile`, `<hash>`) or 2 (`<hh>/<rest>`), so nothing deeper is read.
/// Only a root that cannot be read or listed fails: a folder or file in it that cannot
/// (one removed while listing, say) is skipped with a warning.
pub fn list(store: &Path) -> std::io::Result<Listing> {
    let mut listing = Listing::default();
    let mut found: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    let mut other_files = Vec::new();
    let mut skip = |path: &Path, e: std::io::Error| {
        listing.warnings.push(format!(
            "warning: cannot read {}: {e}; skipped",
            path.display()
        ));
    };
    for top in std::fs::read_dir(store)? {
        // Failing partway through the root fails the store, as failing to open it does (R30)
        let top = top?;
        let file_type = match top.file_type() {
            Ok(file_type) => file_type,
            Err(e) => {
                skip(&top.path(), e);
                continue;
            }
        };
        if file_type.is_dir() {
            let inner = match std::fs::read_dir(top.path()) {
                Ok(inner) => inner,
                Err(e) => {
                    skip(&top.path(), e);
                    continue;
                }
            };
            for inner in inner {
                let inner = match inner {
                    Ok(inner) => inner,
                    Err(e) => {
                        skip(&top.path(), e);
                        break;
                    }
                };
                let relative = Path::new(&top.file_name()).join(inner.file_name());
                match inner.file_type().and_then(|t| Ok((t, inner.metadata()?))) {
                    Ok((t, meta)) if t.is_file() => found.push((relative, meta)),
                    Ok(_) => other_files.push(relative),
                    Err(e) => skip(&inner.path(), e),
                }
            }
        } else if file_type.is_file() {
            match top.metadata() {
                Ok(meta) => found.push((PathBuf::from(top.file_name()), meta)),
                Err(e) => skip(&top.path(), e),
            }
        } else {
            other_files.push(PathBuf::from(top.file_name()));
        }
    }

    let slashed = |p: &Path| p.to_string_lossy().replace('\\', "/");
    for (relative, meta) in found {
        match blob_hash(&relative) {
            Some(hash) => {
                listing.bytes += meta.len();
                listing.blobs.entry(hash).or_default().push(Stored {
                    path: slashed(&relative),
                    size: meta.len(),
                    modified: meta.modified().ok(),
                });
            }
            None => other_files.push(relative),
        }
    }
    for (hash, copies) in listing.blobs.iter_mut() {
        let order: Vec<String> = blob_paths(Path::new(""), hash)
            .iter()
            .map(|p| slashed(p))
            .collect();
        copies.sort_by_key(|copy| order.iter().position(|p| *p == copy.path));
    }
    listing.other_files = other_files.iter().map(|p| slashed(p)).collect();
    listing.other_files.sort();
    Ok(listing)
}

/// The uncompressed bytes, as the server takes them
pub fn open(blob: &Blob) -> Result<Box<dyn Read + Send>> {
    let file = File::open(&blob.path)?;
    Ok(match blob.compression {
        Compression::None => Box::new(file),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(file)?),
    })
}

/// The parts of a store an add would write to and the current user may write: the root,
/// its two-character blob folders and a dvs2 audit log. The check creates nothing.
pub fn writable_parts(store: &Path) -> Vec<PathBuf> {
    let mut parts = vec![store.to_path_buf(), store.join("audit.log.jsonl")];
    if let Ok(dirs) = std::fs::read_dir(store) {
        parts.extend(
            dirs.flatten()
                .filter(|d| d.file_name().len() == 2 && d.path().is_dir())
                .map(|d| d.path()),
        );
    }
    parts.into_iter().filter(|p| can_write(p)).collect()
}

#[cfg(unix)]
fn can_write(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_ok()
}

#[cfg(not(unix))]
fn can_write(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| !m.permissions().readonly())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_hash(dir: &Path) -> String {
        let path = dir.join("hello");
        std::fs::write(&path, b"hello").unwrap();
        dvs::Hashes::compute_from_path(&path, &[]).unwrap().0.blake3
    }

    #[test]
    fn find_takes_the_first_store_holding_the_hash() {
        let dir = tempfile::tempdir().unwrap();
        let hash = hello_hash(dir.path());
        let (empty, full) = (dir.path().join("empty"), dir.path().join("full"));
        let blob = blob_path(&full, &hash);
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, b"hello").unwrap();
        let (empty, full) = (empty.to_str().unwrap(), full.to_str().unwrap());

        let (found, looked_at) = find(&hash, 5, &[empty, full]);
        let found = found.unwrap();
        assert_eq!(
            (found.compression, found.stored_size),
            (Compression::None, 5)
        );
        assert_eq!(looked_at.as_deref(), blob.to_str());

        let (found, looked_at) = find(&hash, 5, &[empty]);
        assert!(found.is_none());
        assert!(looked_at.unwrap().starts_with(empty));
    }

    #[test]
    fn find_reads_any_stores_compression_from_the_blob() {
        let dir = tempfile::tempdir().unwrap();
        let hash = hello_hash(dir.path());
        let blob = blob_path(dir.path(), &hash);
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, zstd::encode_all(&b"hello"[..], 0).unwrap()).unwrap();

        let (found, _) = find(&hash, 5, &[dir.path().to_str().unwrap()]);
        assert_eq!(found.unwrap().compression, Compression::Zstd);
    }

    #[test]
    fn list_reads_every_blob_layout_two_levels_deep() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let (nested, flat, bare) = ("ab".repeat(32), "cd".repeat(32), "ef".repeat(32));
        std::fs::create_dir_all(store.join("ab").join("deeper")).unwrap();
        std::fs::write(blob_path(store, &nested), b"12345").unwrap();
        // The same hash also left flat by an interrupted devious migrate
        std::fs::write(store.join(format!("{nested}.dvsfile")), b"1234567").unwrap();
        std::fs::write(store.join(format!("{flat}.dvsfile")), b"123").unwrap();
        std::fs::write(store.join(&bare), b"1").unwrap();
        std::fs::write(store.join("audit.log.jsonl"), b"log").unwrap();
        std::fs::write(store.join("ab").join("deeper").join(&bare[2..]), b"x").unwrap();

        let listing = list(store).unwrap();
        let sizes: Vec<(&str, u64)> = listing
            .blobs
            .keys()
            .map(|h| (h.as_str(), listing.size(h).unwrap()))
            .collect();
        assert_eq!(
            sizes,
            [(nested.as_str(), 5), (flat.as_str(), 3), (bare.as_str(), 1)]
        );
        // Every copy, the one find_blob finds first first
        let copies: Vec<&str> = listing.blobs[&nested]
            .iter()
            .map(|c| c.path.as_str())
            .collect();
        assert_eq!(
            copies,
            [format!("ab/{}", &nested[2..]), format!("{nested}.dvsfile")]
        );
        assert_eq!(listing.bytes, 5 + 7 + 3 + 1);
        assert_eq!(listing.other_files, ["ab/deeper", "audit.log.jsonl"]);
        assert!(list(&store.join("gone")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_skipped_and_the_rest_listed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path();
        let (shut, open) = ("ab".repeat(32), "cd".repeat(32));
        for hash in [&shut, &open] {
            let path = blob_path(store, hash);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"123").unwrap();
        }
        let folder = store.join("ab");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o000)).unwrap();
        let listing = list(store);
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();

        let listing = listing.unwrap();
        assert_eq!(listing.blobs.keys().collect::<Vec<_>>(), [&open]);
        assert_eq!(listing.bytes, 3);
        let [warning] = listing.warnings.as_slice() else {
            panic!("{:?}", listing.warnings)
        };
        assert!(
            warning.starts_with(&format!("warning: cannot read {}", folder.display())),
            "{warning}"
        );
    }

    #[test]
    fn open_decompresses_zstd_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        std::fs::write(&path, zstd::encode_all(&b"hello"[..], 0).unwrap()).unwrap();
        let blob = Blob {
            path,
            compression: Compression::Zstd,
            stored_size: 0,
        };
        let mut bytes = Vec::new();
        open(&blob).unwrap().read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[cfg(unix)]
    #[test]
    fn writable_parts_lists_what_could_still_take_an_add() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(store.join("ab")).unwrap();
        assert_eq!(
            writable_parts(&store),
            vec![store.clone(), store.join("ab")]
        );

        for p in [store.join("ab"), store.clone()] {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        assert!(writable_parts(&store).is_empty());
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(store.join("ab"), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
