//! From git history to entries: one per (data path, blake3), at the commit that introduced
//! it, with who added it according to the sidecar, the store's audit log and the commit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufReader;
use std::path::Path;
use std::rc::Rc;

use anyhow::Result;
use dvs::ProjectPath;
use dvs::audit::{Action, parse_audit_log};
use dvs::backends::Backend;
use dvs::config::Config;
use dvs::legacy::{DVS1_CONFIG_FILES, Dvs1Config, Sidecar, SidecarShape, classify_sidecar};
use dvs::migration::{
    AttributionSource, EntryCommit, EntryRef, ManifestEntry, Problem, StorageSeen, StoreKind,
};
use dvs::paths::CONFIG_FILE_NAME;
use jiff::Timestamp;

use crate::git::{BlobId, Commit, DvsItem, DvsTree, Objects};

const AUDIT_LOG: &str = "audit.log.jsonl";

pub struct Walk {
    pub commits: usize,
    pub storage: Vec<StorageSeen>,
    pub sidecars: BTreeMap<SidecarShape, usize>,
    pub entries: Vec<ManifestEntry>,
    pub problems: Vec<Problem>,
}

/// A sidecar blob's facts, read once however many commits hold it
struct Parsed {
    shape: SidecarShape,
    hash: String,
    size: u64,
    user: Option<String>,
    time: Option<Timestamp>,
    message: Option<String>,
}

/// A sidecar path, interned in `Fold::paths`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PathId(u32);

/// A pair, interned in `Fold::pairs`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct PairId(u32);

/// A path by the folder it is in, `None` at the top of the tree, and its name
/// (`Objects::name`)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PathKey {
    folder: Option<PathId>,
    name: u32,
}

/// Where a pair is in one commit: the sidecar's path and blob, and the store its config
/// names (a `Fold::storage` index)
#[derive(Clone, Copy)]
struct Site {
    path: PathId,
    blob: BlobId,
    store: Option<usize>,
}

/// The commit that introduced a pair, and where the pair is in it
#[derive(Clone, Copy)]
struct FirstSeen {
    time: Timestamp,
    commit_index: usize,
    site: Site,
}

/// A sidecar as read under one metadata folder (a `Fold::folders` index)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SiteKey {
    path: PathId,
    blob: BlobId,
    metadata_folder: usize,
}

/// The pair a sidecar names, and the sidecar's shape
#[derive(Clone, Copy)]
struct SidecarPair {
    pair: PairId,
    shape: SidecarShape,
}

/// What a commit's `dvs.toml`, `dvs.yaml` or `.dvs.yaml` says about where its sidecars and
/// content are
#[derive(Clone)]
struct ConfigFacts {
    store: Option<(String, StoreKind)>,
    metadata_folder: Option<String>,
}

/// What one commit's config makes of the sidecars in its tree
struct Context {
    folder: usize,
    dvs1_store: Option<usize>,
    dvs2_store: Option<usize>,
}

/// The pairs of each walked commit that still has children to walk, to tell whether a
/// pair was already in a parent. A commit's pairs are dropped once its last child is
/// walked.
struct ParentPairs<'c> {
    children_left: HashMap<&'c str, usize>,
    /// Sorted, for `binary_search`
    pairs: HashMap<&'c str, Vec<PairId>>,
}

impl<'c> ParentPairs<'c> {
    fn new(commits: &'c [Commit]) -> Self {
        let mut children_left: HashMap<&str, usize> = HashMap::new();
        for commit in commits {
            for parent in &commit.parents {
                *children_left.entry(parent.as_str()).or_default() += 1;
            }
        }
        Self {
            children_left,
            pairs: HashMap::new(),
        }
    }

    fn seen_in_parent(&self, commit: &Commit, pair: PairId) -> bool {
        commit.parents.iter().any(|parent| {
            self.pairs
                .get(parent.as_str())
                .is_some_and(|pairs| pairs.binary_search(&pair).is_ok())
        })
    }

    /// Counts `commit` as walked for each of its parents
    fn release_parents(&mut self, commit: &Commit) {
        for parent in &commit.parents {
            if let Some(left) = self.children_left.get_mut(parent.as_str()) {
                *left -= 1;
                if *left == 0 {
                    self.pairs.remove(parent.as_str());
                }
            }
        }
    }

    /// Keeps `commit`'s sorted pairs if it has children to walk
    fn keep(&mut self, commit: &'c Commit, pairs: Vec<PairId>) {
        if self.children_left.contains_key(commit.id.as_str()) {
            self.pairs.insert(&commit.id, pairs);
        }
    }
}

/// Turns every commit's sidecars into entries, one per *pair*: a (data path, blake3), that
/// is one version of one file.
///
/// Commits are walked parents first. Each `.dvs` file in a commit's tree names a pair and
/// gives a *site*: where the pair is in that commit (the sidecar's path and blob, and the
/// store the commit's config names). A pair already in one of the commit's parents is
/// skipped. Otherwise the commit introduces it: on its own branch, or after the pair was
/// removed and added back. Of all the commits that introduce a pair, the oldest by author
/// time wins, the first walked on a tie; its site is where the entry's facts come from.
///
/// Each commit's pairs are kept only until its last child is walked (`ParentPairs`).
/// Sidecar paths (`PathId`), pairs (`PairId`), parsed sidecar blobs (`BlobId`) and config
/// files are each read once and looked up by id after that.
struct Fold<'a> {
    objects: &'a mut Objects,
    storage_map: &'a BTreeMap<String, String>,
    storage: Vec<StorageSeen>,
    pairs: Vec<EntryRef>,
    pair_ids: HashMap<EntryRef, PairId>,
    paths: Vec<String>,
    path_ids: HashMap<PathKey, PathId>,
    /// Each metadata folder a commit's config names; `None` where there is no `dvs.toml`
    folders: Vec<Option<String>>,
    /// The pair each sidecar names, if it names one
    sites: HashMap<SiteKey, Option<SidecarPair>>,
    blobs: HashMap<BlobId, std::result::Result<Rc<Parsed>, String>>,
    configs: HashMap<String, std::result::Result<ConfigFacts, String>>,
    /// (sidecar or config path, blob) of each problem reported
    reported: HashSet<(String, String)>,
    problems: Vec<Problem>,
}

pub fn walk(
    objects: &mut Objects,
    commits: &[Commit],
    storage_map: &BTreeMap<String, String>,
) -> Result<Walk> {
    let mut fold = Fold {
        objects,
        storage_map,
        storage: Vec::new(),
        pairs: Vec::new(),
        pair_ids: HashMap::new(),
        paths: Vec::new(),
        path_ids: HashMap::new(),
        folders: Vec::new(),
        sites: HashMap::new(),
        blobs: HashMap::new(),
        configs: HashMap::new(),
        reported: HashSet::new(),
        problems: Vec::new(),
    };

    let mut parents = ParentPairs::new(commits);
    let mut introduced: HashMap<PairId, FirstSeen> = HashMap::new();
    for (commit_index, commit) in commits.iter().enumerate() {
        let mut found = fold.commit_sites(commit)?;
        // Stable, so a pair keeps its first site in tree order
        found.sort_by_key(|(pair, _)| *pair);
        found.dedup_by_key(|(pair, _)| *pair);
        for (pair, site) in &found {
            if parents.seen_in_parent(commit, *pair) {
                continue;
            }
            let older = introduced
                .get(pair)
                .is_none_or(|first| commit.author_time < first.time);
            if older {
                let first = FirstSeen {
                    time: commit.author_time,
                    commit_index,
                    site: *site,
                };
                introduced.insert(*pair, first);
            }
        }
        parents.release_parents(commit);
        parents.keep(commit, found.into_iter().map(|(pair, _)| pair).collect());
    }

    let mut audit = AuditLogs::default();
    let mut entries = Vec::with_capacity(introduced.len());
    for (pair, first) in introduced {
        let EntryRef { path, hash, .. } = fold.pairs[pair.0 as usize].clone();
        let parsed = fold.blobs[&first.site.blob]
            .clone()
            .expect("only parsed sidecars have pairs");
        let store = first.site.store.map(|i| &fold.storage[i]);
        let logged = match store {
            Some(s) if s.kind == StoreKind::Dvs2 => audit.find(s.effective(), path.as_str(), &hash),
            _ => None,
        };
        entries.push(entry(
            path,
            hash,
            &parsed,
            fold.paths[first.site.path.0 as usize].clone(),
            store.map(|s| s.effective().to_string()),
            logged,
            &commits[first.commit_index],
        ));
    }
    entries.sort_by(|a, b| (&a.path, &a.hash).cmp(&(&b.path, &b.hash)));

    let mut sidecars: BTreeMap<SidecarShape, usize> = [
        (SidecarShape::Dvs1V001, 0),
        (SidecarShape::Dvs1, 0),
        (SidecarShape::Dvs2, 0),
    ]
    .into();
    for parsed in fold.blobs.values().filter_map(|p| p.as_ref().ok()) {
        *sidecars.entry(parsed.shape).or_default() += 1;
    }

    for (storage_path, remapped_to) in storage_map {
        if !fold.storage.iter().any(|s| s.path == *storage_path) {
            fold.problems.push(Problem::UnusedStorageMap {
                storage_path: storage_path.clone(),
                remapped_to: remapped_to.clone(),
            });
        }
    }

    Ok(Walk {
        commits: commits.len(),
        storage: fold.storage,
        sidecars,
        entries,
        problems: fold.problems,
    })
}

/// Who added it and when: the store's audit log, then the sidecar, then the commit.
fn entry(
    path: ProjectPath,
    hash: String,
    parsed: &Parsed,
    sidecar_path: String,
    storage_path: Option<String>,
    logged: Option<LoggedAdd>,
    commit: &Commit,
) -> ManifestEntry {
    let (logged_at, logged_by) = logged.map(|add| (add.time, add.user)).unzip();
    let author = || {
        let name = non_empty(&commit.author_name);
        name.or_else(|| non_empty(&commit.author_email))
            .unwrap_or_else(|| "unknown".to_string())
    };
    let (recorded_user, recorded_user_source) =
        match (logged_by.as_deref().and_then(non_empty), &parsed.user) {
            (Some(user), _) => (user, AttributionSource::AuditLog),
            (None, Some(user)) => (user.clone(), AttributionSource::Sidecar),
            (None, None) => (author(), AttributionSource::Commit),
        };
    let (recorded_at, recorded_at_source) = match (logged_at, parsed.time) {
        (Some(time), _) => (time, AttributionSource::AuditLog),
        (None, Some(time)) => (time, AttributionSource::Sidecar),
        (None, None) => (commit.author_time, AttributionSource::Commit),
    };
    ManifestEntry {
        user_mismatch: parsed
            .user
            .as_deref()
            .is_some_and(|user| user_mismatch(user, commit)),
        operation: None,
        path,
        hash,
        size: parsed.size,
        shape: parsed.shape,
        sidecar_path,
        storage_path,
        sidecar_user: parsed.user.clone(),
        recorded_user,
        recorded_user_source,
        recorded_at,
        recorded_at_source,
        message: parsed.message.clone(),
        commit: Some(EntryCommit {
            id: commit.id.clone(),
            author_name: commit.author_name.clone(),
            author_email: commit.author_email.clone(),
            author_date: commit.author_time,
        }),
    }
}

fn user_mismatch(user: &str, commit: &Commit) -> bool {
    let local_part = commit.author_email.split('@').next().unwrap_or_default();
    let user = user.to_lowercase();
    user != commit.author_name.to_lowercase() && user != local_part.to_lowercase()
}

/// `s` without NUL, which the server refuses in text, or None if that leaves nothing
fn non_empty(s: &str) -> Option<String> {
    let s = s.replace('\0', "");
    (!s.is_empty()).then_some(s)
}

impl Fold<'_> {
    /// The pair and site of each sidecar in a commit's tree, in tree order
    fn commit_sites(&mut self, commit: &Commit) -> Result<Vec<(PairId, Site)>> {
        let mut folder = None;
        let mut dvs2_store = None;
        if let Some(blob) = self.objects.root_file(&commit.tree, CONFIG_FILE_NAME)? {
            let config = self.config(CONFIG_FILE_NAME, &blob, commit)?;
            // An unreadable dvs.toml still has its sidecars in the default folder
            folder = Some(config.metadata_folder.unwrap_or_else(|| ".dvs".to_string()));
            dvs2_store = config.store.map(|(path, kind)| self.store(path, kind));
        }
        let mut dvs1_store = None;
        // dvs1's `dvs.yaml`, else devious's `.dvs.yaml`
        for name in DVS1_CONFIG_FILES {
            if let Some(blob) = self.objects.root_file(&commit.tree, name)? {
                let config = self.config(name, &blob, commit)?;
                dvs1_store = config.store.map(|(path, kind)| self.store(path, kind));
                break;
            }
        }
        let folder = match self.folders.iter().position(|f| *f == folder) {
            Some(i) => i,
            None => {
                self.folders.push(folder);
                self.folders.len() - 1
            }
        };
        let context = Context {
            folder,
            dvs1_store,
            dvs2_store,
        };

        let tree = self.objects.dvs_tree(&commit.tree)?;
        let mut found = Vec::new();
        self.visit(&tree, None, &context, commit, &mut found)?;
        Ok(found)
    }

    fn visit(
        &mut self,
        tree: &DvsTree,
        folder: Option<PathId>,
        context: &Context,
        commit: &Commit,
        found: &mut Vec<(PairId, Site)>,
    ) -> Result<()> {
        for item in &tree.items {
            match item {
                DvsItem::Dir { name, tree } => {
                    let path = self.path(folder, *name);
                    self.visit(tree, Some(path), context, commit, found)?;
                }
                DvsItem::File { name, blob } => {
                    let path = self.path(folder, *name);
                    let Some(named) = self.site(path, *blob, context.folder, commit)? else {
                        continue;
                    };
                    let store = match named.shape {
                        SidecarShape::Dvs2 => context.dvs2_store.or(context.dvs1_store),
                        _ => context.dvs1_store.or(context.dvs2_store),
                    };
                    let site = Site {
                        path,
                        blob: *blob,
                        store,
                    };
                    found.push((named.pair, site));
                }
            }
        }
        Ok(())
    }

    fn path(&mut self, folder: Option<PathId>, name: u32) -> PathId {
        let key = PathKey { folder, name };
        if let Some(id) = self.path_ids.get(&key) {
            return *id;
        }
        let name = self.objects.name(name);
        let path = match folder {
            None => name.to_string(),
            Some(folder) => format!("{}/{name}", self.paths[folder.0 as usize]),
        };
        let id = PathId(self.paths.len() as u32);
        self.paths.push(path);
        self.path_ids.insert(key, id);
        id
    }

    fn pair(&mut self, pair: EntryRef) -> PairId {
        if let Some(id) = self.pair_ids.get(&pair) {
            return *id;
        }
        let id = PairId(self.pairs.len() as u32);
        self.pairs.push(pair.clone());
        self.pair_ids.insert(pair, id);
        id
    }

    /// The pair a sidecar names, read once per path, content and metadata folder
    fn site(
        &mut self,
        path: PathId,
        blob: BlobId,
        metadata_folder: usize,
        commit: &Commit,
    ) -> Result<Option<SidecarPair>> {
        let key = SiteKey {
            path,
            blob,
            metadata_folder,
        };
        if let Some(named) = self.sites.get(&key) {
            return Ok(*named);
        }
        let sidecar_path = self.paths[path.0 as usize].clone();
        let mut named = None;
        if let Some(parsed) = self.sidecar(&sidecar_path, blob, commit)? {
            let in_folder = self.folders[metadata_folder]
                .as_deref()
                .and_then(|f| sidecar_path.strip_prefix(f)?.strip_prefix('/'));
            if parsed.shape == SidecarShape::Dvs2 && in_folder.is_none() {
                self.unrecognized(
                    &sidecar_path,
                    blob,
                    commit,
                    "a dvs2 sidecar outside a dvs2 metadata folder",
                );
            } else if let Some(data) =
                dvs::legacy::sidecar_data_path(in_folder.unwrap_or(&sidecar_path))
            {
                let pair = self.pair(EntryRef {
                    path: data,
                    hash: parsed.hash.clone(),
                    operation: None,
                });
                named = Some(SidecarPair {
                    pair,
                    shape: parsed.shape,
                });
            } else {
                self.unrecognized(&sidecar_path, blob, commit, "names no data file");
            }
        }
        self.sites.insert(key, named);
        Ok(named)
    }

    fn sidecar(&mut self, path: &str, blob: BlobId, commit: &Commit) -> Result<Option<Rc<Parsed>>> {
        if let Some(parsed) = self.blobs.get(&blob).cloned() {
            if let Err(message) = &parsed {
                self.unrecognized(path, blob, commit, message);
            }
            return Ok(parsed.ok());
        }
        let oid = self.objects.blob(blob).to_string();
        let parsed = match classify_sidecar(&self.objects.read(&oid)?) {
            Ok(Sidecar::Dvs1(m)) => Ok(Parsed {
                shape: m.shape,
                time: m.recorded_at.parse().ok(),
                hash: m.blake3,
                size: m.size,
                user: non_empty(&m.saved_by),
                message: non_empty(&m.message),
            }),
            Ok(Sidecar::Dvs2(m)) => Ok(Parsed {
                shape: SidecarShape::Dvs2,
                hash: m.hashes.blake3,
                size: m.size,
                user: non_empty(&m.created_by),
                time: Some(m.add_time),
                message: m.message.as_deref().and_then(non_empty),
            }),
            Err(e) => Err(e.message),
        };
        let parsed = match parsed {
            Ok(p) if i64::try_from(p.size).is_err() => {
                Err(format!("size {} is too large for the server", p.size))
            }
            parsed => parsed,
        };
        let parsed = parsed.map(Rc::new);
        if let Err(message) = &parsed {
            self.unrecognized(path, blob, commit, message);
        }
        self.blobs.insert(blob, parsed.clone());
        Ok(parsed.ok())
    }

    /// Once per sidecar path and content
    fn unrecognized(&mut self, path: &str, blob: BlobId, commit: &Commit, message: &str) {
        let blob = self.objects.blob(blob).to_string();
        if self.reported.insert((path.to_string(), blob.clone())) {
            self.problems.push(Problem::Unrecognized {
                sidecar_path: path.to_string(),
                commit: commit.id.clone(),
                blob,
                message: message.to_string(),
            });
        }
    }

    fn unreadable(&mut self, config_path: &str, blob: &str, commit: &Commit, message: &str) {
        if self
            .reported
            .insert((config_path.to_string(), blob.to_string()))
        {
            self.problems.push(Problem::UnreadableConfig {
                config_path: config_path.to_string(),
                commit: commit.id.clone(),
                message: message.to_string(),
            });
        }
    }

    fn store(&mut self, path: String, kind: StoreKind) -> usize {
        if let Some(i) = self
            .storage
            .iter()
            .position(|s| s.path == path && s.kind == kind)
        {
            return i;
        }
        self.storage.push(StorageSeen {
            remapped_to: self.storage_map.get(&path).cloned(),
            path,
            kind,
        });
        self.storage.len() - 1
    }

    /// A config file, read once per content; an unreadable one is a problem and names no store
    fn config(&mut self, name: &str, blob: &str, commit: &Commit) -> Result<ConfigFacts> {
        let parsed = match self.configs.get(blob) {
            Some(parsed) => parsed.clone(),
            None => {
                let parsed = parse_config(name, &self.objects.read(blob)?);
                self.configs.insert(blob.to_string(), parsed.clone());
                parsed
            }
        };
        Ok(parsed.unwrap_or_else(|message| {
            self.unreadable(name, blob, commit, &message);
            ConfigFacts {
                store: None,
                metadata_folder: None,
            }
        }))
    }
}

fn parse_config(name: &str, bytes: &[u8]) -> std::result::Result<ConfigFacts, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    if DVS1_CONFIG_FILES.contains(&name) {
        let config = Dvs1Config::from_yaml(text).map_err(|e| format!("{e:#}"))?;
        return Ok(ConfigFacts {
            store: Some((
                config.storage_dir.to_string_lossy().into_owned(),
                StoreKind::Dvs1,
            )),
            metadata_folder: None,
        });
    }
    let config: Config = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let store = match config.backend() {
        Backend::Local(local) => Some((local.path.to_string_lossy().into_owned(), StoreKind::Dvs2)),
        Backend::Server(_) => None,
    };
    Ok(ConfigFacts {
        store,
        metadata_folder: Some(config.metadata_folder_name().to_string()),
    })
}

/// An add in a store's audit log: when, and by whom
#[derive(Clone)]
struct LoggedAdd {
    time: Timestamp,
    user: String,
}

/// The earliest add of each (path, hash) in one store's audit log
type Adds = HashMap<(String, String), LoggedAdd>;

/// The dvs2-local audit logs, read once per store
#[derive(Default)]
struct AuditLogs {
    stores: HashMap<String, Adds>,
}

impl AuditLogs {
    fn find(&mut self, store: &str, path: &str, hash: &str) -> Option<LoggedAdd> {
        let adds = self
            .stores
            .entry(store.to_string())
            .or_insert_with(|| read_audit_log(Path::new(store)));
        adds.get(&(path.to_string(), hash.to_string())).cloned()
    }
}

fn read_audit_log(store: &Path) -> Adds {
    let log = store.join(AUDIT_LOG);
    let entries = match std::fs::File::open(&log) {
        Ok(f) => parse_audit_log(BufReader::new(f), &HashSet::new()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return HashMap::new(),
        Err(e) => Err(e.into()),
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("warning: cannot read {}: {e:#}", log.display());
            return HashMap::new();
        }
    };
    let mut adds = Adds::new();
    for e in entries {
        let Action::Add { file, .. } = e.action else {
            continue;
        };
        let path = file.path.to_string_lossy().replace('\\', "/");
        let key = (path, file.hashes.blake3);
        match adds.get(&key) {
            Some(add) if add.time <= e.timestamp => {}
            _ => {
                let add = LoggedAdd {
                    time: e.timestamp,
                    user: e.user,
                };
                adds.insert(key, add);
            }
        }
    }
    adds
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use dvs::migration::ProblemKind;

    use super::*;

    /// A scratch git repo with fixed authors and dates, isolated from the user's git config.
    pub struct Repo {
        _dir: tempfile::TempDir,
        pub path: PathBuf,
    }

    impl Repo {
        pub fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("repo");
            std::fs::create_dir(&path).unwrap();
            let repo = Self { _dir: dir, path };
            repo.git(&["init", "-q", "-b", "main"]);
            repo
        }

        pub fn git(&self, args: &[&str]) -> String {
            git_in(
                &self.path,
                args,
                "2023-01-01T00:00:00Z",
                "Jane Doe",
                "jdoe@example.com",
            )
        }

        pub fn write(&self, path: &str, content: &str) {
            let path = self.path.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        /// Commits everything as `author` <`email`> at `date`; returns the commit id
        pub fn commit(&self, date: &str, author: &str, email: &str) -> String {
            git_in(&self.path, &["add", "-A"], date, author, email);
            git_in(
                &self.path,
                &["commit", "-q", "--allow-empty", "-m", date],
                date,
                author,
                email,
            );
            self.git(&["rev-parse", "HEAD"]).trim().to_string()
        }

        pub fn walk(&self, storage_map: &BTreeMap<String, String>) -> Walk {
            let refs = crate::git::walked_refs(&self.path).unwrap();
            let commits = crate::git::commits(&self.path, &refs).unwrap();
            let mut objects = Objects::open(&self.path).unwrap();
            walk(&mut objects, &commits, storage_map).unwrap()
        }
    }

    pub fn git_in(dir: &Path, args: &[&str], date: &str, author: &str, email: &str) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", author)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_NAME", author)
            .env("GIT_COMMITTER_EMAIL", email)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    pub fn hash(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    /// An entry for `path` and `hash`, attributed to its commit
    pub fn manifest_entry(path: &str, hash: &str) -> ManifestEntry {
        let time: Timestamp = "2023-01-01T00:00:00Z".parse().unwrap();
        ManifestEntry {
            path: ProjectPath::from_canonical(path).unwrap(),
            hash: hash.into(),
            size: 5,
            shape: SidecarShape::Dvs1,
            sidecar_path: format!("{path}.dvs"),
            storage_path: None,
            sidecar_user: None,
            recorded_user: "jdoe".into(),
            recorded_user_source: AttributionSource::Commit,
            recorded_at: time,
            recorded_at_source: AttributionSource::Commit,
            message: None,
            commit: Some(EntryCommit {
                id: "c".into(),
                author_name: "jdoe".into(),
                author_email: String::new(),
                author_date: time,
            }),
            user_mismatch: false,
            operation: None,
        }
    }

    pub fn dvs1(hash: &str, user: &str, time: &str) -> String {
        format!(
            r#"{{"blake3_checksum": "{hash}", "size": 5, "add_time": "{time}", "message": "", "saved_by": "{user}"}}"#
        )
    }

    fn dvs2(hash: &str, user: &str) -> String {
        format!(
            r#"{{"hashes": {{"blake3": "{hash}"}}, "size": 5, "created_by": "{user}",
                "add_time": "2024-01-02T03:04:05Z", "compression": "zstd"}}"#
        )
    }

    fn find<'a>(walk: &'a Walk, path: &str, hash: &str) -> &'a ManifestEntry {
        walk.entries
            .iter()
            .find(|e| e.path.as_str() == path && e.hash == hash)
            .unwrap_or_else(|| panic!("no entry ({path}, {hash})"))
    }

    #[test]
    fn each_pair_is_introduced_by_the_oldest_commit_that_adds_it() {
        let repo = Repo::new();
        repo.write("dvs.yaml", "storage_dir: /store\n");
        repo.write("a.csv.dvs", &dvs1(&hash(1), "jdoe", "2023-01-01T00:00:00Z"));
        let first = repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        // Same pair on two branches; the newer add, and the merge, introduce nothing
        repo.git(&["checkout", "-q", "-b", "dev"]);
        repo.write("b.csv.dvs", &dvs1(&hash(2), "amy", "2023-03-01T00:00:00Z"));
        repo.commit("2023-03-01T00:00:00Z", "Amy", "amy@example.com");
        repo.git(&["checkout", "-q", "main"]);
        repo.write("b.csv.dvs", &dvs1(&hash(2), "bob", "2023-02-01T00:00:00Z"));
        let older = repo.commit("2023-02-01T00:00:00Z", "Bob", "bob@example.com");
        repo.git(&["merge", "-q", "-s", "ours", "dev", "-m", "merge"]);

        // Removed and added back: still the first add
        std::fs::remove_file(repo.path.join("a.csv.dvs")).unwrap();
        repo.commit("2023-04-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        repo.write("a.csv.dvs", &dvs1(&hash(1), "jdoe", "2023-05-01T00:00:00Z"));
        repo.commit("2023-05-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let walk = repo.walk(&BTreeMap::new());
        assert_eq!(walk.entries.len(), 2, "{:?}", walk.entries);
        assert_eq!(
            find(&walk, "a.csv", &hash(1)).commit.as_ref().unwrap().id,
            first
        );
        let b = find(&walk, "b.csv", &hash(2));
        assert_eq!(
            (
                b.commit.as_ref().unwrap().id.as_str(),
                b.recorded_user.as_str()
            ),
            (older.as_str(), "bob")
        );
        assert_eq!(walk.sidecars[&SidecarShape::Dvs1], 4);
    }

    #[test]
    fn provenance_prefers_audit_log_then_sidecar_then_commit() {
        let store = tempfile::tempdir().unwrap();
        let store_path = store.path().to_str().unwrap().to_string();
        let audit = format!(
            r#"{{"operation_id": "x", "timestamp": 1600000000, "user": "logged",
                "action": {{"add": {{"file": {{"path": "d/logged.csv", "hashes": {{"blake3": "{}"}}}},
                "compression": "zstd"}}}}}}"#,
            hash(3)
        );
        std::fs::write(store.path().join(AUDIT_LOG), audit.replace('\n', "") + "\n").unwrap();

        let repo = Repo::new();
        repo.write(
            "dvs.toml",
            &format!("metadata_folder_name = \"meta\"\n\n[backend]\npath = {store_path:?}\n"),
        );
        repo.write("meta/d/logged.csv.dvs", &dvs2(&hash(3), "sidecar-user"));
        repo.write("meta/d/plain.csv.dvs", &dvs2(&hash(4), "JDOE"));
        repo.write("old.csv.dvs", &dvs1(&hash(5), "", "not a time"));
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let walk = repo.walk(&BTreeMap::new());
        let logged = find(&walk, "d/logged.csv", &hash(3));
        assert_eq!(logged.recorded_user, "logged");
        assert_eq!(logged.recorded_user_source, AttributionSource::AuditLog);
        assert_eq!(logged.recorded_at.to_string(), "2020-09-13T12:26:40Z");
        assert_eq!(logged.sidecar_path, "meta/d/logged.csv.dvs");
        assert_eq!(logged.storage_path.as_deref(), Some(store_path.as_str()));

        let plain = find(&walk, "d/plain.csv", &hash(4));
        assert_eq!(
            (plain.recorded_user.as_str(), plain.recorded_user_source),
            ("JDOE", AttributionSource::Sidecar)
        );
        assert_eq!(plain.recorded_at.to_string(), "2024-01-02T03:04:05Z");

        let old = find(&walk, "old.csv", &hash(5));
        assert_eq!(old.sidecar_user, None);
        assert_eq!(
            (old.recorded_user.as_str(), old.recorded_user_source),
            ("Jane Doe", AttributionSource::Commit)
        );
        assert_eq!(
            (old.recorded_at.to_string(), old.recorded_at_source),
            (
                "2023-01-01T00:00:00Z".to_string(),
                AttributionSource::Commit
            )
        );
    }

    #[test]
    fn recorded_user_is_never_empty_and_text_has_no_nul() {
        let parsed = Parsed {
            shape: SidecarShape::Dvs1,
            hash: hash(1),
            size: 5,
            user: non_empty("\0"),
            time: None,
            message: non_empty("first\0 pull"),
        };
        let commit = |name: &str, email: &str| Commit {
            id: "c".into(),
            tree: String::new(),
            parents: Vec::new(),
            author_name: name.into(),
            author_email: email.into(),
            author_time: Timestamp::UNIX_EPOCH,
        };
        let user = |logged: Option<&str>, c: &Commit| {
            let logged = logged.map(|u| LoggedAdd {
                time: Timestamp::UNIX_EPOCH,
                user: u.to_string(),
            });
            let e = entry(
                ProjectPath::from_canonical("a").unwrap(),
                hash(1),
                &parsed,
                "a.dvs".into(),
                None,
                logged,
                c,
            );
            (e.recorded_user, e.recorded_user_source, e.message)
        };
        let message = Some("first pull".to_string());
        assert_eq!(
            user(Some(""), &commit("", "jdoe@example.com")),
            (
                "jdoe@example.com".into(),
                AttributionSource::Commit,
                message.clone()
            )
        );
        assert_eq!(
            user(None, &commit("", "")),
            ("unknown".into(), AttributionSource::Commit, message)
        );
    }

    #[test]
    fn a_size_the_server_cannot_store_is_unrecognized() {
        let repo = Repo::new();
        repo.write(
            "a.csv.dvs",
            &dvs1(&hash(1), "jdoe", "2023-01-01T00:00:00Z").replace("5", &u64::MAX.to_string()),
        );
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        let walk = repo.walk(&BTreeMap::new());
        assert!(walk.entries.is_empty(), "{:?}", walk.entries);
        assert!(matches!(
            &walk.problems[..],
            [Problem::Unrecognized { message, .. }] if message.contains("too large")
        ));
    }

    #[test]
    fn user_mismatch_ignores_case_and_matches_the_email_local_part() {
        let commit = Commit {
            id: String::new(),
            tree: String::new(),
            parents: Vec::new(),
            author_name: "Jane Doe".into(),
            author_email: "JDoe@example.com".into(),
            author_time: Timestamp::UNIX_EPOCH,
        };
        assert!(!user_mismatch("jane doe", &commit));
        assert!(!user_mismatch("jdoe", &commit));
        assert!(user_mismatch("bob", &commit));
    }

    #[test]
    fn storage_comes_from_each_commits_config_with_remaps() {
        let repo = Repo::new();
        repo.write("dvs.yaml", "storage_dir: /old/store\n");
        repo.write("a.csv.dvs", &dvs1(&hash(1), "jdoe", "2023-01-01T00:00:00Z"));
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        repo.write("dvs.yaml", "storage_dir: [unterminated\n");
        repo.write("b.csv.dvs", &dvs1(&hash(2), "jdoe", "2023-02-01T00:00:00Z"));
        repo.write("junk.dvs", "{}");
        repo.commit("2023-02-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let map = BTreeMap::from([("/old/store".to_string(), "/new/store".to_string())]);
        let walk = repo.walk(&map);
        assert_eq!(
            find(&walk, "a.csv", &hash(1)).storage_path.as_deref(),
            Some("/new/store")
        );
        assert_eq!(find(&walk, "b.csv", &hash(2)).storage_path, None);
        assert_eq!(
            walk.storage,
            vec![StorageSeen {
                path: "/old/store".into(),
                kind: StoreKind::Dvs1,
                remapped_to: Some("/new/store".into()),
            }]
        );
        let kinds: Vec<ProblemKind> = walk.problems.iter().map(Problem::kind).collect();
        assert_eq!(
            kinds,
            vec![ProblemKind::UnreadableConfig, ProblemKind::Unrecognized]
        );
    }

    #[test]
    fn a_storage_map_key_seen_nowhere_is_a_problem() {
        let repo = Repo::new();
        repo.write("dvs.yaml", "storage_dir: /old/store\n");
        repo.write("a.csv.dvs", &dvs1(&hash(1), "jdoe", "2023-01-01T00:00:00Z"));
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let map = BTreeMap::from([
            ("/old/store".to_string(), "/new/store".to_string()),
            ("/old/stor".to_string(), "/elsewhere".to_string()),
        ]);
        let walk = repo.walk(&map);
        assert_eq!(
            walk.problems,
            vec![Problem::UnusedStorageMap {
                storage_path: "/old/stor".into(),
                remapped_to: "/elsewhere".into(),
            }]
        );
    }

    #[test]
    fn devious_sidecars_and_config_are_read_from_history() {
        let repo = Repo::new();
        repo.write(".dvs.yaml", "storage-dir: /devious/store\n");
        repo.write(
            "data/a.csv.dvsmeta",
            &format!(
                r#"{{"blake3_checksum": "{}", "file_size_bytes": 5,
                    "timestamp": "2023-08-15T10:11:12.5-04:00", "message": "", "user": "amy"}}"#,
                hash(1)
            ),
        );
        repo.write("b.csv.dvsmeta", &format!(r#"{{"FileHash":"{}"}}"#, hash(2)));
        repo.commit("2023-08-16T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let walk = repo.walk(&BTreeMap::new());
        assert_eq!(
            walk.storage,
            vec![StorageSeen {
                path: "/devious/store".into(),
                kind: StoreKind::Dvs1,
                remapped_to: None,
            }]
        );
        let a = find(&walk, "data/a.csv", &hash(1));
        assert_eq!(a.shape, SidecarShape::DeviousV1);
        assert_eq!(a.sidecar_path, "data/a.csv.dvsmeta");
        assert_eq!(a.storage_path.as_deref(), Some("/devious/store"));
        assert_eq!(
            (a.recorded_user.as_str(), a.recorded_at.to_string()),
            ("amy", "2023-08-15T14:11:12.5Z".to_string())
        );
        let b = find(&walk, "b.csv", &hash(2));
        assert_eq!((b.shape, b.size), (SidecarShape::DeviousV0, 0));
        assert_eq!(b.recorded_user_source, AttributionSource::Commit);
        assert_eq!(walk.sidecars[&SidecarShape::DeviousV1], 1);
        assert!(walk.problems.is_empty(), "{:?}", walk.problems);
    }

    #[test]
    fn a_dvs2_sidecar_outside_a_dvs2_metadata_folder_is_unrecognized() {
        let repo = Repo::new();
        repo.write("dvs.yaml", "storage_dir: /store\n");
        repo.write(".dvs/a.csv.dvs", &dvs2(&hash(1), "jdoe"));
        repo.write("b.csv.dvs", &dvs1(&hash(2), "jdoe", "2023-01-01T00:00:00Z"));
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");

        let walk = repo.walk(&BTreeMap::new());
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["b.csv"]);
        assert!(matches!(
            &walk.problems[..],
            [Problem::Unrecognized { sidecar_path, message, .. }]
                if sidecar_path == ".dvs/a.csv.dvs" && message.contains("outside a dvs2 metadata folder")
        ));
    }
}
