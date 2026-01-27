use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use fs_err as fs;

pub struct FileLock(PathBuf);

impl FileLock {
    pub fn acquire(path: &Path) -> Result<Self> {
        let lock_path = path.with_extension("lock");

        for _ in 0..50 {
            if fs::File::create(lock_path.clone()).is_ok() {
                return Ok(Self(lock_path.to_path_buf()));
            }

            thread::sleep(Duration::from_millis(100));
        }

        bail!("Timeout acquiring lock for {}", path.display())
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
