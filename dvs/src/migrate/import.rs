//! `dvs migrate server --import-history`: moves a dvs2-local project onto the server by
//! replaying its store's audit log through the migration endpoints, then switches the
//! checkout as `dvs migrate server` does (log import D1-D10). The store and its log are
//! only read (I2).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use anyhow::Result;
use fs_err as fs;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use super::{Destination, MigrateOptions, MigrateReport, Prepared, Source, migrate, prepare};
use crate::audit::{Action, AuditEntry, parse_audit_log};
use crate::backends::server::{join_url, status_error};
use crate::error::{Error, ErrorCode};
use crate::legacy::{self, SidecarShape};
use crate::migration::{
    AttributionSource, Content, EntryRef, InProgressRun, LookupRequest, LookupResponse,
    ManifestEntry, ManifestRequest, ManifestResponse, Problem, ReportEntry, Scope, StartRunRequest,
    StartRunResponse, StorageSeen, StoreKind, Totals, WalkKind, chunks,
};
use crate::operation::{duration_ms, rfc3339_ms};
use crate::retry::{After, Answer, Endpoints, Retrying};
use crate::{Compression, ProjectPath, RunOutcome};

const AUDIT_LOG: &str = "audit.log.jsonl";
const NOT_ADMIN: &str = "only a server admin can import history; ask an admin to run this";

/// A refusal (I5, C3): exit status 2, nothing sent that writes and nothing written
#[derive(Debug)]
pub struct Refused(pub Error);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for Refused {}

fn refuse(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    Refused(Error::new(code, message)).into()
}

/// The context of a failure after the run opened: the checkout is not switched (W2)
#[derive(Debug)]
pub struct Interrupted {
    pub migrated: usize,
    pub of: usize,
}

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "interrupted after {} of {} versions; run again to carry on",
            self.migrated, self.of
        )
    }
}

/// An import that passed every refusal (D3, D4), before anything is sent or written
pub struct PreparedImport {
    prepared: Prepared,
    name: String,
    url: Url,
    store: PathBuf,
    /// Sized once the store is read (L3, L8)
    entries: Vec<ManifestEntry>,
    /// Every blob file in the store, by hash: its file's size
    blobs: BTreeMap<String, u64>,
}

/// What `--import-history` does in a checkout that passed every refusal
pub enum ImportPlan {
    Import(PreparedImport),
    /// The checkout already points at this server and project and has no local store: it
    /// goes the way of plain `dvs migrate server`, which changes nothing there (L11)
    AlreadyMigrated(Prepared),
}

/// Refuses what the import cannot take, reading only the checkout, the store's listing
/// and its log (I5). A checkout on another server is `prepare`'s refusal.
pub fn prepare_import(
    cwd: &Path,
    name: String,
    url: Url,
    project_path: Option<&Path>,
    clean: bool,
) -> Result<ImportPlan> {
    let destination = Destination::Server {
        name: name.clone(),
        url: url.clone(),
        storage: None,
    };
    let options = MigrateOptions {
        clean,
        upload_missing: false,
    };
    let prepared = prepare(cwd, &destination, options)?;
    let store = match (prepared.source, &prepared.replaced_store) {
        (Source::Dvs2Local, Some(store)) => store.clone(),
        (Source::Dvs2Server, _) => return Ok(ImportPlan::AlreadyMigrated(prepared)),
        _ => {
            return Err(refuse(
                ErrorCode::InvalidArguments,
                "--import-history imports a dvs2 local store's audit log, and this checkout is \
                 dvs1 or devious: use dvs-migrate to move its history",
            ));
        }
    };

    let log_path = store.join(AUDIT_LOG);
    let has_log = log_path.is_file();
    let log = if has_log {
        let reader = std::io::BufReader::new(fs::File::open(&log_path)?);
        parse_audit_log(reader, &HashSet::new()).map_err(|e| {
            refuse(
                ErrorCode::InvalidArguments,
                format!(
                    "cannot read the audit log {}: {e:#}; use dvs-migrate to migrate this project",
                    log_path.display()
                ),
            )
        })?
    } else {
        Vec::new()
    };
    let blobs = list_blobs(&store)?;

    // D3: the log starts with an init, and names every blob in the store
    let named: HashSet<&str> = log
        .iter()
        .filter_map(|e| match &e.action {
            Action::Add { file, .. } => Some(file.hashes.blake3.as_str()),
            Action::Init { .. } => None,
        })
        .collect();
    let mut found = Vec::new();
    if !has_log {
        found.push("it has no audit log".to_string());
    } else if !matches!(log.first().map(|e| &e.action), Some(Action::Init { .. })) {
        found.push("its audit log does not start with an init entry".to_string());
    }
    let unnamed: Vec<&String> = blobs
        .keys()
        .filter(|hash| !named.contains(hash.as_str()))
        .collect();
    if !unnamed.is_empty() {
        found.push(format!(
            "{} {} in the store that no add in its log names",
            unnamed.len(),
            if unnamed.len() == 1 { "blob" } else { "blobs" }
        ));
    }
    if !found.is_empty() {
        return Err(refuse(
            ErrorCode::InvalidArguments,
            format!(
                "the store {} holds content from before dvs2: {}; use dvs-migrate to migrate this project",
                store.display(),
                found.join("; ")
            ),
        ));
    }

    // D4, L2: one project, or `--project-path` names which of the log's this checkout is
    let mut projects: Vec<&Path> = Vec::new();
    for entry in &log {
        if let Action::Init { project_path, .. } = &entry.action
            && !projects.contains(&project_path.as_path())
        {
            projects.push(project_path);
        }
    }
    let listed = || {
        let paths: Vec<String> = projects.iter().map(|p| p.display().to_string()).collect();
        paths.join(", ")
    };
    // The log's init paths are canonical, so the given one is too before comparing
    let project_path = project_path
        .map(|given| fs::canonicalize(given).or_else(|_| std::path::absolute(given)))
        .transpose()?;
    match project_path.as_deref() {
        Some(given) if !projects.contains(&given) => {
            return Err(refuse(
                ErrorCode::InvalidArguments,
                format!(
                    "--project-path {} is not a project the store's audit log names: {}",
                    given.display(),
                    listed()
                ),
            ));
        }
        None if projects.len() > 1 => {
            return Err(refuse(
                ErrorCode::InvalidArguments,
                format!(
                    "the store's audit log names {} projects: {}; if they are this one, say which \
                     path this checkout is with --project-path",
                    projects.len(),
                    listed()
                ),
            ));
        }
        _ => {}
    }

    let entries = manifest_entries(&log, &store, prepared.config.metadata_folder_name())?;
    Ok(ImportPlan::Import(PreparedImport {
        prepared,
        name,
        url,
        store,
        entries,
        blobs,
    }))
}

/// Every blob file in `store`, by hash, from names and sizes only (D3). The store is read
/// in any layout `legacy::blob_hash` knows, so content from before dvs2 shows.
fn list_blobs(store: &Path) -> Result<BTreeMap<String, u64>> {
    let mut blobs = BTreeMap::new();
    for top in fs::read_dir(store)? {
        let top = top?;
        let file_type = top.file_type()?;
        let mut found = Vec::new();
        if file_type.is_dir() {
            for inner in fs::read_dir(top.path())? {
                let inner = inner?;
                if inner.file_type()?.is_file() {
                    let relative = Path::new(&top.file_name()).join(inner.file_name());
                    found.push((relative, inner.metadata()?.len()));
                }
            }
        } else if file_type.is_file() {
            found.push((PathBuf::from(top.file_name()), top.metadata()?.len()));
        }
        for (relative, size) in found {
            if let Some(hash) = legacy::blob_hash(&relative) {
                blobs.insert(hash, size);
            }
        }
    }
    Ok(blobs)
}

/// A blob in the store that decodes to its hash
#[derive(Debug, Clone)]
struct Blob {
    path: PathBuf,
    compression: Compression,
    /// Decompressed (L3)
    size: u64,
    stored_size: u64,
}

/// What the store holds of one hash
#[derive(Debug, Clone)]
enum Local {
    Found(Blob),
    Absent,
    Unreadable(String),
    /// The blob does not decode to its hash
    Mismatch,
}

/// The `<hh>/<rest>` blob of `hash` and its decompressed size. A zstd blob is decoded and
/// checked against its hash; an uncompressed one is left to the server's check.
fn read_blob(store: &Path, hash: &str) -> Local {
    #[cfg(test)]
    tests::BLOB_READS.with(|n| n.set(n.get() + 1));
    let path = legacy::blob_path(store, hash);
    let Ok(stored_size) = fs::metadata(&path).map(|m| m.len()) else {
        return Local::Absent;
    };
    let compression = match crate::stored_compression(&path, stored_size, hash) {
        Ok(compression) => compression,
        Err(e) => return Local::Unreadable(format!("{e:#}")),
    };
    let size = match compression {
        Compression::None => stored_size,
        Compression::Zstd => {
            let mut hasher = blake3::Hasher::new();
            let decoded = fs::File::open(&path)
                .map_err(anyhow::Error::from)
                .and_then(|file| Ok(zstd::stream::read::Decoder::new(file)?))
                .and_then(|mut decoder| Ok(std::io::copy(&mut decoder, &mut hasher)?));
            match decoded {
                Ok(size) if hasher.finalize().to_hex().as_str() == hash => size,
                // Bytes that do not decode are not this content either
                Ok(_) | Err(_) => return Local::Mismatch,
            }
        }
    };
    Local::Found(Blob {
        path,
        compression,
        size,
        stored_size,
    })
}

/// The uncompressed bytes, as the server takes them (A7)
fn open(blob: &Blob) -> Result<Box<dyn Read + Send>> {
    let file = fs::File::open(&blob.path)?;
    Ok(match blob.compression {
        Compression::None => Box::new(file),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(file)?),
    })
}

/// One manifest entry per logged add, with its operation, in the log's order (D2, D5, L1,
/// L10): a file set back to an earlier version is a version again. A path dvs cannot take
/// is a refusal (L8). Sizes are 0 until the store is read (L3).
fn manifest_entries(
    log: &[AuditEntry],
    store: &Path,
    metadata_folder: &str,
) -> Result<Vec<ManifestEntry>> {
    let mut entries = Vec::new();
    for e in log {
        let Action::Add { file, message, .. } = &e.action else {
            continue;
        };
        let path = ProjectPath::from_path(&file.path).map_err(|err| {
            refuse(
                ErrorCode::InvalidArguments,
                format!(
                    "the store's audit log adds {}, a path dvs cannot take ({err}): operation {} \
                     by {} at {}",
                    file.path.display(),
                    e.operation_id,
                    e.user,
                    e.timestamp
                ),
            )
        })?;
        let hash = file.hashes.blake3.clone();
        entries.push(ManifestEntry {
            sidecar_path: format!("{metadata_folder}/{}.dvs", path.as_str()),
            path,
            hash,
            size: 0,
            shape: SidecarShape::Dvs2,
            storage_path: Some(store.display().to_string()),
            sidecar_user: None,
            recorded_user: e.user.clone(),
            recorded_user_source: AttributionSource::AuditLog,
            recorded_at: e.timestamp,
            recorded_at_source: AttributionSource::AuditLog,
            message: message.clone().filter(|m| !m.is_empty()),
            commit: None,
            user_mismatch: false,
            operation: Some(e.operation_id.clone()),
        });
    }
    Ok(entries)
}

/// The report document (K12): what `--json` prints, and, without `entries`, what the
/// `report` call sends
#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub v: u32,
    pub command: &'static str,
    pub outcome: RunOutcome,
    pub dry_run: bool,
    pub error: Option<Value>,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub run_id: Option<Uuid>,
    pub scope: Scope,
    pub totals: Totals,
    pub entries: Vec<ReportEntry>,
    pub problems: Vec<Problem>,
    /// Why each blob that could not be read was not, by hash
    #[serde(skip)]
    pub unreadable: BTreeMap<String, String>,
}

impl ImportReport {
    fn for_server(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("a report serializes");
        if let Some(map) = value.as_object_mut() {
            map.remove("entries");
        }
        value
    }
}

/// An import whose report the server stored, and the switch after it
pub struct Imported {
    pub report: ImportReport,
    pub switch: MigrateReport,
    /// For stderr: other runs of the project in progress (R35), and blobs that could not
    /// be read
    pub notes: Vec<String>,
    pub migration_page: String,
}

#[derive(Default)]
struct HashInfo {
    paths: Vec<String>,
    local: Option<Local>,
    on_server: bool,
    after: Option<After>,
}

impl HashInfo {
    fn found(&self) -> Option<&Blob> {
        match &self.local {
            Some(Local::Found(blob)) => Some(blob),
            _ => None,
        }
    }

    fn content(&self) -> Content {
        match (&self.after, self.on_server, self.found()) {
            (_, true, _) => Content::OnServer,
            (Some(After::Uploaded { .. }), ..) => Content::Uploaded,
            (Some(After::Corrupt(_)), ..) => Content::Corrupt,
            (Some(After::Refused(..)), ..) => Content::Refused,
            (Some(After::Unreadable(_)), ..) => Content::Missing,
            (None, _, Some(_)) => Content::ToUpload,
            (None, _, None) => match self.local {
                // Not read yet: the run starts before the store is read (L8)
                None => Content::ToUpload,
                Some(Local::Mismatch) => Content::Corrupt,
                _ => Content::Missing,
            },
        }
    }

    fn missing(&self) -> bool {
        matches!(
            self.content(),
            Content::Missing | Content::Corrupt | Content::Refused
        )
    }
}

/// What the import knows and has done
struct Run {
    store: PathBuf,
    entries: Vec<ManifestEntry>,
    hashes: BTreeMap<String, HashInfo>,
    migrated: HashSet<EntryRef>,
    unmatched_users: Vec<String>,
    storage_bytes: u64,
    /// Versions the server recorded as migrated
    recorded: usize,
    /// Of those, the ones whose content the server has
    completed: usize,
    /// Recorded versions waiting for their blob, by hash
    pending: HashMap<String, usize>,
}

impl Run {
    fn already_migrated(&self, e: &ManifestEntry) -> bool {
        self.migrated.contains(&e.entry_ref())
    }

    /// The distinct versions this run had to migrate
    fn to_migrate(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| !self.already_migrated(e))
            .map(ManifestEntry::entry_ref)
            .collect::<HashSet<_>>()
            .len()
    }

    fn totals(&self, ran: bool) -> Totals {
        let count = |f: &dyn Fn(&HashInfo) -> bool| self.hashes.values().filter(|h| f(h)).count();
        let on_server_bytes = self
            .hashes
            .values()
            .filter(|h| !h.missing())
            .filter_map(|h| h.found().map(|b| b.stored_size))
            .sum();
        Totals {
            entries: self.entries.len(),
            unique_hashes: self.hashes.len(),
            bytes: self
                .hashes
                .values()
                .filter_map(|h| h.found().map(|b| b.size))
                .sum(),
            on_server: count(&|h| h.on_server),
            to_upload: count(&|h| !h.on_server && !h.missing()),
            missing: count(&HashInfo::missing),
            already_migrated: self
                .entries
                .iter()
                .filter(|e| self.already_migrated(e))
                .count(),
            uploaded: ran
                .then(|| count(&|h| matches!(h.after, Some(After::Uploaded { created: true })))),
            migrated: ran.then_some(self.recorded),
            storage_bytes: self.storage_bytes,
            referenced_bytes: self
                .hashes
                .values()
                .filter_map(|h| h.found().map(|b| b.stored_size))
                .sum(),
            on_server_bytes: ran.then_some(on_server_bytes),
            ..Totals::default()
        }
    }

    fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (hash, h) in &self.hashes {
            let looked_at = Some(legacy::blob_path(&self.store, hash).display().to_string());
            let (hash, paths) = (hash.clone(), h.paths.clone());
            match (&h.after, h.content()) {
                (Some(After::Corrupt(message)), _) => problems.push(Problem::Corrupt {
                    hash,
                    paths,
                    looked_at,
                    message: message.clone(),
                }),
                (Some(After::Refused(status, message)), _) => {
                    problems.push(Problem::UploadRefused {
                        hash,
                        paths,
                        looked_at,
                        status: *status,
                        message: message.clone(),
                    })
                }
                (_, Content::Corrupt) => problems.push(Problem::Corrupt {
                    hash,
                    paths,
                    looked_at,
                    message: "the blob in the store does not match its hash".into(),
                }),
                (_, Content::Missing) => problems.push(Problem::Missing {
                    hash,
                    size: 0,
                    paths,
                    looked_at,
                    storage: Some(self.store.display().to_string()),
                    storage_exists: Some(true),
                }),
                _ => {}
            }
        }
        for user in &self.unmatched_users {
            let entries = self
                .entries
                .iter()
                .filter(|e| &e.recorded_user == user)
                .count();
            problems.push(Problem::UnmatchedUser {
                user: user.clone(),
                entries,
            });
        }
        problems
    }
}

/// Imports the log (D2, D5), stores the run's report, and only then switches the checkout
/// (D10). A failure once the run is open leaves the checkout as it was (W2), with
/// `Interrupted` as the error's context.
pub fn import(prepared: PreparedImport, retrying: &Retrying) -> Result<Imported> {
    let (started, started_at) = (Instant::now(), SystemTime::now());
    let PreparedImport {
        prepared,
        name,
        url,
        store,
        entries,
        blobs,
    } = prepared;
    let client = Client::new(url.clone(), name.clone(), retrying.clone())?;

    let mut hashes: BTreeMap<String, HashInfo> = BTreeMap::new();
    for e in &entries {
        let info = hashes.entry(e.hash.clone()).or_default();
        if !info.paths.iter().any(|p| p == e.path.as_str()) {
            info.paths.push(e.path.as_str().to_string());
        }
    }
    let users: BTreeSet<String> = entries.iter().map(|e| e.recorded_user.clone()).collect();
    let refs: BTreeSet<EntryRef> = entries.iter().map(ManifestEntry::entry_ref).collect();
    let found = lookup(
        &client,
        hashes.keys().cloned().collect(),
        refs.into_iter().collect(),
        users.into_iter().collect(),
    )?;
    let have: HashSet<String> = found.have.into_iter().collect();
    for (hash, info) in hashes.iter_mut() {
        info.on_server = have.contains(hash);
    }
    let mut run = Run {
        storage_bytes: blobs.values().sum(),
        store: store.clone(),
        entries,
        hashes,
        migrated: found.migrated.into_iter().collect(),
        unmatched_users: found.unmatched_users,
        recorded: 0,
        completed: 0,
        pending: HashMap::new(),
    };

    let scope = Scope {
        source: store.join(AUDIT_LOG).display().to_string(),
        walk: WalkKind::AuditLog,
        partial_reasons: Vec::new(),
        refs: Vec::new(),
        commits: 0,
        storage: vec![StorageSeen {
            path: store.display().to_string(),
            kind: StoreKind::Dvs2,
            remapped_to: None,
        }],
    };
    // L8: a non-admin (D6) or a project migrated the other way (A3) is refused before
    // the store is read
    let started_run = client.start_run(run.totals(false))?;
    for (hash, info) in run.hashes.iter_mut() {
        info.local = Some(read_blob(&store, hash));
    }
    for e in run.entries.iter_mut() {
        if let Some(blob) = run.hashes[&e.hash].found() {
            e.size = blob.size;
        }
    }
    let interrupted = |run: &Run, e: anyhow::Error| {
        e.context(Interrupted {
            migrated: run.completed,
            of: run.to_migrate(),
        })
    };
    if let Err(e) = send(&mut run, &client, started_run.run_id) {
        return Err(interrupted(&run, e));
    }

    let problems = run.problems();
    let mut report = ImportReport {
        v: 1,
        command: "run",
        outcome: if problems.is_empty() {
            RunOutcome::Complete
        } else {
            RunOutcome::Partial
        },
        dry_run: false,
        error: None,
        started_at: rfc3339_ms(started_at),
        run_id: Some(started_run.run_id),
        scope,
        totals: run.totals(true),
        entries: run
            .entries
            .iter()
            .map(|e| ReportEntry {
                entry: e.clone(),
                content: run.hashes[&e.hash].content(),
                already_migrated: run.already_migrated(e),
            })
            .collect(),
        problems,
        unreadable: BTreeMap::new(),
        finished_at: rfc3339_ms(SystemTime::now()),
        duration_ms: duration_ms(started),
    };
    if let Err(e) = client.report(&report.for_server()) {
        return Err(interrupted(&run, e));
    }

    let mut notes = in_progress_notes(&started_run.in_progress);
    for (hash, h) in &run.hashes {
        let unreadable = match (&h.local, &h.after) {
            (Some(Local::Unreadable(message)), _) | (_, Some(After::Unreadable(message))) => {
                message
            }
            _ => continue,
        };
        let blob = legacy::blob_path(&run.store, hash);
        notes.push(format!(
            "warning: cannot read {}: {unreadable}",
            blob.display()
        ));
        report.unreadable.insert(hash.clone(), unreadable.clone());
    }
    // The history is on the server: only now is the checkout pointed at it (D10)
    let switch = migrate(prepared)?;
    Ok(Imported {
        report,
        switch,
        notes,
        migration_page: join_url(&url, &format!("project/{name}/migration"))
            .map_or_else(|_| url.to_string(), |u| u.to_string()),
    })
}

/// Every entry, then every blob the server lacks that the store has
fn send(run: &mut Run, client: &Client, run_id: Uuid) -> Result<()> {
    let mut wanted = BTreeSet::new();
    for chunk in chunks(&run.entries) {
        let answer = client.manifest(run_id, chunk)?;
        // The new versions whose content the server lacks wait for their upload; the
        // rest of what it recorded completed at once
        let lacking: HashSet<&String> = answer.missing.iter().collect();
        let mut waiting = 0;
        for e in chunk {
            if lacking.contains(&e.hash) && !run.migrated.contains(&e.entry_ref()) {
                *run.pending.entry(e.hash.clone()).or_default() += 1;
                waiting += 1;
            }
        }
        run.completed += answer.recorded.saturating_sub(waiting);
        run.recorded += answer.recorded;
        wanted.extend(answer.missing);
    }

    for hash in wanted {
        let Some(blob) = run.hashes.get(&hash).and_then(|h| h.found()).cloned() else {
            continue;
        };
        let after = client.upload(run_id, &hash, &|| open(&blob))?;
        if let After::Uploaded { .. } = after {
            run.completed += run.pending.remove(&hash).unwrap_or(0);
        }
        run.hashes.get_mut(&hash).expect("listed").after = Some(after);
    }
    Ok(())
}

struct Found {
    have: Vec<String>,
    migrated: Vec<EntryRef>,
    unmatched_users: Vec<String>,
}

/// `lookup` as many times as K10's limits need
fn lookup(
    client: &Client,
    hashes: Vec<String>,
    entries: Vec<EntryRef>,
    users: Vec<String>,
) -> Result<Found> {
    let mut found = Found {
        have: Vec::new(),
        migrated: Vec::new(),
        unmatched_users: Vec::new(),
    };
    for req in LookupRequest::chunked(&hashes, &entries, &users, &[]) {
        let answer = client.lookup(&req)?;
        found.have.extend(answer.have);
        found.migrated.extend(answer.migrated);
        found.unmatched_users.extend(answer.unmatched_users);
    }
    found.unmatched_users.sort();
    Ok(found)
}

/// One note per other run in progress; they never stop this one (R35)
fn in_progress_notes(runs: &[InProgressRun]) -> Vec<String> {
    runs.iter().map(InProgressRun::note).collect()
}

/// The migration endpoints, called with the user's dvs login (A1), retried as R18 (C4)
struct Client {
    endpoints: Endpoints,
}

impl Client {
    fn new(url: Url, project: String, retrying: Retrying) -> Result<Self> {
        let token = crate::auth::get_token(&url)?.ok_or_else(|| {
            Error::new(
                ErrorCode::AuthRequired,
                "Could not find access token. Log in first",
            )
        })?;
        Ok(Self {
            endpoints: Endpoints::new(url, project, token, retrying),
        })
    }

    /// A status the call does not take: 403 `admins only` is C3's refusal
    fn refused(&self, answer: &Answer, call: &str) -> anyhow::Error {
        let body = answer.text();
        if answer.status == 403 && body.starts_with("admins only") {
            return refuse(ErrorCode::AccessDenied, NOT_ADMIN);
        }
        status_error(self.endpoints.url(), answer.status, body, call)
    }

    fn send(&self, call: &str, body: &impl Serialize) -> Result<Answer> {
        self.endpoints.post(call, &serde_json::to_value(body)?)
    }

    fn post<R: DeserializeOwned>(&self, call: &str, body: &impl Serialize, ok: u16) -> Result<R> {
        let answer = self.send(call, body)?;
        if answer.status != ok {
            return Err(self.refused(&answer, call));
        }
        answer.json(self.endpoints.url())
    }

    fn lookup(&self, req: &LookupRequest) -> Result<LookupResponse> {
        self.post("lookup", req, 200)
    }

    /// A 409 is A3's refusal: the project was migrated with `dvs-migrate`
    fn start_run(&self, totals: Totals) -> Result<StartRunResponse> {
        let req = StartRunRequest {
            client_version: format!("dvs {}", crate::VERSION),
            walk: WalkKind::AuditLog,
            totals,
            other_source: false,
        };
        let answer = self.send("runs", &req)?;
        match answer.status {
            201 => answer.json(self.endpoints.url()),
            409 => Err(refuse(ErrorCode::OperationFailed, answer.text())),
            _ => Err(self.refused(&answer, "runs")),
        }
    }

    fn manifest(&self, run_id: Uuid, entries: &[ManifestEntry]) -> Result<ManifestResponse> {
        let req = ManifestRequest {
            run_id,
            entries: entries.to_vec(),
        };
        self.post("manifest", &req, 200)
    }

    /// How the server took a blob, or that the local copy could not be read. `open`
    /// gives the uncompressed bytes, again for each try.
    fn upload(
        &self,
        run_id: Uuid,
        hash: &str,
        open: &dyn Fn() -> Result<Box<dyn Read + Send>>,
    ) -> Result<After> {
        let (after, _sent) = self
            .endpoints
            .upload(run_id, hash, open, |answer| self.refused(answer, "upload"))?;
        Ok(after)
    }

    /// A 200 is stored, also when a retry finds the run already reported
    fn report(&self, report: &Value) -> Result<Value> {
        self.post("report", report, 200)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use crate::Backend;
    use crate::config::Config;
    use crate::testutil::{create_file, create_temp_git_repo};
    use crate::{Hashes, StoreRequest};

    thread_local! {
        /// Calls to `read_blob` on this thread
        pub(super) static BLOB_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// A dvs2-local checkout whose store holds `files`, added with the crate's own
    /// `init` and store: one log operation each. Returns the temp dir, root and store.
    fn local_project(
        files: &[(&str, &[u8])],
        compression: Compression,
    ) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (tmp, root) = create_temp_git_repo();
        let store = root.parent().unwrap().join("store");
        let mut config = Config::new_local(&store, None).unwrap();
        config.set_compression(Some(compression));
        crate::init::init(&root, config.clone()).unwrap();
        for (path, content) in files {
            add(&root, &config, path, content, compression);
        }
        (tmp, root, store)
    }

    fn add(root: &Path, config: &Config, path: &str, content: &[u8], compression: Compression) {
        let file = create_file(root, path, content);
        let (hashes, _) = Hashes::compute_from_path(&file, &[]).unwrap();
        config
            .backend()
            .store(StoreRequest {
                hashes: &hashes,
                source: &file,
                compression,
                path: ProjectPath::from_canonical(path).unwrap(),
                operation_id: Uuid::new_v4(),
                message: Some("first pull"),
                on_bytes: None,
            })
            .unwrap();
    }

    fn url() -> Url {
        Url::parse("https://h").unwrap()
    }

    fn ready(plan: ImportPlan) -> PreparedImport {
        match plan {
            ImportPlan::Import(prepared) => prepared,
            ImportPlan::AlreadyMigrated(_) => panic!("already migrated"),
        }
    }

    fn refusal(root: &Path, project_path: Option<&Path>) -> String {
        let error = prepare_import(root, "p".into(), url(), project_path, true)
            .err()
            .expect("refused");
        error
            .downcast_ref::<Refused>()
            .expect("a refusal")
            .0
            .message
            .clone()
    }

    fn append_log(store: &Path, line: &str) {
        let log = store.join(AUDIT_LOG);
        let mut text = fs::read_to_string(&log).unwrap();
        text.push_str(line);
        text.push('\n');
        fs::write(&log, text).unwrap();
    }

    #[test]
    fn a_log_without_init_is_refused() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        let log = fs::read_to_string(store.join(AUDIT_LOG)).unwrap();
        let adds: Vec<&str> = log.lines().filter(|l| !l.contains("\"init\"")).collect();
        fs::write(store.join(AUDIT_LOG), adds.join("\n")).unwrap();
        let message = refusal(&root, None);
        assert!(
            message.contains("its audit log does not start with an init entry")
                && message.ends_with("use dvs-migrate to migrate this project"),
            "{message}"
        );
    }

    #[test]
    fn a_blob_no_add_names_is_refused() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        let stray = "cd".repeat(32);
        create_file(&store, &format!("cd/{}", &stray[2..]), b"dvs1 content");
        let message = refusal(&root, None);
        assert!(
            message.contains("1 blob in the store that no add in its log names"),
            "{message}"
        );
        assert!(!message.contains("init"), "{message}");
    }

    fn init_line(store: &Path, path: &str) -> String {
        let config = Config::new_local(store, None).unwrap();
        let entry = AuditEntry::new_init(Uuid::new_v4(), config, PathBuf::from(path));
        serde_json::to_string(&entry).unwrap()
    }

    #[test]
    fn two_projects_in_the_log_are_refused_and_named() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        append_log(&store, &init_line(&store, "/elsewhere/proj"));
        let message = refusal(&root, None);
        assert!(
            message.starts_with("the store's audit log names 2 projects: ")
                && message.contains(&root.display().to_string())
                && message.contains("/elsewhere/proj")
                && message.contains("--project-path"),
            "{message}"
        );
    }

    #[test]
    fn project_path_must_name_one_of_the_logs_projects() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        append_log(&store, &init_line(&store, "/elsewhere/proj"));
        let message = refusal(&root, Some(Path::new("/other")));
        assert!(
            message.starts_with("--project-path /other is not"),
            "{message}"
        );

        let prepared = ready(
            prepare_import(
                &root,
                "p".into(),
                url(),
                Some(Path::new("/elsewhere/proj")),
                true,
            )
            .unwrap(),
        );
        assert_eq!(prepared.entries.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn project_path_is_compared_canonical() {
        let (tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        append_log(&store, &init_line(&store, "/elsewhere/proj"));
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let prepared = ready(prepare_import(&root, "p".into(), url(), Some(&link), true).unwrap());
        assert_eq!(prepared.entries.len(), 1);
    }

    #[test]
    fn a_path_dvs_cannot_take_is_refused() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        let log = fs::read_to_string(store.join(AUDIT_LOG)).unwrap();
        let log = log.replace(r#""path":"a.csv""#, r#""path":"../a.csv""#);
        assert!(log.contains("../a.csv"));
        fs::write(store.join(AUDIT_LOG), log).unwrap();
        let message = refusal(&root, None);
        assert!(
            message.starts_with("the store's audit log adds ../a.csv, a path dvs cannot take"),
            "{message}"
        );
    }

    #[test]
    fn every_add_is_a_version_and_content_goes_up_once() {
        let (_tmp, root, store) = local_project(&[("a.csv", b"a")], Compression::None);
        let config = Config::find(&root).unwrap().unwrap();
        add(&root, &config, "a.csv", b"b", Compression::None);
        add(&root, &config, "a.csv", b"a", Compression::None);
        let log = fs::read_to_string(store.join(AUDIT_LOG)).unwrap();
        let log = parse_audit_log(log.as_bytes(), &HashSet::new()).unwrap();
        let (url, seen) = server(root.clone(), &[]);
        let imported = run_import(&root, &url).unwrap();

        // L10: set back to A, a third version with its own operation
        let report = &imported.report;
        assert_eq!(report.totals.entries, 3);
        let operations: Vec<_> = report
            .entries
            .iter()
            .map(|e| e.entry.operation.clone().unwrap())
            .collect();
        let logged: Vec<_> = log[1..].iter().map(|e| e.operation_id.clone()).collect();
        assert_eq!(operations, logged);
        assert_eq!(report.entries[2].entry.hash, report.entries[0].entry.hash);
        assert_ne!(operations[2], operations[0]);
        let seen = seen.lock().unwrap();
        let manifest = seen
            .requests
            .iter()
            .find(|r| r.contains("/manifest "))
            .unwrap();
        let manifest: Value = serde_json::from_str(manifest.split_once('\n').unwrap().1).unwrap();
        assert_eq!(manifest["entries"].as_array().unwrap().len(), 3);
        let uploads = seen.requests.iter().filter(|r| r.contains("/blobs/"));
        assert_eq!(uploads.count(), 2);
    }

    #[test]
    fn a_switched_checkout_is_already_migrated() {
        let (_tmp, root, _store) = local_project(&[("a.csv", b"a")], Compression::None);
        let (url, _seen) = server(root.clone(), &[]);
        run_import(&root, &url).unwrap();

        // L11: the same server and project goes plain `dvs migrate server`'s way
        let plan = prepare_import(&root, "p".into(), url.clone(), None, true).unwrap();
        assert!(matches!(plan, ImportPlan::AlreadyMigrated(_)));
        // Another server keeps the refusal
        let error = prepare_import(&root, "p".into(), self::url(), None, true)
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .ends_with("dvs migrate does not move a project between servers"),
            "{error}"
        );
    }

    #[test]
    fn manifest_entries_come_from_the_log() {
        let content = b"some content that zstd compresses, some content that zstd compresses";
        let (_tmp, root, store) = local_project(&[("data/a.csv", content)], Compression::Zstd);
        let prepared = ready(prepare_import(&root, "p".into(), url(), None, true).unwrap());
        let hash = blake3::hash(content).to_string();
        let blob = legacy::blob_path(&store, &hash);
        assert_ne!(fs::metadata(&blob).unwrap().len(), content.len() as u64);

        let log = fs::read_to_string(store.join(AUDIT_LOG)).unwrap();
        let log = parse_audit_log(log.as_bytes(), &HashSet::new()).unwrap();
        let logged = &log[1];
        let e = &prepared.entries[0];
        assert_eq!(e.path.as_str(), "data/a.csv");
        assert_eq!(e.hash, hash);
        assert!(
            matches!(read_blob(&store, &hash), Local::Found(blob) if blob.size == content.len() as u64)
        );
        assert_eq!(e.shape, SidecarShape::Dvs2);
        assert_eq!(e.sidecar_path, ".dvs/data/a.csv.dvs");
        assert_eq!(e.recorded_user, logged.user);
        assert_eq!(e.recorded_at, logged.timestamp);
        assert_eq!(e.recorded_user_source, AttributionSource::AuditLog);
        assert_eq!(e.recorded_at_source, AttributionSource::AuditLog);
        assert_eq!(e.message.as_deref(), Some("first pull"));
        assert_eq!(e.commit, None);
        assert_eq!(e.operation.as_deref(), Some(logged.operation_id.as_str()));
        assert_eq!(e.storage_path, Some(store.display().to_string()));
    }

    /// What the mock server saw: each request's first line and body, and whether the
    /// checkout was still on local storage when the report arrived
    #[derive(Default)]
    struct Seen {
        requests: Vec<String>,
        local_at_report: Option<bool>,
        /// Uploaded blobs, by hash
        blobs: HashMap<String, Vec<u8>>,
    }

    /// A migration server that has nothing, but answers a call `answers` names with its
    /// status and body. Blob uploads are kept by hash.
    fn server(
        root: PathBuf,
        answers: &'static [(&'static str, u16, &'static str)],
    ) -> (Url, Arc<Mutex<Seen>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let (mut first, mut length, mut chunked) = (String::new(), 0, false);
                reader.read_line(&mut first).unwrap();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let lower = line.to_ascii_lowercase();
                    if let Some(n) = lower.strip_prefix("content-length:") {
                        length = n.trim().parse().unwrap();
                    }
                    chunked |= lower.starts_with("transfer-encoding: chunked");
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = Vec::new();
                if chunked {
                    loop {
                        let mut size = String::new();
                        reader.read_line(&mut size).unwrap();
                        let n = usize::from_str_radix(size.trim(), 16).unwrap();
                        let mut part = vec![0; n + 2];
                        reader.read_exact(&mut part).unwrap();
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&part[..n]);
                    }
                } else {
                    body.resize(length, 0);
                    reader.read_exact(&mut body).unwrap();
                }
                let first = first.trim().to_string();
                let path = first.split(' ').nth(1).unwrap_or_default().to_string();
                let json = || serde_json::from_slice::<Value>(&body).unwrap();
                if path.ends_with("/migration/report") {
                    let config = Config::find(&root).unwrap().unwrap();
                    log.lock().unwrap().local_at_report =
                        Some(matches!(config.backend(), Backend::Local(_)));
                }
                let given = answers.iter().find(|(call, ..)| path.contains(call));
                let (status, answer) = if let Some((_, status, body)) = given {
                    (*status, body.to_string())
                } else if path.ends_with("/migration/lookup") {
                    (200, r#"{"have":[],"compression":"zstd"}"#.to_string())
                } else if path.ends_with("/migration/runs") {
                    let id = Uuid::nil();
                    (
                        201,
                        format!(r#"{{"run_id":"{id}","started_at":"t","compression":"zstd"}}"#),
                    )
                } else if path.ends_with("/migration/manifest") {
                    let entries = json()["entries"].as_array().unwrap().clone();
                    let missing: BTreeSet<&str> = entries
                        .iter()
                        .map(|e| e["hash"].as_str().unwrap())
                        .collect();
                    let answer = serde_json::json!({
                        "missing": missing, "compression": "zstd",
                        "recorded": entries.len(), "already_migrated": 0,
                    });
                    (200, answer.to_string())
                } else if path.contains("/migration/blobs/") {
                    let hash = path.split("/blobs/").nth(1).unwrap().split('?').next();
                    let hash = hash.unwrap().to_string();
                    log.lock().unwrap().blobs.insert(hash, body.clone());
                    (201, r#"{"stored_size":1,"compression":"zstd"}"#.to_string())
                } else if path.ends_with("/migration/report") {
                    (200, r#"{"entries":1,"problems":0}"#.to_string())
                } else if path.ends_with("/access") {
                    let access = r#"{"access":"write","user":{"username":"u","email":null},
                                    "groups":[],"compression":"zstd"}"#;
                    (200, access.to_string())
                } else {
                    // `exists`: the server has the content now
                    (200, String::new())
                };
                log.lock()
                    .unwrap()
                    .requests
                    .push(format!("{first}\n{}", String::from_utf8_lossy(&body)));
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
        });
        (url, seen)
    }

    /// Tests share one credentials file: one at a time, so no login is lost
    static SERVER_TESTS: Mutex<()> = Mutex::new(());

    fn run_import(root: &Path, url: &Url) -> Result<Imported> {
        let _one = SERVER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        while crate::auth::get_token(url).unwrap().is_none() {
            crate::auth::store_token(url, "t").unwrap();
        }
        let prepared = ready(prepare_import(root, "p".into(), url.clone(), None, true)?);
        import(prepared, &Retrying::without_waits())
    }

    #[test]
    fn the_checkout_is_switched_only_after_the_report() {
        let content = b"zstd zstd zstd zstd zstd zstd zstd zstd zstd zstd zstd";
        let (_tmp, root, _store) = local_project(&[("a.csv", content)], Compression::Zstd);
        let (url, seen) = server(root.clone(), &[]);
        let imported = run_import(&root, &url).unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.local_at_report, Some(true));
        let config = Config::find(&root).unwrap().unwrap();
        assert!(matches!(config.backend(), Backend::Server(s) if s.url == url));
        assert!(imported.switch.config_written);
        // The blob went up decompressed (A7), and the run's walk is the log
        let hash = blake3::hash(content).to_string();
        assert_eq!(seen.blobs[&hash], content);
        let start = seen.requests.iter().find(|r| r.contains("/runs ")).unwrap();
        let start: Value = serde_json::from_str(start.split_once('\n').unwrap().1).unwrap();
        assert_eq!(start["walk"], "audit_log");
        assert_eq!(imported.report.totals.uploaded, Some(1));
        assert!(imported.migration_page.ends_with("/project/p/migration"));
    }

    #[test]
    fn a_failed_import_does_not_switch_the_checkout() {
        let (_tmp, root, _store) = local_project(&[("a.csv", b"a")], Compression::None);
        let before = fs::read_to_string(root.join(crate::paths::CONFIG_FILE_NAME)).unwrap();
        let (url, seen) = server(root.clone(), &[("/migration/report", 500, "down")]);
        let error = run_import(&root, &url).err().unwrap();

        let interrupted = error.downcast_ref::<Interrupted>().expect("interrupted");
        assert_eq!(
            interrupted.to_string(),
            "interrupted after 1 of 1 versions; run again to carry on"
        );
        let after = fs::read_to_string(root.join(crate::paths::CONFIG_FILE_NAME)).unwrap();
        assert_eq!(before, after);
        let seen = seen.lock().unwrap();
        let reports = seen.requests.iter().filter(|r| r.contains("/report "));
        assert_eq!(reports.count(), crate::retry::RETRIES + 1);
    }

    #[test]
    fn missing_content_is_reported_and_the_rest_go_on() {
        let (_tmp, root, store) =
            local_project(&[("a.csv", b"a"), ("b.csv", b"b")], Compression::None);
        let a = blake3::hash(b"a").to_string();
        fs::remove_file(legacy::blob_path(&store, &a)).unwrap();
        // D3 sees the add naming it, so only the import finds it gone
        let (url, seen) = server(root.clone(), &[]);
        let imported = run_import(&root, &url).unwrap();

        let report = &imported.report;
        assert_eq!(report.outcome, RunOutcome::Partial);
        assert_eq!(report.totals.missing, 1);
        assert_eq!(report.totals.uploaded, Some(1));
        assert!(matches!(
            &report.problems[..],
            [Problem::Missing { hash, paths, .. }] if *hash == a && paths == &["a.csv"]
        ));
        let seen = seen.lock().unwrap();
        assert!(seen.blobs.contains_key(&blake3::hash(b"b").to_string()));
        assert!(imported.switch.config_written);
    }

    #[test]
    fn server_refusals_change_nothing() {
        let (_tmp, root, _store) = local_project(&[("a.csv", b"a")], Compression::None);
        let config = || fs::read_to_string(root.join(crate::paths::CONFIG_FILE_NAME)).unwrap();
        let before = config();
        let refusals: [&'static [(&'static str, u16, &'static str)]; 2] = [
            &[("/migration/lookup", 403, "admins only")],
            &[(
                "/migration/runs",
                409,
                "proj was migrated with dvs-migrate by ann",
            )],
        ];
        let mut messages = Vec::new();
        for answers in refusals {
            let (url, seen) = server(root.clone(), answers);
            BLOB_READS.with(|n| n.set(0));
            let error = run_import(&root, &url).err().unwrap();
            let refused = error.downcast_ref::<Refused>().expect("a refusal");
            messages.push(refused.0.message.clone());
            // L8: the server answered before the store was read
            assert_eq!(BLOB_READS.with(|n| n.get()), 0);
            let seen = seen.lock().unwrap();
            assert!(!seen.requests.iter().any(|r| r.contains("/manifest ")));
        }
        assert_eq!(
            messages,
            [NOT_ADMIN, "proj was migrated with dvs-migrate by ann"]
        );
        assert_eq!(before, config());
    }
}
