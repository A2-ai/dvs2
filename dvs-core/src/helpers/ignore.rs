//! Ignore pattern utilities.
//!
//! Supports both Git-style ignore files (`.gitignore`) and DVS-specific
//! ignore files (`.dvsignore`, `.ignore`).

use fs_err::{self as fs, OpenOptions};
use std::io::Write;
use std::path::Path;
use glob::Pattern;
use crate::DvsError;

/// Source of ignore patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreSource {
    /// Patterns from `.gitignore`
    GitIgnore,
    /// Patterns from `.dvsignore`
    DvsIgnore,
    /// Patterns from `.ignore` (generic fallback)
    Ignore,
}

/// A parsed ignore pattern with its source.
#[derive(Debug, Clone)]
pub struct IgnorePattern {
    /// The pattern string.
    pub pattern: String,
    /// Compiled glob pattern for matching.
    compiled: Option<Pattern>,
    /// Whether this is a negation pattern (starts with `!`).
    pub negated: bool,
    /// Whether this pattern only matches directories (ends with `/`).
    pub dir_only: bool,
    /// Whether pattern should match anywhere (no `/` in pattern).
    pub match_anywhere: bool,
    /// Source file this pattern came from.
    pub source: IgnoreSource,
}

impl IgnorePattern {
    /// Parse a pattern string.
    pub fn parse(pattern: &str, source: IgnoreSource) -> Option<Self> {
        let pattern = pattern.trim();

        // Skip empty lines and comments
        if pattern.is_empty() || pattern.starts_with('#') {
            return None;
        }

        let negated = pattern.starts_with('!');
        let pattern = if negated { &pattern[1..] } else { pattern };

        let dir_only = pattern.ends_with('/');
        let pattern = if dir_only {
            pattern.trim_end_matches('/')
        } else {
            pattern
        };

        // Pattern matches anywhere if it doesn't contain a slash
        // (except trailing slash already removed)
        let match_anywhere = !pattern.contains('/');

        // Compile the glob pattern
        let compiled = Pattern::new(pattern).ok();

        Some(Self {
            pattern: pattern.to_string(),
            compiled,
            negated,
            dir_only,
            match_anywhere,
            source,
        })
    }

    /// Check if this pattern matches the given path.
    ///
    /// For gitignore semantics:
    /// - Patterns without `/` match any basename
    /// - Patterns with `/` match from root
    /// - `dir_only` patterns only match directories
    pub fn matches(&self, path: &Path, is_dir: bool) -> bool {
        // dir_only patterns only match directories
        if self.dir_only && !is_dir {
            return false;
        }

        let Some(compiled) = &self.compiled else {
            return false;
        };

        if self.match_anywhere {
            // Match against the basename
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                return compiled.matches(name);
            }
            false
        } else {
            // Match against the full path
            if let Some(path_str) = path.to_str() {
                compiled.matches(path_str)
            } else {
                false
            }
        }
    }
}

/// Collection of ignore patterns from multiple sources.
#[derive(Debug, Default)]
pub struct IgnorePatterns {
    patterns: Vec<IgnorePattern>,
}

impl IgnorePatterns {
    /// Create an empty pattern collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add patterns from a file.
    pub fn add_from_file(&mut self, path: &Path, source: IgnoreSource) -> Result<(), DvsError> {
        let content = fs::read_to_string(path)?;
        for line in content.lines() {
            self.add_pattern(line, source);
        }
        Ok(())
    }

    /// Add a single pattern.
    pub fn add_pattern(&mut self, pattern: &str, source: IgnoreSource) {
        if let Some(p) = IgnorePattern::parse(pattern, source) {
            self.patterns.push(p);
        }
    }

    /// Check if a path matches any ignore pattern.
    ///
    /// Follows gitignore semantics: patterns are checked in order,
    /// and negation patterns (`!`) can un-ignore previously matched paths.
    pub fn is_ignored(&self, path: &Path) -> bool {
        self.is_ignored_with_dir(path, path.is_dir())
    }

    /// Check if a path matches any ignore pattern, with explicit directory flag.
    ///
    /// Use this when you already know if the path is a directory to avoid
    /// redundant filesystem checks.
    pub fn is_ignored_with_dir(&self, path: &Path, is_dir: bool) -> bool {
        let mut ignored = false;

        for pattern in &self.patterns {
            if pattern.matches(path, is_dir) {
                ignored = !pattern.negated;
            }
        }

        ignored
    }

    /// Get all patterns.
    pub fn patterns(&self) -> &[IgnorePattern] {
        &self.patterns
    }
}

// ============================================================================
// DVS-specific ignore helpers
// ============================================================================

/// Load ignore patterns from `.dvsignore` file.
pub fn load_dvsignore_patterns(repo_root: &Path) -> Result<Vec<String>, DvsError> {
    load_patterns_from_file(&repo_root.join(".dvsignore"))
}

/// Load ignore patterns from `.ignore` file.
pub fn load_ignore_patterns(repo_root: &Path) -> Result<Vec<String>, DvsError> {
    load_patterns_from_file(&repo_root.join(".ignore"))
}

/// Load patterns from an ignore file as raw strings.
fn load_patterns_from_file(path: &Path) -> Result<Vec<String>, DvsError> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)?;
    let patterns: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect();

    Ok(patterns)
}

/// Load combined ignore patterns from both `.dvsignore` and `.ignore`.
///
/// Resolution order: `.dvsignore` patterns first, then `.ignore`.
pub fn load_dvs_ignore_patterns(repo_root: &Path) -> Result<IgnorePatterns, DvsError> {
    let mut patterns = IgnorePatterns::new();

    // Load .dvsignore if it exists
    let dvsignore_path = repo_root.join(".dvsignore");
    if dvsignore_path.exists() {
        patterns.add_from_file(&dvsignore_path, IgnoreSource::DvsIgnore)?;
    }

    // Load .ignore if it exists
    let ignore_path = repo_root.join(".ignore");
    if ignore_path.exists() {
        patterns.add_from_file(&ignore_path, IgnoreSource::Ignore)?;
    }

    Ok(patterns)
}

/// Check if a path should be ignored based on DVS ignore patterns.
pub fn should_ignore(path: &Path, patterns: &[String]) -> bool {
    let mut collection = IgnorePatterns::new();
    for pattern in patterns {
        collection.add_pattern(pattern, IgnoreSource::DvsIgnore);
    }
    collection.is_ignored(path)
}

/// Add a pattern to `.dvsignore`.
pub fn add_dvsignore_pattern(repo_root: &Path, pattern: &str) -> Result<(), DvsError> {
    append_pattern_to_file(&repo_root.join(".dvsignore"), pattern)
}

/// Add a pattern to `.ignore`.
pub fn add_ignore_pattern(repo_root: &Path, pattern: &str) -> Result<(), DvsError> {
    append_pattern_to_file(&repo_root.join(".ignore"), pattern)
}

// ============================================================================
// Git-specific ignore helpers
// ============================================================================

/// Load patterns from `.gitignore`.
pub fn load_gitignore_patterns(_repo_root: &Path) -> Result<IgnorePatterns, DvsError> {
    let mut patterns = IgnorePatterns::new();
    let gitignore_path = _repo_root.join(".gitignore");
    if gitignore_path.exists() {
        patterns.add_from_file(&gitignore_path, IgnoreSource::GitIgnore)?;
    }
    Ok(patterns)
}

/// Add a pattern to `.gitignore`.
pub fn add_gitignore_pattern(repo_root: &Path, pattern: &str) -> Result<(), DvsError> {
    append_pattern_to_file(&repo_root.join(".gitignore"), pattern)
}

/// Append a pattern to an ignore file, creating it if it doesn't exist.
///
/// Ensures the file ends with a newline and the pattern is on its own line.
fn append_pattern_to_file(path: &Path, pattern: &str) -> Result<(), DvsError> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Ok(());
    }

    // Check if pattern already exists
    if path.exists() {
        let content = fs::read_to_string(path)?;
        if content.lines().any(|line| line.trim() == pattern) {
            return Ok(()); // Pattern already exists
        }
    }

    // Append the pattern
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    // Ensure we start on a new line
    if path.exists() {
        let content = fs::read_to_string(path)?;
        if !content.is_empty() && !content.ends_with('\n') {
            writeln!(file)?;
        }
    }

    writeln!(file, "{}", pattern)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_simple_pattern() {
        let p = IgnorePattern::parse("*.log", IgnoreSource::DvsIgnore).unwrap();
        assert_eq!(p.pattern, "*.log");
        assert!(!p.negated);
        assert!(!p.dir_only);
        assert!(p.match_anywhere);
    }

    #[test]
    fn test_parse_negated_pattern() {
        let p = IgnorePattern::parse("!important.log", IgnoreSource::DvsIgnore).unwrap();
        assert_eq!(p.pattern, "important.log");
        assert!(p.negated);
    }

    #[test]
    fn test_parse_dir_only_pattern() {
        let p = IgnorePattern::parse("build/", IgnoreSource::DvsIgnore).unwrap();
        assert_eq!(p.pattern, "build");
        assert!(p.dir_only);
    }

    #[test]
    fn test_parse_path_pattern() {
        let p = IgnorePattern::parse("src/*.log", IgnoreSource::DvsIgnore).unwrap();
        assert_eq!(p.pattern, "src/*.log");
        assert!(!p.match_anywhere); // contains /
    }

    #[test]
    fn test_parse_comment() {
        assert!(IgnorePattern::parse("# this is a comment", IgnoreSource::DvsIgnore).is_none());
    }

    #[test]
    fn test_parse_empty() {
        assert!(IgnorePattern::parse("", IgnoreSource::DvsIgnore).is_none());
        assert!(IgnorePattern::parse("   ", IgnoreSource::DvsIgnore).is_none());
    }

    #[test]
    fn test_pattern_matches_basename() {
        let p = IgnorePattern::parse("*.log", IgnoreSource::DvsIgnore).unwrap();
        assert!(p.matches(Path::new("test.log"), false));
        assert!(p.matches(Path::new("src/test.log"), false));
        assert!(!p.matches(Path::new("test.txt"), false));
    }

    #[test]
    fn test_pattern_matches_path() {
        let p = IgnorePattern::parse("src/*.log", IgnoreSource::DvsIgnore).unwrap();
        assert!(p.matches(Path::new("src/test.log"), false));
        assert!(!p.matches(Path::new("other/test.log"), false));
    }

    #[test]
    fn test_dir_only_pattern() {
        let p = IgnorePattern::parse("build/", IgnoreSource::DvsIgnore).unwrap();
        assert!(p.matches(Path::new("build"), true));  // is directory
        assert!(!p.matches(Path::new("build"), false)); // is file
    }

    #[test]
    fn test_ignore_patterns_collection() {
        let mut patterns = IgnorePatterns::new();
        patterns.add_pattern("*.log", IgnoreSource::DvsIgnore);
        patterns.add_pattern("!important.log", IgnoreSource::DvsIgnore);

        // *.log should be ignored
        assert!(patterns.is_ignored_with_dir(&PathBuf::from("test.log"), false));
        // But important.log should NOT be ignored (negation)
        assert!(!patterns.is_ignored_with_dir(&PathBuf::from("important.log"), false));
        // .txt files are not ignored
        assert!(!patterns.is_ignored_with_dir(&PathBuf::from("test.txt"), false));
    }

    #[test]
    fn test_should_ignore_helper() {
        let patterns = vec!["*.log".to_string(), "temp/".to_string()];
        assert!(should_ignore(Path::new("test.log"), &patterns));
        assert!(!should_ignore(Path::new("test.txt"), &patterns));
    }
}
