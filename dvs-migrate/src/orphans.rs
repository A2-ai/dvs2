//! `orphans`: the blobs in the stores some repos' histories name that none of their
//! commits references (R23). Reads only: it never deletes, and needs no key or server.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;
use dvs::migration::{Problem, WalkKind};
use serde::Serialize;

use crate::commands::{Walked, walk_repo};
use crate::report::plural;
use crate::storage;

#[derive(clap::Args)]
pub struct Args {
    /// The projects' git repos, each a local clone or a URL, walked as `plan` walks one
    #[arg(required = true)]
    repos: Vec<String>,
    /// A storage path as written in the projects' configs, and where it is now (repeatable)
    #[arg(long = "storage-map", value_name = "OLD=NEW", value_parser = crate::parse_mapping)]
    storage_map: Vec<(String, String)>,
    /// Print the list as JSON instead of text
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize)]
pub struct Orphans {
    pub repos: Vec<String>,
    pub stores: Vec<StoreOrphans>,
    pub orphan_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StoreOrphans {
    pub path: String,
    pub orphans: Vec<Orphan>,
    pub other_files: Vec<String>,
    pub orphan_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct Orphan {
    pub hash: String,
    /// Store-relative: a hash can be in a store in more than one layout
    pub path: String,
    pub size: u64,
    /// RFC 3339; null where the filesystem has no modification time
    pub modified: Option<String>,
}

pub fn main(args: &Args) -> i32 {
    let storage_map: BTreeMap<String, String> = args.storage_map.iter().cloned().collect();
    let found = args
        .repos
        .iter()
        .map(|repo| walk_repo(repo, &storage_map, args.json))
        .collect::<Result<Vec<_>>>()
        .map(|walks| find(&walks, &storage_map));
    let (orphans, warnings) = match found {
        Ok(found) => found,
        Err(e) => {
            eprintln!("error: {}", crate::git::redact(&format!("{e:#}")));
            return 1;
        }
    };
    if args.json {
        match serde_json::to_string(&orphans) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    } else {
        print!("{}", orphans.text());
    }
    for warning in warnings {
        eprintln!("{warning}");
    }
    0
}

/// The orphans of every store the walks saw that is here, and the warnings for stderr
pub fn find(walks: &[Walked], storage_map: &BTreeMap<String, String>) -> (Orphans, Vec<String>) {
    let mut warnings = Vec::new();
    let referenced: BTreeSet<&str> = walks
        .iter()
        .flat_map(|w| &w.walk.entries)
        .map(|e| e.hash.as_str())
        .collect();
    let unread_sidecars = walks
        .iter()
        .flat_map(|w| &w.walk.problems)
        .filter(|p| matches!(p, Problem::Unrecognized { .. }))
        .count();
    if unread_sidecars > 0 {
        warnings.push(format!(
            "warning: {} in history could not be read; content they name may be listed as \
             orphans",
            plural(unread_sidecars, "sidecar")
        ));
    }
    let mut stores: Vec<&str> = Vec::new();
    for seen in walks.iter().flat_map(|w| &w.scope.storage) {
        if !stores.contains(&seen.effective()) {
            stores.push(seen.effective());
        }
    }
    for walked in walks {
        if walked.scope.walk == WalkKind::Partial {
            warnings.push(format!(
                "warning: the walk of {} was partial, so blobs only its unwalked history \
                 references are listed as orphans",
                walked.scope.source
            ));
        }
    }
    for old in storage_map.keys() {
        let seen = walks
            .iter()
            .flat_map(|w| &w.scope.storage)
            .any(|s| s.path == *old);
        if !seen {
            warnings.push(format!(
                "warning: --storage-map {old} matches no storage path in history"
            ));
        }
    }

    let mut found = Vec::new();
    for store in stores {
        let listing = match storage::list(Path::new(store)) {
            Ok(listing) => listing,
            Err(e) => {
                warnings.push(format!(
                    "warning: cannot list storage folder {store}: {e}; point it elsewhere \
                     with --storage-map OLD=NEW"
                ));
                continue;
            }
        };
        warnings.extend(listing.warnings);
        // Every file of an unreferenced hash; of a referenced one, all but the copy
        // `find_blob` reads
        let orphans: Vec<Orphan> = listing
            .blobs
            .iter()
            .flat_map(|(hash, copies)| {
                let kept = usize::from(referenced.contains(hash.as_str()));
                copies.iter().skip(kept).map(move |stored| Orphan {
                    hash: hash.clone(),
                    path: stored.path.clone(),
                    size: stored.size,
                    modified: stored
                        .modified
                        .and_then(|m| jiff::Timestamp::try_from(m).ok())
                        .map(|m| m.to_string()),
                })
            })
            .collect();
        let orphan_bytes = orphans.iter().map(|o| o.size).sum();
        let blob_bytes = listing.bytes;
        if orphan_bytes * 2 > blob_bytes {
            warnings.push(format!(
                "warning: most of {store} is orphaned ({} of {}); the store may be shared \
                 with projects other than {}",
                dvs::format_size(orphan_bytes),
                dvs::format_size(blob_bytes),
                these_repos(walks.len())
            ));
        }
        found.push(StoreOrphans {
            path: store.to_string(),
            orphans,
            other_files: listing.other_files,
            orphan_bytes,
        });
    }
    let orphans = Orphans {
        repos: walks.iter().map(|w| w.scope.source.clone()).collect(),
        orphan_bytes: found.iter().map(|s| s.orphan_bytes).sum(),
        stores: found,
    };
    (orphans, warnings)
}

fn these_repos(n: usize) -> String {
    if n == 1 {
        "this repo".to_string()
    } else {
        format!("these {n} repos")
    }
}

impl Orphans {
    pub fn text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "\nOrphaned with respect to {}: blobs no commit references\n",
            these_repos(self.repos.len())
        );
        for store in &self.stores {
            let _ = writeln!(out, "  {}", store.path);
            for o in &store.orphans {
                let _ = writeln!(
                    out,
                    "    {}  {:>9}  {}",
                    o.path,
                    dvs::format_size(o.size),
                    o.modified.as_deref().unwrap_or("-")
                );
            }
            let _ = writeln!(
                out,
                "    {}, {}",
                plural(store.orphans.len(), "orphan"),
                dvs::format_size(store.orphan_bytes)
            );
            if !store.other_files.is_empty() {
                let _ = writeln!(
                    out,
                    "    {} (not blobs, never orphans):",
                    plural(store.other_files.len(), "other file")
                );
                for file in &store.other_files {
                    let _ = writeln!(out, "      {file}");
                }
            }
            let _ = writeln!(out);
        }
        let count = self.stores.iter().map(|s| s.orphans.len()).sum();
        let _ = writeln!(
            out,
            "Total: {}, {} in {}",
            plural(count, "orphan"),
            dvs::format_size(self.orphan_bytes),
            plural(self.stores.len(), "store")
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::tests::{Repo, dvs1, hash};

    /// Writes `content` as blob `n` of `store` in the dvs1 layout; returns its hash
    fn blob(store: &Path, n: u8, content: &str) -> String {
        let hash = hash(n);
        let path = dvs::legacy::blob_path(store, &hash);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
        hash
    }

    /// One repo, its store mapped to `store`, whose one sidecar names `hash`, plus `extra`
    /// files in its tree
    fn walk_one(store: &Path, hash: &str, extra: &[(&str, &str)]) -> (Repo, Walked) {
        let repo = Repo::new();
        repo.write("dvs.yaml", "storage_dir: /old\n");
        repo.write("a.dvs", &dvs1(hash, "jdoe", "2023-01-01T00:00:00Z"));
        for (path, content) in extra {
            repo.write(path, content);
        }
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        let map = BTreeMap::from([("/old".to_string(), store.to_str().unwrap().to_string())]);
        let walked = walk_repo(repo.path.to_str().unwrap(), &map, true).unwrap();
        (repo, walked)
    }

    #[test]
    fn lists_blobs_no_repo_references_per_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let (a, b) = (blob(&store, 1, "a"), blob(&store, 2, "bb"));
        let orphan = blob(&store, 3, "orphan, and the larger part of the store");
        std::fs::write(store.join("audit.log.jsonl"), "").unwrap();
        let store_path = store.to_str().unwrap();

        let (_ra, wa) = walk_one(&store, &a, &[]);
        let (_rb, wb) = walk_one(&store, &b, &[]);
        let (found, warnings) = find(&[wa, wb], &BTreeMap::new());

        // The R23 shape, in its order
        let json = serde_json::to_string(&found).unwrap();
        let mut from = 0;
        let mut next = |key: &str| {
            let at = json[from..]
                .find(&format!("\"{key}\":"))
                .map(|at| from + at);
            from = at.map_or(from, |at| at + 1);
            at
        };
        let order = [
            "repos",
            "stores",
            "path",
            "orphans",
            "hash",
            "path",
            "size",
            "modified",
            "other_files",
        ];
        assert!(order.iter().all(|key| next(key).is_some()), "{json}");
        assert!(json.ends_with(&format!(",\"orphan_bytes\":{}}}", found.orphan_bytes)));
        let store = &found.stores[0];
        assert_eq!(store.path, store_path);
        let hashes: Vec<&str> = store.orphans.iter().map(|o| o.hash.as_str()).collect();
        assert_eq!(hashes, [orphan.as_str()]);
        assert_eq!(store.other_files, ["audit.log.jsonl"]);
        assert!(store.orphans[0].modified.is_some());
        assert_eq!(found.orphan_bytes, 40);
        assert!(found.text().contains("with respect to these 2 repos"));
        assert!(
            warnings.iter().any(|w| w.contains("may be shared")),
            "{warnings:?}"
        );
    }

    #[test]
    fn the_shared_store_warning_needs_most_bytes_orphaned() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let kept = blob(&store, 1, "kept, the larger part");
        blob(&store, 2, "orphan");
        let (_repo, walked) = walk_one(&store, &kept, &[]);
        let (found, warnings) = find(&[walked], &BTreeMap::new());
        assert_eq!(found.stores[0].orphans.len(), 1);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(found.text().contains("with respect to this repo"));
    }

    #[test]
    fn every_file_of_a_hash_is_its_own_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let kept = blob(&store, 1, "kept, the larger part of the store, by far");
        let orphan = blob(&store, 2, "12");
        // Both hashes also left flat by an interrupted devious migrate
        std::fs::write(store.join(format!("{kept}.dvsfile")), "123").unwrap();
        std::fs::write(store.join(format!("{orphan}.dvsfile")), "1234").unwrap();
        let (_repo, walked) = walk_one(&store, &kept, &[]);
        let (found, _) = find(&[walked], &BTreeMap::new());

        let listed: Vec<(&str, &str, u64)> = found.stores[0]
            .orphans
            .iter()
            .map(|o| (o.hash.as_str(), o.path.as_str(), o.size))
            .collect();
        let (kept_flat, orphan_flat) = (format!("{kept}.dvsfile"), format!("{orphan}.dvsfile"));
        let orphan_nested = format!("02/{}", &orphan[2..]);
        assert_eq!(
            listed,
            [
                (kept.as_str(), kept_flat.as_str(), 3),
                (orphan.as_str(), orphan_nested.as_str(), 2),
                (orphan.as_str(), orphan_flat.as_str(), 4),
            ]
        );
        assert_eq!(found.orphan_bytes, 3 + 2 + 4);
        assert_eq!(found.stores[0].orphan_bytes, 3 + 2 + 4);
    }

    #[test]
    fn unreadable_sidecars_warn_that_what_they_name_may_be_listed() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let kept = blob(&store, 1, "kept, the larger part");
        blob(&store, 2, "orphan");
        let (_repo, walked) = walk_one(&store, &kept, &[("b.dvs", "not a sidecar")]);
        let (found, warnings) = find(&[walked], &BTreeMap::new());
        let line = "1 sidecar in history could not be read; content they name may be \
                    listed as orphans";
        assert_eq!(warnings, [format!("warning: {line}")]);
        // stderr only (R14): not in the text on stdout
        assert!(
            !found.text().contains("could not be read"),
            "{}",
            found.text()
        );

        let (_repo, walked) = walk_one(&store, &kept, &[]);
        let (found, warnings) = find(&[walked], &BTreeMap::new());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!found.text().contains("could not be read"));
    }
}
