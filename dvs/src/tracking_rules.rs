use anyhow::{Result, bail};
use globset::{Glob, GlobMatcher};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct TrackingRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<String>,
}

impl TrackingRule {
    /// Returns the explicit label or auto-generates one from the rule's path matcher.
    pub fn label(&self) -> String {
        let base = if let Some(label) = &self.label {
            label.clone()
        } else if let Some(glob) = &self.glob {
            format!("glob: {glob}")
        } else if let Some(exts) = &self.extensions {
            format!("extensions: {}", exts.join(", "))
        } else if let Some(regex) = &self.regex {
            format!("regex: {regex}")
        } else if self.min_size.is_some() {
            "min_size".to_string()
        } else {
            "empty rule".to_string()
        };

        if let Some(size) = &self.min_size {
            format!("{base} (>= {size})")
        } else {
            base
        }
    }

    /// Validates and compiles this rule into a `CompiledRule`.
    pub fn compile(&self) -> Result<CompiledRule> {
        let matchers = [
            self.glob.is_some(),
            self.extensions.is_some(),
            self.regex.is_some(),
        ];
        let count = matchers.iter().filter(|&&b| b).count();
        if count > 1 {
            bail!(
                "tracking rule must specify at most one of `glob`, `extensions`, or `regex`; multiple provided"
            );
        }
        if count == 0 && self.min_size.is_none() {
            bail!("tracking rule must specify at least a path matcher or `min_size`");
        }

        let path_matcher = if let Some(pattern) = &self.glob {
            let matcher = Glob::new(pattern)
                .map_err(|e| anyhow::anyhow!("invalid glob pattern '{pattern}': {e}"))?
                .compile_matcher();
            Some(PathMatcher::Glob(matcher))
        } else if let Some(exts) = &self.extensions {
            let exts: Vec<&str> = exts
                .iter()
                .map(|e| e.strip_prefix('.').unwrap_or(e))
                .collect();
            let pattern = format!("**/*.{{{}}}", exts.join(","));
            let matcher = Glob::new(&pattern)
                .map_err(|e| anyhow::anyhow!("invalid extensions pattern '{pattern}': {e}"))?
                .compile_matcher();
            Some(PathMatcher::Glob(matcher))
        } else if let Some(pattern) = &self.regex {
            let re = regex::Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("invalid regex '{pattern}': {e}"))?;
            Some(PathMatcher::Regex(re))
        } else {
            None
        };

        let min_bytes = match &self.min_size {
            Some(s) => Some(parse_size(s)?),
            None => None,
        };

        Ok(CompiledRule {
            label: self.label(),
            path_matcher,
            min_bytes,
        })
    }
}

/// Parses a human-readable size string into bytes.
///
/// Supports: `"5MB"`, `"500KB"`, `"1.5GB"`, `"100"` (raw bytes).
/// Units are 1024-based and case-insensitive: B, KB, MB or GB.
pub fn parse_size(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty size string");
    }

    // Split into numeric and unit parts
    let unit_start = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(unit_start);
    let num_str = num_str.trim();
    let unit_str = unit_str.trim();

    let value: f64 = num_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size number: '{num_str}'"))?;

    if value < 0.0 {
        bail!("size cannot be negative: '{s}'");
    }

    let multiplier: u64 = match unit_str.to_uppercase().as_str() {
        "" | "B" => 1,
        "KB" => 1024,
        "MB" => 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        _ => bail!("unknown size unit: '{unit_str}' (expected B, KB, MB, or GB)"),
    };

    Ok((value * multiplier as f64) as u64)
}

#[derive(Debug)]
enum PathMatcher {
    Glob(GlobMatcher),
    Regex(regex::Regex),
}

/// A compiled, ready-to-match tracking rule.
#[derive(Debug)]
pub struct CompiledRule {
    label: String,
    path_matcher: Option<PathMatcher>,
    min_bytes: Option<u64>,
}

impl CompiledRule {
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns true if the given path and file size match this rule.
    ///
    /// Both path and size (if configured) must match (AND logic).
    pub fn matches(&self, relative_path: &Path, file_size: u64) -> bool {
        let path_matches = match &self.path_matcher {
            Some(PathMatcher::Glob(m)) => m.is_match(relative_path),
            Some(PathMatcher::Regex(re)) => {
                let path_str = relative_path.to_string_lossy();
                re.is_match(&path_str)
            }
            None => true,
        };

        if !path_matches {
            return false;
        }

        match self.min_bytes {
            Some(min) => file_size >= min,
            None => true,
        }
    }
}

/// Compiles a slice of tracking rules into compiled rules.
pub fn compile_rules(rules: &[TrackingRule]) -> Result<Vec<CompiledRule>> {
    rules.iter().map(|r| r.compile()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_size_all_units() {
        assert_eq!(parse_size("100").unwrap(), 100);
        assert_eq!(parse_size("100B").unwrap(), 100);
        assert_eq!(parse_size("500KB").unwrap(), 500 * 1024);
        assert_eq!(parse_size("5MB").unwrap(), 5 * 1024 * 1024);
        assert_eq!(parse_size("1GB").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_size("1.5GB").unwrap(),
            (1.5 * 1024.0 * 1024.0 * 1024.0) as u64
        );
        assert_eq!(parse_size("5mb").unwrap(), 5 * 1024 * 1024);

        assert!(parse_size("").is_err());
        assert!(parse_size("MB").is_err());
        assert!(parse_size("5XB").is_err());
    }

    #[test]
    fn label_generation() {
        let r = TrackingRule {
            glob: Some("**/*.csv".into()),
            ..Default::default()
        };
        assert_eq!(r.label(), "glob: **/*.csv");

        let r = TrackingRule {
            extensions: Some(vec![".rds".into(), ".parquet".into()]),
            ..Default::default()
        };
        assert_eq!(r.label(), "extensions: .rds, .parquet");

        let r = TrackingRule {
            regex: Some("^data/.*$".into()),
            ..Default::default()
        };
        assert_eq!(r.label(), "regex: ^data/.*$");

        let r = TrackingRule {
            glob: Some("**/*.csv".into()),
            min_size: Some("5MB".into()),
            ..Default::default()
        };
        assert_eq!(r.label(), "glob: **/*.csv (>= 5MB)");

        let r = TrackingRule {
            label: Some("Large CSVs".into()),
            glob: Some("**/*.csv".into()),
            min_size: Some("1MB".into()),
            ..Default::default()
        };
        assert_eq!(r.label(), "Large CSVs (>= 1MB)");
    }

    #[test]
    fn compile_validates_matcher_count() {
        assert!(TrackingRule::default().compile().is_err());

        let r = TrackingRule {
            glob: Some("**/*.csv".into()),
            extensions: Some(vec![".csv".into()]),
            ..Default::default()
        };
        assert!(r.compile().unwrap_err().to_string().contains("multiple"));
    }

    #[test]
    fn size_only_rule() {
        let c = TrackingRule {
            min_size: Some("10MB".into()),
            ..Default::default()
        }
        .compile()
        .unwrap();
        let ten_mb = 10 * 1024 * 1024;
        assert!(c.matches(Path::new("anything.txt"), ten_mb));
        assert!(c.matches(Path::new("data/deep/file.bin"), ten_mb + 1));
        assert!(!c.matches(Path::new("anything.txt"), ten_mb - 1));
    }

    #[test]
    fn each_matcher_type_works() {
        let c = TrackingRule {
            glob: Some("**/*.csv".into()),
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert!(c.matches(Path::new("data/file.csv"), 0));
        assert!(!c.matches(Path::new("file.txt"), 0));

        let c = TrackingRule {
            extensions: Some(vec![".csv".into(), "rds".into()]),
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert!(c.matches(Path::new("deep/file.csv"), 0));
        assert!(c.matches(Path::new("file.rds"), 0));
        assert!(!c.matches(Path::new("file.txt"), 0));

        let c = TrackingRule {
            regex: Some(r"^results/.*\.(csv|tsv)$".into()),
            ..Default::default()
        }
        .compile()
        .unwrap();
        assert!(c.matches(Path::new("results/output.csv"), 0));
        assert!(!c.matches(Path::new("other/output.csv"), 0));
    }

    #[test]
    fn min_size_is_and_with_path() {
        let c = TrackingRule {
            glob: Some("**/*.csv".into()),
            min_size: Some("1MB".into()),
            ..Default::default()
        }
        .compile()
        .unwrap();
        let one_mb = 1024 * 1024;
        assert!(!c.matches(Path::new("data.csv"), one_mb - 1));
        assert!(c.matches(Path::new("data.csv"), one_mb));
        assert!(!c.matches(Path::new("data.txt"), one_mb));
    }

    #[test]
    fn or_across_rules() {
        let compiled = compile_rules(&[
            TrackingRule {
                glob: Some("**/*.csv".into()),
                ..Default::default()
            },
            TrackingRule {
                extensions: Some(vec![".rds".into()]),
                ..Default::default()
            },
        ])
        .unwrap();
        assert!(compiled.iter().any(|r| r.matches(Path::new("file.csv"), 0)));
        assert!(compiled.iter().any(|r| r.matches(Path::new("file.rds"), 0)));
        assert!(!compiled.iter().any(|r| r.matches(Path::new("file.txt"), 0)));
    }
}
