//! DVS init command.

use std::path::PathBuf;

use super::{CliError, Result};
use crate::output::Output;
use crate::paths;

/// Run the init command.
pub fn run(
    output: &Output,
    storage_dir: PathBuf,
    permissions: Option<String>,
    group: Option<String>,
) -> Result<()> {
    // Resolve the storage directory path
    let storage_dir = paths::resolve_path(&storage_dir)?;

    // Parse permissions if provided
    let permissions = if let Some(ref perm_str) = permissions {
        Some(parse_permissions(perm_str)?)
    } else {
        None
    };

    // Call dvs-core init
    let config = dvs_core::init(&storage_dir, permissions, group.as_deref())?;

    // Output success message
    output.success(&format!(
        "Initialized DVS with storage at: {}",
        config.storage_dir.display()
    ));

    if let Some(perms) = config.permissions {
        output.info(&format!("File permissions: {:o}", perms));
    }

    if let Some(ref grp) = config.group {
        output.info(&format!("Group: {}", grp));
    }

    Ok(())
}

/// Parse octal permissions string.
fn parse_permissions(s: &str) -> Result<u32> {
    u32::from_str_radix(s, 8).map_err(|_| {
        CliError::InvalidArg(format!(
            "Invalid permissions '{}': expected octal (e.g., 664)",
            s
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_permissions() {
        assert_eq!(parse_permissions("644").unwrap(), 0o644);
        assert_eq!(parse_permissions("755").unwrap(), 0o755);
        assert_eq!(parse_permissions("664").unwrap(), 0o664);
    }

    #[test]
    fn test_parse_permissions_invalid() {
        assert!(parse_permissions("999").is_err());
        assert!(parse_permissions("abc").is_err());
    }
}
