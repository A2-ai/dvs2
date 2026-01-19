//! DVS ignore pattern utilities.

use std::path::Path;
use crate::DvsError;

/// Load ignore patterns from .dvsignore file.
pub fn load_ignore_patterns(_repo_root: &Path) -> Result<Vec<String>, DvsError> {
    todo!("Load ignore patterns from .dvsignore")
}

/// Check if a path should be ignored based on .dvsignore patterns.
pub fn should_ignore(_path: &Path, _patterns: &[String]) -> bool {
    todo!("Check if path matches ignore patterns")
}

/// Add a pattern to .dvsignore.
pub fn add_ignore_pattern(_repo_root: &Path, _pattern: &str) -> Result<(), DvsError> {
    todo!("Add pattern to .dvsignore")
}
