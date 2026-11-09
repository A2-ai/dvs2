//! Reading dvs1 projects: `dvs.yaml` and the `.dvs` sidecars of every dvs version.
//! Only migration uses this; normal dvs2 commands never read dvs1 files.

use std::path::{Path, PathBuf};

use anyhow::Result;
use saphyr::{LoadableYamlNode, Yaml};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::backends::local::stored_compression;
use crate::error::{Error, ErrorCode};
use crate::{Compression, FileMetadata, Hashes, ProjectPath};

pub const DVS1_CONFIG_FILE: &str = "dvs.yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SidecarShape {
    #[serde(rename = "dvs1-v0.0.1")]
    Dvs1V001,
    #[serde(rename = "dvs1")]
    Dvs1,
    #[serde(rename = "dvs2")]
    Dvs2,
}

impl SidecarShape {
    /// The name it serializes as
    pub fn as_str(self) -> &'static str {
        match self {
            SidecarShape::Dvs1V001 => "dvs1-v0.0.1",
            SidecarShape::Dvs1 => "dvs1",
            SidecarShape::Dvs2 => "dvs2",
        }
    }
}

/// A dvs1 sidecar of either shape, v0.0.1 keys mapped onto the later names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dvs1Sidecar {
    pub shape: SidecarShape,
    pub blake3: String,
    pub size: u64,
    /// `time_stamp` or `add_time` as written
    pub recorded_at: String,
    pub message: String,
    pub saved_by: String,
}

#[derive(Debug, Clone)]
pub enum Sidecar {
    Dvs1(Dvs1Sidecar),
    Dvs2(FileMetadata),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unrecognized {
    pub message: String,
}

impl std::fmt::Display for Unrecognized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Unrecognized {}

#[derive(Deserialize)]
struct V001 {
    blake3_checksum: String,
    file_size_bytes: u64,
    time_stamp: String,
    message: String,
    saved_by: String,
}

#[derive(Deserialize)]
struct V002 {
    blake3_checksum: String,
    size: u64,
    add_time: String,
    message: String,
    saved_by: String,
}

/// Classifies a `.dvs` file by which shape's required keys it has. Exactly one
/// shape must match: a file that fits none or several is not guessed at. Its
/// hash must be a blake3 one.
pub fn classify_sidecar(bytes: &[u8]) -> Result<Sidecar, Unrecognized> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| Unrecognized {
        message: format!("not JSON: {e}"),
    })?;

    fn try_shape<T: DeserializeOwned>(
        value: &serde_json::Value,
        shape: SidecarShape,
        errors: &mut Vec<String>,
    ) -> Option<T> {
        T::deserialize(value)
            .inspect_err(|e| errors.push(format!("{}: {e}", shape.as_str())))
            .ok()
    }

    let mut errors = Vec::new();
    let mut found = Vec::new();
    if let Some(m) = try_shape::<V001>(&value, SidecarShape::Dvs1V001, &mut errors) {
        found.push(Sidecar::Dvs1(Dvs1Sidecar {
            shape: SidecarShape::Dvs1V001,
            blake3: m.blake3_checksum,
            size: m.file_size_bytes,
            recorded_at: m.time_stamp,
            message: m.message,
            saved_by: m.saved_by,
        }));
    }
    if let Some(m) = try_shape::<V002>(&value, SidecarShape::Dvs1, &mut errors) {
        found.push(Sidecar::Dvs1(Dvs1Sidecar {
            shape: SidecarShape::Dvs1,
            blake3: m.blake3_checksum,
            size: m.size,
            recorded_at: m.add_time,
            message: m.message,
            saved_by: m.saved_by,
        }));
    }
    if let Some(m) = try_shape::<FileMetadata>(&value, SidecarShape::Dvs2, &mut errors) {
        found.push(Sidecar::Dvs2(m));
    }

    if found.len() == 1 {
        let sidecar = found.pop().expect("one shape");
        let hash = match &sidecar {
            Sidecar::Dvs1(m) => &m.blake3,
            Sidecar::Dvs2(m) => &m.hashes.blake3,
        };
        if !is_blake3(hash) {
            return Err(Unrecognized {
                message: format!("invalid blake3 hash {hash:?}"),
            });
        }
        return Ok(sidecar);
    }
    let message = if found.is_empty() {
        format!("matches no sidecar shape ({})", errors.join("; "))
    } else {
        let shapes: Vec<_> = found
            .iter()
            .map(|s| match s {
                Sidecar::Dvs1(m) => m.shape.as_str(),
                Sidecar::Dvs2(_) => SidecarShape::Dvs2.as_str(),
            })
            .collect();
        format!("matches more than one sidecar shape: {}", shapes.join(", "))
    };
    Err(Unrecognized { message })
}

impl Dvs1Sidecar {
    /// The dvs2 sidecar for this file. A dvs1 store holds uncompressed blobs,
    /// so compression is always none.
    pub fn to_metadata(&self) -> Result<FileMetadata> {
        let add_time = self.recorded_at.parse::<jiff::Timestamp>().map_err(|e| {
            Error::new(
                ErrorCode::BadMetadata,
                format!("invalid dvs1 add time {:?}: {e}", self.recorded_at),
            )
        })?;
        Ok(FileMetadata {
            hashes: Hashes {
                blake3: self.blake3.clone(),
                md5: None,
            },
            size: self.size,
            created_by: self.saved_by.clone(),
            add_time,
            compression: Compression::None,
            message: (!self.message.is_empty()).then(|| self.message.clone()),
        })
    }
}

/// dvs1's `dvs.yaml`. v0.0.1 always wrote `permissions` and `group`, later
/// versions only when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dvs1Config {
    pub storage_dir: PathBuf,
    pub permissions: Option<i32>,
    pub group: Option<String>,
}

impl Dvs1Config {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let invalid = |why: String| -> anyhow::Error {
            Error::new(
                ErrorCode::InvalidConfiguration,
                format!("cannot read {DVS1_CONFIG_FILE}: {why}"),
            )
            .into()
        };
        let docs = Yaml::load_from_str(yaml).map_err(|e| invalid(e.to_string()))?;
        let Some(doc) = docs.first().filter(|d| d.is_mapping()) else {
            return Err(invalid("not a mapping".to_string()));
        };
        // A key set to `null` is the same as a missing one
        let key = |name: &str| doc.as_mapping_get(name).filter(|v| !v.is_null());

        let storage_dir = match key("storage_dir") {
            None => return Err(invalid("missing field `storage_dir`".to_string())),
            Some(v) => match v.as_str() {
                Some(dir) => PathBuf::from(dir),
                None => return Err(invalid("storage_dir is not a string".to_string())),
            },
        };
        let permissions = match key("permissions") {
            None => None,
            Some(v) => match v.as_integer().and_then(|p| i32::try_from(p).ok()) {
                Some(permissions) => Some(permissions),
                None => return Err(invalid("permissions is not an integer".to_string())),
            },
        };
        let group = match key("group") {
            None => None,
            Some(v) => match v.as_str() {
                Some(group) => Some(group.to_string()),
                None => return Err(invalid("group is not a string".to_string())),
            },
        };
        Ok(Dvs1Config {
            storage_dir,
            permissions,
            group,
        })
    }
}

/// The data file a dvs1 sidecar describes: `data/a.csv.dvs` is `data/a.csv`.
pub fn sidecar_data_path(path: impl AsRef<Path>) -> Option<ProjectPath> {
    let sidecar = ProjectPath::from_path(path.as_ref()).ok()?;
    let data = sidecar.as_str().strip_suffix(".dvs")?;
    ProjectPath::from_canonical(data).ok()
}

/// A blake3 hash as dvs writes it: 64 lowercase hex digits.
pub fn is_blake3(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Where a blob lives in a dvs1 or dvs2 local store: `<storage_dir>/<hash[..2]>/<hash[2..]>`.
/// Callers are expected to validate the hash first; this only guarantees the
/// result stays inside `storage_dir`.
pub fn blob_path(storage_dir: &Path, hash: &str) -> PathBuf {
    if is_blake3(hash) {
        storage_dir.join(&hash[..2]).join(&hash[2..])
    } else {
        storage_dir.join("invalid-hash")
    }
}

/// The first of `stores` that holds the blob for `hash`: its path, and how it
/// is stored as read from the blob, which can fail. A store may hold a blob
/// compressed or not whatever its sidecar says.
pub fn find_blob(
    stores: &[impl AsRef<Path>],
    hash: &str,
    size: u64,
) -> Option<(PathBuf, Result<Compression>)> {
    for store in stores {
        let path = blob_path(store.as_ref(), hash);
        if path.is_file() {
            let compression = stored_compression(&path, size, hash);
            return Some((path, compression));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash() -> String {
        "ab".repeat(32)
    }

    fn dvs1(shape: SidecarShape) -> Dvs1Sidecar {
        match classify_sidecar(sidecar_json(shape).as_bytes()).unwrap() {
            Sidecar::Dvs1(m) => m,
            Sidecar::Dvs2(_) => panic!("classified as dvs2"),
        }
    }

    fn sidecar_json(shape: SidecarShape) -> String {
        let hash = hash();
        match shape {
            SidecarShape::Dvs1V001 => format!(
                r#"{{"blake3_checksum": "{hash}", "file_size_bytes": 11,
                    "time_stamp": "2023-04-05T14:11:12.345Z", "message": "", "saved_by": "jdoe"}}"#
            ),
            SidecarShape::Dvs1 => format!(
                r#"{{"blake3_checksum": "{hash}", "size": 11, "add_time": "2023-04-05T14:11:12.345Z",
                    "message": "first pull", "saved_by": "jdoe", "extra": 1}}"#
            ),
            SidecarShape::Dvs2 => format!(
                r#"{{"hashes": {{"blake3": "{hash}"}}, "size": 11, "created_by": "jdoe",
                    "add_time": "2024-01-02T03:04:05Z", "compression": "zstd"}}"#
            ),
        }
    }

    #[test]
    fn classifies_dvs1_v001() {
        let m = dvs1(SidecarShape::Dvs1V001);
        assert_eq!(
            m,
            Dvs1Sidecar {
                shape: SidecarShape::Dvs1V001,
                blake3: hash(),
                size: 11,
                recorded_at: "2023-04-05T14:11:12.345Z".to_string(),
                message: String::new(),
                saved_by: "jdoe".to_string(),
            }
        );
    }

    #[test]
    fn classifies_dvs1_ignoring_extra_keys() {
        let m = dvs1(SidecarShape::Dvs1);
        assert_eq!(
            m,
            Dvs1Sidecar {
                shape: SidecarShape::Dvs1,
                blake3: hash(),
                size: 11,
                recorded_at: "2023-04-05T14:11:12.345Z".to_string(),
                message: "first pull".to_string(),
                saved_by: "jdoe".to_string(),
            }
        );
    }

    #[test]
    fn classifies_dvs2() {
        let json = sidecar_json(SidecarShape::Dvs2);
        let Sidecar::Dvs2(m) = classify_sidecar(json.as_bytes()).unwrap() else {
            panic!("not classified as dvs2");
        };
        assert_eq!(m.hashes.blake3, hash());
        assert_eq!(m.compression, Compression::Zstd);
    }

    #[test]
    fn unrecognized_when_no_shape_matches() {
        let hash = hash();
        let wrong_type = format!(
            r#"{{"blake3_checksum": "{hash}", "size": "11", "add_time": "t", "message": "", "saved_by": "u"}}"#
        );
        for bytes in [b"not json".as_slice(), b"[]", wrong_type.as_bytes()] {
            assert!(classify_sidecar(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn unrecognized_when_several_shapes_match() {
        let hash = hash();
        let both = format!(
            r#"{{"blake3_checksum": "{hash}", "file_size_bytes": 1, "time_stamp": "t",
                "size": 1, "add_time": "t", "message": "", "saved_by": "u"}}"#
        );
        let err = classify_sidecar(both.as_bytes()).unwrap_err();
        assert!(err.message.contains("dvs1-v0.0.1, dvs1"), "{err}");
    }

    #[test]
    fn unrecognized_when_hash_is_not_blake3() {
        for shape in [
            SidecarShape::Dvs1V001,
            SidecarShape::Dvs1,
            SidecarShape::Dvs2,
        ] {
            let json = sidecar_json(shape).replace(&hash(), "abc");
            let err = classify_sidecar(json.as_bytes()).unwrap_err();
            assert_eq!(err.message, r#"invalid blake3 hash "abc""#);
        }
    }

    #[test]
    fn shape_serializes_as_report_names() {
        let shapes = [
            SidecarShape::Dvs1V001,
            SidecarShape::Dvs1,
            SidecarShape::Dvs2,
        ];
        let names = serde_json::to_string(&shapes).unwrap();
        assert_eq!(names, r#"["dvs1-v0.0.1","dvs1","dvs2"]"#);
        for shape in shapes {
            assert_eq!(serde_json::to_value(shape).unwrap(), shape.as_str());
        }
    }

    #[test]
    fn to_metadata_maps_dvs1_fields() {
        let m = dvs1(SidecarShape::Dvs1V001).to_metadata().unwrap();
        assert_eq!(m.hashes.blake3, hash());
        assert_eq!(m.size, 11);
        assert_eq!(m.created_by, "jdoe");
        assert_eq!(m.add_time.to_string(), "2023-04-05T14:11:12.345Z");
        assert_eq!(m.compression, Compression::None);
        assert_eq!(m.message, None);

        let m = dvs1(SidecarShape::Dvs1).to_metadata().unwrap();
        assert_eq!(m.message.as_deref(), Some("first pull"));
    }

    #[test]
    fn to_metadata_rejects_bad_add_time() {
        let mut m = dvs1(SidecarShape::Dvs1);
        m.recorded_at = "yesterday".to_string();
        let error = m.to_metadata().unwrap_err();
        assert_eq!(
            crate::backend_error_code(&error),
            Some(ErrorCode::BadMetadata)
        );
    }

    #[test]
    fn reads_dvs1_configs_of_both_versions() {
        let old = Dvs1Config::from_yaml("storage_dir: /data/store\npermissions: 777\ngroup: ''\n")
            .unwrap();
        assert_eq!(old.storage_dir, PathBuf::from("/data/store"));
        assert_eq!(old.permissions, Some(777));
        assert_eq!(old.group.as_deref(), Some(""));

        let new = Dvs1Config::from_yaml("storage_dir: /data/store\n").unwrap();
        assert_eq!((new.permissions, new.group), (None, None));

        assert!(Dvs1Config::from_yaml("permissions: 777\n").is_err());
    }

    #[test]
    fn sidecar_data_path_strips_dvs() {
        assert_eq!(
            sidecar_data_path("data/a.csv.dvs").unwrap().as_str(),
            "data/a.csv"
        );
        assert_eq!(sidecar_data_path("data/a.csv"), None);
        assert_eq!(sidecar_data_path("data/.dvs"), None);
    }

    #[test]
    fn find_blob_takes_the_first_store_holding_it_and_reads_its_compression() {
        let tmp = tempfile::tempdir().unwrap();
        let (empty, full) = (tmp.path().join("empty"), tmp.path().join("full"));
        // Content that is itself a zstd frame, stored raw, and other content
        // stored compressed: neither magic bytes nor the sidecar tell them apart
        let raw = zstd::encode_all(&b"raw"[..], 0).unwrap();
        let plain = b"compressed".to_vec();
        for (content, stored, expected) in [
            (raw.clone(), raw, Compression::None),
            (
                plain.clone(),
                zstd::encode_all(&plain[..], 0).unwrap(),
                Compression::Zstd,
            ),
        ] {
            let hash = blake3::hash(&content).to_hex().to_string();
            let blob = blob_path(&full, &hash);
            fs_err::create_dir_all(blob.parent().unwrap()).unwrap();
            fs_err::write(&blob, stored).unwrap();

            let (path, compression) =
                find_blob(&[&empty, &full], &hash, content.len() as u64).unwrap();
            assert_eq!((path, compression.unwrap()), (blob, expected));
            assert!(find_blob(&[&empty], &hash, content.len() as u64).is_none());
        }
    }

    #[test]
    fn blob_path_splits_hash() {
        let hash = hash();
        assert_eq!(
            blob_path(Path::new("/store"), &hash),
            Path::new("/store").join("ab").join(&hash[2..])
        );

        let escaped = blob_path(Path::new("/store"), "ab/../../../etc/passwd");
        assert!(escaped.starts_with("/store"), "{escaped:?}");
        assert!(
            !escaped
                .components()
                .any(|c| c == std::path::Component::ParentDir),
            "{escaped:?}"
        );
    }
}
