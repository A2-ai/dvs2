//! DVS local-config command.
//!
//! View and edit local DVS configuration settings (.dvs/config.toml).

use serde::Serialize;
use std::path::Path;

use dvs_core::{helpers::layout::Layout, LocalConfig};

use super::{CliError, Result};
use crate::output::Output;

/// JSON output for local-config show command.
#[derive(Serialize)]
struct LocalConfigShowOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_token: Option<String>,
}

/// JSON output for local-config get command.
#[derive(Serialize)]
struct LocalConfigGetOutput {
    key: String,
    value: String,
}

/// JSON output for local-config set command.
#[derive(Serialize)]
struct LocalConfigSetOutput {
    key: String,
    value: String,
    success: bool,
}

/// JSON output for local-config unset command.
#[derive(Serialize)]
struct LocalConfigUnsetOutput {
    key: String,
    success: bool,
}

/// Local config subcommand action.
pub enum LocalConfigAction {
    /// Show all local configuration values.
    Show,
    /// Get a specific local configuration value.
    Get { key: String },
    /// Set a local configuration value.
    Set { key: String, value: String },
    /// Remove a local configuration value.
    Unset { key: String },
}

/// Valid local configuration keys.
const VALID_KEYS: &[&str] = &["base_url", "auth_token"];

/// Run the local-config command.
pub fn run(output: &Output, action: LocalConfigAction) -> Result<()> {
    // Find repo root
    let repo_root = dvs_core::helpers::config::find_repo_root_from(&std::env::current_dir()?)?;
    let layout = Layout::new(repo_root);
    let config_path = layout.config_path();

    match action {
        LocalConfigAction::Show => show_config(output, &config_path),
        LocalConfigAction::Get { key } => get_config(output, &config_path, &key),
        LocalConfigAction::Set { key, value } => set_config(output, &config_path, &key, &value),
        LocalConfigAction::Unset { key } => unset_config(output, &config_path, &key),
    }
}

/// Show all local configuration values.
fn show_config(output: &Output, config_path: &Path) -> Result<()> {
    let config = LocalConfig::load(config_path)?;

    if output.is_json() {
        let json_output = LocalConfigShowOutput {
            base_url: config.base_url().map(|s| s.to_string()),
            auth_token: config.auth_token().map(|_| "(set)".to_string()),
        };
        output.json(&json_output);
    } else {
        if let Some(url) = config.base_url() {
            output.println(&format!("base_url: {}", url));
        } else {
            output.println("base_url: (not set)");
        }

        if config.auth_token().is_some() {
            output.println("auth_token: (set)");
        } else {
            output.println("auth_token: (not set)");
        }
    }

    Ok(())
}

/// Get a specific local configuration value.
fn get_config(output: &Output, config_path: &Path, key: &str) -> Result<()> {
    let config = LocalConfig::load(config_path)?;

    let value = match key {
        "base_url" => config.base_url().unwrap_or("").to_string(),
        "auth_token" => {
            if config.auth_token().is_some() {
                "(set)".to_string()
            } else {
                String::new()
            }
        }
        _ => {
            return Err(CliError::InvalidArg(format!(
                "Unknown local config key: '{}'. Valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            )));
        }
    };

    if output.is_json() {
        output.json(&LocalConfigGetOutput {
            key: key.to_string(),
            value: value.clone(),
        });
    } else {
        output.println(&value);
    }
    Ok(())
}

/// Set a local configuration value.
fn set_config(output: &Output, config_path: &Path, key: &str, value: &str) -> Result<()> {
    let mut config = LocalConfig::load(config_path)?;

    match key {
        "base_url" => {
            config.set_base_url(Some(value.to_string()));
        }
        "auth_token" => {
            config.set_auth_token(Some(value.to_string()));
        }
        _ => {
            return Err(CliError::InvalidArg(format!(
                "Unknown local config key: '{}'. Valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            )));
        }
    }

    // Ensure .dvs directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Save
    config.save(config_path)?;

    // For auth_token, don't echo the actual value
    let display_value = if key == "auth_token" { "(set)" } else { value };

    if output.is_json() {
        output.json(&LocalConfigSetOutput {
            key: key.to_string(),
            value: display_value.to_string(),
            success: true,
        });
    } else {
        output.success(&format!("Set {} = {}", key, display_value));
    }
    Ok(())
}

/// Remove a local configuration value.
fn unset_config(output: &Output, config_path: &Path, key: &str) -> Result<()> {
    let mut config = LocalConfig::load(config_path)?;

    match key {
        "base_url" => {
            config.set_base_url(None);
        }
        "auth_token" => {
            config.set_auth_token(None);
        }
        _ => {
            return Err(CliError::InvalidArg(format!(
                "Unknown local config key: '{}'. Valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            )));
        }
    }

    // Save
    config.save(config_path)?;

    if output.is_json() {
        output.json(&LocalConfigUnsetOutput {
            key: key.to_string(),
            success: true,
        });
    } else {
        output.success(&format!("Unset {}", key));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_keys() {
        assert!(VALID_KEYS.contains(&"base_url"));
        assert!(VALID_KEYS.contains(&"auth_token"));
        assert!(!VALID_KEYS.contains(&"invalid_key"));
    }
}
