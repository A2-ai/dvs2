//! Reading dvs1 projects: `dvs.yaml` and the `.dvs` sidecars of every dvs version,
//! and those of devious, the Go dvs before dvs1 (`.dvs.yaml`, `.dvsmeta`).
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

/// The names a dvs1 or devious config has at the repo root, in the order to try them.
/// devious before v2.0.0 wrote `.dvs.yaml`.
pub const DVS1_CONFIG_FILES: [&str; 2] = [DVS1_CONFIG_FILE, ".dvs.yaml"];

/// The suffixes of a dvs1 or devious sidecar: devious before v2.0.0 wrote `.dvsmeta`.
pub const SIDECAR_SUFFIXES: [&str; 2] = [".dvs", ".dvsmeta"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SidecarShape {
    #[serde(rename = "dvs1-v0.0.1")]
    Dvs1V001,
    #[serde(rename = "dvs1")]
    Dvs1,
    #[serde(rename = "dvs2")]
    Dvs2,
    /// devious v0.1-v0.7.2: Go field names, only `FileHash` always written
    #[serde(rename = "devious-v0")]
    DeviousV0,
    /// devious v1.0-v1.2.1: `timestamp`, `user`
    #[serde(rename = "devious-v1")]
    DeviousV1,
    /// devious v2.0.0-v2.0.3: `timestamp`, `saved_by`
    #[serde(rename = "devious-v2")]
    DeviousV2,
}

impl SidecarShape {
    /// The name it serializes as
    pub fn as_str(self) -> &'static str {
        match self {
            SidecarShape::Dvs1V001 => "dvs1-v0.0.1",
            SidecarShape::Dvs1 => "dvs1",
            SidecarShape::Dvs2 => "dvs2",
            SidecarShape::DeviousV0 => "devious-v0",
            SidecarShape::DeviousV1 => "devious-v1",
            SidecarShape::DeviousV2 => "devious-v2",
        }
    }
}

/// A dvs1 or devious sidecar of any shape, keys mapped onto dvs1's names.
/// devious v0 may lack all but the hash: a missing size is 0, and a missing
/// time, message or user is empty, so the commit fills them in (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dvs1Sidecar {
    pub shape: SidecarShape,
    pub blake3: String,
    pub size: u64,
    /// `time_stamp`, `add_time`, `timestamp` or `Timestamp` as written. devious
    /// wrote Go's RFC 3339 with nanoseconds and the local offset.
    pub recorded_at: String,
    pub message: String,
    /// `saved_by`, or devious's `user`/`User`
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

// devious wrote Go's `encoding/json` of its `Metadata` struct. Before v1.0 the
// struct had no json tags, so the keys are the Go field names.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DeviousV0 {
    file_hash: String,
    #[serde(default)]
    file_size: u64,
    #[serde(default)]
    timestamp: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    user: String,
}

#[derive(Deserialize)]
struct DeviousV1 {
    blake3_checksum: String,
    file_size_bytes: u64,
    timestamp: String,
    message: String,
    user: String,
}

#[derive(Deserialize)]
struct DeviousV2 {
    blake3_checksum: String,
    file_size_bytes: u64,
    timestamp: String,
    message: String,
    saved_by: String,
}

/// Classifies a `.dvs` or `.dvsmeta` file by which shape's required keys it
/// has. Exactly one shape must match: a file that fits none or several is not
/// guessed at. Its hash must be a blake3 one.
///
/// Extra keys are ignored, as each tool's own reader ignores them. Every shape
/// requires a key no other tool ever wrote alongside it (`FileHash`; `user`;
/// `timestamp` with `saved_by`; `time_stamp`; `add_time` with `saved_by`;
/// `hashes`), so a real sidecar matches one shape only.
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
    if let Some(m) = try_shape::<DeviousV0>(&value, SidecarShape::DeviousV0, &mut errors) {
        found.push(Sidecar::Dvs1(Dvs1Sidecar {
            shape: SidecarShape::DeviousV0,
            blake3: m.file_hash,
            size: m.file_size,
            recorded_at: m.timestamp,
            message: m.message,
            saved_by: m.user,
        }));
    }
    if let Some(m) = try_shape::<DeviousV1>(&value, SidecarShape::DeviousV1, &mut errors) {
        found.push(Sidecar::Dvs1(Dvs1Sidecar {
            shape: SidecarShape::DeviousV1,
            blake3: m.blake3_checksum,
            size: m.file_size_bytes,
            recorded_at: m.timestamp,
            message: m.message,
            saved_by: m.user,
        }));
    }
    if let Some(m) = try_shape::<DeviousV2>(&value, SidecarShape::DeviousV2, &mut errors) {
        found.push(Sidecar::Dvs1(Dvs1Sidecar {
            shape: SidecarShape::DeviousV2,
            blake3: m.blake3_checksum,
            size: m.file_size_bytes,
            recorded_at: m.timestamp,
            message: m.message,
            saved_by: m.saved_by,
        }));
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

/// dvs1's `dvs.yaml`, or devious's `dvs.yaml`/`.dvs.yaml`. v0.0.1 always wrote
/// `permissions` and `group`, later versions only when set. devious wrote
/// `storage-dir` and nothing else.
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

        let storage_dir = match (key("storage_dir"), key("storage-dir")) {
            (None, None) => return Err(invalid("missing field `storage_dir`".to_string())),
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "both storage_dir and storage-dir are set".to_string(),
                ));
            }
            (Some(v), None) | (None, Some(v)) => match v.as_str() {
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

/// The data file a dvs1 or devious sidecar describes: `data/a.csv.dvs` and
/// `data/a.csv.dvsmeta` are `data/a.csv`.
pub fn sidecar_data_path(path: impl AsRef<Path>) -> Option<ProjectPath> {
    let sidecar = ProjectPath::from_path(path.as_ref()).ok()?;
    let data = SIDECAR_SUFFIXES
        .iter()
        .find_map(|suffix| sidecar.as_str().strip_suffix(suffix))?;
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

/// Every place a store may hold the blob for `hash`, in the order to look:
/// `<hh>/<rest>` (dvs1, dvs2 local, devious v2), `<hash>.dvsfile` (devious
/// before v2), then `<hash>` (a devious v2 migrate interrupted between its
/// rename and its move). An invalid hash gets only `blob_path`'s safe path.
pub fn blob_paths(storage_dir: &Path, hash: &str) -> Vec<PathBuf> {
    if !is_blake3(hash) {
        return vec![blob_path(storage_dir, hash)];
    }
    vec![
        blob_path(storage_dir, hash),
        storage_dir.join(format!("{hash}.dvsfile")),
        storage_dir.join(hash),
    ]
}

/// The hash whose blob a store-relative file path holds, in any `blob_paths`
/// layout; `None` for any other file. The inverse of `blob_paths`, for listing
/// a store's blobs.
pub fn blob_hash(relative: &Path) -> Option<String> {
    let parts = relative
        .components()
        .map(|c| match c {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let hash = match parts.as_slice() {
        [hh, rest] if hh.len() == 2 => format!("{hh}{rest}"),
        [name] => name.strip_suffix(".dvsfile").unwrap_or(name).to_string(),
        _ => return None,
    };
    is_blake3(&hash).then_some(hash)
}

/// The first of `stores` that holds the blob for `hash`, trying each store's
/// `blob_paths` in turn: its path, and how it is stored as read from the blob,
/// which can fail. A store may hold a blob compressed or not whatever its
/// sidecar says.
pub fn find_blob(
    stores: &[impl AsRef<Path>],
    hash: &str,
    size: u64,
) -> Option<(PathBuf, Result<Compression>)> {
    for store in stores {
        for path in blob_paths(store.as_ref(), hash) {
            if path.is_file() {
                let compression = stored_compression(&path, size, hash);
                return Some((path, compression));
            }
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

    const SHAPES: [SidecarShape; 6] = [
        SidecarShape::Dvs1V001,
        SidecarShape::Dvs1,
        SidecarShape::Dvs2,
        SidecarShape::DeviousV0,
        SidecarShape::DeviousV1,
        SidecarShape::DeviousV2,
    ];

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
            // devious wrote these with Go's `json.MarshalIndent(metadata, "", "  ")`
            // and `time.Time`'s RFC 3339 with nanoseconds and the local offset
            SidecarShape::DeviousV0 => format!(
                "{{\n  \"FileHash\": \"{hash}\",\n  \"FileSize\": 11,\n  \
                 \"Timestamp\": \"2023-08-14T10:11:12.123456789-04:00\",\n  \
                 \"Message\": \"\",\n  \"User\": \"jdoe\"\n}}"
            ),
            SidecarShape::DeviousV1 => format!(
                "{{\n  \"blake3_checksum\": \"{hash}\",\n  \"file_size_bytes\": 11,\n  \
                 \"timestamp\": \"2023-08-15T10:11:12.5-04:00\",\n  \
                 \"message\": \"first pull\",\n  \"user\": \"jdoe\"\n}}"
            ),
            SidecarShape::DeviousV2 => format!(
                "{{\n  \"blake3_checksum\": \"{hash}\",\n  \"file_size_bytes\": 11,\n  \
                 \"timestamp\": \"2023-10-11T16:01:02.987654321Z\",\n  \
                 \"message\": \"\",\n  \"saved_by\": \"jdoe\"\n}}"
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
    fn classifies_devious_sidecars_as_written() {
        let hash = hash();
        let jdoe = |shape, recorded_at: &str, message: &str| Dvs1Sidecar {
            shape,
            blake3: hash.clone(),
            size: 11,
            recorded_at: recorded_at.to_string(),
            message: message.to_string(),
            saved_by: "jdoe".to_string(),
        };
        assert_eq!(
            dvs1(SidecarShape::DeviousV0),
            jdoe(
                SidecarShape::DeviousV0,
                "2023-08-14T10:11:12.123456789-04:00",
                ""
            )
        );
        assert_eq!(
            dvs1(SidecarShape::DeviousV1),
            jdoe(
                SidecarShape::DeviousV1,
                "2023-08-15T10:11:12.5-04:00",
                "first pull"
            )
        );
        assert_eq!(
            dvs1(SidecarShape::DeviousV2),
            jdoe(
                SidecarShape::DeviousV2,
                "2023-10-11T16:01:02.987654321Z",
                ""
            )
        );
    }

    /// devious v0.1-v0.6 wrote `json.NewEncoder(f).Encode(metadata)`: compact,
    /// with only the hash, or the hash and size.
    #[test]
    fn devious_v0_needs_only_the_hash() {
        let hash = hash();
        for (json, size) in [
            (format!("{{\"FileHash\":\"{hash}\"}}\n"), 0),
            (format!("{{\"FileHash\":\"{hash}\",\"FileSize\":11}}\n"), 11),
        ] {
            let Sidecar::Dvs1(m) = classify_sidecar(json.as_bytes()).unwrap() else {
                panic!("classified as dvs2");
            };
            assert_eq!(
                m,
                Dvs1Sidecar {
                    shape: SidecarShape::DeviousV0,
                    blake3: hash.clone(),
                    size,
                    recorded_at: String::new(),
                    message: String::new(),
                    saved_by: String::new(),
                }
            );
        }
    }

    /// dvs1 v0.0.1 and devious v2 differ only in `time_stamp` against
    /// `timestamp`: a file with both is not guessed at.
    #[test]
    fn dvs1_v001_and_devious_v2_are_told_apart_by_their_time_key() {
        let both = sidecar_json(SidecarShape::DeviousV2)
            .replace("\"timestamp\"", "\"time_stamp\": \"t\", \"timestamp\"");
        let err = classify_sidecar(both.as_bytes()).unwrap_err();
        assert!(err.message.contains("dvs1-v0.0.1, devious-v2"), "{err}");
    }

    #[test]
    fn to_metadata_converts_go_times_to_utc() {
        let m = dvs1(SidecarShape::DeviousV1).to_metadata().unwrap();
        assert_eq!(m.add_time.to_string(), "2023-08-15T14:11:12.5Z");
        assert_eq!(m.created_by, "jdoe");
    }

    #[test]
    fn unrecognized_when_hash_is_not_blake3() {
        for shape in SHAPES {
            let json = sidecar_json(shape).replace(&hash(), "abc");
            let err = classify_sidecar(json.as_bytes()).unwrap_err();
            assert_eq!(err.message, r#"invalid blake3 hash "abc""#);
        }
    }

    #[test]
    fn shape_serializes_as_report_names() {
        let names = serde_json::to_string(&SHAPES).unwrap();
        assert_eq!(
            names,
            r#"["dvs1-v0.0.1","dvs1","dvs2","devious-v0","devious-v1","devious-v2"]"#
        );
        for shape in SHAPES {
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
    fn reads_devious_storage_dir_and_refuses_both_spellings() {
        let devious = Dvs1Config::from_yaml("storage-dir: /data/store\n").unwrap();
        assert_eq!(devious.storage_dir, PathBuf::from("/data/store"));

        let error =
            Dvs1Config::from_yaml("storage_dir: /data/a\nstorage-dir: /data/b\n").unwrap_err();
        assert_eq!(
            crate::backend_error_code(&error),
            Some(ErrorCode::InvalidConfiguration)
        );
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
    fn devious_dvsmeta_is_a_sidecar() {
        assert_eq!(
            sidecar_data_path("data/a.csv.dvsmeta").unwrap().as_str(),
            "data/a.csv"
        );
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
    fn find_blob_tries_each_layout_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let content = b"devious".to_vec();
        let hash = blake3::hash(&content).to_hex().to_string();
        let paths = blob_paths(tmp.path(), &hash);
        assert_eq!(
            paths,
            [
                blob_path(tmp.path(), &hash),
                tmp.path().join(format!("{hash}.dvsfile")),
                tmp.path().join(&hash),
            ]
        );
        fs_err::create_dir_all(paths[0].parent().unwrap()).unwrap();
        // Written last to first: each new file is the one found
        for path in paths.iter().rev() {
            fs_err::write(path, &content).unwrap();
            let (found, _) = find_blob(&[tmp.path()], &hash, content.len() as u64).unwrap();
            assert_eq!(&found, path);
        }
    }

    #[test]
    fn blob_hash_reads_every_layout_and_nothing_else() {
        let hash = hash();
        let store = Path::new("/store");
        for path in blob_paths(store, &hash) {
            let relative = path.strip_prefix(store).unwrap();
            assert_eq!(
                blob_hash(relative).as_deref(),
                Some(hash.as_str()),
                "{relative:?}"
            );
        }
        for other in [
            "audit.log.jsonl".to_string(),
            format!("{hash}.tmp"),
            format!("a/b{}", &hash[2..]),
            format!("ab/{hash}"),
            format!("x/ab/{}", &hash[2..]),
            format!("../{hash}"),
        ] {
            assert_eq!(blob_hash(Path::new(&other)), None, "{other}");
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
