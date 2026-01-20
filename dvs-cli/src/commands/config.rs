//! DVS config command.
//!
//! View and edit DVS configuration settings.

use std::path::{Path, PathBuf};

use dvs_core::{Config, HashAlgo, MetadataFormat};

use super::{CliError, Result};
use crate::output::Output;

/// Config subcommand action.
pub enum ConfigAction {
    /// Show all configuration values.
    Show,
    /// Get a specific configuration value.
    Get { key: String },
    /// Set a configuration value.
    Set { key: String, value: String },
}

/// Valid configuration keys.
const VALID_KEYS: &[&str] = &[
    "storage_dir",
    "permissions",
    "group",
    "hash_algo",
    "metadata_format",
];

/// Run the config command.
pub fn run(output: &Output, action: ConfigAction) -> Result<()> {
    // Find repo root and load config
    let repo_root = dvs_core::helpers::config::find_repo_root_from(&std::env::current_dir()?)?;

    let config_path = repo_root.join(Config::config_filename());

    match action {
        ConfigAction::Show => show_config(output, &config_path),
        ConfigAction::Get { key } => get_config(output, &config_path, &key),
        ConfigAction::Set { key, value } => set_config(output, &config_path, &key, &value),
    }
}

/// Show all configuration values.
fn show_config(output: &Output, config_path: &Path) -> Result<()> {
    let config = Config::load(config_path)?;

    output.println(&format!("storage_dir: {}", config.storage_dir.display()));

    if let Some(perms) = config.permissions {
        output.println(&format!("permissions: {:o}", perms));
    } else {
        output.println("permissions: (not set)");
    }

    if let Some(ref group) = config.group {
        output.println(&format!("group: {}", group));
    } else {
        output.println("group: (not set)");
    }

    output.println(&format!("hash_algo: {}", format_hash_algo(config.hash_algorithm())));

    output.println(&format!("metadata_format: {}", format_metadata_format(config.metadata_format())));

    if let Some(ref gen) = config.generated_by {
        output.println("generated_by:");
        output.println(&format!("  version: {}", gen.version));
        if let Some(ref commit) = gen.commit {
            output.println(&format!("  commit: {}", commit));
        }
    }

    Ok(())
}

/// Get a specific configuration value.
fn get_config(output: &Output, config_path: &Path, key: &str) -> Result<()> {
    let config = Config::load(config_path)?;

    let value = match key {
        "storage_dir" => config.storage_dir.display().to_string(),
        "permissions" => config
            .permissions
            .map(|p| format!("{:o}", p))
            .unwrap_or_default(),
        "group" => config.group.clone().unwrap_or_default(),
        "hash_algo" => format_hash_algo(config.hash_algorithm()),
        "metadata_format" => format_metadata_format(config.metadata_format()),
        _ => {
            return Err(CliError::InvalidArg(format!(
                "Unknown config key: '{}'. Valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            )));
        }
    };

    output.println(&value);
    Ok(())
}

/// Set a configuration value.
fn set_config(output: &Output, config_path: &Path, key: &str, value: &str) -> Result<()> {
    let mut config = Config::load(config_path)?;

    match key {
        "storage_dir" => {
            config.storage_dir = PathBuf::from(value);
        }
        "permissions" => {
            let perms = parse_permissions(value)?;
            config.permissions = Some(perms);
        }
        "group" => {
            if value.is_empty() {
                config.group = None;
            } else {
                config.group = Some(value.to_string());
            }
        }
        "hash_algo" => {
            let algo = parse_hash_algo(value)?;
            config.hash_algo = Some(algo);
        }
        "metadata_format" => {
            let format = parse_metadata_format(value)?;
            config.metadata_format = Some(format);
        }
        _ => {
            return Err(CliError::InvalidArg(format!(
                "Unknown config key: '{}'. Valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            )));
        }
    }

    // Update version info
    config.generated_by = Some(dvs_core::GeneratedBy::current());

    // Save
    config.save(config_path)?;

    output.success(&format!("Set {} = {}", key, value));
    Ok(())
}

/// Parse permissions from string (octal format).
fn parse_permissions(s: &str) -> Result<u32> {
    // Strip optional "0o" prefix
    let s = s.strip_prefix("0o").unwrap_or(s);

    u32::from_str_radix(s, 8).map_err(|_| {
        CliError::InvalidArg(format!(
            "Invalid permissions '{}'. Use octal format (e.g., 664, 0o664)",
            s
        ))
    })
}

/// Parse hash algorithm from string.
fn parse_hash_algo(s: &str) -> Result<HashAlgo> {
    match s.to_lowercase().as_str() {
        "blake3" => Ok(HashAlgo::Blake3),
        "sha256" => Ok(HashAlgo::Sha256),
        "xxh3" => Ok(HashAlgo::Xxh3),
        _ => Err(CliError::InvalidArg(format!(
            "Invalid hash_algo '{}'. Valid values: blake3, sha256, xxh3",
            s
        ))),
    }
}

/// Parse metadata format from string.
fn parse_metadata_format(s: &str) -> Result<MetadataFormat> {
    match s.to_lowercase().as_str() {
        "json" => Ok(MetadataFormat::Json),
        "toml" => Ok(MetadataFormat::Toml),
        _ => Err(CliError::InvalidArg(format!(
            "Invalid metadata_format '{}'. Valid values: json, toml",
            s
        ))),
    }
}

/// Format hash algorithm for display.
fn format_hash_algo(algo: HashAlgo) -> String {
    match algo {
        HashAlgo::Blake3 => "blake3".to_string(),
        HashAlgo::Sha256 => "sha256".to_string(),
        HashAlgo::Xxh3 => "xxh3".to_string(),
    }
}

/// Format metadata format for display.
fn format_metadata_format(format: MetadataFormat) -> String {
    match format {
        MetadataFormat::Json => "json".to_string(),
        MetadataFormat::Toml => "toml".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_permissions() {
        assert_eq!(parse_permissions("664").unwrap(), 0o664);
        assert_eq!(parse_permissions("0o664").unwrap(), 0o664);
        assert_eq!(parse_permissions("755").unwrap(), 0o755);
        assert!(parse_permissions("abc").is_err());
    }

    #[test]
    fn test_parse_hash_algo() {
        assert!(matches!(parse_hash_algo("blake3").unwrap(), HashAlgo::Blake3));
        assert!(matches!(parse_hash_algo("BLAKE3").unwrap(), HashAlgo::Blake3));
        assert!(matches!(parse_hash_algo("sha256").unwrap(), HashAlgo::Sha256));
        assert!(matches!(parse_hash_algo("xxh3").unwrap(), HashAlgo::Xxh3));
        assert!(parse_hash_algo("invalid").is_err());
    }

    #[test]
    fn test_parse_metadata_format() {
        assert!(matches!(
            parse_metadata_format("json").unwrap(),
            MetadataFormat::Json
        ));
        assert!(matches!(
            parse_metadata_format("TOML").unwrap(),
            MetadataFormat::Toml
        ));
        assert!(parse_metadata_format("xml").is_err());
    }
}
