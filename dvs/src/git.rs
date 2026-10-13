//! The checkout a transfer runs in, read with gix: no Git process, no dependency on
//! the process's current directory, and never an error.

use std::path::Path;

use crate::api::v1::GitContext;

fn state(state: &str) -> GitContext {
    GitContext {
        state: state.to_string(),
        branch: None,
        commit: None,
    }
}

/// `{state, branch, commit}` for the repository containing `project_root`:
/// - `branch`: on a branch, with its short name and HEAD's commit;
/// - `detached`: HEAD is a commit, not a branch;
/// - `unborn`: on a branch with no commits yet;
/// - `no_repo`: not inside a Git repository;
/// - `unavailable`: a repository that could not be read (corrupt, or a format this
///   build cannot open).
pub fn current_git_context(project_root: &Path) -> GitContext {
    let repo = match gix::discover(project_root) {
        Ok(repo) => repo,
        Err(gix::discover::Error::Discover(
            gix::discover::upwards::Error::NoGitRepository { .. }
            | gix::discover::upwards::Error::NoGitRepositoryWithinCeiling { .. }
            | gix::discover::upwards::Error::NoGitRepositoryWithinFs { .. }
            | gix::discover::upwards::Error::InaccessibleDirectory { .. },
        )) => return state("no_repo"),
        Err(e) => {
            log::debug!("cannot read the Git repository: {e}");
            return state("unavailable");
        }
    };
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) => {
            log::debug!("cannot read HEAD: {e}");
            return state("unavailable");
        }
    };
    let branch = head
        .referent_name()
        .and_then(|name| {
            let full = std::str::from_utf8(name.as_bstr().as_ref()).ok()?;
            full.strip_prefix("refs/heads/").map(str::to_owned)
        })
        .filter(|b| !b.is_empty());
    let commit = head.id().map(|id| id.to_string());
    match (head.is_detached(), head.is_unborn(), branch, commit) {
        (true, _, _, Some(commit)) => GitContext {
            state: "detached".to_string(),
            branch: None,
            commit: Some(commit),
        },
        (false, true, Some(branch), _) => GitContext {
            state: "unborn".to_string(),
            branch: Some(branch),
            commit: None,
        },
        (false, false, Some(branch), Some(commit)) => GitContext {
            state: "branch".to_string(),
            branch: Some(branch),
            commit: Some(commit),
        },
        // HEAD names something that is not a branch, or no commit could be read
        _ => state("unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_err as fs;

    fn git(root: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn ctx(state: &str, branch: Option<&str>, commit: Option<&str>) -> GitContext {
        GitContext {
            state: state.into(),
            branch: branch.map(Into::into),
            commit: commit.map(Into::into),
        }
    }

    #[test]
    fn every_checkout_state() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(current_git_context(temp.path()), ctx("no_repo", None, None));

        let root = temp.path().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "--initial-branch=feature/unborn"]);
        assert_eq!(
            current_git_context(&root),
            ctx("unborn", Some("feature/unborn"), None)
        );
        fs::create_dir(root.join("nested")).unwrap();
        assert_eq!(
            current_git_context(&root.join("nested")).state,
            "unborn",
            "found from a subdirectory"
        );

        git(&root, &["commit", "--allow-empty", "-m", "one"]);
        git(&root, &["switch", "-c", "feature/attached"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        assert_eq!(
            current_git_context(&root),
            ctx("branch", Some("feature/attached"), Some(&head))
        );

        let worktree = temp.path().join("linked");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-b",
                "feature/linked",
                worktree.to_str().unwrap(),
            ],
        );
        assert_eq!(
            current_git_context(&worktree),
            ctx("branch", Some("feature/linked"), Some(&head))
        );

        git(&root, &["checkout", "--detach"]);
        assert_eq!(
            current_git_context(&root),
            ctx("detached", None, Some(&head))
        );
    }

    #[test]
    fn a_broken_or_unreadable_repository_is_unavailable() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        fs::write(
            temp.path().join(".git/HEAD"),
            b"ref: refs/tags/not-a-branch\n",
        )
        .unwrap();
        assert_eq!(current_git_context(temp.path()).state, "unavailable");
        fs::write(temp.path().join(".git/HEAD"), b"garbage\n").unwrap();
        assert_eq!(current_git_context(temp.path()).state, "unavailable");
    }

    #[test]
    fn a_sha256_repository_never_fails_the_transfer() {
        let temp = tempfile::tempdir().unwrap();
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(temp.path())
            .args(["init", "--object-format=sha256"])
            .output()
            .unwrap()
            .status
            .success();
        if !ok {
            return; // this Git cannot make one
        }
        git(temp.path(), &["commit", "--allow-empty", "-m", "one"]);
        let context = current_git_context(temp.path());
        match context.state.as_str() {
            // Read: the commit is the 64-character id
            "branch" => assert_eq!(context.commit.unwrap().len(), 64),
            // This build of gix cannot open SHA-256 repositories
            "unavailable" => {}
            other => panic!("{other}"),
        }
    }
}
