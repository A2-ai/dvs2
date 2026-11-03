use std::path::Path;
use std::process::Command;

/// Represents the current checkout.
/// If a command fails, it just returns None, it doesn't error.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Checkout {
    pub branch: Option<String>,
    pub commit: Option<String>,
}

/// The stdout of `git -C root <args>`, as it is; `None` if it fails.
pub(crate) fn output(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let value = output(root, args)?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub fn current_checkout(root: &Path) -> Checkout {
    // Fails on an unborn branch, which has no commit yet
    let commit = git(root, &["rev-parse", "HEAD"]);
    // Fails on a detached HEAD, which is on no branch
    let branch = git(root, &["symbolic-ref", "--short", "-q", "HEAD"]);
    Checkout { branch, commit }
}
