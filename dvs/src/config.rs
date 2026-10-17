use std::io;
use std::io::Write;
use std::path::Path;

use crate::backends::Backend;
use crate::backends::local::LocalBackend;
use crate::backends::server::ServerBackend;
use crate::paths::{CONFIG_FILE_NAME, DEFAULT_FOLDER_NAME, find_repo_root};
use crate::progress::ProgressReader;
use crate::utils::parse_size;
use anyhow::Result;
use fs_err as fs;
use serde::{Deserialize, Deserializer, Serialize};
use url::Url;

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
pub struct CliConfig {
    /// Defaults to 500MB if not set in the config file
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_size_option"
    )]
    progress_threshold: Option<u64>,
}

/// What `get` does with a working file that differs from its target and that
/// dvs cannot show it left there itself (local changes, or no baseline).
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum OnConflict {
    /// Change no files if any requested file has a conflict or another known problem
    #[default]
    Abort,
    /// Leave conflicting files as they are and get the rest
    Skip,
    /// Replace conflicting files; their local changes are lost
    Overwrite,
}

impl OnConflict {
    pub const CHOICES: [&'static str; 3] = ["abort", "skip", "overwrite"];

    pub fn as_str(&self) -> &'static str {
        match self {
            OnConflict::Abort => "abort",
            OnConflict::Skip => "skip",
            OnConflict::Overwrite => "overwrite",
        }
    }
}

impl std::fmt::Display for OnConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OnConflict {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "abort" => Ok(OnConflict::Abort),
            "skip" => Ok(OnConflict::Skip),
            "overwrite" => Ok(OnConflict::Overwrite),
            other => Err(format!(
                "invalid on_conflict value {other:?}: expected one of abort, skip, overwrite"
            )),
        }
    }
}

/// Where an effective setting came from, reported with every result.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SettingSource {
    Argument,
    Env,
    Config,
    Default,
}

impl SettingSource {
    /// The stable name used in JSON and R results.
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingSource::Argument => "argument",
            SettingSource::Env => "env",
            SettingSource::Config => "config",
            SettingSource::Default => "default",
        }
    }
}

/// For people: names the file rather than "config".
impl std::fmt::Display for SettingSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SettingSource::Argument => "argument",
            SettingSource::Env => "env",
            SettingSource::Config => "dvs.toml",
            SettingSource::Default => "default",
        })
    }
}

/// A problem with dvs.toml or a DVS_* environment variable. Frontends report it
/// distinctly (the CLI exits 2) because no operation was attempted.
#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

pub const GET_ON_CONFLICT_ENV: &str = "DVS_GET_ON_CONFLICT";

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct GetConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    on_conflict: Option<OnConflict>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    /// Compression algorithm to use for files in the storage directory
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compression: Option<Compression>,
    /// By default, all the metadata files (the .dvs files) will be stored in a `.dvs` folder
    /// at the root of the repository
    /// If this option is set, dvs will use that folder name instead of `.dvs`
    metadata_folder_name: Option<String>,
    pub(crate) backend: Backend,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cli: Option<CliConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    get: Option<GetConfig>,
}

impl Config {
    pub fn new_local(path: impl AsRef<Path>, group: Option<String>) -> Result<Config> {
        let backend = LocalBackend::new(path.as_ref(), group)?;
        Ok(Config {
            compression: Some(Compression::Zstd),
            metadata_folder_name: None,
            backend: Backend::Local(backend),
            cli: None,
            get: None,
        })
    }

    pub fn new_server(name: String, url: Url, group: Option<String>) -> Config {
        let backend = ServerBackend::new(name, group, url);
        Config {
            compression: Some(Compression::Zstd),
            metadata_folder_name: None,
            backend: Backend::Server(backend),
            cli: None,
            get: None,
        }
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
            Some(toml::from_str(&content).map_err(|e| {
                ConfigError(format!("Failed to parse {}: {e}", config_path.display())).into()
            }))
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
        self.compression.unwrap_or_default()
    }

    pub fn set_compression(&mut self, compression: Option<Compression>) {
        self.compression = compression;
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    pub fn server_url(&self) -> Option<&Url> {
        match &self.backend {
            Backend::Local(_) => None,
            Backend::Server(s) => Some(&s.url),
        }
    }

    /// The `on_conflict` policy for `get`: an explicit argument, else
    /// `DVS_GET_ON_CONFLICT`, else `[get] on_conflict` in dvs.toml, else abort.
    ///
    /// dvs.toml is shared by everyone working on the project, so it may not say
    /// `overwrite`: that would discard collaborators' unsaved work without them
    /// asking. It is a configuration error, never silently replaced.
    pub fn get_on_conflict(
        &self,
        explicit: Option<OnConflict>,
    ) -> std::result::Result<(OnConflict, SettingSource), ConfigError> {
        if let Some(policy) = explicit {
            return Ok((policy, SettingSource::Argument));
        }
        if let Ok(value) = std::env::var(GET_ON_CONFLICT_ENV) {
            let policy = value
                .parse()
                .map_err(|e| ConfigError(format!("{GET_ON_CONFLICT_ENV}: {e}")))?;
            return Ok((policy, SettingSource::Env));
        }
        match self.get.as_ref().and_then(|g| g.on_conflict) {
            Some(OnConflict::Overwrite) => Err(ConfigError(format!(
                "{CONFIG_FILE_NAME}: [get] on_conflict = \"overwrite\" is not allowed in project \
                 configuration, because it would discard collaborators' local changes. Use \
                 \"abort\" or \"skip\" here; pass --force (or on_conflict = \"overwrite\" in R, \
                 or {GET_ON_CONFLICT_ENV}=overwrite) when you mean it."
            ))),
            Some(policy) => Ok((policy, SettingSource::Config)),
            None => Ok((OnConflict::default(), SettingSource::Default)),
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

#[cfg(test)]
mod get_config_tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    fn config_with(get_section: &str) -> std::result::Result<Config, String> {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.parent().unwrap().join(".storage");
        Config::new_local(&storage, None)
            .unwrap()
            .save(&root)
            .unwrap();
        let path = root.join(CONFIG_FILE_NAME);
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str(get_section);
        fs::write(&path, text).unwrap();
        Config::find(&root).unwrap().map_err(|e| e.to_string())
    }

    // These tests never set DVS_GET_ON_CONFLICT; an explicit argument wins over it anyway.
    #[test]
    fn explicit_argument_wins() {
        let config = config_with("\n[get]\non_conflict = \"skip\"\n").unwrap();
        assert_eq!(
            config.get_on_conflict(Some(OnConflict::Overwrite)).unwrap(),
            (OnConflict::Overwrite, SettingSource::Argument)
        );
    }

    #[test]
    fn project_config_may_choose_skip() {
        let config = config_with("\n[get]\non_conflict = \"skip\"\n").unwrap();
        if std::env::var(GET_ON_CONFLICT_ENV).is_err() {
            assert_eq!(
                config.get_on_conflict(None).unwrap(),
                (OnConflict::Skip, SettingSource::Config)
            );
        }
    }

    #[test]
    fn project_config_may_not_choose_overwrite() {
        let config = config_with("\n[get]\non_conflict = \"overwrite\"\n").unwrap();
        if std::env::var(GET_ON_CONFLICT_ENV).is_err() {
            let err = config.get_on_conflict(None).unwrap_err();
            assert!(err.0.contains("not allowed"), "{err}");
        }
    }

    #[test]
    fn unknown_get_keys_and_values_are_config_errors() {
        let err = config_with("\n[get]\non_conflicts = \"skip\"\n").unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
        let err = config_with("\n[get]\non_conflict = \"sometimes\"\n").unwrap_err();
        assert!(err.contains("sometimes"), "{err}");
    }
}
