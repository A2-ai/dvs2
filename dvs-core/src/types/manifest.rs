//! Manifest types for tracking remote data.
//!
//! The manifest (`dvs.lock`) is the source of truth tracked in Git.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use super::oid::Oid;

/// Compression algorithm for stored objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    /// No compression.
    None,
    /// Zstandard compression.
    Zstd,
    /// Gzip compression.
    Gzip,
    /// LZ4 compression.
    Lz4,
}

impl Default for Compression {
    fn default() -> Self {
        Compression::None
    }
}

/// A single entry in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path from repo root.
    pub path: PathBuf,

    /// Object ID (algo:hex).
    pub oid: Oid,

    /// File size in bytes (uncompressed).
    pub bytes: u64,

    /// Compression algorithm (if any).
    #[serde(default, skip_serializing_if = "is_compression_none")]
    pub compression: Compression,

    /// Remote name (default: "origin").
    #[serde(default = "default_remote", skip_serializing_if = "is_default_remote")]
    pub remote: String,
}

fn is_compression_none(c: &Compression) -> bool {
    matches!(c, Compression::None)
}

fn default_remote() -> String {
    "origin".to_string()
}

fn is_default_remote(s: &str) -> bool {
    s == "origin"
}

impl ManifestEntry {
    /// Create a new manifest entry.
    pub fn new(path: PathBuf, oid: Oid, bytes: u64) -> Self {
        Self {
            path,
            oid,
            bytes,
            compression: Compression::None,
            remote: default_remote(),
        }
    }

    /// Create with compression.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Create with remote name.
    pub fn with_remote(mut self, remote: String) -> Self {
        self.remote = remote;
        self
    }
}

/// The manifest file format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version.
    pub version: u32,

    /// Base URL for HTTP CAS (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Tracked entries.
    pub entries: Vec<ManifestEntry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self::new()
    }
}

impl Manifest {
    /// Create a new empty manifest.
    pub fn new() -> Self {
        Self {
            version: 1,
            base_url: None,
            entries: Vec::new(),
        }
    }

    /// Create with a base URL.
    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = Some(url);
        self
    }

    /// Get the manifest filename.
    pub const fn filename() -> &'static str {
        "dvs.lock"
    }

    /// Load manifest from a file.
    pub fn load(path: &Path) -> Result<Self, crate::DvsError> {
        let contents = std::fs::read_to_string(path)?;
        let manifest: Manifest = serde_json::from_str(&contents)?;
        Ok(manifest)
    }

    /// Save manifest to a file.
    pub fn save(&self, path: &Path) -> Result<(), crate::DvsError> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Add or update an entry.
    ///
    /// If an entry with the same path exists, it is replaced.
    pub fn upsert(&mut self, entry: ManifestEntry) {
        if let Some(existing) = self.entries.iter_mut().find(|e| e.path == entry.path) {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }

    /// Remove an entry by path.
    pub fn remove(&mut self, path: &Path) -> Option<ManifestEntry> {
        if let Some(idx) = self.entries.iter().position(|e| e.path == path) {
            Some(self.entries.remove(idx))
        } else {
            None
        }
    }

    /// Get an entry by path.
    pub fn get(&self, path: &Path) -> Option<&ManifestEntry> {
        self.entries.iter().find(|e| e.path == path)
    }

    /// Get all unique OIDs in the manifest.
    pub fn unique_oids(&self) -> Vec<&Oid> {
        let mut seen = std::collections::HashSet::new();
        self.entries
            .iter()
            .filter(|e| seen.insert(&e.oid))
            .map(|e| &e.oid)
            .collect()
    }

    /// Build a map from path to entry.
    pub fn by_path(&self) -> HashMap<&Path, &ManifestEntry> {
        self.entries.iter().map(|e| (e.path.as_path(), e)).collect()
    }

    /// Build a map from OID to entries (multiple paths can share an OID).
    pub fn by_oid(&self) -> HashMap<&Oid, Vec<&ManifestEntry>> {
        let mut map: HashMap<&Oid, Vec<&ManifestEntry>> = HashMap::new();
        for entry in &self.entries {
            map.entry(&entry.oid).or_default().push(entry);
        }
        map
    }

    /// Merge another manifest into this one.
    ///
    /// Entries from `other` replace entries with the same path.
    pub fn merge(&mut self, other: &Manifest) {
        for entry in &other.entries {
            self.upsert(entry.clone());
        }
        // Update base_url if other has one and we don't
        if self.base_url.is_none() && other.base_url.is_some() {
            self.base_url = other.base_url.clone();
        }
    }

    /// Check if manifest is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::oid::HashAlgo;

    fn test_oid() -> Oid {
        Oid::new(HashAlgo::Blake3, "a".repeat(64))
    }

    #[test]
    fn test_manifest_entry_new() {
        let entry = ManifestEntry::new(
            PathBuf::from("data/train.parquet"),
            test_oid(),
            1024,
        );
        assert_eq!(entry.path, PathBuf::from("data/train.parquet"));
        assert_eq!(entry.bytes, 1024);
        assert_eq!(entry.compression, Compression::None);
        assert_eq!(entry.remote, "origin");
    }

    #[test]
    fn test_manifest_new() {
        let manifest = Manifest::new();
        assert_eq!(manifest.version, 1);
        assert!(manifest.entries.is_empty());
        assert!(manifest.base_url.is_none());
    }

    #[test]
    fn test_manifest_upsert() {
        let mut manifest = Manifest::new();

        let entry1 = ManifestEntry::new(PathBuf::from("a.txt"), test_oid(), 100);
        manifest.upsert(entry1);
        assert_eq!(manifest.len(), 1);

        // Same path, different OID
        let entry2 = ManifestEntry::new(
            PathBuf::from("a.txt"),
            Oid::new(HashAlgo::Blake3, "b".repeat(64)),
            200,
        );
        manifest.upsert(entry2);
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest.entries[0].bytes, 200);

        // Different path
        let entry3 = ManifestEntry::new(PathBuf::from("b.txt"), test_oid(), 300);
        manifest.upsert(entry3);
        assert_eq!(manifest.len(), 2);
    }

    #[test]
    fn test_manifest_remove() {
        let mut manifest = Manifest::new();
        manifest.upsert(ManifestEntry::new(PathBuf::from("a.txt"), test_oid(), 100));

        let removed = manifest.remove(Path::new("a.txt"));
        assert!(removed.is_some());
        assert!(manifest.is_empty());

        let not_found = manifest.remove(Path::new("b.txt"));
        assert!(not_found.is_none());
    }

    #[test]
    fn test_manifest_get() {
        let mut manifest = Manifest::new();
        manifest.upsert(ManifestEntry::new(PathBuf::from("a.txt"), test_oid(), 100));

        assert!(manifest.get(Path::new("a.txt")).is_some());
        assert!(manifest.get(Path::new("b.txt")).is_none());
    }

    #[test]
    fn test_manifest_serde_roundtrip() {
        let mut manifest = Manifest::new().with_base_url("https://example.com/dvcs".to_string());
        manifest.upsert(ManifestEntry::new(
            PathBuf::from("data/train.parquet"),
            test_oid(),
            1024,
        ).with_compression(Compression::Zstd));

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.base_url, manifest.base_url);
        assert_eq!(parsed.len(), manifest.len());
        assert_eq!(parsed.entries[0].compression, Compression::Zstd);
    }

    #[test]
    fn test_manifest_unique_oids() {
        let mut manifest = Manifest::new();
        let oid1 = test_oid();
        let oid2 = Oid::new(HashAlgo::Blake3, "b".repeat(64));

        manifest.upsert(ManifestEntry::new(PathBuf::from("a.txt"), oid1.clone(), 100));
        manifest.upsert(ManifestEntry::new(PathBuf::from("b.txt"), oid1.clone(), 100)); // same OID
        manifest.upsert(ManifestEntry::new(PathBuf::from("c.txt"), oid2, 200));

        let unique = manifest.unique_oids();
        assert_eq!(unique.len(), 2);
    }

    #[test]
    fn test_manifest_merge() {
        let mut m1 = Manifest::new();
        m1.upsert(ManifestEntry::new(PathBuf::from("a.txt"), test_oid(), 100));

        let mut m2 = Manifest::new().with_base_url("https://example.com".to_string());
        m2.upsert(ManifestEntry::new(PathBuf::from("b.txt"), test_oid(), 200));

        m1.merge(&m2);
        assert_eq!(m1.len(), 2);
        assert_eq!(m1.base_url, Some("https://example.com".to_string()));
    }
}
