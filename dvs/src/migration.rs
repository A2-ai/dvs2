//! The JSON `dvs-migrate` and dvs-server exchange to move a project's history
//! onto the server, declared once for both sides.

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::legacy::SidecarShape;
use crate::{Compression, ProjectPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreKind {
    Dvs1,
    Dvs2,
}

/// A storage path seen in history, and where the walk looked for it instead
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSeen {
    pub path: String,
    #[serde(rename = "shape")]
    pub kind: StoreKind,
    #[serde(default)]
    pub remapped_to: Option<String>,
}

impl StorageSeen {
    pub fn effective(&self) -> &str {
        self.remapped_to.as_deref().unwrap_or(&self.path)
    }
}

/// Where an entry's recorded user or time came from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionSource {
    AuditLog,
    Sidecar,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryCommit {
    pub id: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: Timestamp,
}

/// One (path, hash) the walk found, with where and from whom it came
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: ProjectPath,
    pub hash: String,
    pub size: u64,
    pub shape: SidecarShape,
    pub sidecar_path: String,
    pub storage_path: Option<String>,
    pub sidecar_user: Option<String>,
    pub recorded_user: String,
    pub recorded_user_source: AttributionSource,
    pub recorded_at: Timestamp,
    pub recorded_at_source: AttributionSource,
    pub message: Option<String>,
    pub commit: EntryCommit,
    pub user_mismatch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryRef {
    pub path: ProjectPath,
    pub hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LookupRequest {
    #[serde(default)]
    pub hashes: Vec<String>,
    #[serde(default)]
    pub entries: Vec<EntryRef>,
    #[serde(default)]
    pub users: Vec<String>,
    /// The walk's commit ids, to tell whether earlier runs walked the same repo (R17)
    #[serde(default)]
    pub commits: Vec<String>,
}

/// What the project already has of a walk: content, migrated versions and directory users
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LookupResponse {
    #[serde(default)]
    pub have: Vec<String>,
    #[serde(default)]
    pub migrated: Vec<EntryRef>,
    #[serde(default)]
    pub unmatched_users: Vec<String>,
    pub compression: Compression,
    /// The project's earlier runs; `None` when there have been none (R16)
    #[serde(default)]
    pub previous: Option<PreviousRuns>,
    /// How many of the request's `commits` earlier runs recorded for their entries (R17)
    #[serde(default)]
    pub known_commits: u64,
}

/// What earlier runs migrated into the project, and who ran the last one
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreviousRuns {
    pub runs: u64,
    pub migrated: u64,
    pub last_run_by: String,
    pub last_run_at: Timestamp,
    /// The repo the latest such run walked, as its report gave it (R17, R25)
    #[serde(default)]
    pub last_source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalkKind {
    Full,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRunRequest {
    pub client_version: String,
    pub walk: WalkKind,
    pub totals: Totals,
    /// Run although no walked commit is one earlier runs recorded (R17)
    #[serde(default)]
    pub other_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRunResponse {
    pub run_id: Uuid,
    /// RFC 3339, as the server writes it
    pub started_at: String,
    pub compression: Compression,
    /// The project's other runs in progress when this one started (R35)
    #[serde(default)]
    pub in_progress: Vec<InProgressRun>,
}

/// Another run of the project with recent activity (R35)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InProgressRun {
    pub started_by: String,
    /// RFC 3339, as the server writes it, like `StartRunResponse::started_at`
    pub started_at: String,
    pub last_activity_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRequest {
    pub run_id: Uuid,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestResponse {
    /// Content the newly recorded migrated versions still need
    pub missing: Vec<String>,
    pub compression: Compression,
    pub recorded: usize,
    pub already_migrated: usize,
}

/// A walk's counts. Keys left out are zero or empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Totals {
    pub sidecars: BTreeMap<SidecarShape, usize>,
    pub entries: usize,
    pub unique_hashes: usize,
    pub bytes: u64,
    pub on_server: usize,
    pub to_upload: usize,
    pub missing: usize,
    pub already_migrated: usize,
    pub user_mismatches: usize,
    /// What a run stored and newly recorded; `None` for a plan
    pub uploaded: Option<usize>,
    pub migrated: Option<usize>,
    /// Every blob file in the stores the walk saw (R22)
    pub storage_bytes: u64,
    /// The stored size of each hash in history found in local storage, once per hash
    pub referenced_bytes: u64,
    /// What the project holds of history after a run; `None` for a plan
    pub on_server_bytes: Option<u64>,
}

/// What a walk covered. Keys left out other than `walk` are empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(default)]
    pub source: String,
    pub walk: WalkKind,
    #[serde(default)]
    pub partial_reasons: Vec<PartialReason>,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub commits: usize,
    #[serde(default)]
    pub storage: Vec<StorageSeen>,
}

/// Why a walk did not see everything
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PartialReason {
    UnreachableRemote {
        #[serde(default)]
        remote: String,
        #[serde(default)]
        url: String,
        #[serde(default)]
        message: String,
    },
    Shallow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Problem {
    Missing {
        hash: String,
        size: u64,
        paths: Vec<String>,
        looked_at: Option<String>,
        /// The storage folder the config named, and whether it exists on this machine
        #[serde(default, skip_serializing_if = "Option::is_none")]
        storage: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        storage_exists: Option<bool>,
    },
    Corrupt {
        hash: String,
        paths: Vec<String>,
        looked_at: Option<String>,
        message: String,
    },
    UploadRefused {
        hash: String,
        paths: Vec<String>,
        looked_at: Option<String>,
        status: u16,
        message: String,
    },
    SizeMismatch {
        hash: String,
        recorded_sizes: Vec<u64>,
        actual_size: Option<u64>,
        looked_at: Option<String>,
    },
    Unrecognized {
        sidecar_path: String,
        commit: String,
        blob: String,
        message: String,
    },
    UnreadableConfig {
        config_path: String,
        commit: String,
        message: String,
    },
    UnmatchedUser {
        user: String,
        entries: usize,
    },
    UnusedStorageMap {
        storage_path: String,
        remapped_to: String,
    },
}

/// A problem's `kind`, for reading it without the rest
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    Missing,
    Corrupt,
    UploadRefused,
    SizeMismatch,
    Unrecognized,
    UnreadableConfig,
    UnmatchedUser,
    UnusedStorageMap,
}

impl Problem {
    pub fn kind(&self) -> ProblemKind {
        match self {
            Problem::Missing { .. } => ProblemKind::Missing,
            Problem::Corrupt { .. } => ProblemKind::Corrupt,
            Problem::UploadRefused { .. } => ProblemKind::UploadRefused,
            Problem::SizeMismatch { .. } => ProblemKind::SizeMismatch,
            Problem::Unrecognized { .. } => ProblemKind::Unrecognized,
            Problem::UnreadableConfig { .. } => ProblemKind::UnreadableConfig,
            Problem::UnmatchedUser { .. } => ProblemKind::UnmatchedUser,
            Problem::UnusedStorageMap { .. } => ProblemKind::UnusedStorageMap,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes `value`, checks it is `json`, and reads `json` back to the same text.
    fn pin<T: Serialize + serde::de::DeserializeOwned>(value: &T, json: &str) {
        assert_eq!(serde_json::to_string(value).unwrap(), json);
        let read: T = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&read).unwrap(), json);
    }

    #[test]
    fn wire_json_is_pinned() {
        let hash = "ab".repeat(32);
        let manifest = ManifestRequest {
            run_id: Uuid::nil(),
            entries: vec![ManifestEntry {
                path: ProjectPath::from_canonical("data/a.csv").unwrap(),
                hash: hash.clone(),
                size: 3,
                shape: SidecarShape::Dvs1V001,
                sidecar_path: "data/a.csv.dvs".into(),
                storage_path: Some("/data/store".into()),
                sidecar_user: None,
                recorded_user: "ann".into(),
                recorded_user_source: AttributionSource::AuditLog,
                recorded_at: "2023-04-05T14:11:12.12Z".parse().unwrap(),
                recorded_at_source: AttributionSource::Sidecar,
                message: None,
                commit: EntryCommit {
                    id: "c1".into(),
                    author_name: "Ann".into(),
                    author_email: "ann@x".into(),
                    author_date: "2024-01-02T03:04:05Z".parse().unwrap(),
                },
                user_mismatch: false,
            }],
        };
        pin(
            &manifest,
            &format!(
                r#"{{"run_id":"00000000-0000-0000-0000-000000000000","entries":[{{"path":"data/a.csv","hash":"{hash}","size":3,"shape":"dvs1-v0.0.1","sidecar_path":"data/a.csv.dvs","storage_path":"/data/store","sidecar_user":null,"recorded_user":"ann","recorded_user_source":"audit_log","recorded_at":"2023-04-05T14:11:12.12Z","recorded_at_source":"sidecar","message":null,"commit":{{"id":"c1","author_name":"Ann","author_email":"ann@x","author_date":"2024-01-02T03:04:05Z"}},"user_mismatch":false}}]}}"#
            ),
        );

        let scope = Scope {
            source: "/repo".into(),
            walk: WalkKind::Partial,
            partial_reasons: vec![PartialReason::Shallow],
            refs: vec!["main".into()],
            commits: 2,
            storage: vec![StorageSeen {
                path: "/data/store".into(),
                kind: StoreKind::Dvs1,
                remapped_to: None,
            }],
        };
        pin(
            &scope,
            r#"{"source":"/repo","walk":"partial","partial_reasons":[{"kind":"shallow"}],"refs":["main"],"commits":2,"storage":[{"path":"/data/store","shape":"dvs1","remapped_to":null}]}"#,
        );

        let totals = Totals {
            sidecars: [(SidecarShape::Dvs1, 1)].into(),
            entries: 1,
            uploaded: Some(1),
            ..Totals::default()
        };
        pin(
            &totals,
            r#"{"sidecars":{"dvs1":1},"entries":1,"unique_hashes":0,"bytes":0,"on_server":0,"to_upload":0,"missing":0,"already_migrated":0,"user_mismatches":0,"uploaded":1,"migrated":null,"storage_bytes":0,"referenced_bytes":0,"on_server_bytes":null}"#,
        );
        let totals = Totals {
            storage_bytes: 30,
            referenced_bytes: 20,
            on_server_bytes: Some(10),
            ..Totals::default()
        };
        assert_eq!(
            serde_json::to_value(&totals).unwrap()["on_server_bytes"],
            10
        );

        let lookup = LookupRequest {
            hashes: vec![hash.clone()],
            commits: vec!["c1".into()],
            ..LookupRequest::default()
        };
        pin(
            &lookup,
            &format!(r#"{{"hashes":["{hash}"],"entries":[],"users":[],"commits":["c1"]}}"#),
        );

        let found = LookupResponse {
            have: Vec::new(),
            migrated: Vec::new(),
            unmatched_users: Vec::new(),
            compression: Compression::Zstd,
            previous: Some(PreviousRuns {
                runs: 2,
                migrated: 42,
                last_run_by: "alice".into(),
                last_run_at: "2026-11-07T10:00:00Z".parse().unwrap(),
                last_source: Some("/old/clone/project".into()),
            }),
            known_commits: 3,
        };
        pin(
            &found,
            r#"{"have":[],"migrated":[],"unmatched_users":[],"compression":"zstd","previous":{"runs":2,"migrated":42,"last_run_by":"alice","last_run_at":"2026-11-07T10:00:00Z","last_source":"/old/clone/project"},"known_commits":3}"#,
        );

        let start = StartRunRequest {
            client_version: "dvs-migrate 0.1.0".into(),
            walk: WalkKind::Full,
            totals: Totals::default(),
            other_source: true,
        };
        assert_eq!(serde_json::to_value(&start).unwrap()["other_source"], true);

        let started = StartRunResponse {
            run_id: Uuid::nil(),
            started_at: "2026-11-05T14:02:11.123Z".into(),
            compression: Compression::None,
            in_progress: vec![InProgressRun {
                started_by: "alice".into(),
                started_at: "2026-11-05T13:00:00Z".into(),
                last_activity_at: "2026-11-05T14:01:00Z".into(),
            }],
        };
        pin(
            &started,
            r#"{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"2026-11-05T14:02:11.123Z","compression":"none","in_progress":[{"started_by":"alice","started_at":"2026-11-05T13:00:00Z","last_activity_at":"2026-11-05T14:01:00Z"}]}"#,
        );

        let problem = Problem::Missing {
            hash: hash.clone(),
            size: 3,
            paths: vec!["data/a.csv".into()],
            looked_at: None,
            storage: None,
            storage_exists: None,
        };
        pin(
            &problem,
            &format!(
                r#"{{"kind":"missing","hash":"{hash}","size":3,"paths":["data/a.csv"],"looked_at":null}}"#
            ),
        );
        assert_eq!(
            serde_json::to_value(problem.kind()).unwrap(),
            serde_json::to_value(&problem).unwrap()["kind"]
        );
    }

    /// Bodies from before R16-R22 still parse, the new keys taking their defaults.
    #[test]
    fn older_bodies_parse_with_defaults() {
        let lookup: LookupRequest = serde_json::from_str(r#"{"hashes":[]}"#).unwrap();
        assert!(lookup.commits.is_empty());

        let found: LookupResponse =
            serde_json::from_str(r#"{"have":[],"compression":"zstd"}"#).unwrap();
        assert_eq!((found.previous, found.known_commits), (None, 0));
        let previous: PreviousRuns = serde_json::from_str(
            r#"{"runs":1,"migrated":1,"last_run_by":"a","last_run_at":"2026-11-07T10:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(previous.last_source, None);

        let start: StartRunRequest =
            serde_json::from_str(r#"{"client_version":"v","walk":"full","totals":{}}"#).unwrap();
        assert!(!start.other_source);
        assert_eq!(start.totals.on_server_bytes, None);

        let started: StartRunResponse = serde_json::from_str(
            r#"{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"t","compression":"none"}"#,
        )
        .unwrap();
        assert!(started.in_progress.is_empty());
    }

    /// A total of the wrong type is refused, not defaulted: the server's 400
    /// `invalid report` relies on it.
    #[test]
    fn totals_of_the_wrong_type_are_refused() {
        for key in ["storage_bytes", "referenced_bytes", "on_server_bytes"] {
            let json = format!(r#"{{"{key}":"5"}}"#);
            assert!(serde_json::from_str::<Totals>(&json).is_err(), "{key}");
        }
    }
}
