//! DVS install command.
//!
//! Installs git-status-dvs shim and shell completions.

use std::path::PathBuf;
use fs_err as fs;

use crate::output::Output;
use super::Result;

/// Shell types for completion generation.
#[derive(Debug, Clone, Copy)]
pub enum Shell {
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
            Shell::PowerShell => None, // PowerShell completions are typically sourced from profile
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

    /// Convert to clap_complete shell type.
    fn to_clap_shell(&self) -> clap_complete::Shell {
        match self {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Zsh => clap_complete::Shell::Zsh,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::PowerShell => clap_complete::Shell::PowerShell,
        }
    }
}

/// Get the user's home directory.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Find a writable bin directory.
fn find_bin_dir() -> Option<PathBuf> {
    let home = dirs_home()?;

    // Try common user bin directories
    let candidates = [
        home.join(".local/bin"),
        home.join("bin"),
    ];

    for dir in candidates {
        if dir.exists() && is_writable(&dir) {
            return Some(dir);
        }
    }

    // Try to create ~/.local/bin if it doesn't exist
    let local_bin = home.join(".local/bin");
    if let Ok(()) = fs::create_dir_all(&local_bin) {
        return Some(local_bin);
    }

    None
}

/// Check if a directory is writable.
fn is_writable(path: &std::path::Path) -> bool {
    // Try to create a temp file to check writability
    let test_path = path.join(".dvs-write-test");
    match fs::write(&test_path, "") {
        Ok(()) => {
            let _ = fs::remove_file(&test_path);
            true
        }
        Err(_) => false,
    }
}

/// Get the path to the current dvs executable.
fn dvs_executable_path() -> Result<PathBuf> {
    std::env::current_exe()
        .map_err(|e| super::CliError::Io(e))
}

/// Generate the git-status-dvs shim script.
fn generate_shim_script(dvs_path: &std::path::Path) -> String {
    format!(
        r#"#!/bin/sh
# git-status-dvs - Git subcommand shim for DVS
# Installed by: dvs install

exec "{}" git-status "$@"
"#,
        dvs_path.display()
    )
}

/// Run the install command.
pub fn run(
    output: &Output,
    install_dir: Option<PathBuf>,
    completions_only: bool,
    shells: Vec<String>,
) -> Result<()> {
    let mut installed_something = false;

    // Install git-status-dvs shim (unless completions-only)
    if !completions_only {
        let bin_dir = match install_dir {
            Some(dir) => {
                if !dir.exists() {
                    fs::create_dir_all(&dir)?;
                }
                dir
            }
            None => find_bin_dir().ok_or_else(|| {
                super::CliError::Path(
                    "Could not find a writable bin directory. \
                     Try creating ~/.local/bin or use --install-dir".to_string()
                )
            })?,
        };

        let shim_path = bin_dir.join("git-status-dvs");
        let dvs_path = dvs_executable_path()?;
        let shim_content = generate_shim_script(&dvs_path);

        fs::write(&shim_path, &shim_content)?;

        // Make executable on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(&shim_path, perms)?;
        }

        output.success(&format!("Installed: {}", shim_path.display()));
        installed_something = true;

        // Check if bin_dir is in PATH
        if let Ok(path_var) = std::env::var("PATH") {
            let in_path = path_var.split(':').any(|p| PathBuf::from(p) == bin_dir);
            if !in_path {
                output.info(&format!(
                    "Note: {} is not in your PATH. Add it to use 'git status-dvs'.",
                    bin_dir.display()
                ));
            }
        }
    }

    // Install shell completions
    let target_shells: Vec<Shell> = if shells.is_empty() {
        // Default: try all shells
        vec![Shell::Bash, Shell::Zsh, Shell::Fish]
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
            // Create completion directory if it doesn't exist
            if let Err(e) = fs::create_dir_all(&completion_dir) {
                output.info(&format!(
                    "Skipping {:?} completions: could not create {}: {}",
                    shell, completion_dir.display(), e
                ));
                continue;
            }

            let completion_path = completion_dir.join(shell.completion_filename());

            // Generate completions
            let mut cmd = crate::build_cli();
            let mut buf = Vec::new();
            clap_complete::generate(shell.to_clap_shell(), &mut cmd, "dvs", &mut buf);

            match fs::write(&completion_path, &buf) {
                Ok(()) => {
                    output.success(&format!("Installed: {}", completion_path.display()));
                    installed_something = true;
                }
                Err(e) => {
                    output.info(&format!(
                        "Skipping {:?} completions: could not write {}: {}",
                        shell, completion_path.display(), e
                    ));
                }
            }
        }
    }

    if !installed_something {
        output.info("Nothing was installed.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_shim_script() {
        let script = generate_shim_script(&PathBuf::from("/usr/local/bin/dvs"));
        assert!(script.contains("#!/bin/sh"));
        assert!(script.contains("git-status-dvs"));
        assert!(script.contains("/usr/local/bin/dvs"));
        assert!(script.contains("git-status"));
    }

    #[test]
    fn test_shell_completion_filename() {
        assert_eq!(Shell::Bash.completion_filename(), "dvs");
        assert_eq!(Shell::Zsh.completion_filename(), "_dvs");
        assert_eq!(Shell::Fish.completion_filename(), "dvs.fish");
        assert_eq!(Shell::PowerShell.completion_filename(), "_dvs.ps1");
    }
}
