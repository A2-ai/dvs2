//! The query strings of the server's list routes, built on the client side: every
//! field optional, repeatable ones as lists, sent only when set.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Query parameters, in order; a repeatable key appears once per value.
pub type Pairs = Vec<(&'static str, String)>;

fn one(pairs: &mut Pairs, key: &'static str, value: &Option<impl ToString>) {
    if let Some(v) = value {
        pairs.push((key, v.to_string()));
    }
}

fn each(pairs: &mut Pairs, key: &'static str, values: &[String]) {
    pairs.extend(values.iter().map(|v| (key, v.clone())));
}

/// `GET /api/v1/projects`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectsQuery {
    /// Minimum role: `read`, `write` or `admin`
    pub role: Option<String>,
    /// `false`, `true` or `any`; the server lists unlocked projects when unset
    pub locked: Option<String>,
    /// Name contains, case-insensitive
    pub q: Option<String>,
    pub stats: Option<bool>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl ProjectsQuery {
    pub fn pairs(&self) -> Pairs {
        let mut p = Pairs::new();
        one(&mut p, "role", &self.role);
        one(&mut p, "locked", &self.locked);
        one(&mut p, "q", &self.q);
        one(&mut p, "stats", &self.stats);
        one(&mut p, "limit", &self.limit);
        one(&mut p, "cursor", &self.cursor);
        p
    }
}

/// `GET /api/v1/groups`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupsQuery {
    /// Name contains, case-insensitive
    pub q: Option<String>,
    /// Only groups you belong to
    pub mine: Option<bool>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl GroupsQuery {
    pub fn pairs(&self) -> Pairs {
        let mut p = Pairs::new();
        one(&mut p, "q", &self.q);
        one(&mut p, "mine", &self.mine);
        one(&mut p, "limit", &self.limit);
        one(&mut p, "cursor", &self.cursor);
        p
    }
}

/// `GET …/files` and `…/files/summary`: the filters apply to each path's latest upload.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FilesQuery {
    /// Paths starting with this, e.g. `data/derived/`
    pub path_prefix: Option<String>,
    /// `*` within a folder, `**` across folders
    pub path_glob: Option<String>,
    /// A hash, or a prefix of at least 8 characters
    pub hash: Option<String>,
    /// Username or email, or `me`
    pub user: Vec<String>,
    /// RFC 3339 or YYYY-MM-DD
    pub from: Option<String>,
    pub to: Option<String>,
    pub branch: Vec<String>,
    /// Bytes, inclusive
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub yanked: Option<bool>,
    /// `path`, `uploaded_at`, `size`, `versions`, …; `-` first for descending
    pub sort: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl FilesQuery {
    pub fn pairs(&self) -> Pairs {
        let mut p = Pairs::new();
        one(&mut p, "path_prefix", &self.path_prefix);
        one(&mut p, "path_glob", &self.path_glob);
        one(&mut p, "hash", &self.hash);
        each(&mut p, "user", &self.user);
        one(&mut p, "from", &self.from);
        one(&mut p, "to", &self.to);
        each(&mut p, "branch", &self.branch);
        one(&mut p, "min_size", &self.min_size);
        one(&mut p, "max_size", &self.max_size);
        one(&mut p, "yanked", &self.yanked);
        one(&mut p, "sort", &self.sort);
        one(&mut p, "limit", &self.limit);
        one(&mut p, "cursor", &self.cursor);
        p
    }

    /// The same filters for `…/files/summary`, which takes no paging or sort.
    pub fn summary_pairs(&self) -> Pairs {
        let mut p = self.pairs();
        p.retain(|(k, _)| !matches!(*k, "sort" | "limit" | "cursor"));
        p
    }
}

/// `GET …/activity`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ActivityQuery {
    /// Exact paths
    pub path: Vec<String>,
    pub path_prefix: Option<String>,
    pub path_glob: Option<String>,
    /// A hash, or a prefix of at least 8 characters
    pub hash: Option<String>,
    pub user: Vec<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub branch: Vec<String>,
    /// A prefix of at least 7 characters
    pub commit: Option<String>,
    /// `upload`, `download`, or a project change such as `lock` or `yank`
    pub kind: Vec<String>,
    pub batch: Option<Uuid>,
    /// `batch` lists runs of `dvs add` and `dvs get` instead of single files
    pub group_by: Option<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

impl ActivityQuery {
    pub fn pairs(&self) -> Pairs {
        let mut p = Pairs::new();
        each(&mut p, "path", &self.path);
        one(&mut p, "path_prefix", &self.path_prefix);
        one(&mut p, "path_glob", &self.path_glob);
        one(&mut p, "hash", &self.hash);
        each(&mut p, "user", &self.user);
        one(&mut p, "from", &self.from);
        one(&mut p, "to", &self.to);
        each(&mut p, "branch", &self.branch);
        one(&mut p, "commit", &self.commit);
        each(&mut p, "kind", &self.kind);
        one(&mut p, "batch", &self.batch);
        one(&mut p, "group_by", &self.group_by);
        one(&mut p, "limit", &self.limit);
        one(&mut p, "cursor", &self.cursor);
        p
    }

    /// Search one path argument the way the web's search box does: a folder (ending in
    /// `/`) by prefix, a pattern with `*` as a glob, anything else exactly.
    pub fn set_path(&mut self, path: &str) {
        if path.ends_with('/') {
            self.path_prefix = Some(path.to_string());
        } else if path.contains('*') {
            self.path_glob = Some(path.to_string());
        } else {
            self.path.push(path.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_set_fields_are_sent_and_lists_repeat() {
        assert!(FilesQuery::default().pairs().is_empty());
        let q = FilesQuery {
            path_prefix: Some("data/".into()),
            user: vec!["sam".into(), "me".into()],
            min_size: Some(1024),
            yanked: Some(false),
            sort: Some("-size".into()),
            limit: Some(5),
            ..Default::default()
        };
        let owned = q.pairs();
        let pairs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("path_prefix", "data/"),
                ("user", "sam"),
                ("user", "me"),
                ("min_size", "1024"),
                ("yanked", "false"),
                ("sort", "-size"),
                ("limit", "5"),
            ]
        );
        assert!(
            q.summary_pairs()
                .iter()
                .all(|(k, _)| *k != "sort" && *k != "limit")
        );
    }

    #[test]
    fn a_path_argument_reads_as_the_search_box_does() {
        let mut q = ActivityQuery::default();
        q.set_path("data/");
        q.set_path("data/**/*.csv");
        q.set_path("data/a.csv");
        assert_eq!(q.path_prefix.as_deref(), Some("data/"));
        assert_eq!(q.path_glob.as_deref(), Some("data/**/*.csv"));
        assert_eq!(q.path, ["data/a.csv"]);
        let batch = Uuid::nil();
        q.batch = Some(batch);
        q.kind = vec!["upload".into(), "lock".into()];
        let keys: Vec<&str> = q.pairs().iter().map(|(k, _)| *k).collect();
        assert_eq!(
            keys,
            ["path", "path_prefix", "path_glob", "kind", "kind", "batch"]
        );
    }
}
