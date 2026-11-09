//! The report document every command prints with `--json` (and `run` sends): dvs2's
//! `--json` envelope plus scope, totals, entries and problems.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io;

use dvs::migration::{ManifestEntry, PartialReason, Problem, Scope, Totals, WalkKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
use uuid::Uuid;

use crate::git::redact;

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
    pub entry: ManifestEntry,
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
        target: Option<&ServerTarget>,
        out: &mut dyn io::Write,
    ) -> io::Result<()> {
        if json {
            serde_json::to_writer(&mut *out, self)?;
            writeln!(out)
        } else {
            out.write_all(self.summary(source, target).as_bytes())
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
    pub fn summary(&self, source: &str, target: Option<&ServerTarget>) -> String {
        let mut out = String::new();
        self.write_header(&mut out, source, target);
        self.write_facts(&mut out);
        self.write_missing(&mut out);
        self.write_stores_not_found(&mut out);
        self.write_also(&mut out);
        self.write_footer(&mut out, target);
        out
    }

    fn write_header(&self, out: &mut String, source: &str, target: Option<&ServerTarget>) {
        let command = match self.command {
            Command::Plan => "plan",
            Command::Run => "run",
        };
        // A blank line keeps the summary apart from the progress lines above it on a terminal
        let _ = match target {
            Some(t) => writeln!(
                out,
                "\ndvs-migrate {command}  {source} -> {} (project {})\n",
                redact(t.server.as_str()),
                t.project
            ),
            None => writeln!(out, "\ndvs-migrate {command}  {source}\n"),
        };
    }

    /// Failure, walk and counts, as aligned `key  value` lines
    fn write_facts(&self, out: &mut String) {
        let mut facts: Vec<(&str, String)> = Vec::new();
        if let Some(error) = &self.error {
            facts.push((
                "Failed",
                error["message"].as_str().unwrap_or_default().to_string(),
            ));
        }
        if let Some(scope) = &self.scope {
            facts.push((
                "Walk",
                format!(
                    "{} on {} ({})",
                    plural(scope.commits, "commit"),
                    plural(scope.refs.len(), "ref"),
                    walk_description(scope)
                ),
            ));
        }
        if let Some(t) = &self.totals {
            facts.push((
                "Versions",
                format!(
                    "{} ({} distinct, {})",
                    t.entries,
                    t.unique_hashes,
                    dvs::format_size(t.bytes)
                ),
            ));
            match self.command {
                Command::Plan => {
                    facts.push(("On server", t.on_server.to_string()));
                    facts.push(("To upload", t.to_upload.to_string()));
                }
                Command::Run => {
                    // Counted from the entries: `totals.uploaded` leaves out content the
                    // server already had when it was sent
                    let uploaded = self.distinct_uploaded();
                    // Content the server did not ask for was on the server by then, even
                    // where the lookup before the run missed it
                    let on_server = t.unique_hashes.saturating_sub(uploaded + t.missing);
                    facts.push(("On server", on_server.to_string()));
                    facts.push(("Uploaded", uploaded.to_string()));
                }
            }
            facts.push(("Missing", t.missing.to_string()));
        }
        let width = facts.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (key, value) in &facts {
            let _ = writeln!(out, "  {key:<width$}  {value}");
        }
    }

    /// Distinct contents this run sent, stored now or already
    fn distinct_uploaded(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.content == Content::Uploaded)
            .map(|e| e.entry.hash.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// The first few contents a run cannot bring over, by path
    fn write_missing(&self, out: &mut String) {
        let missing: Vec<MissingRow> = self.problems.iter().filter_map(missing_row).collect();
        if missing.is_empty() {
            return;
        }
        let _ = writeln!(out, "\n  Missing files");
        let shown = &missing[..missing.len().min(MISSING_SHOWN)];
        let width = shown.iter().map(|row| row.path.len()).max().unwrap_or(0);
        for row in shown {
            let _ = writeln!(
                out,
                "    {:<width$}  {}  {}",
                row.path, row.hash, row.reason
            );
        }
        if missing.len() > MISSING_SHOWN {
            let _ = writeln!(out, "    and {} more", missing.len() - MISSING_SHOWN);
        }
    }

    fn write_stores_not_found(&self, out: &mut String) {
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
        if not_found.is_empty() {
            return;
        }
        let _ = writeln!(
            out,
            "\n  Storage folders not found here (point them elsewhere with --storage-map OLD=NEW)"
        );
        for (store, n) in &not_found {
            let _ = writeln!(out, "    {store}  ({})", plural(*n, "missing content"));
        }
    }

    /// One line counting the other problems
    fn write_also(&self, out: &mut String) {
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
        if others.is_empty() {
            return;
        }
        let others: Vec<String> = others
            .iter()
            .map(|((one, many), n)| format!("{n} {}", if *n == 1 { one } else { many }))
            .collect();
        let _ = writeln!(out, "\n  Also: {}", others.join(", "));
    }

    fn write_footer(&self, out: &mut String, target: Option<&ServerTarget>) {
        let _ = writeln!(out);
        if self.command == Command::Plan {
            let _ = writeln!(out, "  Nothing was sent (plan).");
        }
        if let Some(t) = target {
            let _ = writeln!(out, "  Migration page: {}", redact(&t.migration_page()));
        }
    }
}

fn walk_description(scope: &Scope) -> String {
    match scope.walk {
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
    }
}

/// A line of the summary's missing list
struct MissingRow {
    /// The first path, and how many others share the content
    path: String,
    /// Shortened
    hash: String,
    reason: String,
}

fn missing_row(problem: &Problem) -> Option<MissingRow> {
    let (hash, paths, reason) = match problem {
        Problem::Missing {
            hash,
            paths,
            storage,
            storage_exists,
            ..
        } => {
            let reason = match (storage, storage_exists) {
                (Some(_), Some(false)) => "storage folder not found",
                (Some(_), _) => "not in storage folder",
                (None, _) => "no storage path in config",
            };
            (hash, paths, reason.to_string())
        }
        Problem::Corrupt { hash, paths, .. } => {
            (hash, paths, "local copy doesn't match its hash".to_string())
        }
        Problem::UploadRefused {
            hash,
            paths,
            status,
            ..
        } => {
            let reason = match status {
                413 => "refused by the server (too large)".to_string(),
                s => format!("refused by the server ({s})"),
            };
            (hash, paths, reason)
        }
        _ => return None,
    };
    let mut path = paths.first().map_or("?", String::as_str).to_string();
    if paths.len() > 1 {
        path = format!("{path} (+{})", plural(paths.len() - 1, "other path"));
    }
    Some(MissingRow {
        path,
        hash: hash.get(..12).unwrap_or(hash).to_string(),
        reason,
    })
}

const MISSING_SHOWN: usize = 20;

pub fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The server and project a command sends to
pub struct ServerTarget {
    pub server: Url,
    pub project: String,
}

impl ServerTarget {
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
    use crate::walk::tests::{hash, manifest_entry};

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
            migrated: Some(37),
            ..Totals::default()
        });
        report.entries = (100..120)
            .map(|n| ReportEntry {
                entry: manifest_entry(&format!("up/{n}.csv"), &hash(n)),
                content: Content::Uploaded,
                already_migrated: false,
            })
            .collect();
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
        let target = ServerTarget {
            server: "https://admin:pw@dvs.example.com".parse().unwrap(),
            project: "proj".into(),
        };
        let mut out = Vec::new();
        report
            .print(json, "git@host:org/proj.git", Some(&target), &mut out)
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
        (totals.uploaded, plan.problems) = (None, Vec::new());
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
