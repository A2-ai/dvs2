//! plan and run: walk, ask the server what it has, and fill in the report.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use dvs::RunOutcome;
use serde_json::{Value, json};

use dvs::migration::{EntryRef, LookupRequest, ManifestEntry, Problem, Scope, Totals, WalkKind};

use crate::client::{Client, chunks};
use crate::git::{self, redact};
use crate::report::{Content, Report, ReportEntry, plural};
use crate::storage::{self, Blob};
use crate::walk::{self, Walk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Before {
    OnServer,
    Local,
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

struct HashInfo {
    size: u64,
    sizes: BTreeSet<u64>,
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
}

/// What a fresh walk found and what the server already has of it
pub struct Plan {
    scope: Scope,
    walk: Walk,
    hashes: BTreeMap<String, HashInfo>,
    migrated: HashSet<EntryRef>,
    unmatched_users: Vec<String>,
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

pub fn plan(
    source: &str,
    storage_map: &BTreeMap<String, String>,
    client: &Client,
    quiet: bool,
) -> Result<Plan> {
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

    progress(quiet, "Asking the server what it has");
    let mut hashes: BTreeMap<String, HashInfo> = BTreeMap::new();
    for e in &walk.entries {
        let info = hashes.entry(e.hash.clone()).or_insert_with(|| HashInfo {
            size: e.size,
            sizes: BTreeSet::new(),
            paths: Vec::new(),
            blob: None,
            looked_at: None,
            storage: None,
            before: Before::Absent,
            after: None,
        });
        info.sizes.insert(e.size);
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
    )?;

    let mut stores: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &walk.entries {
        if let Some(store) = e.storage_path.as_deref() {
            stores.entry(&e.hash).or_default().push(store);
        }
    }
    for (hash, info) in hashes.iter_mut() {
        if lookup.have.contains(hash) {
            info.before = Before::OnServer;
            continue;
        }
        let stores = stores.get(hash.as_str()).map_or(&[][..], Vec::as_slice);
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
) -> Result<Lookup> {
    let hashes = chunks(&hashes);
    let entries = chunks(&entries);
    let users = chunks(&users);
    // Call `i` takes chunk `i` of each list, and there is always at least one call
    let calls = hashes.len().max(entries.len()).max(users.len()).max(1);
    let mut found = Lookup {
        have: HashSet::new(),
        migrated: HashSet::new(),
        unmatched_users: Vec::new(),
    };
    for i in 0..calls {
        let req = LookupRequest {
            hashes: chunk_at(&hashes, i),
            entries: chunk_at(&entries, i),
            users: chunk_at(&users, i),
        };
        let answer = client.lookup(&req)?;
        found.have.extend(answer.have);
        found.migrated.extend(answer.migrated);
        found.unmatched_users.extend(answer.unmatched_users);
    }
    found.unmatched_users.sort();
    Ok(found)
}

impl Plan {
    fn already_migrated(&self, e: &ManifestEntry) -> bool {
        self.migrated.contains(&entry_ref(e))
    }

    pub fn totals(&self, phase: Phase) -> Totals {
        let count = |f: &dyn Fn(&HashInfo) -> bool| self.hashes.values().filter(|h| f(h)).count();
        let missing = |h: &HashInfo| {
            matches!(
                h.content(),
                Content::Missing | Content::Corrupt | Content::Refused
            )
        };
        let (uploaded, migrated) = match phase {
            Phase::Plan => (None, None),
            Phase::Run { recorded } => {
                let created =
                    count(&|h| matches!(h.after, Some(After::Uploaded { created: true })));
                (Some(created), Some(recorded))
            }
        };
        Totals {
            sidecars: self.walk.sidecars.clone(),
            entries: self.walk.entries.len(),
            unique_hashes: self.hashes.len(),
            bytes: self.hashes.values().map(|h| h.size).sum(),
            on_server: count(&|h| h.before == Before::OnServer),
            to_upload: count(&|h| h.before == Before::Local && !missing(h)),
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
                    message: message.clone(),
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
            if h.sizes.len() > 1 || actual_size.is_some_and(|s| s != h.size) {
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

/// Opens a run, sends every entry, uploads what the server asks for that is here, and
/// fills the report. The caller sends it.
pub fn run(plan: &mut Plan, client: &Client, report: &mut Report, quiet: bool) -> Result<()> {
    let run = client.start_run(plan.scope.walk, plan.totals(Phase::Plan))?;
    report.run_id = Some(run.run_id);

    let mut wanted = BTreeSet::new();
    let mut recorded = 0;
    for chunk in chunks(&plan.walk.entries) {
        let answer = client.manifest(run.run_id, chunk)?;
        wanted.extend(answer.missing);
        recorded += answer.recorded;
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
        let after = match storage::open(&blob) {
            Err(e) => After::Unreadable(format!("{e:#}")),
            Ok(bytes) => client.upload(run.run_id, hash, bytes)?,
        };
        if let After::Unreadable(message) = &after {
            eprintln!("warning: cannot read {}: {message}", blob.path.display());
        }
        plan.hashes.get_mut(hash).expect("listed").after = Some(after);
    }
    plan.fill(report, Phase::Run { recorded });
    Ok(())
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

    use crate::walk::tests::{hash, manifest_entry};

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
                        paths: Vec::new(),
                        blob: None,
                        looked_at: None,
                        storage: None,
                        before,
                        after,
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
        }
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
