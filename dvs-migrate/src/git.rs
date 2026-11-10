//! The git side: a private mirror clone of the project, and reading its objects.
//! Everything goes through the `git` CLI; the admin's clone is only ever read.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use dvs::legacy::SIDECAR_SUFFIXES;
use dvs::migration::PartialReason;
use tempfile::TempDir;

pub struct Mirror {
    _dir: TempDir,
    pub path: PathBuf,
    pub partial: Vec<PartialReason>,
}

fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir);
    cmd
}

/// How a git command that ran went
enum GitOutput {
    /// Its stdout
    Succeeded(String),
    /// Its stderr, redacted
    Failed(String),
}

fn output(cmd: &mut Command) -> Result<GitOutput> {
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot run git; is it installed?")?;
    let pid = child.id();
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner()).push(pid);
    let out = child.wait_with_output();
    CHILDREN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|p| *p != pid);
    let out = out?;
    if out.status.success() {
        Ok(GitOutput::Succeeded(
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    } else {
        Ok(GitOutput::Failed(redact(
            String::from_utf8_lossy(&out.stderr).trim(),
        )))
    }
}

fn run(cmd: &mut Command) -> Result<String> {
    match output(cmd)? {
        GitOutput::Succeeded(out) => Ok(out),
        GitOutput::Failed(stderr) => bail!("{}", redact(&format!("{cmd:?} failed: {stderr}"))),
    }
}

/// stdout of `cmd` fed `input`, which is written from a thread so neither pipe can fill up
fn run_with_input(cmd: &mut Command, input: String) -> Result<String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot run git; is it installed?")?;
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output()?;
    writer.join().expect("writer thread")?;
    if !out.status.success() {
        bail!(
            "{}",
            redact(&format!(
                "{cmd:?} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `text` with the userinfo of every URL in it hidden: a token in a remote URL must not be
/// printed, written to the report or sent to the server.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| "/?#'\"<>".contains(c) || c.is_whitespace())
            .unwrap_or(tail.len());
        rest = match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str("***");
                &tail[at..]
            }
            None => tail,
        };
    }
    out.push_str(rest);
    out
}

/// Temp mirrors to remove if the binary is stopped by a signal
static TEMP_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Running git children that may be writing into a temp mirror
static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// On SIGINT or SIGTERM, removes the temp mirrors and exits as the signal would. The
/// signals are blocked here, before any other thread starts, and taken by one waiting
/// thread, so the cleanup is not limited to what a signal handler may do.
#[cfg(unix)]
pub fn clean_up_on_signal() {
    use nix::sys::signal::{SigSet, Signal};
    let mut signals = SigSet::empty();
    signals.add(Signal::SIGINT);
    signals.add(Signal::SIGTERM);
    if signals.thread_block().is_err() {
        return;
    }
    std::thread::spawn(move || {
        let signal = loop {
            if let Ok(signal) = signals.wait() {
                break signal;
            }
        };
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        for pid in CHILDREN.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            let _ = kill(Pid::from_raw(*pid as i32), Signal::SIGKILL);
        }
        // A git that was writing into the mirror may still be dying, so a file can appear
        // after a first removal
        let dirs = TEMP_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        for dir in dirs.iter() {
            for _ in 0..50 {
                let _ = std::fs::remove_dir_all(dir);
                if !dir.exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        eprintln!("interrupted");
        std::process::exit(128 + signal as i32);
    });
}

#[cfg(not(unix))]
pub fn clean_up_on_signal() {}

fn is_shallow(dir: &Path) -> Result<bool> {
    Ok(run(git(dir).args(["rev-parse", "--is-shallow-repository"]))?.trim() == "true")
}

/// A mirror of `source` in a temp dir. A URL is mirrored as is. A local clone is mirrored
/// with its own branches, then each of its remotes is fetched into `refs/remotes/<name>/*`
/// so work that was never pulled is included too.
pub fn mirror(source: &str) -> Result<Mirror> {
    let dir = tempfile::Builder::new().prefix("dvs-migrate-").tempdir()?;
    TEMP_DIRS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(dir.path().to_path_buf());
    let path = dir.path().join("mirror.git");
    run(Command::new("git")
        .args(["clone", "--mirror", "--quiet", "--", source])
        .arg(&path))?;

    let mut partial = Vec::new();
    let local = Path::new(source);
    if local.is_dir() {
        for remote in run(git(local).arg("remote"))?.lines() {
            let url = run(git(local).args(["remote", "get-url", remote]))?
                .trim()
                .to_string();
            // A relative remote path is relative to the clone, not to the mirror
            let fetch_from = match local.join(&url) {
                joined if Path::new(&url).is_relative() && joined.exists() => {
                    joined.to_string_lossy().into_owned()
                }
                _ => url.clone(),
            };
            let fetched = output(git(&path).args([
                "fetch",
                "--quiet",
                "--no-tags",
                &fetch_from,
                &format!("+refs/heads/*:refs/remotes/{remote}/*"),
                "+refs/tags/*:refs/tags/*",
            ]))?;
            if let GitOutput::Failed(message) = fetched {
                partial.push(PartialReason::UnreachableRemote {
                    remote: remote.to_string(),
                    url: redact(&url),
                    message,
                });
            }
        }
    }
    if is_shallow(&path)? {
        partial.push(PartialReason::Shallow);
    }
    Ok(Mirror {
        _dir: dir,
        path,
        partial,
    })
}

/// The refs walked: branches, remote-tracking branches and tags that lead to a commit,
/// through any number of tags. Symbolic refs are left out, their target is listed already.
/// A remote's `HEAD` is too: fetching it into the mirror turns it into a plain ref.
pub fn walked_refs(repo: &Path) -> Result<Vec<String>> {
    let out = run(git(repo).args([
        "for-each-ref",
        "--format=%(symref) %(refname)",
        "refs/heads",
        "refs/remotes",
        "refs/tags",
    ]))?;
    let names: Vec<&str> = out
        .lines()
        .filter_map(|line| {
            let (symref, name) = line.split_once(' ')?;
            let remote_head = name.starts_with("refs/remotes/") && name.ends_with("/HEAD");
            (symref.is_empty() && !remote_head).then_some(name)
        })
        .collect();
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let peeled: String = names.iter().map(|name| format!("{name}^{{}}\n")).collect();
    let kinds = run_with_input(
        git(repo).args(["cat-file", "--batch-check=%(objecttype)"]),
        peeled,
    )?;
    Ok(names
        .into_iter()
        .zip(kinds.lines())
        .filter(|(_, kind)| *kind == "commit")
        .map(|(name, _)| name.to_string())
        .collect())
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub id: String,
    pub tree: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub author_time: jiff::Timestamp,
}

/// Every commit reachable from `refs`, each once, parents before children.
pub fn commits(repo: &Path, refs: &[String]) -> Result<Vec<Commit>> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let out = run_with_input(
        git(repo).args([
            "-c",
            "log.showSignature=false",
            "log",
            "--stdin",
            "--topo-order",
            "--reverse",
            "-z",
            "--format=%H%x1f%T%x1f%P%x1f%an%x1f%ae%x1f%at",
        ]),
        refs.join("\n") + "\n",
    )?;

    out.split('\0')
        .map(|record| record.trim_start_matches('\n'))
        .filter(|record| !record.is_empty())
        .map(|record| {
            let fields: Vec<&str> = record.split('\x1f').collect();
            let [id, tree, parents, name, email, time] = fields[..] else {
                bail!("unexpected git log record {record:?}");
            };
            Ok(Commit {
                id: id.to_string(),
                tree: tree.to_string(),
                parents: parents.split_whitespace().map(str::to_string).collect(),
                author_name: name.to_string(),
                author_email: email.to_string(),
                author_time: jiff::Timestamp::from_second(time.trim().parse()?)?,
            })
        })
        .collect()
}

/// A long-running `git cat-file --batch`, with the `.dvs` files of each tree remembered:
/// most subtrees are shared by many commits.
pub struct Objects {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    oid_len: usize,
    names: Interner,
    blobs: Interner,
    trees: HashMap<String, Rc<DvsTree>>,
}

/// Strings stored once and named by a small id
#[derive(Default)]
struct Interner {
    ids: HashMap<Rc<str>, u32>,
    strings: Vec<Rc<str>>,
}

impl Interner {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(id) = self.ids.get(s) {
            return *id;
        }
        let id = self.strings.len() as u32;
        let s: Rc<str> = s.into();
        self.strings.push(s.clone());
        self.ids.insert(s, id);
        id
    }
}

/// A `.dvs` file's blob, interned by `Objects` and read back with `Objects::blob`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobId(u32);

/// A tree cut down to its sidecars (`*.dvs`, and devious's `*.dvsmeta`) and the folders
/// holding some, in tree order, with names and blob ids interned (`Objects::name`,
/// `Objects::blob`)
#[derive(Debug, Default)]
pub struct DvsTree {
    pub items: Vec<DvsItem>,
}

#[derive(Debug)]
pub enum DvsItem {
    File { name: u32, blob: BlobId },
    Dir { name: u32, tree: Rc<DvsTree> },
}

impl Objects {
    pub fn open(repo: &Path) -> Result<Self> {
        let oid_len = match output(git(repo).args(["rev-parse", "--show-object-format"]))? {
            GitOutput::Succeeded(format) if format.trim() == "sha256" => 32,
            // Git before 2.25 has no --show-object-format, and only sha1
            _ => 20,
        };
        let mut child = git(repo)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("cannot run git; is it installed?")?;
        let stdin = BufWriter::new(child.stdin.take().expect("piped"));
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Self {
            child,
            stdin,
            stdout,
            oid_len,
            names: Interner::default(),
            blobs: Interner::default(),
            trees: HashMap::new(),
        })
    }

    pub fn read(&mut self, oid: &str) -> Result<Vec<u8>> {
        writeln!(self.stdin, "{oid}")?;
        self.stdin.flush()?;
        let mut header = String::new();
        self.stdout.read_line(&mut header)?;
        let size = match header.split_whitespace().collect::<Vec<_>>()[..] {
            [_, _, size] => size.parse::<usize>()?,
            _ => bail!("git cat-file cannot read {oid}: {}", header.trim()),
        };
        let mut content = vec![0; size + 1];
        self.stdout.read_exact(&mut content)?;
        content.pop();
        Ok(content)
    }

    /// (mode, name, oid) of each entry of a tree
    fn tree(&mut self, oid: &str) -> Result<Vec<(String, String, String)>> {
        let raw = self.read(oid)?;
        let mut entries = Vec::new();
        let mut rest = &raw[..];
        while !rest.is_empty() {
            let nul = rest
                .iter()
                .position(|&b| b == 0)
                .context("malformed tree")?;
            let head = String::from_utf8_lossy(&rest[..nul]);
            let (mode, name) = head.split_once(' ').context("malformed tree")?;
            let id = rest
                .get(nul + 1..nul + 1 + self.oid_len)
                .context("malformed tree")?;
            let hex = id.iter().map(|b| format!("{b:02x}")).collect();
            entries.push((mode.to_string(), name.to_string(), hex));
            rest = &rest[nul + 1 + self.oid_len..];
        }
        Ok(entries)
    }

    /// The blob of a regular file at the top of a tree
    pub fn root_file(&mut self, tree: &str, name: &str) -> Result<Option<String>> {
        Ok(self
            .tree(tree)?
            .into_iter()
            .find(|(mode, n, _)| mode.starts_with("100") && n == name)
            .map(|(_, _, oid)| oid))
    }

    pub fn name(&self, id: u32) -> &str {
        &self.names.strings[id as usize]
    }

    pub fn blob(&self, id: BlobId) -> &str {
        &self.blobs.strings[id.0 as usize]
    }

    /// Every regular file named `*.dvs` or `*.dvsmeta` under a tree
    pub fn dvs_tree(&mut self, tree: &str) -> Result<Rc<DvsTree>> {
        if let Some(found) = self.trees.get(tree) {
            return Ok(found.clone());
        }
        let mut items = Vec::new();
        for (mode, name, oid) in self.tree(tree)? {
            if mode == "40000" {
                let sub = self.dvs_tree(&oid)?;
                if !sub.items.is_empty() {
                    items.push(DvsItem::Dir {
                        name: self.names.id(&name),
                        tree: sub,
                    });
                }
            } else if mode.starts_with("100")
                && SIDECAR_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
            {
                items.push(DvsItem::File {
                    name: self.names.id(&name),
                    blob: BlobId(self.blobs.id(&oid)),
                });
            }
        }
        let found = Rc::new(DvsTree { items });
        self.trees.insert(tree.to_string(), found.clone());
        Ok(found)
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::walk::tests::Repo;

    #[test]
    fn a_local_clone_is_mirrored_with_its_remotes_and_left_alone() {
        let upstream = Repo::new();
        upstream.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        let clone = Repo::new();
        clone.git(&["remote", "add", "origin", upstream.path.to_str().unwrap()]);
        clone.git(&["fetch", "-q", "origin"]);
        clone.git(&["remote", "set-head", "origin", "main"]);
        clone.git(&["checkout", "-q", "-b", "local-only", "origin/main"]);
        clone.commit("2023-02-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        // Pushed upstream after the clone last fetched
        upstream.git(&["checkout", "-q", "-b", "unpulled"]);
        upstream.commit("2023-03-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        clone.git(&["remote", "add", "gone", "/nonexistent/repo.git"]);
        let refs_before = clone.git(&["for-each-ref"]);

        let mirror = mirror(clone.path.to_str().unwrap()).unwrap();
        let refs = walked_refs(&mirror.path).unwrap();
        for name in ["refs/heads/local-only", "refs/remotes/origin/unpulled"] {
            assert!(refs.contains(&name.to_string()), "{name} not in {refs:?}");
        }
        assert!(!refs.iter().any(|r| r.ends_with("/HEAD")), "{refs:?}");
        assert_eq!(commits(&mirror.path, &refs).unwrap().len(), 3);
        assert!(matches!(
            &mirror.partial[..],
            [PartialReason::UnreachableRemote { remote, .. }] if remote == "gone"
        ));
        assert_eq!(clone.git(&["for-each-ref"]), refs_before);
    }

    #[test]
    fn a_tag_of_a_tag_leads_to_its_commit() {
        let repo = Repo::new();
        repo.commit("2023-01-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        // A commit no branch reaches, only a tag of a tag
        repo.git(&["checkout", "-q", "--orphan", "gone"]);
        let orphan = repo.commit("2023-02-01T00:00:00Z", "Jane Doe", "jdoe@example.com");
        repo.git(&["tag", "-a", "inner", "-m", "inner", &orphan]);
        repo.git(&["tag", "-a", "outer", "-m", "outer", "inner"]);
        repo.git(&["checkout", "-q", "main"]);
        repo.git(&["branch", "-D", "gone"]);
        repo.git(&["tag", "-d", "inner"]);
        let tree = repo.git(&["rev-parse", "HEAD^{tree}"]);
        repo.git(&["tag", "-a", "a-tree", "-m", "tree", tree.trim()]);

        let refs = walked_refs(&repo.path).unwrap();
        assert_eq!(refs, vec!["refs/heads/main", "refs/tags/outer"]);
        let commits = commits(&repo.path, &refs).unwrap();
        assert!(commits.iter().any(|c| c.id == orphan));
    }

    #[test]
    fn redact_hides_url_userinfo() {
        assert_eq!(
            redact("fatal: unable to access 'https://me:s3cret@host/r.git/': 403"),
            "fatal: unable to access 'https://***@host/r.git/': 403"
        );
        assert_eq!(
            redact("https://tok@a.example/x and http://b.example/y@z"),
            "https://***@a.example/x and http://b.example/y@z"
        );
        assert_eq!(
            redact("git@github.com:org/proj.git"),
            "git@github.com:org/proj.git"
        );
    }
}
