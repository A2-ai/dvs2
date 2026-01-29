//! DVS version information.
//!
//! Provides compile-time version and commit information for tracking
//! which build of DVS created a configuration file.

/// Version string for display (e.g., "0.1.0 (abc12345)").
///
/// This is set at compile time and includes the commit hash if available.
pub const VERSION_STRING: &str = env!("DVS_VERSION_STRING");

/// DVS version descriptor.
#[derive(Debug, Clone, Copy)]
pub struct DvsVersion {
    /// Package version string (e.g., "0.0.0-9000").
    pub version: &'static str,
    /// Git commit SHA (short hash), if available at build time.
    pub commit: Option<&'static str>,
}

impl DvsVersion {
    /// Get the version string for display (e.g., "0.0.0-9000 (abc12345)").
    pub fn display(&self) -> String {
        match self.commit {
            Some(sha) => format!("{} ({})", self.version, sha),
            None => self.version.to_string(),
        }
    }
}

impl std::fmt::Display for DvsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display())
    }
}

/// Get the current DVS version.
///
/// This includes the package version and, if available, the git commit SHA
/// from when the binary was built.
pub fn version() -> DvsVersion {
    DvsVersion {
        version: env!("DVS_VERSION"),
        commit: option_env!("DVS_COMMIT_SHA"),
    }
}

/// Get just the version string (without commit).
pub fn version_string() -> &'static str {
    env!("DVS_VERSION")
}

/// Get the commit SHA if available.
pub fn commit_sha() -> Option<&'static str> {
    option_env!("DVS_COMMIT_SHA")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version() {
        let v = version();
        assert!(!v.version.is_empty());
        // Commit may or may not be present depending on build environment
    }

    #[test]
    fn test_version_display() {
        let v = DvsVersion {
            version: "1.0.0",
            commit: Some("abc12345"),
        };
        assert_eq!(v.display(), "1.0.0 (abc12345)");

        let v_no_commit = DvsVersion {
            version: "1.0.0",
            commit: None,
        };
        assert_eq!(v_no_commit.display(), "1.0.0");
    }

    #[test]
    fn test_version_string() {
        let s = version_string();
        assert!(!s.is_empty());
    }
}
