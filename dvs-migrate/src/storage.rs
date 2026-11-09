//! Local storage, read-only: where a hash's content is, its bytes, and whether a store
//! could still take adds.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Result;
use dvs::Compression;

use crate::walk::StoreKind;

/// A blob in a local store and how it is stored
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub path: PathBuf,
    pub compression: Compression,
    pub stored_size: u64,
}

/// The first of `candidates` (store root, kind) holding `hash`, and the path looked at:
/// the hit, else the first tried. A dvs2 store may hold a blob compressed or not whatever
/// the sidecar says, so its compression is read from the blob; dvs1 blobs are raw.
pub fn find(
    hash: &str,
    size: u64,
    candidates: &[(&str, StoreKind)],
) -> (Option<Blob>, Option<String>) {
    let mut first = None;
    for (store, kind) in candidates {
        let path = dvs::legacy::blob_path(Path::new(store), hash);
        let meta = std::fs::metadata(&path).ok().filter(|m| m.is_file());
        if let Some(meta) = meta {
            let looked_at = path.to_string_lossy().into_owned();
            // Unreadable here is unreadable at upload too, where it is reported
            let compression = match kind {
                StoreKind::Dvs1 => Compression::None,
                StoreKind::Dvs2 => {
                    dvs::stored_compression(&path, size, hash).unwrap_or(Compression::None)
                }
            };
            let blob = Blob {
                path,
                compression,
                stored_size: meta.len(),
            };
            return (Some(blob), Some(looked_at));
        }
        first.get_or_insert_with(|| path.to_string_lossy().into_owned());
    }
    (None, first)
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
        let blob = dvs::legacy::blob_path(&full, &hash);
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, b"hello").unwrap();
        let (empty, full) = (empty.to_str().unwrap(), full.to_str().unwrap());

        let (found, looked_at) = find(
            &hash,
            5,
            &[(empty, StoreKind::Dvs1), (full, StoreKind::Dvs2)],
        );
        let found = found.unwrap();
        assert_eq!(
            (found.compression, found.stored_size),
            (Compression::None, 5)
        );
        assert_eq!(looked_at.as_deref(), blob.to_str());

        let (found, looked_at) = find(&hash, 5, &[(empty, StoreKind::Dvs1)]);
        assert!(found.is_none());
        assert!(looked_at.unwrap().starts_with(empty));
    }

    #[test]
    fn find_reads_a_dvs2_blobs_compression_from_the_blob() {
        let dir = tempfile::tempdir().unwrap();
        let hash = hello_hash(dir.path());
        let blob = dvs::legacy::blob_path(dir.path(), &hash);
        std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
        std::fs::write(&blob, zstd::encode_all(&b"hello"[..], 0).unwrap()).unwrap();
        let store = dir.path().to_str().unwrap();

        let kind = |kind| find(&hash, 5, &[(store, kind)]).0.unwrap().compression;
        assert_eq!(kind(StoreKind::Dvs2), Compression::Zstd);
        assert_eq!(kind(StoreKind::Dvs1), Compression::None);
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
