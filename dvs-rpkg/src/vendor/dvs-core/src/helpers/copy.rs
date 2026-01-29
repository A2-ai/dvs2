//! File copy utilities.

use crate::DvsError;
use fs_err::{self as fs, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Buffer size for file copying (64KB).
const COPY_BUFFER_SIZE: usize = 64 * 1024;

/// Copy a file to the storage directory.
///
/// Creates parent directories as needed. Sets permissions and group if configured.
#[allow(unused_variables)] // permissions and group are only used on Unix
pub fn copy_to_storage(
    source: &Path,
    dest: &Path,
    permissions: Option<u32>,
    group: Option<&str>,
) -> Result<(), DvsError> {
    // Create parent directories
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    // Copy the file
    copy_file(source, dest)?;

    // Set permissions if specified (Unix only)
    #[cfg(unix)]
    if let Some(perms) = permissions {
        set_permissions(dest, perms)?;
    }

    // Set group if specified (Unix only)
    #[cfg(unix)]
    if let Some(grp) = group {
        set_group(dest, grp)?;
    }

    Ok(())
}

/// Copy a file from storage to a local path.
pub fn copy_from_storage(source: &Path, dest: &Path) -> Result<(), DvsError> {
    // Create parent directories
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    // Copy the file
    copy_file(source, dest)?;

    Ok(())
}

/// Copy a file using buffered I/O.
fn copy_file(source: &Path, dest: &Path) -> Result<u64, DvsError> {
    let source_file = File::open(source)?;
    let dest_file = File::create(dest)?;

    let mut reader = BufReader::with_capacity(COPY_BUFFER_SIZE, source_file);
    let mut writer = BufWriter::with_capacity(COPY_BUFFER_SIZE, dest_file);

    let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
    let mut total_bytes = 0u64;

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        writer.write_all(&buffer[..bytes_read])?;
        total_bytes += bytes_read as u64;
    }

    writer.flush()?;
    Ok(total_bytes)
}

/// Set file permissions (Unix-only).
#[cfg(unix)]
pub fn set_permissions(path: &Path, permissions: u32) -> Result<(), DvsError> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let perms = Permissions::from_mode(permissions);
    fs::set_permissions(path, perms)?;
    Ok(())
}

/// Set file group ownership (Unix-only).
///
/// # Errors
///
/// Returns an error if:
/// - The group name contains null bytes
/// - The group does not exist on the system
/// - The path contains null bytes
/// - The file does not exist
/// - Permission is denied to change group ownership
#[cfg(unix)]
pub fn set_group(path: &Path, group: &str) -> Result<(), DvsError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    // Verify the file exists before attempting to change ownership
    if !path.exists() {
        return Err(DvsError::file_not_found(path));
    }

    // Get the group ID from the group name
    let group_cstr = CString::new(group).map_err(|_| {
        DvsError::config_error(format!(
            "Invalid group name (contains null byte): {}",
            group
        ))
    })?;

    // SAFETY: getgrnam is safe to call with a valid null-terminated C string.
    // The returned pointer is either null (group not found) or points to a
    // static buffer that is valid until the next getgrnam/getgrgid call.
    // We check for null and immediately copy the gid before any other calls.
    let grp = unsafe { libc::getgrnam(group_cstr.as_ptr()) };
    if grp.is_null() {
        return Err(DvsError::group_not_set(group));
    }

    // SAFETY: We just verified grp is not null. The gr_gid field is a simple
    // integer copy, so this is safe even if the static buffer is later reused.
    let gid = unsafe { (*grp).gr_gid };

    // Get the path as a C string
    let path_cstr = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        DvsError::config_error(format!(
            "Invalid path (contains null byte): {}",
            path.display()
        ))
    })?;

    // SAFETY: chown is safe to call with:
    // - A valid null-terminated path (we created it from CString)
    // - uid of (uid_t)-1 to leave owner unchanged (POSIX standard)
    // - A valid gid obtained from getgrnam
    //
    // Per POSIX, passing (uid_t)-1 for uid leaves the owner unchanged.
    // We use `!0` which produces all 1-bits, equivalent to -1 when cast to uid_t.
    let result = unsafe { libc::chown(path_cstr.as_ptr(), !0, gid) };
    if result != 0 {
        let errno = std::io::Error::last_os_error();
        return Err(DvsError::permission_denied(format!(
            "Failed to set group {} on {}: {}",
            group,
            path.display(),
            errno
        )));
    }

    Ok(())
}

/// Check if a group exists on the system (Unix-only).
///
/// Returns `false` if the group name contains null bytes or if the group
/// does not exist in the system group database.
#[cfg(unix)]
pub fn group_exists(group: &str) -> bool {
    use std::ffi::CString;

    let Ok(group_cstr) = CString::new(group) else {
        return false;
    };

    // SAFETY: getgrnam is safe to call with a valid null-terminated C string.
    // We only check if the result is null (group not found) or not.
    let grp = unsafe { libc::getgrnam(group_cstr.as_ptr()) };
    !grp.is_null()
}

/// Non-Unix stubs
#[cfg(not(unix))]
pub fn set_permissions(_path: &Path, _permissions: u32) -> Result<(), DvsError> {
    // No-op on non-Unix systems
    Ok(())
}

#[cfg(not(unix))]
pub fn set_group(_path: &Path, _group: &str) -> Result<(), DvsError> {
    // No-op on non-Unix systems
    Ok(())
}

#[cfg(not(unix))]
pub fn group_exists(_group: &str) -> bool {
    // Always return true on non-Unix (we can't check)
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_copy_file() {
        let temp_dir = std::env::temp_dir().join("dvs-test-copy");
        let _ = fs::create_dir_all(&temp_dir);

        let source = temp_dir.join("source.txt");
        let dest = temp_dir.join("dest.txt");

        // Create source file
        let mut file = File::create(&source).unwrap();
        file.write_all(b"test content for copying").unwrap();
        drop(file);

        // Copy the file
        let bytes = copy_file(&source, &dest).unwrap();
        assert_eq!(bytes, 24);

        // Verify contents match
        let source_content = fs::read_to_string(&source).unwrap();
        let dest_content = fs::read_to_string(&dest).unwrap();
        assert_eq!(source_content, dest_content);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_copy_to_storage_creates_dirs() {
        let temp_dir = std::env::temp_dir().join("dvs-test-copy-storage");
        let _ = fs::remove_dir_all(&temp_dir);

        let source = temp_dir.join("source.txt");
        let dest = temp_dir.join("deep/nested/path/dest.txt");

        // Create source file
        fs::create_dir_all(&temp_dir).unwrap();
        let mut file = File::create(&source).unwrap();
        file.write_all(b"nested content").unwrap();
        drop(file);

        // Copy to storage (creates directories)
        copy_to_storage(&source, &dest, None, None).unwrap();

        // Verify the file exists
        assert!(dest.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(unix)]
    #[test]
    fn test_set_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = std::env::temp_dir().join("dvs-test-perms");
        let _ = fs::create_dir_all(&temp_dir);

        let path = temp_dir.join("perms.txt");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"test").unwrap();
        drop(file);

        // Set permissions to 0o644
        set_permissions(&path, 0o644).unwrap();

        // Verify permissions
        let metadata = fs::metadata(&path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
