//! plan and run: walk, ask the server what it has, and fill in the report.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use dvs::RunOutcome;
use dvs::legacy::SidecarShape;
use serde_json::{Value, json};

use dvs::migration::{
    EntryRef, InProgressRun, LookupRequest, ManifestEntry, PreviousRuns, Problem, Scope, Totals,
    WalkKind,
};

use crate::client::{Client, RETRIES, Refused, chunks};
use crate::git::{self, redact};
use crate::report::{Content, LocalStorage, PATH_HASH_HINT, Report, ReportEntry, plural};
use crate::storage::{self, Blob};
use crate::walk::{self, Walk};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Before {
    OnServer,
    Local,
    #[default]
    Absent,
}

/// How the server took a blob a run sent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum After {
    /// 201 if the server stored it now, 200 if it had it already
    Uploaded { created: bool },
    /// 400: the bytes do not hash to the claimed hash
    Corrupt(String),
    /// Any other refusal of the bytes themselves, as 413 over the size limit
    Refused(u16, String),
    /// The local copy could not be read
    Unreadable(String),
}

/// A storage folder looked in, and whether it exists here
struct StorageFolder {
    path: String,
    exists: bool,
}

#[derive(Default)]
struct HashInfo {
    /// The first size a sidecar recorded, 0 while none has
    size: u64,
    /// Every size recorded; devious v0's missing size is none of them
    sizes: BTreeSet<u64>,
    /// The blob file's size in the first of its stores holding it, from the store listing
    stored_size: Option<u64>,
    /// Named by a devious v0 sidecar, whose hash may be of the path (R24)
    devious_v0: bool,
    paths: Vec<String>,
    blob: Option<Blob>,
    looked_at: Option<String>,
    storage: Option<StorageFolder>,
    before: Before,
    after: Option<After>,
}

impl HashInfo {
    fn content(&self) -> Content {
        match (self.before, &self.after) {
            (Before::OnServer, _) => Content::OnServer,
            (Before::Absent, _) | (_, Some(After::Unreadable(_))) => Content::Missing,
            (Before::Local, None) => Content::ToUpload,
            (Before::Local, Some(After::Uploaded { .. })) => Content::Uploaded,
            (Before::Local, Some(After::Corrupt(_))) => Content::Corrupt,
            (Before::Local, Some(After::Refused(..))) => Content::Refused,
        }
    }

    /// Not on the server after this command, and not coming
    fn missing(&self) -> bool {
        matches!(
            self.content(),
            Content::Missing | Content::Corrupt | Content::Refused
        )
    }
}

/// The size a sidecar recorded, unless it recorded none: devious v0 before v0.5 wrote only
/// the hash, which reads as size 0
fn known_size(e: &ManifestEntry) -> Option<u64> {
    (e.shape != SidecarShape::DeviousV0 || e.size != 0).then_some(e.size)
}

/// What a fresh walk found and what the server already has of it
pub struct Plan {
    scope: Scope,
    walk: Walk,
    hashes: BTreeMap<String, HashInfo>,
    migrated: HashSet<EntryRef>,
    unmatched_users: Vec<String>,
    /// The project's earlier runs (R16)
    previous: Option<PreviousRuns>,
    /// How many of the walk's commits earlier runs recorded (R17)
    known_commits: u64,
    /// Every blob file in the stores the walk saw that are here, and how many those are
    storage_bytes: u64,
    stores_found: usize,
    /// How far `run` got, for when it stops early (R18)
    progress: Progress,
}

/// What a run has done so far
#[derive(Debug, Default)]
struct Progress {
    opened: bool,
    /// Versions the server recorded as migrated
    recorded: usize,
    /// Of those, the ones whose content the server has: R18's N (R30)
    migrated: usize,
    /// Recorded versions waiting for their blob, by hash
    pending: HashMap<String, usize>,
    /// Bytes of the blobs the server took
    uploaded_bytes: u64,
}

/// Whether a plan is filled in as a plan, or as a run that recorded `recorded` versions
#[derive(Debug, Clone, Copy)]
pub enum Phase {
    Plan,
    Run { recorded: usize },
}

pub fn progress(quiet: bool, message: impl AsRef<str>) {
    if !quiet {
        eprintln!("{}", message.as_ref());
    }
}

/// A repo's history as the walk reads it
pub struct Walked {
    pub scope: Scope,
    pub walk: Walk,
    pub commit_ids: Vec<String>,
}

/// Mirrors `source` and walks every commit on its refs
pub fn walk_repo(
    source: &str,
    storage_map: &BTreeMap<String, String>,
    quiet: bool,
) -> Result<Walked> {
    let shown = redact(source);
    progress(quiet, format!("Cloning {shown}"));
    let mirror = git::mirror(source)?;
    let refs = git::walked_refs(&mirror.path)?;
    let commits = git::commits(&mirror.path, &refs)?;
    progress(
        quiet,
        format!(
            "Walking {} on {}",
            plural(commits.len(), "commit"),
            plural(refs.len(), "ref")
        ),
    );
    let mut objects = git::Objects::open(&mirror.path)?;
    let walk = walk::walk(&mut objects, &commits, storage_map)?;
    let scope = Scope {
        source: shown,
        walk: if mirror.partial.is_empty() {
            WalkKind::Full
        } else {
            WalkKind::Partial
        },
        partial_reasons: mirror.partial.clone(),
        refs,
        commits: walk.commits,
        storage: walk.storage.clone(),
    };
    Ok(Walked {
        scope,
        walk,
        commit_ids: commits.into_iter().map(|c| c.id).collect(),
    })
}

pub fn plan(
    source: &str,
    storage_map: &BTreeMap<String, String>,
    client: &Client,
    quiet: bool,
) -> Result<Plan> {
    let Walked {
        scope,
        walk,
        commit_ids,
    } = walk_repo(source, storage_map, quiet)?;

    progress(quiet, "Asking the server what it has");
    let mut hashes: BTreeMap<String, HashInfo> = BTreeMap::new();
    for e in &walk.entries {
        let info = hashes.entry(e.hash.clone()).or_default();
        if let Some(size) = known_size(e) {
            if info.sizes.is_empty() {
                info.size = size;
            }
            info.sizes.insert(size);
        }
        info.devious_v0 |= e.shape == SidecarShape::DeviousV0;
        info.paths.push(e.path.as_str().to_string());
    }
    let users: BTreeSet<&str> = walk
        .entries
        .iter()
        .map(|e| e.recorded_user.as_str())
        .collect();
    let lookup = lookup(
        client,
        hashes.keys().cloned().collect(),
        walk.entries.iter().map(entry_ref).collect(),
        users.into_iter().map(str::to_string).collect(),
        commit_ids,
    )?;

    let mut stores: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &walk.entries {
        if let Some(store) = e.storage_path.as_deref() {
            stores.entry(&e.hash).or_default().push(store);
        }
    }

    // Sizes only, for R22: every store the walk saw that is here
    progress(quiet, "Measuring local storage");
    let mut listings: BTreeMap<&str, Option<storage::Listing>> = BTreeMap::new();
    for seen in &walk.storage {
        listings.entry(seen.effective()).or_insert_with_key(|path| {
            let listing = match storage::list(Path::new(path)) {
                Ok(listing) => listing,
                // A store that isn't here is reported with the stores not found
                Err(e) if Path::new(path).exists() => {
                    eprintln!("warning: cannot read {path}: {e}; its size is left out");
                    return None;
                }
                Err(_) => return None,
            };
            listing.warnings.iter().for_each(|w| eprintln!("{w}"));
            Some(listing)
        });
    }
    let storage_bytes = listings.values().flatten().map(|l| l.bytes).sum();
    let stores_found = listings.values().flatten().count();

    for (hash, info) in hashes.iter_mut() {
        let stores = stores.get(hash.as_str()).map_or(&[][..], Vec::as_slice);
        info.stored_size = stores.iter().find_map(|store| {
            let listing = listings.get(store)?.as_ref()?;
            listing.size(hash)
        });
        if lookup.have.contains(hash) {
            info.before = Before::OnServer;
            continue;
        }
        let (blob, looked_at) = storage::find(hash, info.size, stores);
        info.before = if blob.is_some() {
            Before::Local
        } else {
            Before::Absent
        };
        info.blob = blob;
        info.looked_at = looked_at;
        // Prefer a folder that exists: then the hash is simply not in it
        info.storage = stores
            .iter()
            .map(|store| StorageFolder {
                path: store.to_string(),
                exists: Path::new(store).is_dir(),
            })
            .max_by_key(|folder| folder.exists);
    }

    Ok(Plan {
        scope,
        walk,
        hashes,
        migrated: lookup.migrated,
        unmatched_users: lookup.unmatched_users,
        previous: lookup.previous,
        known_commits: lookup.known_commits,
        storage_bytes,
        stores_found,
        progress: Progress::default(),
    })
}

fn entry_ref(e: &ManifestEntry) -> EntryRef {
    EntryRef {
        path: e.path.clone(),
        hash: e.hash.clone(),
    }
}

struct Lookup {
    have: HashSet<String>,
    migrated: HashSet<EntryRef>,
    unmatched_users: Vec<String>,
    previous: Option<PreviousRuns>,
    known_commits: u64,
}

/// The `i`th chunk, or nothing once the list has run out
fn chunk_at<T: Clone>(chunks: &[&[T]], i: usize) -> Vec<T> {
    chunks
        .get(i)
        .map(|chunk| chunk.to_vec())
        .unwrap_or_default()
}

fn lookup(
    client: &Client,
    hashes: Vec<String>,
    entries: Vec<EntryRef>,
    users: Vec<String>,
    commits: Vec<String>,
) -> Result<Lookup> {
    let hashes = chunks(&hashes);
    let entries = chunks(&entries);
    let users = chunks(&users);
    let commits = chunks(&commits);
    // Call `i` takes chunk `i` of each list, and there is always at least one call
    let calls = [hashes.len(), entries.len(), users.len(), commits.len(), 1]
        .into_iter()
        .max()
        .unwrap_or(1);
    let mut found = Lookup {
        have: HashSet::new(),
        migrated: HashSet::new(),
        unmatched_users: Vec::new(),
        previous: None,
        known_commits: 0,
    };
    for i in 0..calls {
        let req = LookupRequest {
            hashes: chunk_at(&hashes, i),
            entries: chunk_at(&entries, i),
            users: chunk_at(&users, i),
            commits: chunk_at(&commits, i),
        };
        let answer = client.lookup(&req)?;
        found.have.extend(answer.have);
        found.migrated.extend(answer.migrated);
        found.unmatched_users.extend(answer.unmatched_users);
        // The same on every call; each call counts its own chunk of commits
        found.previous = answer.previous.or(found.previous);
        found.known_commits += answer.known_commits;
    }
    found.unmatched_users.sort();
    Ok(found)
}

impl Plan {
    fn already_migrated(&self, e: &ManifestEntry) -> bool {
        self.migrated.contains(&entry_ref(e))
    }

    /// The walk looks like another repo: the project has runs, and none of them recorded
    /// any commit this walk visited (R17). Why, for a warning or a refusal.
    pub fn different_source(&self) -> Option<String> {
        let previous = self.previous.as_ref()?;
        if self.scope.commits == 0 || self.known_commits > 0 {
            return None;
        }
        let from = previous
            .last_source
            .as_ref()
            .map(|source| format!(" from {source}"))
            .unwrap_or_default();
        Some(format!(
            "none of the {} walked is in the project's earlier migration runs ({}, last by {} \
             on {}{from}); this looks like a different repo",
            plural(self.scope.commits, "commit"),
            plural(previous.runs as usize, "run"),
            previous.last_run_by,
            previous.last_run_at.strftime("%Y-%m-%d"),
        ))
    }

    /// The bytes of history's content on the server after this command, or that would be
    /// after a run: stored sizes where the content is here, else recorded ones
    fn on_server_bytes(&self) -> u64 {
        self.hashes
            .values()
            .filter(|h| !h.missing())
            .map(|h| h.stored_size.unwrap_or(h.size))
            .sum()
    }

    /// What a run that stops now has done, for stderr: of the versions it had to
    /// migrate, how many the server has the content of, and the bytes it took
    pub fn interrupted(&self) -> String {
        if !self.progress.opened {
            return nothing_started();
        }
        let to_migrate = self
            .walk
            .entries
            .iter()
            .filter(|e| !self.already_migrated(e))
            .count();
        format!(
            "interrupted after {} of {} versions ({}); run again to carry on",
            thousands(self.progress.migrated as u64),
            thousands(to_migrate as u64),
            dvs::format_size(self.progress.uploaded_bytes)
        )
    }

    pub fn totals(&self, phase: Phase) -> Totals {
        let count = |f: &dyn Fn(&HashInfo) -> bool| self.hashes.values().filter(|h| f(h)).count();
        let missing = HashInfo::missing;
        let (uploaded, migrated, on_server_bytes) = match phase {
            Phase::Plan => (None, None, None),
            Phase::Run { recorded } => {
                let created =
                    count(&|h| matches!(h.after, Some(After::Uploaded { created: true })));
                (Some(created), Some(recorded), Some(self.on_server_bytes()))
            }
        };
        Totals {
            sidecars: self.walk.sidecars.clone(),
            entries: self.walk.entries.len(),
            unique_hashes: self.hashes.len(),
            bytes: self.hashes.values().map(|h| h.size).sum(),
            on_server: count(&|h| h.before == Before::OnServer),
            to_upload: count(&|h| h.before == Before::Local && !h.missing()),
            missing: count(&missing),
            already_migrated: self
                .walk
                .entries
                .iter()
                .filter(|e| self.already_migrated(e))
                .count(),
            user_mismatches: self.walk.entries.iter().filter(|e| e.user_mismatch).count(),
            uploaded,
            migrated,
            storage_bytes: self.storage_bytes,
            referenced_bytes: self.hashes.values().filter_map(|h| h.stored_size).sum(),
            on_server_bytes,
        }
    }

    fn problems(&self) -> Vec<Problem> {
        let mut problems = self.walk.problems.clone();
        for (hash, h) in &self.hashes {
            let (paths, looked_at) = (h.paths.clone(), h.looked_at.clone());
            let hash = hash.clone();
            match &h.after {
                Some(After::Corrupt(message)) => problems.push(Problem::Corrupt {
                    hash: hash.clone(),
                    paths,
                    looked_at,
                    message: if h.devious_v0 {
                        format!("{message}; {PATH_HASH_HINT}")
                    } else {
                        message.clone()
                    },
                }),
                Some(After::Refused(status, message)) => problems.push(Problem::UploadRefused {
                    hash: hash.clone(),
                    paths,
                    looked_at,
                    status: *status,
                    message: message.clone(),
                }),
                _ if h.content() == Content::Missing => problems.push(Problem::Missing {
                    hash: hash.clone(),
                    size: h.size,
                    paths,
                    looked_at,
                    storage: h.storage.as_ref().map(|folder| folder.path.clone()),
                    storage_exists: h.storage.as_ref().map(|folder| folder.exists),
                }),
                _ => {}
            }
            let actual_size = h
                .blob
                .as_ref()
                .filter(|b| b.compression == dvs::Compression::None)
                .map(|b| b.stored_size);
            let recorded = !h.sizes.is_empty();
            if h.sizes.len() > 1 || actual_size.is_some_and(|s| recorded && s != h.size) {
                problems.push(Problem::SizeMismatch {
                    hash,
                    recorded_sizes: h.sizes.iter().copied().collect(),
                    actual_size,
                    looked_at: h.looked_at.clone(),
                });
            }
        }
        for user in &self.unmatched_users {
            let entries = self
                .walk
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

    /// Fills the report's scope, totals, entries and problems
    pub fn fill(&self, report: &mut Report, phase: Phase) {
        report.scope = Some(self.scope.clone());
        report.totals = Some(self.totals(phase));
        report.previous = self.previous.clone();
        report.local = Some(LocalStorage {
            stores: self.stores_found,
            on_server_bytes: self.on_server_bytes(),
        });
        report.entries = self
            .walk
            .entries
            .iter()
            .map(|e| ReportEntry {
                entry: e.clone(),
                content: self.hashes[&e.hash].content(),
                already_migrated: self.already_migrated(e),
            })
            .collect();
        report.problems = self.problems();
        report.outcome = if report.problems.is_empty() {
            RunOutcome::Complete
        } else {
            RunOutcome::Partial
        };
    }
}

/// Opens a run, notes the project's other runs in progress (R35), sends every entry,
/// uploads what the server asks for that is here, and fills the report. The caller sends it.
pub fn run(
    plan: &mut Plan,
    client: &Client,
    report: &mut Report,
    quiet: bool,
    other_source: bool,
) -> Result<()> {
    if let (false, Some(why)) = (other_source, plan.different_source()) {
        return Err(Refused(format!(
            "{why}; if it is the right repo, run again with --other-source"
        ))
        .into());
    }
    let run = client.start_run(plan.scope.walk, plan.totals(Phase::Plan), other_source)?;
    plan.progress.opened = true;
    report.run_id = Some(run.run_id);
    for note in in_progress_notes(&run.in_progress) {
        eprintln!("{note}");
    }
    send(plan, client, run.run_id, quiet)?;
    let recorded = plan.progress.recorded;
    plan.fill(report, Phase::Run { recorded });
    Ok(())
}

/// Every entry, then every blob the server lacks that is here
fn send(plan: &mut Plan, client: &Client, run_id: uuid::Uuid, quiet: bool) -> Result<()> {
    let mut wanted = BTreeSet::new();
    for chunk in chunks(&plan.walk.entries) {
        let answer = client.manifest(run_id, chunk)?;
        // The new versions whose content the server lacks wait for their upload; the
        // rest of what it recorded completed at once
        let lacking: HashSet<&String> = answer.missing.iter().collect();
        let mut waiting = 0;
        for e in chunk {
            if lacking.contains(&e.hash) && !plan.migrated.contains(&entry_ref(e)) {
                *plan.progress.pending.entry(e.hash.clone()).or_default() += 1;
                waiting += 1;
            }
        }
        plan.progress.migrated += answer.recorded.saturating_sub(waiting);
        plan.progress.recorded += answer.recorded;
        wanted.extend(answer.missing);
    }

    let to_send: Vec<(&String, Blob)> = wanted
        .iter()
        .filter_map(|hash| Some((hash, plan.hashes.get(hash)?.blob.clone()?)))
        .collect();
    let bytes: u64 = to_send.iter().map(|(h, _)| plan.hashes[*h].size).sum();
    progress(
        quiet,
        format!(
            "Uploading {} blobs ({})",
            to_send.len(),
            dvs::format_size(bytes)
        ),
    );
    for (hash, blob) in to_send {
        let (after, sent) = client.upload(run_id, hash, &|| storage::open(&blob))?;
        plan.progress.uploaded_bytes += sent;
        if let After::Unreadable(message) = &after {
            eprintln!("warning: cannot read {}: {message}", blob.path.display());
        }
        if let After::Uploaded { .. } = after {
            plan.progress.migrated += plan.progress.pending.remove(hash).unwrap_or(0);
        }
        plan.hashes.get_mut(hash).expect("listed").after = Some(after);
    }
    Ok(())
}

/// Retries ran out before a run was opened (R18)
pub fn nothing_started() -> String {
    format!("could not reach the server after {RETRIES} retries; nothing was started")
}

/// `n` with thousands separators: 1,204
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// One note per other run in progress; they never stop this one (R35)
fn in_progress_notes(runs: &[InProgressRun]) -> Vec<String> {
    let utc = |at: &str, format: &str| {
        at.parse::<jiff::Timestamp>()
            .map_or_else(|_| at.to_string(), |t| t.strftime(format).to_string())
    };
    runs.iter()
        .map(|r| {
            format!(
                "note: a run by {} is in progress (started {} UTC, last activity {} UTC); \
                 both can run, the server keeps one copy of each version",
                r.started_by,
                utc(&r.started_at, "%Y-%m-%d %H:%M"),
                utc(&r.last_activity_at, "%H:%M")
            )
        })
        .collect()
}

/// The warning `run` ends with while an add could still land in local storage. Making it
/// read-only is the admin's step, so this only names the command.
pub fn storage_writable_warning(scope: &Scope) -> Option<String> {
    let writable: BTreeSet<&str> = scope
        .storage
        .iter()
        .map(|s| s.effective())
        .filter(|p| !storage::writable_parts(Path::new(p)).is_empty())
        .collect();
    if writable.is_empty() {
        return None;
    }
    let paths: Vec<String> = writable.iter().map(|p| format!("'{p}'")).collect();
    Some(format!(
        "warning: local storage is still writable, so new dvs adds can still land there; \
         after the last run, make it read-only: chmod -R a-w {}",
        paths.join(" ")
    ))
}

/// The report's error object, as dvs2's `--json` envelope words it
pub fn error_json(error: &anyhow::Error) -> Value {
    let code = dvs::backend_error_code(error).unwrap_or(dvs::ErrorCode::OperationFailed);
    json!({"code": code.as_str(), "message": redact(&format!("{error:#}")), "hint": null})
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs::migration::{ProblemKind, StorageSeen, StoreKind};

    use crate::client::tests::{client, serve};
    use crate::walk::tests::{Repo, hash, manifest_entry};
    use dvs::legacy::blob_path;

    /// One entry per (hash, before, after); every entry already migrated
    fn plan(hashes: Vec<(String, Before, Option<After>)>, storage: Vec<StorageSeen>) -> Plan {
        let entries: Vec<ManifestEntry> = hashes
            .iter()
            .enumerate()
            .map(|(i, (h, ..))| manifest_entry(&format!("f{i}"), h))
            .collect();
        Plan {
            scope: Scope {
                source: "repo".into(),
                walk: WalkKind::Full,
                partial_reasons: Vec::new(),
                refs: Vec::new(),
                commits: 1,
                storage,
            },
            migrated: entries.iter().map(entry_ref).collect(),
            hashes: hashes
                .into_iter()
                .map(|(h, before, after)| {
                    let info = HashInfo {
                        size: 5,
                        sizes: BTreeSet::from([5]),
                        before,
                        after,
                        ..HashInfo::default()
                    };
                    (h, info)
                })
                .collect(),
            walk: Walk {
                commits: 1,
                storage: Vec::new(),
                sidecars: BTreeMap::new(),
                entries,
                problems: Vec::new(),
            },
            unmatched_users: Vec::new(),
            previous: None,
            known_commits: 0,
            storage_bytes: 0,
            stores_found: 0,
            progress: Progress::default(),
        }
    }

    #[test]
    fn a_devious_history_is_planned_from_its_flat_store_and_a_new_source_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(store.join("02")).unwrap();
        std::fs::write(store.join(format!("{}.dvsfile", hash(1))), b"hello").unwrap();
        std::fs::write(blob_path(&store, &hash(2)), b"world!").unwrap();
        std::fs::write(store.join(format!("{}.dvsfile", hash(9))), b"xyz").unwrap();

        let repo = Repo::new();
        repo.write(
            ".dvs.yaml",
            &format!("storage-dir: {}\n", store.to_str().unwrap()),
        );
        let v1 = format!(
            r#"{{"blake3_checksum": "{}", "file_size_bytes": 5, "timestamp": "t",
                "message": "", "user": "amy"}}"#,
            hash(1)
        );
        repo.write("a.csv.dvsmeta", &v1);
        // devious v0 before v0.5: no size, so the stored 6 bytes are no mismatch
        repo.write("b.csv.dvsmeta", &format!(r#"{{"FileHash":"{}"}}"#, hash(2)));
        let commit = repo.commit("2023-08-16T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let lookup = r#"{"have":[],"compression":"zstd","known_commits":0,
            "previous":{"runs":2,"migrated":42,"last_run_by":"alice",
            "last_run_at":"2026-11-07T10:00:00Z"}}"#;
        let (url, seen) = serve(vec![(200, lookup)]);
        let client = client(url);
        let mut plan =
            super::plan(repo.path.to_str().unwrap(), &BTreeMap::new(), &client, true).unwrap();

        let t = plan.totals(Phase::Plan);
        assert_eq!((t.to_upload, t.missing), (2, 0));
        assert_eq!(
            (t.storage_bytes, t.referenced_bytes, t.on_server_bytes),
            (5 + 6 + 3, 5 + 6, None)
        );
        assert!(plan.problems().is_empty(), "{:?}", plan.problems());

        let why = plan.different_source().unwrap();
        assert!(
            why.starts_with(
                "none of the 1 commit walked is in the project's earlier migration runs \
                 (2 runs, last by alice on 2026-11-07); "
            ),
            "{why}"
        );
        let mut report = Report::new(crate::report::Command::Run);
        let error = run(&mut plan, &client, &mut report, true, false).unwrap_err();
        let refused = error.downcast_ref::<Refused>().expect("a refusal");
        assert!(
            refused.0.ends_with("run again with --other-source"),
            "{}",
            refused.0
        );
        // Refused before any run was started; the lookup sent the walk's commits
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let sent: Value = serde_json::from_str(seen[0].split_once('\n').unwrap().1).unwrap();
        assert_eq!(sent["commits"], json!([commit]));

        // R25: the earlier runs' source is named when the server knows it
        plan.previous.as_mut().unwrap().last_source = Some("/old/clone/project".into());
        let why = plan.different_source().unwrap();
        assert!(
            why.contains("(2 runs, last by alice on 2026-11-07 from /old/clone/project); "),
            "{why}"
        );

        plan.known_commits = 1;
        assert_eq!(plan.different_source(), None);
    }

    #[test]
    fn a_devious_v0_hash_the_content_does_not_match_gets_the_path_hash_hint() {
        let mut plan = plan(
            vec![
                (
                    hash(1),
                    Before::Local,
                    Some(After::Corrupt("content hash mismatch".into())),
                ),
                (
                    hash(2),
                    Before::Local,
                    Some(After::Corrupt("content hash mismatch".into())),
                ),
            ],
            Vec::new(),
        );
        plan.hashes.get_mut(&hash(1)).unwrap().devious_v0 = true;
        let messages: Vec<String> = plan
            .problems()
            .into_iter()
            .filter_map(|p| match p {
                Problem::Corrupt { message, .. } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(
            messages,
            [
                format!("content hash mismatch; {PATH_HASH_HINT}"),
                "content hash mismatch".to_string()
            ]
        );
    }

    #[test]
    fn local_storage_bytes_and_what_an_interrupted_run_did() {
        let mut plan = plan(
            vec![
                (hash(1), Before::OnServer, None),
                (
                    hash(2),
                    Before::Local,
                    Some(After::Uploaded { created: true }),
                ),
                (hash(3), Before::Local, None),
                (hash(4), Before::Absent, None),
            ],
            Vec::new(),
        );
        // Stored sizes where the content is here, recorded ones (5) elsewhere
        for (n, stored) in [(1, 3), (2, 7), (4, 100)] {
            plan.hashes.get_mut(&hash(n)).unwrap().stored_size = Some(stored);
        }
        plan.storage_bytes = 1000;
        let t = plan.totals(Phase::Run { recorded: 4 });
        assert_eq!(
            (t.storage_bytes, t.referenced_bytes, t.on_server_bytes),
            (1000, 110, Some(3 + 7 + 5))
        );
        assert_eq!(plan.totals(Phase::Plan).on_server_bytes, None);
    }

    #[test]
    fn an_interrupted_run_counts_what_it_had_to_do_and_did() {
        let entries = (1..=1500).map(|n| (hash((n % 250) as u8), Before::Local, None));
        let mut plan = plan(entries.collect(), Vec::new());
        assert_eq!(
            plan.interrupted(),
            "could not reach the server after 5 retries; nothing was started"
        );
        // 1,500 versions, of which 2 earlier runs migrated
        plan.migrated = plan.walk.entries[..2].iter().map(entry_ref).collect();
        plan.progress = Progress {
            opened: true,
            recorded: 1300,
            migrated: 1204,
            uploaded_bytes: 38 * 1024 * 1024 * 1024,
            ..Progress::default()
        };
        assert_eq!(
            plan.interrupted(),
            "interrupted after 1,204 of 1,498 versions (38.0 GB); run again to carry on"
        );
    }

    #[test]
    fn a_run_that_gives_up_uploading_counts_only_versions_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = plan(
            (1..=3).map(|n| (hash(n), Before::Local, None)).collect(),
            Vec::new(),
        );
        plan.migrated.clear();
        for n in [2, 3] {
            let path = dir.path().join(hash(n));
            std::fs::write(&path, b"hello").unwrap();
            plan.hashes.get_mut(&hash(n)).unwrap().blob = Some(Blob {
                path,
                compression: dvs::Compression::None,
                stored_size: 5,
            });
        }
        let started = r#"{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"t","compression":"none"}"#;
        // The server has hash 1's content; hash 2 uploads, hash 3's upload gives up
        let manifest = format!(
            r#"{{"missing":["{}","{}"],"compression":"none","recorded":3,"already_migrated":0}}"#,
            hash(2),
            hash(3)
        );
        let mut responses = vec![
            (201, started),
            (200, manifest.leak() as &str),
            (201, r#"{"stored_size":5,"compression":"none"}"#),
        ];
        responses.extend(vec![(503, "down"); RETRIES + 1]);
        // Listening still, so a report would be seen
        responses.push((200, "{}"));
        let (url, seen) = serve(responses);
        let error = run(
            &mut plan,
            &client(url),
            &mut Report::new(crate::report::Command::Run),
            true,
            false,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<crate::client::GaveUp>().is_some());
        assert_eq!(
            plan.interrupted(),
            "interrupted after 2 of 3 versions (5 B); run again to carry on"
        );
        // No report is sent: the run stays open and shows as interrupted (R35)
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3 + RETRIES + 1);
        assert!(!seen.iter().any(|r| r.contains("/migration/report")));
    }

    #[test]
    fn a_run_refused_once_open_sends_no_report() {
        let mut plan = plan(vec![(hash(1), Before::Local, None)], Vec::new());
        let started = r#"{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"t","compression":"none"}"#;
        let (url, seen) = serve(vec![
            (201, started),
            (423, "Project locked by alice"),
            (200, "{}"),
        ]);
        let mut report = Report::new(crate::report::Command::Run);
        let error = run(&mut plan, &client(url), &mut report, true, false).unwrap_err();
        assert_eq!(
            format!("{error:#}"),
            "manifest refused (423): Project locked by alice"
        );
        // No report is sent: the run stays open (R35)
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn retries_running_out_before_a_run_starts_nothing() {
        let (url, seen) = serve(vec![(503, "down"); RETRIES + 1]);
        let mut plan = plan(Vec::new(), Vec::new());
        let error = run(
            &mut plan,
            &client(url),
            &mut Report::new(crate::report::Command::Run),
            true,
            false,
        )
        .unwrap_err();
        assert!(error.downcast_ref::<crate::client::GaveUp>().is_some());
        assert_eq!(seen.lock().unwrap().len(), RETRIES + 1);
        assert_eq!(plan.interrupted(), nothing_started());
    }

    #[test]
    fn other_runs_in_progress_are_noted() {
        let by = |who: &str, started: &str, last: &str| InProgressRun {
            started_by: who.into(),
            started_at: started.into(),
            last_activity_at: last.into(),
        };
        assert!(in_progress_notes(&[]).is_empty());
        let bob = by("bob", "2026-11-10T14:02:11.123Z", "2026-11-10T14:31:59Z");
        let note = "note: a run by bob is in progress (started 2026-11-10 14:02 UTC, \
                    last activity 14:31 UTC); both can run, the server keeps one copy of \
                    each version";
        assert_eq!(in_progress_notes(std::slice::from_ref(&bob)), [note]);
        // An offset is shown in UTC
        let carol = by(
            "carol",
            "2026-11-09T23:50:00-02:00",
            "2026-11-10T03:05:00+02:00",
        );
        let notes = in_progress_notes(&[bob, carol]);
        assert_eq!(notes[0], note);
        assert_eq!(
            notes[1],
            "note: a run by carol is in progress (started 2026-11-10 01:50 UTC, \
             last activity 01:05 UTC); both can run, the server keeps one copy of each \
             version"
        );
    }

    #[test]
    fn a_run_carries_on_past_other_runs_in_progress() {
        for in_progress in [
            "[]",
            r#"[{"started_by":"bob","started_at":"t","last_activity_at":"t"}]"#,
            r#"[{"started_by":"bob","started_at":"t","last_activity_at":"t"},
                {"started_by":"carol","started_at":"t","last_activity_at":"t"}]"#,
        ] {
            let mut plan = plan(vec![(hash(1), Before::OnServer, None)], Vec::new());
            let started = format!(
                r#"{{"run_id":"00000000-0000-0000-0000-000000000000","started_at":"t",
                    "compression":"none","in_progress":{in_progress}}}"#
            );
            let manifest =
                r#"{"missing":[],"compression":"none","recorded":1,"already_migrated":0}"#;
            let (url, seen) = serve(vec![(201, started.leak() as &str), (200, manifest)]);
            let mut report = Report::new(crate::report::Command::Run);
            run(&mut plan, &client(url), &mut report, true, false).unwrap();
            assert_eq!(report.run_id, Some(uuid::Uuid::nil()));
            assert_eq!(plan.progress.migrated, 1);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 2);
            assert!(seen[1].contains("/migration/manifest"), "{}", seen[1]);
        }
    }

    #[test]
    fn thousands_are_separated() {
        assert_eq!(
            [0, 999, 1000, 1_234_567].map(thousands),
            ["0", "999", "1,000", "1,234,567"]
        );
    }

    #[test]
    fn refused_content_counts_as_missing() {
        let plan = plan(
            vec![
                (hash(1), Before::OnServer, None),
                (
                    hash(2),
                    Before::Local,
                    Some(After::Uploaded { created: true }),
                ),
                (
                    hash(3),
                    Before::Local,
                    Some(After::Corrupt("content hash mismatch".into())),
                ),
                (hash(4), Before::Absent, None),
            ],
            Vec::new(),
        );
        let t = plan.totals(Phase::Run { recorded: 2 });
        assert_eq!(
            (t.unique_hashes, t.on_server, t.to_upload, t.missing),
            (4, 1, 1, 2)
        );
        assert_eq!(t.uploaded, Some(1));
        let kinds: Vec<ProblemKind> = plan.problems().iter().map(Problem::kind).collect();
        assert_eq!(kinds, vec![ProblemKind::Corrupt, ProblemKind::Missing]);
    }

    #[test]
    fn the_summary_counts_content_the_server_already_had_as_uploaded() {
        let plan = plan(
            vec![
                (hash(1), Before::OnServer, None),
                (
                    hash(2),
                    Before::Local,
                    Some(After::Uploaded { created: false }),
                ),
                (hash(3), Before::Local, None),
                (hash(4), Before::Absent, None),
            ],
            Vec::new(),
        );
        let mut report = Report::new(crate::report::Command::Run);
        plan.fill(&mut report, Phase::Run { recorded: 4 });
        assert_eq!(report.totals.as_ref().unwrap().uploaded, Some(0));
        let summary = report.summary("repo", None);
        assert!(
            summary.contains("\n  On server  2\n  Uploaded   1\n  Missing    1\n"),
            "{summary}"
        );
    }

    #[test]
    fn unreadable_content_is_reported_missing() {
        let plan = plan(
            vec![(
                hash(1),
                Before::Local,
                Some(After::Unreadable("gone".into())),
            )],
            Vec::new(),
        );
        assert_eq!(plan.totals(Phase::Run { recorded: 0 }).missing, 1);
        let missing: Vec<Option<String>> = plan
            .problems()
            .into_iter()
            .map(|p| match p {
                Problem::Missing { hash, .. } => Some(hash),
                _ => None,
            })
            .collect();
        assert_eq!(missing, vec![Some(hash(1))]);
    }

    #[test]
    fn writable_storage_ends_run_with_the_command_to_make_it_read_only() {
        let store = tempfile::tempdir().unwrap();
        let path = store.path().to_string_lossy().into_owned();
        let mut scope = plan(Vec::new(), Vec::new()).scope;
        assert_eq!(storage_writable_warning(&scope), None);
        scope.storage = vec![StorageSeen {
            path: path.clone(),
            kind: StoreKind::Dvs1,
            remapped_to: None,
        }];
        let warning = storage_writable_warning(&scope).unwrap();
        assert!(warning.starts_with("warning: local storage is still writable"));
        assert!(
            warning.ends_with(&format!("chmod -R a-w '{path}'")),
            "{warning}"
        );
    }
}
