use std::io;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use fs_err as fs;
use serde::{Deserialize, Deserializer, Serialize};

use crate::backends::Backend as BackendTrait;
use crate::backends::local::LocalBackend;
use crate::paths::{CONFIG_FILE_NAME, DEFAULT_FOLDER_NAME, find_repo_root};
use crate::progress::ProgressReader;
use crate::utils::parse_size;

const DEFAULT_PROGRESS_BYTE_SIZE_THRESHOLD: u64 = 524_288_000;

fn deserialize_size_option<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        None => Ok(None),
        Some(s) => parse_size(&s).map(Some).map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    None,
    #[default]
    Zstd,
}

impl std::fmt::Display for Compression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Compression::None => write!(f, "none"),
            Compression::Zstd => write!(f, "zstd"),
        }
    }
}

impl Compression {
    pub fn compress(
        &self,
        source: &Path,
        dest: &Path,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<u64> {
        match self {
            Compression::None => {
                if let Some(cb) = on_bytes {
                    let input = fs::File::open(source)?;
                    let output = fs::File::create(dest)?;
                    let mut reader = ProgressReader::new(input, cb);
                    let mut writer = io::BufWriter::new(output);
                    let bytes = io::copy(&mut reader, &mut writer)?;
                    writer.flush()?;
                    Ok(bytes)
                } else {
                    let bytes = fs::copy(source, dest)?;
                    Ok(bytes)
                }
            }
            Compression::Zstd => {
                let input = fs::File::open(source)?;
                let output = fs::File::create(dest)?;

                if let Some(cb) = on_bytes {
                    let tracked = ProgressReader::new(input, cb);
                    let mut encoder = zstd::stream::read::Encoder::new(tracked, 0)?;
                    let mut writer = io::BufWriter::new(output);
                    let bytes = io::copy(&mut encoder, &mut writer)?;
                    writer.flush()?;
                    Ok(bytes)
                } else {
                    let mut encoder = zstd::stream::read::Encoder::new(input, 0)?;
                    let mut writer = io::BufWriter::new(output);
                    let bytes = io::copy(&mut encoder, &mut writer)?;
                    writer.flush()?;
                    Ok(bytes)
                }
            }
        }
    }

    pub fn decompress(
        &self,
        source: &Path,
        dest: &Path,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<()> {
        match self {
            Compression::None => {
                if let Some(cb) = on_bytes {
                    let input = fs::File::open(source)?;
                    let output = fs::File::create(dest)?;
                    let mut reader = ProgressReader::new(input, cb);
                    let mut writer = io::BufWriter::new(output);
                    io::copy(&mut reader, &mut writer)?;
                    writer.flush()?;
                } else {
                    let mut input = fs::File::open(source)?;
                    let mut output = io::BufWriter::new(fs::File::create(dest)?);
                    io::copy(&mut input, &mut output)?;
                    output.flush()?;
                }
                Ok(())
            }
            Compression::Zstd => {
                let input = fs::File::open(source)?;
                let output = fs::File::create(dest)?;

                let mut decoder = zstd::stream::read::Decoder::new(input)?;
                let mut writer = io::BufWriter::new(output);
                if let Some(cb) = on_bytes {
                    let mut reader = ProgressReader::new(&mut decoder, cb);
                    io::copy(&mut reader, &mut writer)?;
                } else {
                    io::copy(&mut decoder, &mut writer)?;
                }
                writer.flush()?;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Backend {
    Local(LocalBackend),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct CliConfig {
    /// Defaults to 500MB if not set in the config file
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_size_option"
    )]
    progress_threshold: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    /// Compression algorithm to use for files in the storage directory
    compression: Compression,
    /// By default, all the metadata files (the .dvs files) will be stored in a `.dvs` folder
    /// at the root of the repository
    /// If this option is set, dvs will use that folder name instead of `.dvs`
    metadata_folder_name: Option<String>,
    pub(crate) backend: Backend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cli: Option<CliConfig>,
}

impl Config {
    pub fn new_local(path: impl AsRef<Path>, group: Option<String>) -> Result<Config> {
        let backend = LocalBackend::new(path.as_ref(), group)?;
        Ok(Config {
            compression: Compression::Zstd,
            metadata_folder_name: None,
            backend: Backend::Local(backend),
            cli: None,
        })
    }

    pub fn save(&self, directory: impl AsRef<Path>) -> Result<()> {
        let config_path = directory.as_ref().join(CONFIG_FILE_NAME);
        let content = toml::to_string_pretty(&self)?;
        fs::write(&config_path, content)?;
        log::info!("Configuration saved to {}", config_path.display());
        Ok(())
    }

    pub fn find(current_directory: impl AsRef<Path>) -> Option<Result<Self>> {
        let repo_root = find_repo_root(current_directory);
        let config_path = repo_root.join(CONFIG_FILE_NAME);
        log::debug!("Looking for config at {}", config_path.display());
        if config_path.exists() {
            let content = match fs::read_to_string(&config_path) {
                Ok(c) => c,
                Err(e) => return Some(Err(e.into())),
            };
            Some(
                toml::from_str(&content)
                    .with_context(|| format!("Failed to parse {}", config_path.display())),
            )
        } else {
            log::debug!("No config file found at {}", config_path.display());
            None
        }
    }

    pub fn set_metadata_folder_name(&mut self, name: String) {
        self.metadata_folder_name = Some(name);
    }

    pub fn metadata_folder_name(&self) -> &str {
        if let Some(name) = &self.metadata_folder_name {
            name.as_str()
        } else {
            DEFAULT_FOLDER_NAME
        }
    }

    pub fn compression(&self) -> Compression {
        self.compression
    }

    pub fn set_compression(&mut self, compression: Compression) {
        self.compression = compression;
    }

    pub fn backend(&self) -> &dyn BackendTrait {
        match &self.backend {
            Backend::Local(b) => b,
        }
    }

    pub fn progress_bytes_threshold(&self) -> u64 {
        self.cli
            .as_ref()
            .and_then(|x| x.progress_threshold)
            .unwrap_or(DEFAULT_PROGRESS_BYTE_SIZE_THRESHOLD)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn config_save_and_find_roundtrip() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let original = Config::new_local(&storage, None).unwrap();
        original.save(&root).unwrap();

        let loaded = Config::find(&root).unwrap().unwrap();
        assert_eq!(original, loaded);
    }

    #[test]
    fn config_find_returns_none_without_config_file() {
        let (_tmp, root) = create_temp_git_repo();
        assert!(Config::find(&root).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn new_local_validates_group_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join(".storage");

        // Non-existent group should fail
        let result = Config::new_local(&storage, Some("nonexistent_group_12345".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn config_with_custom_metadata_folder() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let mut config = Config::new_local(&storage, None).unwrap();
        config.set_metadata_folder_name(".custom_dvs".to_string());
        config.save(&root).unwrap();

        let loaded = Config::find(&root).unwrap().unwrap();
        assert_eq!(loaded.metadata_folder_name(), ".custom_dvs");
    }
}
