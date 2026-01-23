use std::path::Path;

use anyhow::{Result, anyhow, bail};
use fs_err as fs;

use crate::config::{Backend, Config, find_repo_root};

pub fn init(directory: impl AsRef<Path>, config: Config) -> Result<()> {
    if Config::find(&directory).is_some() {
        bail!(
            "Configuration already exists in {}",
            directory.as_ref().display()
        );
    }
    let repo_root =
        find_repo_root(&directory).ok_or_else(|| anyhow!("Cannot find repository root"))?;
    config.save(&repo_root)?;
    fs::create_dir(repo_root.join(config.metadata_folder_name()))?;

    match config.backend() {
        Backend::Local(b) => {
            fs::create_dir_all(&b.path)?;
        }
    }
    Ok(())
}
