//! DVS metadata types (.dvs files).
//!
//! Metadata can be stored in either JSON (default) or TOML format:
//! - JSON: `file.ext.dvs` (existing format)
//! - TOML: `file.ext.dvs.toml` (new format)
//!
//! When loading, the TOML file is preferred if it exists.

use fs_err as fs;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use crate::HashAlgo;

/// Format for metadata files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetadataFormat {
    /// JSON format (default): `file.ext.dvs`
    #[default]
    Json,
    /// TOML format: `file.ext.dvs.toml`
    Toml,
}

impl MetadataFormat {
    /// Get the file extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            MetadataFormat::Json => "dvs",
            MetadataFormat::Toml => "dvs.toml",
        }
    }

    /// Parse format from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "json" => Some(MetadataFormat::Json),
            "toml" => Some(MetadataFormat::Toml),
            _ => None,
        }
    }
}

/// Metadata stored in `.dvs` files alongside tracked data files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Hash of the file contents (hex string).
    /// Field name kept as `blake3_checksum` for backward compatibility.
    pub blake3_checksum: String,

    /// File size in bytes.
    pub size: u64,

    /// Timestamp when the file was added/updated.
    pub add_time: DateTime<Utc>,

    /// User-provided message describing this version.
    #[serde(default)]
    pub message: String,

    /// Username of the person who added this file.
    pub saved_by: String,

    /// Hash algorithm used for this file.
    /// Defaults to Blake3 for backward compatibility with existing metadata.
    #[serde(default = "default_hash_algo", skip_serializing_if = "is_blake3")]
    pub hash_algo: HashAlgo,
}

/// Default hash algorithm for backward compatibility.
fn default_hash_algo() -> HashAlgo {
    HashAlgo::Blake3
}

/// Check if algorithm is Blake3 (for skip_serializing_if).
fn is_blake3(algo: &HashAlgo) -> bool {
    *algo == HashAlgo::Blake3
}

impl Metadata {
    /// Create new metadata for a file.
    pub fn new(
        blake3_checksum: String,
        size: u64,
        message: Option<String>,
        saved_by: String,
    ) -> Self {
        Self {
            blake3_checksum,
            size,
            add_time: Utc::now(),
            message: message.unwrap_or_default(),
            saved_by,
            hash_algo: HashAlgo::Blake3,
        }
    }

    /// Create new metadata with a specific hash algorithm.
    pub fn with_algo(
        checksum: String,
        size: u64,
        message: Option<String>,
        saved_by: String,
        hash_algo: HashAlgo,
    ) -> Self {
        Self {
            blake3_checksum: checksum,
            size,
            add_time: Utc::now(),
            message: message.unwrap_or_default(),
            saved_by,
            hash_algo,
        }
    }

    /// Get the checksum (alias for blake3_checksum field).
    pub fn checksum(&self) -> &str {
        &self.blake3_checksum
    }

    /// Load metadata from a metadata file.
    ///
    /// Tries to load from the given path. If the path ends with `.dvs.toml`,
    /// parses as TOML; if it ends with `.dvs`, parses as JSON.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        let contents = fs::read_to_string(path)?;
        let filename = path.file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default();

        if filename.ends_with(".dvs.toml") {
            Self::from_toml(&contents)
        } else {
            Self::from_json(&contents)
        }
    }

    /// Load metadata for a data file, trying TOML first, then JSON.
    ///
    /// This is the preferred way to load metadata when you have the data file path.
    /// It automatically checks for `.dvs.toml` first (if it exists), then `.dvs`.
    pub fn load_for_data_file(data_path: &std::path::Path) -> Result<Self, crate::DvsError> {
        let toml_path = Self::metadata_path_for_format(data_path, MetadataFormat::Toml);
        if toml_path.exists() {
            return Self::load(&toml_path);
        }

        let json_path = Self::metadata_path_for_format(data_path, MetadataFormat::Json);
        Self::load(&json_path)
    }

    /// Parse metadata from JSON string.
    pub fn from_json(contents: &str) -> Result<Self, crate::DvsError> {
        let metadata: Metadata = serde_json::from_str(contents)?;
        Ok(metadata)
    }

    /// Parse metadata from TOML string.
    #[cfg(feature = "toml-config")]
    pub fn from_toml(contents: &str) -> Result<Self, crate::DvsError> {
        let metadata: Metadata = toml::from_str(contents)?;
        Ok(metadata)
    }

    #[cfg(not(feature = "toml-config"))]
    pub fn from_toml(_contents: &str) -> Result<Self, crate::DvsError> {
        Err(crate::DvsError::config("TOML metadata support requires the toml-config feature"))
    }

    /// Save metadata to a file.
    ///
    /// Format is determined by the file extension:
    /// - `.dvs.toml` -> TOML format
    /// - `.dvs` -> JSON format
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        let filename = path.file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default();

        if filename.ends_with(".dvs.toml") {
            self.save_toml(path)
        } else {
            self.save_json(path)
        }
    }

    /// Save metadata in JSON format.
    pub fn save_json(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Save metadata in TOML format.
    #[cfg(feature = "toml-config")]
    pub fn save_toml(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        let toml_str = toml::to_string_pretty(self)?;
        fs::write(path, toml_str)?;
        Ok(())
    }

    #[cfg(not(feature = "toml-config"))]
    pub fn save_toml(&self, _path: &std::path::Path) -> Result<(), crate::DvsError> {
        Err(crate::DvsError::config("TOML metadata support requires the toml-config feature"))
    }

    /// Save metadata with a specific format.
    pub fn save_with_format(
        &self,
        data_path: &std::path::Path,
        format: MetadataFormat,
    ) -> Result<std::path::PathBuf, crate::DvsError> {
        let meta_path = Self::metadata_path_for_format(data_path, format);
        self.save(&meta_path)?;
        Ok(meta_path)
    }

    /// Get the metadata file path for a given data file (JSON format, backward compatible).
    pub fn metadata_path(data_path: &std::path::Path) -> std::path::PathBuf {
        Self::metadata_path_for_format(data_path, MetadataFormat::Json)
    }

    /// Get the metadata file path for a given data file and format.
    pub fn metadata_path_for_format(
        data_path: &std::path::Path,
        format: MetadataFormat,
    ) -> std::path::PathBuf {
        let mut path = data_path.to_path_buf();
        let filename = path.file_name().unwrap().to_string_lossy();
        path.set_file_name(format!("{}.{}", filename, format.extension()));
        path
    }

    /// Get the data file path from a metadata file path.
    pub fn data_path(metadata_path: &std::path::Path) -> Option<std::path::PathBuf> {
        let filename = metadata_path.file_name()?.to_string_lossy();

        // Check for .dvs.toml first (longer extension)
        if filename.ends_with(".dvs.toml") {
            let mut path = metadata_path.to_path_buf();
            path.set_file_name(&filename[..filename.len() - 9]); // strip ".dvs.toml"
            return Some(path);
        }

        // Then check for .dvs
        if filename.ends_with(".dvs") {
            let mut path = metadata_path.to_path_buf();
            path.set_file_name(&filename[..filename.len() - 4]); // strip ".dvs"
            return Some(path);
        }

        None
    }

    /// Check which metadata format exists for a data file.
    ///
    /// Returns the format that exists, preferring TOML if both exist.
    pub fn find_existing_format(data_path: &std::path::Path) -> Option<MetadataFormat> {
        let toml_path = Self::metadata_path_for_format(data_path, MetadataFormat::Toml);
        if toml_path.exists() {
            return Some(MetadataFormat::Toml);
        }

        let json_path = Self::metadata_path_for_format(data_path, MetadataFormat::Json);
        if json_path.exists() {
            return Some(MetadataFormat::Json);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_new() {
        let metadata = Metadata::new(
            "abc123".to_string(),
            1024,
            Some("test message".to_string()),
            "testuser".to_string(),
        );
        assert_eq!(metadata.blake3_checksum, "abc123");
        assert_eq!(metadata.size, 1024);
        assert_eq!(metadata.message, "test message");
        assert_eq!(metadata.saved_by, "testuser");
    }

    #[test]
    fn test_metadata_roundtrip_json() {
        let temp_dir = std::env::temp_dir().join("dvs-test-metadata-roundtrip-json");
        let _ = fs::create_dir_all(&temp_dir);

        let meta_path = temp_dir.join("test.csv.dvs");
        let metadata = Metadata::new(
            "deadbeef1234567890abcdef".to_string(),
            2048,
            Some("first version".to_string()),
            "alice".to_string(),
        );

        // Save as JSON
        metadata.save(&meta_path).unwrap();

        // Verify it's JSON
        let contents = fs::read_to_string(&meta_path).unwrap();
        assert!(contents.starts_with('{'), "Should be JSON format");

        // Load
        let loaded = Metadata::load(&meta_path).unwrap();
        assert_eq!(loaded.blake3_checksum, metadata.blake3_checksum);
        assert_eq!(loaded.size, metadata.size);
        assert_eq!(loaded.message, metadata.message);
        assert_eq!(loaded.saved_by, metadata.saved_by);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "toml-config")]
    #[test]
    fn test_metadata_roundtrip_toml() {
        let temp_dir = std::env::temp_dir().join("dvs-test-metadata-roundtrip-toml");
        let _ = fs::create_dir_all(&temp_dir);

        let meta_path = temp_dir.join("test.csv.dvs.toml");
        let metadata = Metadata::new(
            "deadbeef1234567890abcdef".to_string(),
            2048,
            Some("first version".to_string()),
            "alice".to_string(),
        );

        // Save as TOML
        metadata.save(&meta_path).unwrap();

        // Verify it's TOML (contains key = value, not key: value)
        let contents = fs::read_to_string(&meta_path).unwrap();
        assert!(contents.contains(" = "), "Should be TOML format");
        assert!(!contents.starts_with('{'), "Should not be JSON format");

        // Load
        let loaded = Metadata::load(&meta_path).unwrap();
        assert_eq!(loaded.blake3_checksum, metadata.blake3_checksum);
        assert_eq!(loaded.size, metadata.size);
        assert_eq!(loaded.message, metadata.message);
        assert_eq!(loaded.saved_by, metadata.saved_by);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "toml-config")]
    #[test]
    fn test_metadata_save_with_format() {
        let temp_dir = std::env::temp_dir().join("dvs-test-metadata-save-format");
        let _ = fs::create_dir_all(&temp_dir);

        let data_path = temp_dir.join("data.csv");
        let metadata = Metadata::new(
            "hash123".to_string(),
            1024,
            None,
            "user".to_string(),
        );

        // Save as JSON
        let json_path = metadata.save_with_format(&data_path, MetadataFormat::Json).unwrap();
        assert_eq!(json_path, temp_dir.join("data.csv.dvs"));
        assert!(json_path.exists());

        // Save as TOML
        let toml_path = metadata.save_with_format(&data_path, MetadataFormat::Toml).unwrap();
        assert_eq!(toml_path, temp_dir.join("data.csv.dvs.toml"));
        assert!(toml_path.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "toml-config")]
    #[test]
    fn test_load_for_data_file_prefers_toml() {
        let temp_dir = std::env::temp_dir().join("dvs-test-load-prefers-toml");
        let _ = fs::create_dir_all(&temp_dir);

        let data_path = temp_dir.join("data.csv");

        // Create JSON metadata
        let json_metadata = Metadata::new("json_hash".to_string(), 100, None, "user".to_string());
        json_metadata.save_with_format(&data_path, MetadataFormat::Json).unwrap();

        // Create TOML metadata with different hash
        let toml_metadata = Metadata::new("toml_hash".to_string(), 200, None, "user".to_string());
        toml_metadata.save_with_format(&data_path, MetadataFormat::Toml).unwrap();

        // Load should prefer TOML
        let loaded = Metadata::load_for_data_file(&data_path).unwrap();
        assert_eq!(loaded.blake3_checksum, "toml_hash");
        assert_eq!(loaded.size, 200);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_metadata_path() {
        let data = std::path::PathBuf::from("/data/file.csv");
        let meta = Metadata::metadata_path(&data);
        assert_eq!(meta, std::path::PathBuf::from("/data/file.csv.dvs"));
    }

    #[test]
    fn test_metadata_path_for_format() {
        let data = std::path::PathBuf::from("/data/file.csv");

        let json_path = Metadata::metadata_path_for_format(&data, MetadataFormat::Json);
        assert_eq!(json_path, std::path::PathBuf::from("/data/file.csv.dvs"));

        let toml_path = Metadata::metadata_path_for_format(&data, MetadataFormat::Toml);
        assert_eq!(toml_path, std::path::PathBuf::from("/data/file.csv.dvs.toml"));
    }

    #[test]
    fn test_data_path() {
        // JSON metadata path
        let json_meta = std::path::PathBuf::from("/data/file.csv.dvs");
        let data = Metadata::data_path(&json_meta).unwrap();
        assert_eq!(data, std::path::PathBuf::from("/data/file.csv"));

        // TOML metadata path
        let toml_meta = std::path::PathBuf::from("/data/file.csv.dvs.toml");
        let data = Metadata::data_path(&toml_meta).unwrap();
        assert_eq!(data, std::path::PathBuf::from("/data/file.csv"));

        // Non-metadata file should return None
        let not_meta = std::path::PathBuf::from("/data/file.csv");
        assert!(Metadata::data_path(&not_meta).is_none());
    }

    #[test]
    fn test_metadata_no_message() {
        let metadata = Metadata::new(
            "hash".to_string(),
            100,
            None,
            "user".to_string(),
        );
        assert_eq!(metadata.message, "");
    }

    #[test]
    fn test_metadata_format_extension() {
        assert_eq!(MetadataFormat::Json.extension(), "dvs");
        assert_eq!(MetadataFormat::Toml.extension(), "dvs.toml");
    }

    #[test]
    fn test_metadata_format_from_str() {
        assert_eq!(MetadataFormat::from_str("json"), Some(MetadataFormat::Json));
        assert_eq!(MetadataFormat::from_str("JSON"), Some(MetadataFormat::Json));
        assert_eq!(MetadataFormat::from_str("toml"), Some(MetadataFormat::Toml));
        assert_eq!(MetadataFormat::from_str("TOML"), Some(MetadataFormat::Toml));
        assert_eq!(MetadataFormat::from_str("yaml"), None);
    }
}
