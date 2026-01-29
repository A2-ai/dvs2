use std::io::BufReader;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::FileMetadata;
use crate::config::Config;
use crate::hashes::Hashes;
use crate::paths::DEFAULT_METADATA_FOLDER_NAME;

#[derive(Deserialize)]
struct V1Config {
    storage_dir: PathBuf,
    permissions: Option<i32>,
    group: Option<String>,
}

impl V1Config {
    pub fn migrate(self) -> Result<Config> {
        let perms = self.permissions.map(|p| format!("{p}"));
        Config::new_local(self.storage_dir, perms, self.group)
    }
}

#[derive(Deserialize)]
struct V1Metadata {
    blake3_checksum: String,
    size: u64,
    add_time: String,
    message: Option<String>,
    saved_by: String,
}

impl V1Metadata {
    pub fn migrate(self, md5: String) -> FileMetadata {
        FileMetadata {
            hashes: Hashes {
                blake3: self.blake3_checksum,
                md5,
            },
            size: self.size,
            created_by: self.saved_by,
            add_time: self.add_time,
            message: self.message,
        }
    }
}

fn delete_files(files: &[PathBuf]) {
    for path in files {
        // we ignore errors, not much we can do about it
        let _ = fs::remove_file(path);
    }
}

/// Migrate a DVS v1 repository to v2 format.
///
/// This will:
/// 1. Convert `dvs.yaml` to `dvs.toml`
/// 2. Update all `.dvs` metadata files to the v2 format (adding MD5 hashes) and save them to strings
/// 3. Save all the new files
/// 3. Delete all the v1 files (config + .dvs metadata files)
///
/// The migration is atomic: all new files are written before any old files are deleted.
/// If any write fails, all newly written files are cleaned up.
/// If a file in the storage doesn't match the hash from the metadata file, the process fill fail.
pub fn migrate(root: impl AsRef<Path>) -> Result<usize> {
    let root = fs::canonicalize(root.as_ref())?;
    let yaml_path = root.join("dvs.yaml");
    let toml_path = root.join("dvs.toml");

    // 1. Validate: dvs.yaml must exist, dvs.toml must not
    if !yaml_path.exists() {
        bail!("No dvs.yaml found - not a DVS v1 repository");
    }
    if toml_path.exists() {
        bail!("dvs.toml already exists - repository already migrated?");
    }

    // 2. Parse config (don't write yet)
    let yaml_content = fs::read_to_string(&yaml_path)?;
    let old_config: V1Config = serde_yaml::from_str(&yaml_content)?;
    let storage_dir = old_config.storage_dir.clone();
    let new_config = old_config.migrate()?;

    // Collect files to write and delete
    let mut files_to_write: Vec<(PathBuf, String)> = vec![];
    let mut files_to_delete: Vec<PathBuf> = vec![yaml_path];

    // Add config to write list
    let config_content = toml::to_string_pretty(&new_config)?;
    files_to_write.push((toml_path, config_content));

    // 3. Process all metadata files
    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|e| {
            // Skip .git/.dvs directory
            !e.file_name()
                .to_str()
                .is_some_and(|s| s == ".git" || s == ".dvs")
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "dvs"))
    {
        let dvs_path = fs::canonicalize(entry.path())?;

        // Parse v1 metadata
        let content = fs::read_to_string(&dvs_path)?;
        let old_meta: V1Metadata = serde_json::from_str(&content)?;

        // Read file from storage and verify hash
        let blake3 = &old_meta.blake3_checksum;
        let storage_path = storage_dir.join(&blake3[..2]).join(&blake3[2..]);
        let file = fs::File::open(&storage_path)?;
        let computed = Hashes::from_reader(BufReader::new(file))?;
        if computed.blake3 != *blake3 {
            bail!(
                "Hash mismatch for {}: stored file has {}, metadata claims {}",
                dvs_path.display(),
                computed.blake3,
                blake3
            );
        }
        let md5 = computed.md5;

        // Migrate metadata
        let new_meta = old_meta.migrate(md5);
        let relative_path = dvs_path.strip_prefix(&root)?;
        let out = root.join(DEFAULT_METADATA_FOLDER_NAME).join(relative_path);
        let json = serde_json::to_string(&new_meta)?;

        files_to_write.push((out, json));
        files_to_delete.push(dvs_path);
    }

    let migrated_count = files_to_write.len() - 1; // -1 for config file

    // 4. Write all new files
    let mut written: Vec<PathBuf> = vec![];
    for (path, content) in &files_to_write {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                delete_files(&written);
                return Err(e.into());
            }
        }
        if let Err(e) = fs::write(path, content) {
            delete_files(&written);
            return Err(e.into());
        }
        written.push(path.clone());
    }

    // 5. Delete old files (only after all writes succeeded)
    let mut delete_errors = vec![];
    for p in &files_to_delete {
        if let Err(e) = fs::remove_file(p) {
            delete_errors.push((p.display().to_string(), e));
        }
    }
    if !delete_errors.is_empty() {
        let paths: Vec<_> = delete_errors.iter().map(|(p, _)| p.as_str()).collect();
        bail!(
            "Migration completed but failed to delete {} old file(s): {}",
            delete_errors.len(),
            paths.join(", ")
        );
    }

    Ok(migrated_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    fn create_v1_repo(root: &Path, storage_dir: &Path) {
        // Create dvs.yaml
        let yaml = format!("storage_dir: {}\n", storage_dir.display());
        fs::write(root.join("dvs.yaml"), yaml).unwrap();

        // Create storage directory
        fs::create_dir_all(storage_dir).unwrap();
    }

    fn create_v1_metadata(
        dvs_dir: &Path,
        relative_path: &str,
        blake3: &str,
        size: u64,
        saved_by: &str,
    ) {
        let dvs_path = dvs_dir.join(format!("{}.dvs", relative_path));
        if let Some(parent) = dvs_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let meta = serde_json::json!({
            "blake3_checksum": blake3,
            "size": size,
            "add_time": "2024-01-01T00:00:00.000Z",
            "message": null,
            "saved_by": saved_by
        });
        fs::write(dvs_path, serde_json::to_string(&meta).unwrap()).unwrap();
    }

    fn store_file_v1(storage_dir: &Path, blake3: &str, content: &[u8]) {
        let path = storage_dir.join(&blake3[..2]).join(&blake3[2..]);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn migrate_full_repo_success() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");
        create_v1_repo(&root, &storage);

        // Create 3 files with different content at various nesting levels
        let content_root = b"root file content";
        let blake3_root = format!("{}", blake3::hash(content_root));
        store_file_v1(&storage, &blake3_root, content_root);

        let content_data = b"data file one";
        let blake3_data = format!("{}", blake3::hash(content_data));
        store_file_v1(&storage, &blake3_data, content_data);

        let content_nested = b"nested file two";
        let blake3_nested = format!("{}", blake3::hash(content_nested));
        store_file_v1(&storage, &blake3_nested, content_nested);

        // Create v1 metadata files at various locations
        create_v1_metadata(
            &root,
            "root.txt",
            &blake3_root,
            content_root.len() as u64,
            "user_root",
        );
        create_v1_metadata(
            &root,
            "data/file1.txt",
            &blake3_data,
            content_data.len() as u64,
            "user_data",
        );
        create_v1_metadata(
            &root,
            "data/nested/file2.txt",
            &blake3_nested,
            content_nested.len() as u64,
            "user_nested",
        );

        // Run migration
        let result = migrate(&root).unwrap();

        // 1. Returns Ok(3) - 3 metadata files migrated
        assert_eq!(result, 3);

        // 2. dvs.yaml is deleted
        assert!(!root.join("dvs.yaml").exists());

        // 3. dvs.toml exists and is valid
        assert!(root.join("dvs.toml").exists());
        let config = Config::find(&root).unwrap().unwrap();
        assert!(matches!(config.backend(), _));

        // 4. All old .dvs files at original locations are deleted
        assert!(!root.join("root.txt.dvs").exists());
        assert!(!root.join("data/file1.txt.dvs").exists());
        assert!(!root.join("data/nested/file2.txt.dvs").exists());

        // 5. New .dvs folder contains all migrated metadata
        let dvs_dir = root.join(".dvs");
        assert!(dvs_dir.join("root.txt.dvs").exists());
        assert!(dvs_dir.join("data/file1.txt.dvs").exists());
        assert!(dvs_dir.join("data/nested/file2.txt.dvs").exists());

        // 6. Each migrated metadata has correct blake3 and md5 hashes
        let meta_root: FileMetadata =
            serde_json::from_str(&fs::read_to_string(dvs_dir.join("root.txt.dvs")).unwrap())
                .unwrap();
        assert_eq!(meta_root.hashes.blake3, blake3_root);
        assert_eq!(
            meta_root.hashes.md5,
            format!("{:x}", md5::compute(content_root))
        );

        let meta_data: FileMetadata =
            serde_json::from_str(&fs::read_to_string(dvs_dir.join("data/file1.txt.dvs")).unwrap())
                .unwrap();
        assert_eq!(meta_data.hashes.blake3, blake3_data);
        assert_eq!(
            meta_data.hashes.md5,
            format!("{:x}", md5::compute(content_data))
        );

        let meta_nested: FileMetadata = serde_json::from_str(
            &fs::read_to_string(dvs_dir.join("data/nested/file2.txt.dvs")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta_nested.hashes.blake3, blake3_nested);
        assert_eq!(
            meta_nested.hashes.md5,
            format!("{:x}", md5::compute(content_nested))
        );
    }

    #[test]
    fn migrate_fails_when_storage_file_missing() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");
        create_v1_repo(&root, &storage);

        // Create v1 metadata referencing a file that doesn't exist in storage
        let fake_blake3 = "a".repeat(64);
        create_v1_metadata(&root, "missing.txt", &fake_blake3, 100, "user");

        // Migration should fail
        let result = migrate(&root);
        assert!(result.is_err());

        // No partial migration: dvs.yaml still exists, dvs.toml not created
        assert!(root.join("dvs.yaml").exists());
        assert!(!root.join("dvs.toml").exists());
    }

    #[test]
    fn migrate_fails_when_storage_file_corrupted() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");
        create_v1_repo(&root, &storage);

        // Create a file and store it
        let original_content = b"original content";
        let blake3_hash = format!("{}", blake3::hash(original_content));

        // Store DIFFERENT content than what the hash claims
        let corrupted_content = b"corrupted content";
        store_file_v1(&storage, &blake3_hash, corrupted_content);

        // Create v1 metadata with the original hash
        create_v1_metadata(
            &root,
            "file.txt",
            &blake3_hash,
            original_content.len() as u64,
            "user",
        );

        // Migration should fail with hash mismatch error
        let result = migrate(&root);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Hash mismatch"));

        // No partial migration: dvs.yaml still exists, dvs.toml not created
        assert!(root.join("dvs.yaml").exists());
        assert!(!root.join("dvs.toml").exists());
    }
}
