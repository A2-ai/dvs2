//! plan and run: walk, ask the server what it has, and fill in the report.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use dvs::RunOutcome;
use serde_json::{Value, json};

use crate::client::{Client, EntryRef, LookupRequest, Upload, chunks};
use crate::git::{self, redact};
use crate::report::{Content, Problem, Report, ReportEntry, Scope, Totals, WalkKind, plural};
use crate::storage::{self, Blob};
use crate::walk::{self, Entry, StoreKind, Walk};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Before {
    OnServer,
    Local,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum After {
    Uploaded { created: bool },
    Corrupt(String),
    Refused(u16, String),
    Unreadable,
}

struct HashInfo {
    size: u64,
    sizes: BTreeSet<u64>,
    paths: Vec<String>,
    blob: Option<Blob>,
    looked_at: Option<String>,
    /// The storage folder looked in, and whether it exists here
    storage: Option<(String, bool)>,
    before: Before,
    after: Option<After>,
}

impl HashInfo {
    fn content(&self) -> Content {
        match (self.before, &self.after) {
            (Before::OnServer, _) => Content::OnServer,
            (Before::Absent, _) | (_, Some(After::Unreadable)) => Content::Missing,
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
    migrated: HashSet<(String, String)>,
    unmatched_users: Vec<String>,
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
        info.paths.push(e.path.clone());
    }
    let users: BTreeSet<&str> = walk
        .entries
        .iter()
        .map(|e| e.recorded_user.as_str())
        .collect();
    let lookup = lookup(
        client,
        hashes.keys().cloned().collect(),
        walk.entries
            .iter()
            .map(|e| EntryRef {
                path: e.path.clone(),
                hash: e.hash.clone(),
            })
            .collect(),
        users.into_iter().map(str::to_string).collect(),
    )?;

    let mut candidates: HashMap<&str, Vec<(&str, StoreKind)>> = HashMap::new();
    for e in &walk.entries {
        if let (Some(store), Some(kind)) = (e.storage_path.as_deref(), e.store_kind) {
            candidates.entry(&e.hash).or_default().push((store, kind));
        }
    }
    for (hash, info) in hashes.iter_mut() {
        if lookup.have.contains(hash) {
            info.before = Before::OnServer;
            continue;
        }
        let candidates = candidates.get(hash.as_str()).map_or(&[][..], Vec::as_slice);
        let (blob, looked_at) = storage::find(hash, info.size, candidates);
        info.before = if blob.is_some() {
            Before::Local
        } else {
            Before::Absent
        };
        info.blob = blob;
        info.looked_at = looked_at;
        // Prefer a folder that exists: then the hash is simply not in it
        info.storage = candidates
            .iter()
            .map(|(store, _)| (store.to_string(), Path::new(store).is_dir()))
            .max_by_key(|(_, exists)| *exists);
    }

    Ok(Plan {
        scope,
        walk,
        hashes,
        migrated: lookup.migrated,
        unmatched_users: lookup.unmatched_users,
    })
}

struct Lookup {
    have: HashSet<String>,
    migrated: HashSet<(String, String)>,
    unmatched_users: Vec<String>,
}

fn lookup(
    client: &Client,
    hashes: Vec<String>,
    entries: Vec<EntryRef>,
    users: Vec<String>,
) -> Result<Lookup> {
    let (h, e, u) = (chunks(&hashes), chunks(&entries), chunks(&users));
    let calls = h.len().max(e.len()).max(u.len()).max(1);
    let mut found = Lookup {
        have: HashSet::new(),
        migrated: HashSet::new(),
        unmatched_users: Vec::new(),
    };
    for i in 0..calls {
        let req = LookupRequest {
            hashes: h.get(i).map(|c| c.to_vec()).unwrap_or_default(),
            entries: e.get(i).map(|c| c.to_vec()).unwrap_or_default(),
            users: u.get(i).map(|c| c.to_vec()).unwrap_or_default(),
        };
        let answer = client.lookup(&req)?;
        found.have.extend(answer.have);
        found
            .migrated
            .extend(answer.migrated.into_iter().map(|e| (e.path, e.hash)));
        found.unmatched_users.extend(answer.unmatched_users);
    }
    found.unmatched_users.sort();
    Ok(found)
}

impl Plan {
    fn already_migrated(&self, e: &Entry) -> bool {
        self.migrated.contains(&(e.path.clone(), e.hash.clone()))
    }

    pub fn totals(&self, run: Option<usize>) -> Totals {
        let count = |f: &dyn Fn(&HashInfo) -> bool| self.hashes.values().filter(|h| f(h)).count();
        let missing = |h: &HashInfo| {
            matches!(
                h.content(),
                Content::Missing | Content::Corrupt | Content::Refused
            )
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
            uploaded: run
                .map(|_| count(&|h| matches!(h.after, Some(After::Uploaded { created: true })))),
            migrated: run,
            sent: run.map(|_| count(&|h| matches!(h.after, Some(After::Uploaded { .. })))),
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
                    storage: h.storage.as_ref().map(|(store, _)| store.clone()),
                    storage_exists: h.storage.as_ref().map(|(_, exists)| *exists),
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

    /// Fills the report's scope, totals, entries and problems; `migrated` is a run's count.
    pub fn fill(&self, report: &mut Report, migrated: Option<usize>) {
        report.scope = Some(self.scope.clone());
        report.totals = Some(self.totals(migrated));
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
    let walk_kind = serde_json::to_value(plan.scope.walk)?;
    let totals = serde_json::to_value(plan.totals(None))?;
    let run = client.start_run(walk_kind.as_str().unwrap_or("partial"), &totals)?;
    report.run_id = Some(run.run_id);

    let entries: Vec<&Entry> = plan.walk.entries.iter().collect();
    let mut wanted = BTreeSet::new();
    let mut recorded = 0;
    for chunk in chunks(&entries) {
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
            Err(e) => {
                eprintln!("warning: cannot read {}: {e:#}", blob.path.display());
                After::Unreadable
            }
            Ok(bytes) => match client.upload(run.run_id, hash, bytes)? {
                Upload::Stored { created } => After::Uploaded { created },
                Upload::Corrupt(message) => After::Corrupt(message),
                Upload::Refused(status, message) => After::Refused(status, message),
                Upload::Unreadable(message) => {
                    eprintln!("warning: cannot read {}: {message}", blob.path.display());
                    After::Unreadable
                }
            },
        };
        plan.hashes.get_mut(hash).expect("listed").after = Some(after);
    }
    plan.fill(report, Some(recorded));
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
    use crate::walk::tests::hash;
    use crate::walk::{EntryCommit, Source, StorageSeen};

    fn entry(path: &str, hash: &str) -> Entry {
        Entry {
            path: path.into(),
            hash: hash.into(),
            size: 5,
            shape: dvs::legacy::SidecarShape::Dvs1,
            sidecar_path: format!("{path}.dvs"),
            storage_path: None,
            sidecar_user: None,
            recorded_user: "jdoe".into(),
            recorded_user_source: Source::Commit,
            recorded_at: "2023-01-01T00:00:00Z".into(),
            recorded_at_source: Source::Commit,
            message: None,
            commit: EntryCommit {
                id: "c".into(),
                author_name: "jdoe".into(),
                author_email: String::new(),
                author_date: "2023-01-01T00:00:00Z".into(),
            },
            user_mismatch: false,
            store_kind: None,
        }
    }

    /// One entry per (hash, before, after); every entry already migrated
    fn plan(hashes: Vec<(String, Before, Option<After>)>, storage: Vec<StorageSeen>) -> Plan {
        let entries: Vec<Entry> = hashes
            .iter()
            .enumerate()
            .map(|(i, (h, ..))| entry(&format!("f{i}"), h))
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
            migrated: entries
                .iter()
                .map(|e| (e.path.clone(), e.hash.clone()))
                .collect(),
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
        let t = plan.totals(Some(2));
        assert_eq!(
            (t.unique_hashes, t.on_server, t.to_upload, t.missing),
            (4, 1, 1, 2)
        );
        assert_eq!(t.uploaded, Some(1));
        let kinds: Vec<&str> = plan.problems().iter().map(Problem::kind).collect();
        assert_eq!(kinds, vec!["corrupt", "missing"]);
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
        plan.fill(&mut report, Some(4));
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
            vec![(hash(1), Before::Local, Some(After::Unreadable))],
            Vec::new(),
        );
        assert_eq!(plan.totals(Some(0)).missing, 1);
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
            shape: StoreKind::Dvs1,
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
