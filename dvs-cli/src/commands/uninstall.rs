//! DVS uninstall command.
//!
//! Removes git-status-dvs shim and shell completions installed by `dvs install`.

use fs_err as fs;
use std::path::PathBuf;

use super::Result;
use crate::output::Output;

/// Shell types for completion removal.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl Shell {
    /// Get the default completion directory for this shell.
    fn default_completion_dir(&self) -> Option<PathBuf> {
        let home = dirs_home()?;
        match self {
            Shell::Bash => Some(home.join(".local/share/bash-completion/completions")),
            Shell::Zsh => Some(home.join(".local/share/zsh/site-functions")),
            Shell::Fish => Some(home.join(".config/fish/completions")),
            Shell::PowerShell => None,
        }
    }

    /// Get the completion filename for this shell.
    fn completion_filename(&self) -> &'static str {
        match self {
            Shell::Bash => "dvs",
            Shell::Zsh => "_dvs",
            Shell::Fish => "dvs.fish",
            Shell::PowerShell => "_dvs.ps1",
        }
    }
}

/// Get the user's home directory.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Find the installed git-status-dvs shim.
fn find_shim() -> Vec<PathBuf> {
    let mut found = Vec::new();

    if let Some(home) = dirs_home() {
        let candidates = [
            home.join(".local/bin/git-status-dvs"),
            home.join("bin/git-status-dvs"),
        ];

        for path in candidates {
            if path.exists() {
                found.push(path);
            }
        }
    }

    found
}

/// Run the uninstall command.
pub fn run(
    output: &Output,
    uninstall_dir: Option<PathBuf>,
    completions_only: bool,
    shells: Vec<String>,
) -> Result<()> {
    let mut removed_something = false;

    // Remove git-status-dvs shim (unless completions-only)
    if !completions_only {
        let shim_paths = match uninstall_dir {
            Some(dir) => vec![dir.join("git-status-dvs")],
            None => find_shim(),
        };

        for shim_path in shim_paths {
            if shim_path.exists() {
                match fs::remove_file(&shim_path) {
                    Ok(()) => {
                        output.success(&format!("Removed: {}", shim_path.display()));
                        removed_something = true;
                    }
                    Err(e) => {
                        output.info(&format!("Could not remove {}: {}", shim_path.display(), e));
                    }
                }
            }
        }
    }

    // Remove shell completions
    let target_shells: Vec<Shell> = if shells.is_empty() {
        // Default: try all shells
        vec![Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell]
    } else {
        shells
            .iter()
            .filter_map(|s| match s.to_lowercase().as_str() {
                "bash" => Some(Shell::Bash),
                "zsh" => Some(Shell::Zsh),
                "fish" => Some(Shell::Fish),
                "powershell" | "pwsh" => Some(Shell::PowerShell),
                _ => {
                    output.info(&format!("Unknown shell: {}", s));
                    None
                }
            })
            .collect()
    };

    for shell in target_shells {
        if let Some(completion_dir) = shell.default_completion_dir() {
            let completion_path = completion_dir.join(shell.completion_filename());

            if completion_path.exists() {
                match fs::remove_file(&completion_path) {
                    Ok(()) => {
                        output.success(&format!("Removed: {}", completion_path.display()));
                        removed_something = true;
                    }
                    Err(e) => {
                        output.info(&format!(
                            "Could not remove {}: {}",
                            completion_path.display(),
                            e
                        ));
                    }
                }
            }
        }
    }

    if !removed_something {
        output.info(
            "Nothing was removed. DVS may not be installed, or was installed to a custom location.",
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_completion_filename() {
        assert_eq!(Shell::Bash.completion_filename(), "dvs");
        assert_eq!(Shell::Zsh.completion_filename(), "_dvs");
        assert_eq!(Shell::Fish.completion_filename(), "dvs.fish");
        assert_eq!(Shell::PowerShell.completion_filename(), "_dvs.ps1");
    }
}
