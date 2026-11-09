//! From git history to entries: one per (data path, blake3), at the commit that introduced
//! it, with who added it according to the sidecar, the store's audit log and the commit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::BufReader;
use std::path::Path;
use std::rc::Rc;

use anyhow::Result;
use dvs::audit::{Action, parse_audit_log};
use dvs::backends::Backend;
use dvs::config::Config;
use dvs::legacy::{
    DVS1_CONFIG_FILE, Dvs1Config, Sidecar, SidecarShape, classify_sidecar, is_blake3,
};
use dvs::paths::CONFIG_FILE_NAME;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::git::{Commit, DvsItem, DvsTree, Objects};
use crate::report::Problem;

const AUDIT_LOG: &str = "audit.log.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreKind {
    Dvs1,
    Dvs2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSeen {
    pub path: String,
    pub shape: StoreKind,
    pub remapped_to: Option<String>,
}

impl StorageSeen {
    pub fn effective(&self) -> &str {
        self.remapped_to.as_deref().unwrap_or(&self.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    AuditLog,
    Sidecar,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryCommit {
    pub id: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
}

/// One (path, hash), as the manifest sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub shape: SidecarShape,
    pub sidecar_path: String,
    pub storage_path: Option<String>,
    pub sidecar_user: Option<String>,
    pub recorded_user: String,
    pub recorded_user_source: Source,
    pub recorded_at: String,
    pub recorded_at_source: Source,
    pub message: Option<String>,
    pub commit: EntryCommit,
    pub user_mismatch: bool,
    /// The kind of store `storage_path` is: dvs2-local blobs may be compressed
    #[serde(skip)]
    pub store_kind: Option<StoreKind>,
}

pub struct Walk {
    pub commits: usize,
    pub storage: Vec<StorageSeen>,
    pub sidecars: BTreeMap<SidecarShape, usize>,
    pub entries: Vec<Entry>,
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

/// Where a pair is in one commit: the sidecar path (interned in `Fold::paths`), its blob
/// (`Objects::blob`) and the store its config names
#[derive(Clone, Copy)]
struct Site {
    path: u32,
    blob: u32,
    store: Option<usize>,
}

/// What a commit's `dvs.toml` or `dvs.yaml` says about where its sidecars and content are
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

/// `Fold::paths` index of a path with no parent folder
const TOP: u32 = u32::MAX;

struct Fold<'a> {
    objects: &'a mut Objects,
    storage_map: &'a BTreeMap<String, String>,
    storage: Vec<StorageSeen>,
    pairs: HashMap<(String, String), u32>,
    paths: Vec<String>,
    path_ids: HashMap<(u32, u32), u32>,
    folders: Vec<Option<String>>,
    /// The pair and shape of a sidecar (path, blob) under a metadata folder, if it has one
    sites: HashMap<(u32, u32, usize), Option<(u32, SidecarShape)>>,
    blobs: HashMap<u32, std::result::Result<Rc<Parsed>, String>>,
    configs: HashMap<String, std::result::Result<ConfigFacts, String>>,
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
        pairs: HashMap::new(),
        paths: Vec::new(),
        path_ids: HashMap::new(),
        folders: Vec::new(),
        sites: HashMap::new(),
        blobs: HashMap::new(),
        configs: HashMap::new(),
        reported: HashSet::new(),
        problems: Vec::new(),
    };

    // A commit's pairs are kept only until its last child is walked
    let mut children: HashMap<&str, usize> = HashMap::new();
    for commit in commits {
        for parent in &commit.parents {
            *children.entry(parent.as_str()).or_default() += 1;
        }
    }
    let mut pairs_of: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut introduced: HashMap<u32, (Timestamp, usize, Site)> = HashMap::new();
    for (index, commit) in commits.iter().enumerate() {
        let mut found = fold.commit_sites(commit)?;
        // Stable, so a pair keeps its first site in tree order
        found.sort_by_key(|(pair, _)| *pair);
        found.dedup_by_key(|(pair, _)| *pair);
        for (pair, site) in &found {
            let in_a_parent = commit.parents.iter().any(|p| {
                pairs_of
                    .get(p.as_str())
                    .is_some_and(|pairs| pairs.binary_search(pair).is_ok())
            });
            if in_a_parent {
                continue;
            }
            let older = introduced
                .get(pair)
                .is_none_or(|(time, _, _)| commit.author_time < *time);
            if older {
                introduced.insert(*pair, (commit.author_time, index, *site));
            }
        }
        for parent in &commit.parents {
            if let Some(left) = children.get_mut(parent.as_str()) {
                *left -= 1;
                if *left == 0 {
                    pairs_of.remove(parent.as_str());
                }
            }
        }
        if children.contains_key(commit.id.as_str()) {
            pairs_of.insert(
                &commit.id,
                found.into_iter().map(|(pair, _)| pair).collect(),
            );
        }
    }

    let mut audit = AuditLogs::default();
    let keys: HashMap<u32, (String, String)> =
        fold.pairs.iter().map(|(k, v)| (*v, k.clone())).collect();
    let mut entries = Vec::with_capacity(introduced.len());
    for (pair, (_, index, site)) in introduced {
        let (path, hash) = keys[&pair].clone();
        let parsed = fold.blobs[&site.blob]
            .clone()
            .expect("only parsed sidecars have pairs");
        let store = site.store.map(|i| &fold.storage[i]);
        let logged = match store {
            Some(s) if s.shape == StoreKind::Dvs2 => audit.find(s.effective(), &path, &hash),
            _ => None,
        };
        let mut entry = entry(
            path,
            hash,
            &parsed,
            fold.paths[site.path as usize].clone(),
            store.map(|s| s.effective().to_string()),
            logged,
            &commits[index],
        );
        entry.store_kind = store.map(|s| s.shape);
        entries.push(entry);
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
    path: String,
    hash: String,
    parsed: &Parsed,
    sidecar_path: String,
    storage_path: Option<String>,
    logged: Option<(Timestamp, String)>,
    commit: &Commit,
) -> Entry {
    let (logged_at, logged_by) = logged.unzip();
    let author = || {
        let name = non_empty(&commit.author_name);
        name.or_else(|| non_empty(&commit.author_email))
            .unwrap_or_else(|| "unknown".to_string())
    };
    let (recorded_user, recorded_user_source) =
        match (logged_by.as_deref().and_then(non_empty), &parsed.user) {
            (Some(user), _) => (user, Source::AuditLog),
            (None, Some(user)) => (user.clone(), Source::Sidecar),
            (None, None) => (author(), Source::Commit),
        };
    let (recorded_at, recorded_at_source) = match (logged_at, parsed.time) {
        (Some(time), _) => (time, Source::AuditLog),
        (None, Some(time)) => (time, Source::Sidecar),
        (None, None) => (commit.author_time, Source::Commit),
    };
    Entry {
        user_mismatch: parsed
            .user
            .as_deref()
            .is_some_and(|user| user_mismatch(user, commit)),
        path,
        hash,
        size: parsed.size,
        shape: parsed.shape,
        sidecar_path,
        storage_path,
        sidecar_user: parsed.user.clone(),
        recorded_user,
        recorded_user_source,
        recorded_at: recorded_at.to_string(),
        recorded_at_source,
        message: parsed.message.clone(),
        commit: EntryCommit {
            id: commit.id.clone(),
            author_name: commit.author_name.clone(),
            author_email: commit.author_email.clone(),
            author_date: commit.author_time.to_string(),
        },
        store_kind: None,
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
    fn commit_sites(&mut self, commit: &Commit) -> Result<Vec<(u32, Site)>> {
        let mut folder = None;
        let mut dvs2_store = None;
        if let Some(blob) = self.objects.root_file(&commit.tree, CONFIG_FILE_NAME)? {
            let config = self.config(CONFIG_FILE_NAME, &blob, commit)?;
            // An unreadable dvs.toml still has its sidecars in the default folder
            folder = Some(config.metadata_folder.unwrap_or_else(|| ".dvs".to_string()));
            dvs2_store = config.store.map(|(path, kind)| self.store(path, kind));
        }
        let mut dvs1_store = None;
        if let Some(blob) = self.objects.root_file(&commit.tree, DVS1_CONFIG_FILE)? {
            let config = self.config(DVS1_CONFIG_FILE, &blob, commit)?;
            dvs1_store = config.store.map(|(path, kind)| self.store(path, kind));
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
        self.visit(&tree, TOP, &context, commit, &mut found)?;
        Ok(found)
    }

    fn visit(
        &mut self,
        tree: &DvsTree,
        folder: u32,
        context: &Context,
        commit: &Commit,
        found: &mut Vec<(u32, Site)>,
    ) -> Result<()> {
        for item in &tree.items {
            match item {
                DvsItem::Dir { name, tree } => {
                    let path = self.path(folder, *name);
                    self.visit(tree, path, context, commit, found)?;
                }
                DvsItem::File { name, blob } => {
                    let path = self.path(folder, *name);
                    let Some((pair, shape)) = self.site(path, *blob, context.folder, commit)?
                    else {
                        continue;
                    };
                    let store = match shape {
                        SidecarShape::Dvs2 => context.dvs2_store.or(context.dvs1_store),
                        _ => context.dvs1_store.or(context.dvs2_store),
                    };
                    let site = Site {
                        path,
                        blob: *blob,
                        store,
                    };
                    found.push((pair, site));
                }
            }
        }
        Ok(())
    }

    fn path(&mut self, folder: u32, name: u32) -> u32 {
        if let Some(id) = self.path_ids.get(&(folder, name)) {
            return *id;
        }
        let name_str = self.objects.name(name);
        let path = match folder {
            TOP => name_str.to_string(),
            _ => format!("{}/{name_str}", self.paths[folder as usize]),
        };
        let id = self.paths.len() as u32;
        self.paths.push(path);
        self.path_ids.insert((folder, name), id);
        id
    }

    /// The pair a sidecar names, read once per path, content and metadata folder
    fn site(
        &mut self,
        path: u32,
        blob: u32,
        folder: usize,
        commit: &Commit,
    ) -> Result<Option<(u32, SidecarShape)>> {
        if let Some(site) = self.sites.get(&(path, blob, folder)) {
            return Ok(*site);
        }
        let sidecar_path = self.paths[path as usize].clone();
        let mut site = None;
        if let Some(parsed) = self.sidecar(&sidecar_path, blob, commit)? {
            let in_folder = self.folders[folder]
                .as_deref()
                .and_then(|f| sidecar_path.strip_prefix(f)?.strip_prefix('/'));
            let within = in_folder.unwrap_or(&sidecar_path);
            match dvs::legacy::sidecar_data_path(within) {
                _ if parsed.shape == SidecarShape::Dvs2 && in_folder.is_none() => self
                    .unrecognized(
                        &sidecar_path,
                        blob,
                        commit,
                        "a dvs2 sidecar outside a dvs2 metadata folder",
                    ),
                Some(data) => {
                    let next = self.pairs.len() as u32;
                    let pair = *self
                        .pairs
                        .entry((data.as_str().to_string(), parsed.hash.clone()))
                        .or_insert(next);
                    site = Some((pair, parsed.shape));
                }
                None => self.unrecognized(&sidecar_path, blob, commit, "names no data file"),
            }
        }
        self.sites.insert((path, blob, folder), site);
        Ok(site)
    }

    fn sidecar(&mut self, path: &str, blob: u32, commit: &Commit) -> Result<Option<Rc<Parsed>>> {
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
            Ok(p) if !is_blake3(&p.hash) => Err(format!("invalid blake3 hash {:?}", p.hash)),
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
    fn unrecognized(&mut self, path: &str, blob: u32, commit: &Commit, message: &str) {
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

    fn store(&mut self, path: String, shape: StoreKind) -> usize {
        if let Some(i) = self
            .storage
            .iter()
            .position(|s| s.path == path && s.shape == shape)
        {
            return i;
        }
        self.storage.push(StorageSeen {
            remapped_to: self.storage_map.get(&path).cloned(),
            path,
            shape,
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
    if name == DVS1_CONFIG_FILE {
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

/// The earliest add of each (path, hash) in one store's audit log: when, and by whom
type Adds = HashMap<(String, String), (Timestamp, String)>;

/// The dvs2-local audit logs, read once per store
#[derive(Default)]
struct AuditLogs {
    stores: HashMap<String, Adds>,
}

impl AuditLogs {
    fn find(&mut self, store: &str, path: &str, hash: &str) -> Option<(Timestamp, String)> {
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
            Some((time, _)) if *time <= e.timestamp => {}
            _ => {
                adds.insert(key, (e.timestamp, e.user));
            }
        }
    }
    adds
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

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

    fn find<'a>(walk: &'a Walk, path: &str, hash: &str) -> &'a Entry {
        walk.entries
            .iter()
            .find(|e| e.path == path && e.hash == hash)
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
        assert_eq!(find(&walk, "a.csv", &hash(1)).commit.id, first);
        let b = find(&walk, "b.csv", &hash(2));
        assert_eq!(
            (b.commit.id.as_str(), b.recorded_user.as_str()),
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
        assert_eq!(logged.recorded_user_source, Source::AuditLog);
        assert_eq!(logged.recorded_at, "2020-09-13T12:26:40Z");
        assert_eq!(logged.sidecar_path, "meta/d/logged.csv.dvs");
        assert_eq!(logged.storage_path.as_deref(), Some(store_path.as_str()));

        let plain = find(&walk, "d/plain.csv", &hash(4));
        assert_eq!(
            (plain.recorded_user.as_str(), plain.recorded_user_source),
            ("JDOE", Source::Sidecar)
        );
        assert_eq!(plain.recorded_at, "2024-01-02T03:04:05Z");
        assert_eq!(plain.store_kind, Some(StoreKind::Dvs2));

        let old = find(&walk, "old.csv", &hash(5));
        assert_eq!(old.sidecar_user, None);
        assert_eq!(
            (old.recorded_user.as_str(), old.recorded_user_source),
            ("Jane Doe", Source::Commit)
        );
        assert_eq!(
            (old.recorded_at.as_str(), old.recorded_at_source),
            ("2023-01-01T00:00:00Z", Source::Commit)
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
            let logged = logged.map(|u| (Timestamp::UNIX_EPOCH, u.to_string()));
            let e = entry(
                "a".into(),
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
            ("jdoe@example.com".into(), Source::Commit, message.clone())
        );
        assert_eq!(
            user(None, &commit("", "")),
            ("unknown".into(), Source::Commit, message)
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
                shape: StoreKind::Dvs1,
                remapped_to: Some("/new/store".into()),
            }]
        );
        let kinds: Vec<&str> = walk.problems.iter().map(Problem::kind).collect();
        assert_eq!(kinds, vec!["unreadable_config", "unrecognized"]);
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
