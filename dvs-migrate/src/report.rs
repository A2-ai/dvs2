//! The report document every command prints with `--json` (and `run` sends): dvs2's
//! `--json` envelope plus scope, totals, entries and problems.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io;

use dvs::legacy::SidecarShape;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::git::{PartialReason, redact};
use crate::walk::{Entry, StorageSeen};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[cfg(test)]
impl Problem {
    pub fn kind(&self) -> &'static str {
        match self {
            Problem::Missing { .. } => "missing",
            Problem::Corrupt { .. } => "corrupt",
            Problem::UploadRefused { .. } => "upload_refused",
            Problem::SizeMismatch { .. } => "size_mismatch",
            Problem::Unrecognized { .. } => "unrecognized",
            Problem::UnreadableConfig { .. } => "unreadable_config",
            Problem::UnmatchedUser { .. } => "unmatched_user",
            Problem::UnusedStorageMap { .. } => "unused_storage_map",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WalkKind {
    Full,
    Partial,
}

#[derive(Debug, Clone, Serialize)]
pub struct Scope {
    pub source: String,
    pub walk: WalkKind,
    pub partial_reasons: Vec<PartialReason>,
    pub refs: Vec<String>,
    pub commits: usize,
    pub storage: Vec<StorageSeen>,
}

#[derive(Debug, Clone, Default, Serialize)]
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
    pub uploaded: Option<usize>,
    pub migrated: Option<usize>,
    /// Distinct contents this run sent, stored now or already (200); only the summary uses
    /// it, so its line adds up to the distinct contents.
    #[serde(skip)]
    pub sent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Content {
    OnServer,
    ToUpload,
    Uploaded,
    Missing,
    Corrupt,
    Refused,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub content: Content,
    pub already_migrated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Command {
    Plan,
    Run,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub v: u32,
    pub command: Command,
    pub outcome: dvs::RunOutcome,
    pub dry_run: bool,
    pub error: Option<Value>,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub run_id: Option<Uuid>,
    pub scope: Option<Scope>,
    pub totals: Option<Totals>,
    pub entries: Vec<ReportEntry>,
    pub problems: Vec<Problem>,
}

impl Report {
    pub fn new(command: Command) -> Self {
        Report {
            v: 1,
            command,
            outcome: dvs::RunOutcome::Complete,
            dry_run: command == Command::Plan,
            error: None,
            started_at: String::new(),
            finished_at: String::new(),
            duration_ms: 0,
            run_id: None,
            scope: None,
            totals: None,
            entries: Vec::new(),
            problems: Vec::new(),
        }
    }

    /// The document the `report` call takes: all of it but the entries, which the
    /// manifest already sent.
    pub fn for_server(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("a report serializes");
        if let Some(map) = value.as_object_mut() {
            map.remove("entries");
        }
        value
    }

    /// What stdout gets: the document with `--json`, else the summary. Nothing else is
    /// written; the server keeps each run's report.
    pub fn print(
        &self,
        json: bool,
        source: &str,
        dest: Option<&Destination>,
        out: &mut dyn io::Write,
    ) -> io::Result<()> {
        if json {
            serde_json::to_writer(&mut *out, self)?;
            writeln!(out)
        } else {
            out.write_all(self.summary(source, dest).as_bytes())
        }
    }

    /// For stderr, whatever stdout gets
    pub fn warnings(&self) -> Vec<String> {
        self.problems
            .iter()
            .filter_map(|p| match p {
                Problem::UnusedStorageMap { storage_path, .. } => Some(format!(
                    "warning: --storage-map {storage_path} matches no storage path in history"
                )),
                _ => None,
            })
            .collect()
    }

    /// A few lines to read at a glance; every entry and problem is in `--json` and on the
    /// Migration page.
    pub fn summary(&self, source: &str, dest: Option<&Destination>) -> String {
        let mut out = String::new();
        let command = match self.command {
            Command::Plan => "plan",
            Command::Run => "run",
        };
        // A blank line keeps the summary apart from the progress lines above it on a terminal
        let _ = match dest {
            Some(d) => writeln!(
                out,
                "\ndvs-migrate {command}  {source} -> {} (project {})\n",
                redact(d.server.as_str()),
                d.project
            ),
            None => writeln!(out, "\ndvs-migrate {command}  {source}\n"),
        };

        let mut facts: Vec<(&str, String)> = Vec::new();
        if let Some(error) = &self.error {
            facts.push((
                "Failed",
                error["message"].as_str().unwrap_or_default().to_string(),
            ));
        }
        if let Some(scope) = &self.scope {
            let walk = match scope.walk {
                WalkKind::Full => "full".to_string(),
                WalkKind::Partial => {
                    let reasons: Vec<String> = scope
                        .partial_reasons
                        .iter()
                        .map(|r| match r {
                            PartialReason::Shallow => "the clone is shallow".to_string(),
                            PartialReason::UnreachableRemote { remote, url, .. } => {
                                format!("remote {remote} ({url}) unreachable")
                            }
                        })
                        .collect();
                    format!("PARTIAL: {}", reasons.join("; "))
                }
            };
            facts.push((
                "Walk",
                format!(
                    "{} on {} ({walk})",
                    plural(scope.commits, "commit"),
                    plural(scope.refs.len(), "ref")
                ),
            ));
        }
        if let Some(t) = &self.totals {
            // A hash the server never asked for was there by then, though `lookup` missed it.
            let (on_server, done) = match t.sent {
                Some(n) => (
                    t.unique_hashes.saturating_sub(n + t.missing),
                    ("Uploaded", n),
                ),
                None => (t.on_server, ("To upload", t.to_upload)),
            };
            facts.push((
                "Versions",
                format!(
                    "{} ({} distinct, {})",
                    t.entries,
                    t.unique_hashes,
                    dvs::format_size(t.bytes)
                ),
            ));
            facts.push(("On server", on_server.to_string()));
            facts.push((done.0, done.1.to_string()));
            facts.push(("Missing", t.missing.to_string()));
        }
        let width = facts.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (key, value) in &facts {
            let _ = writeln!(out, "  {key:<width$}  {value}");
        }

        let missing: Vec<(String, &str, String)> = self
            .problems
            .iter()
            .filter_map(|p| {
                let (hash, paths, reason) = match p {
                    Problem::Missing {
                        hash,
                        paths,
                        storage,
                        storage_exists,
                        ..
                    } => (
                        hash,
                        paths,
                        match (storage, storage_exists) {
                            (Some(_), Some(false)) => "storage folder not found",
                            (Some(_), _) => "not in storage folder",
                            (None, _) => "no storage path in config",
                        }
                        .to_string(),
                    ),
                    Problem::Corrupt { hash, paths, .. } => {
                        (hash, paths, "local copy doesn't match its hash".to_string())
                    }
                    Problem::UploadRefused {
                        hash,
                        paths,
                        status,
                        ..
                    } => (
                        hash,
                        paths,
                        match status {
                            413 => "refused by the server (too large)".to_string(),
                            s => format!("refused by the server ({s})"),
                        },
                    ),
                    _ => return None,
                };
                let mut path = paths.first().map_or("?", String::as_str).to_string();
                if paths.len() > 1 {
                    path = format!("{path} (+{})", plural(paths.len() - 1, "other path"));
                }
                Some((path, hash.get(..12).unwrap_or(hash), reason))
            })
            .collect();
        if !missing.is_empty() {
            let _ = writeln!(out, "\n  Missing files");
            let shown = &missing[..missing.len().min(MISSING_SHOWN)];
            let width = shown.iter().map(|(path, ..)| path.len()).max().unwrap_or(0);
            for (path, hash, reason) in shown {
                let _ = writeln!(out, "    {path:<width$}  {hash}  {reason}");
            }
            if missing.len() > MISSING_SHOWN {
                let _ = writeln!(out, "    and {} more", missing.len() - MISSING_SHOWN);
            }
        }

        let mut not_found: BTreeMap<&str, usize> = BTreeMap::new();
        for p in &self.problems {
            if let Problem::Missing {
                storage: Some(store),
                storage_exists: Some(false),
                ..
            } = p
            {
                *not_found.entry(store).or_default() += 1;
            }
        }
        if !not_found.is_empty() {
            let _ = writeln!(
                out,
                "\n  Storage folders not found here (point them elsewhere with --storage-map OLD=NEW)"
            );
            for (store, n) in &not_found {
                let _ = writeln!(out, "    {store}  ({})", plural(*n, "missing content"));
            }
        }

        let mut others: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for p in &self.problems {
            let words = match p {
                Problem::SizeMismatch { .. } => ("size mismatch", "size mismatches"),
                Problem::Unrecognized { .. } => {
                    (".dvs file not recognized", ".dvs files not recognized")
                }
                Problem::UnreadableConfig { .. } => (
                    "config that could not be read",
                    "configs that could not be read",
                ),
                Problem::UnmatchedUser { .. } => (
                    "recorded user not on the server",
                    "recorded users not on the server",
                ),
                _ => continue,
            };
            *others.entry(words).or_default() += 1;
        }
        if !others.is_empty() {
            let others: Vec<String> = others
                .iter()
                .map(|((one, many), n)| format!("{n} {}", if *n == 1 { one } else { many }))
                .collect();
            let _ = writeln!(out, "\n  Also: {}", others.join(", "));
        }

        let _ = writeln!(out);
        if self.command == Command::Plan {
            let _ = writeln!(out, "  Nothing was sent (plan).");
        }
        if let Some(d) = dest {
            let _ = writeln!(out, "  Migration page: {}", redact(&d.migration_page()));
        }
        out
    }
}

const MISSING_SHOWN: usize = 20;

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Where a command sends to
pub struct Destination {
    pub server: Url,
    pub project: String,
}

impl Destination {
    pub fn migration_page(&self) -> String {
        dvs::backends::server::join_url(
            &self.server,
            &format!("project/{}/migration", self.project),
        )
        .map_or_else(|_| self.server.to_string(), |u| u.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::tests::hash;

    fn report() -> Report {
        let mut report = Report::new(Command::Run);
        report.scope = Some(Scope {
            source: "git@host:org/proj.git".into(),
            walk: WalkKind::Partial,
            partial_reasons: vec![PartialReason::Shallow],
            refs: vec!["refs/heads/main".into(), "refs/tags/v1".into()],
            commits: 12,
            storage: Vec::new(),
        });
        report.totals = Some(Totals {
            entries: 60,
            unique_hashes: 45,
            bytes: 2048,
            on_server: 1,
            to_upload: 20,
            missing: 24,
            uploaded: Some(20),
            sent: Some(20),
            migrated: Some(37),
            ..Totals::default()
        });
        // data/1.csv's storage folder is gone; the rest are simply not in theirs
        let missing = |n: u8| Problem::Missing {
            hash: hash(n),
            size: 5,
            paths: vec![format!("data/{n}.csv")],
            looked_at: None,
            storage: Some(if n == 1 { "/old-share" } else { "/store" }.into()),
            storage_exists: Some(n != 1),
        };
        report.problems = vec![
            missing(1),
            Problem::UploadRefused {
                hash: hash(2),
                paths: vec!["big.bin".into(), "copy/big.bin".into()],
                looked_at: None,
                status: 413,
                message: "too large".into(),
            },
            Problem::Corrupt {
                hash: hash(3),
                paths: vec!["bad.csv".into()],
                looked_at: None,
                message: "content hash mismatch".into(),
            },
            Problem::SizeMismatch {
                hash: hash(4),
                recorded_sizes: vec![5, 6],
                actual_size: None,
                looked_at: None,
            },
            Problem::UnmatchedUser {
                user: "jdoe".into(),
                entries: 4,
            },
            Problem::UnusedStorageMap {
                storage_path: "/old".into(),
                remapped_to: "/new".into(),
            },
        ];
        report.problems.extend((5..=25).map(missing));
        report
    }

    fn printed(report: &Report, json: bool) -> String {
        let dest = Destination {
            server: "https://admin:pw@dvs.example.com".parse().unwrap(),
            project: "proj".into(),
        };
        let mut out = Vec::new();
        report
            .print(json, "git@host:org/proj.git", Some(&dest), &mut out)
            .unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn the_summary_is_a_few_aligned_sections() {
        let summary = printed(&report(), false);
        let lines: Vec<&str> = summary.lines().collect();
        assert_eq!(
            lines[..13],
            [
                "",
                "dvs-migrate run  git@host:org/proj.git -> https://***@dvs.example.com/ (project proj)",
                "",
                "  Walk       12 commits on 2 refs (PARTIAL: the clone is shallow)",
                "  Versions   60 (45 distinct, 2.0 KB)",
                "  On server  1",
                "  Uploaded   20",
                "  Missing    24",
                "",
                "  Missing files",
                &format!(
                    "    data/1.csv               {}  storage folder not found",
                    &hash(1)[..12]
                ),
                &format!(
                    "    big.bin (+1 other path)  {}  refused by the server (too large)",
                    &hash(2)[..12]
                ),
                &format!(
                    "    bad.csv                  {}  local copy doesn't match its hash",
                    &hash(3)[..12]
                ),
            ]
        );
        assert_eq!(
            lines[lines.len() - 8..],
            [
                "    and 4 more",
                "",
                "  Storage folders not found here (point them elsewhere with --storage-map OLD=NEW)",
                "    /old-share  (1 missing content)",
                "",
                "  Also: 1 recorded user not on the server, 1 size mismatch",
                "",
                "  Migration page: https://***@dvs.example.com/project/proj/migration",
            ]
        );
        assert_eq!(lines.len(), 10 + 20 + 8);

        let mut plan = report();
        plan.command = Command::Plan;
        let totals = plan.totals.as_mut().unwrap();
        (totals.uploaded, totals.sent, plan.problems) = (None, None, Vec::new());
        let summary = printed(&plan, false);
        assert!(
            summary.contains("\n  To upload  20\n  Missing    24\n"),
            "{summary}"
        );
        assert!(
            summary.contains("\n  Nothing was sent (plan).\n"),
            "{summary}"
        );
    }

    #[test]
    fn json_prints_only_the_document() {
        let report = report();
        let out = printed(&report, true);
        let parsed: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed, serde_json::to_value(&report).unwrap());
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn an_unused_storage_map_is_a_warning() {
        assert_eq!(
            report().warnings(),
            ["warning: --storage-map /old matches no storage path in history"]
        );
    }
}
