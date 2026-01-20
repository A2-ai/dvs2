//! Integration tests for full DVS workflows.
//!
//! These tests exercise complete end-to-end workflows using temp directories
//! and real git repos. They verify that multiple operations work correctly
//! together in realistic scenarios.

#[cfg(test)]
mod tests {
    use crate::repo::TestRepo;
    use crate::runner::{CoreRunner, InterfaceRunner, Op};
    use std::path::PathBuf;

    // ============================================================================
    // Full workflow tests (init -> add -> get -> status)
    // ============================================================================

    #[test]
    fn test_full_workflow_single_file() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // 1. Initialize
        let result = runner.run(&repo, &Op::init(".dvs-storage"));
        assert!(result.success, "Init failed: {}", result.stderr);

        // 2. Create and add file
        let content = b"id,name,value\n1,test,100\n2,demo,200\n";
        repo.write_file("data.csv", content).unwrap();

        let result = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(result.success, "Add failed: {}", result.stderr);

        // 3. Verify metadata exists
        assert!(repo.file_exists("data.csv.dvs") || repo.file_exists("data.csv.dvs.toml"));

        // 4. Delete original and get
        std::fs::remove_file(repo.path("data.csv")).unwrap();
        assert!(!repo.file_exists("data.csv"));

        let result = runner.run(&repo, &Op::get(&["data.csv"]));
        assert!(result.success, "Get failed: {}", result.stderr);

        // 5. Verify content restored
        let restored = repo.read_file("data.csv").unwrap();
        assert_eq!(restored, content);

        // 6. Check status
        let result = runner.run(&repo, &Op::status(&[]));
        assert!(result.success, "Status failed: {}", result.stderr);
    }

    #[test]
    fn test_full_workflow_multiple_files() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // Initialize
        runner.run(&repo, &Op::init(".dvs-storage"));

        // Create multiple files in different directories
        repo.write_file("data/train.csv", b"id,x,y\n1,10,20\n")
            .unwrap();
        repo.write_file("data/test.csv", b"id,x,y\n2,30,40\n")
            .unwrap();
        repo.write_file("models/model.bin", b"\x00\x01\x02\x03")
            .unwrap();

        // Add all files
        let result = runner.run(
            &repo,
            &Op::add(&["data/train.csv", "data/test.csv", "models/model.bin"]),
        );
        assert!(result.success, "Add multiple failed: {}", result.stderr);

        // Verify all have metadata
        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.tracked_count(), 3);
        assert!(snapshot.is_tracked(&PathBuf::from("data/train.csv")));
        assert!(snapshot.is_tracked(&PathBuf::from("data/test.csv")));
        assert!(snapshot.is_tracked(&PathBuf::from("models/model.bin")));
    }

    #[test]
    fn test_full_workflow_add_modify_add() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // Initialize
        runner.run(&repo, &Op::init(".dvs-storage"));

        // Create and add initial version
        repo.write_file("data.txt", b"version 1").unwrap();
        let result = runner.run(&repo, &Op::add(&["data.txt"]));
        assert!(result.success);

        // Get initial hash from snapshot
        let snapshot1 = result.snapshot.unwrap();
        let hash1 = snapshot1
            .get_file(&PathBuf::from("data.txt"))
            .map(|f| f.checksum.clone());

        // Modify file
        repo.write_file("data.txt", b"version 2 - modified content")
            .unwrap();

        // Add again
        let result = runner.run(&repo, &Op::add(&["data.txt"]));
        assert!(result.success, "Add modified failed: {}", result.stderr);

        // Verify hash changed
        let snapshot2 = result.snapshot.unwrap();
        let hash2 = snapshot2
            .get_file(&PathBuf::from("data.txt"))
            .map(|f| f.checksum.clone());

        assert!(hash1.is_some());
        assert!(hash2.is_some());
        assert_ne!(hash1, hash2, "Hash should change after modification");
    }

    // ============================================================================
    // Git integration tests
    // ============================================================================

    #[test]
    fn test_workflow_with_git_repo() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // Verify it's a git repo
        assert!(repo.root().join(".git").exists());

        // Initialize DVS
        let result = runner.run(&repo, &Op::init(".dvs-storage"));
        assert!(result.success);

        // Create and add file
        repo.write_file("data.csv", b"a,b,c\n1,2,3\n").unwrap();
        let result = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(result.success);

        // Verify .dvs directory created
        assert!(repo.dvs_dir().exists());

        // Verify config file created
        assert!(repo.config_path().exists());
    }

    #[test]
    fn test_workflow_dvs_only_workspace() {
        let repo = TestRepo::new_dvs_only().unwrap();
        let runner = CoreRunner::new();

        // Verify it's NOT a git repo
        assert!(!repo.root().join(".git").exists());

        // But .dvs exists (from TestRepo::new_dvs_only)
        assert!(repo.dvs_dir().exists());

        // Initialize DVS
        let result = runner.run(&repo, &Op::init(".dvs-storage"));
        assert!(result.success);

        // Create and add file
        repo.write_file("data.csv", b"x,y\n1,2\n").unwrap();
        let result = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(result.success);

        // Should work same as git repo
        let snapshot = result.snapshot.unwrap();
        assert!(snapshot.is_tracked(&PathBuf::from("data.csv")));
    }

    // ============================================================================
    // Edge case tests
    // ============================================================================

    #[test]
    fn test_add_same_file_twice() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));
        repo.write_file("data.csv", b"content").unwrap();

        // Add first time
        let result1 = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(result1.success);

        // Add again without modification - should succeed (idempotent)
        let result2 = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(result2.success);

        // Hashes should be identical
        let hash1 = result1
            .snapshot
            .unwrap()
            .get_file(&PathBuf::from("data.csv"))
            .map(|f| f.checksum.clone());
        let hash2 = result2
            .snapshot
            .unwrap()
            .get_file(&PathBuf::from("data.csv"))
            .map(|f| f.checksum.clone());
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_get_without_delete() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));
        repo.write_file("data.csv", b"original").unwrap();
        runner.run(&repo, &Op::add(&["data.csv"]));

        // Get when file still exists - should succeed
        let result = runner.run(&repo, &Op::get(&["data.csv"]));
        assert!(
            result.success,
            "Get existing file failed: {}",
            result.stderr
        );

        // Content should still be there
        let content = repo.read_file("data.csv").unwrap();
        assert_eq!(content, b"original");
    }

    #[test]
    fn test_status_empty_repo() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Status on empty repo should succeed
        let result = runner.run(&repo, &Op::status(&[]));
        assert!(result.success, "Status empty failed: {}", result.stderr);

        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.tracked_count(), 0);
    }

    #[test]
    fn test_add_empty_file() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));
        repo.write_file("empty.txt", b"").unwrap();

        // Adding empty file should work
        let result = runner.run(&repo, &Op::add(&["empty.txt"]));
        assert!(result.success, "Add empty failed: {}", result.stderr);

        let snapshot = result.snapshot.unwrap();
        assert!(snapshot.is_tracked(&PathBuf::from("empty.txt")));
    }

    #[test]
    fn test_add_binary_file() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Binary content with null bytes
        let binary_content: Vec<u8> = (0u8..=255).collect();
        repo.write_file("binary.bin", &binary_content).unwrap();

        let result = runner.run(&repo, &Op::add(&["binary.bin"]));
        assert!(result.success, "Add binary failed: {}", result.stderr);

        // Delete and restore
        std::fs::remove_file(repo.path("binary.bin")).unwrap();
        let result = runner.run(&repo, &Op::get(&["binary.bin"]));
        assert!(result.success, "Get binary failed: {}", result.stderr);

        // Verify content
        let restored = repo.read_file("binary.bin").unwrap();
        assert_eq!(restored, binary_content);
    }

    #[test]
    fn test_add_large_file_simulation() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Create a "large" file (1MB of repeated pattern)
        let pattern = b"0123456789abcdef";
        let large_content: Vec<u8> = pattern.iter().cycle().take(1024 * 1024).copied().collect();
        repo.write_file("large.bin", &large_content).unwrap();

        let result = runner.run(&repo, &Op::add(&["large.bin"]));
        assert!(result.success, "Add large failed: {}", result.stderr);

        // Delete and restore
        std::fs::remove_file(repo.path("large.bin")).unwrap();
        let result = runner.run(&repo, &Op::get(&["large.bin"]));
        assert!(result.success, "Get large failed: {}", result.stderr);

        // Verify content
        let restored = repo.read_file("large.bin").unwrap();
        assert_eq!(restored.len(), large_content.len());
        assert_eq!(restored, large_content);
    }

    #[test]
    fn test_nested_directory_structure() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Create deeply nested structure
        repo.write_file("a/b/c/d/e/deep.txt", b"deep content")
            .unwrap();
        repo.write_file("x/y/z/another.txt", b"another").unwrap();

        let result = runner.run(
            &repo,
            &Op::add(&["a/b/c/d/e/deep.txt", "x/y/z/another.txt"]),
        );
        assert!(result.success, "Add nested failed: {}", result.stderr);

        let snapshot = result.snapshot.unwrap();
        assert!(snapshot.is_tracked(&PathBuf::from("a/b/c/d/e/deep.txt")));
        assert!(snapshot.is_tracked(&PathBuf::from("x/y/z/another.txt")));
    }

    #[test]
    fn test_special_characters_in_filename() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Files with spaces and special chars
        repo.write_file("file with spaces.txt", b"content 1")
            .unwrap();
        repo.write_file("file-with-dashes.txt", b"content 2")
            .unwrap();
        repo.write_file("file_with_underscores.txt", b"content 3")
            .unwrap();
        repo.write_file("file.multiple.dots.txt", b"content 4")
            .unwrap();

        let result = runner.run(
            &repo,
            &Op::add(&[
                "file with spaces.txt",
                "file-with-dashes.txt",
                "file_with_underscores.txt",
                "file.multiple.dots.txt",
            ]),
        );
        assert!(
            result.success,
            "Add special chars failed: {}",
            result.stderr
        );

        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.tracked_count(), 4);
    }

    // ============================================================================
    // Error handling tests
    // ============================================================================

    #[test]
    fn test_add_nonexistent_file() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Try to add file that doesn't exist
        // Note: add returns success with per-file error details (batch operation behavior)
        // The operation "succeeds" but no files are actually tracked
        let result = runner.run(&repo, &Op::add(&["nonexistent.txt"]));
        assert!(
            result.success,
            "Add returns success even for nonexistent files (per-file errors in result)"
        );

        // Verify no files were actually tracked
        let snapshot = result.snapshot.unwrap();
        assert_eq!(
            snapshot.tracked_count(),
            0,
            "Nonexistent file should not be tracked"
        );
    }

    #[test]
    fn test_get_nonexistent_file() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Try to get file that was never tracked
        // Note: get returns success with per-file error details (batch operation behavior)
        let result = runner.run(&repo, &Op::get(&["nonexistent.txt"]));
        assert!(
            result.success,
            "Get returns success even for untracked files (per-file errors in result)"
        );
    }

    #[test]
    fn test_add_without_init() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        // Create file but don't initialize DVS
        repo.write_file("data.csv", b"content").unwrap();

        // Add should fail without init
        let result = runner.run(&repo, &Op::add(&["data.csv"]));
        assert!(!result.success, "Add without init should fail");
    }

    // ============================================================================
    // Storage verification tests
    // ============================================================================

    #[test]
    fn test_storage_directory_structure() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));
        repo.write_file("data.txt", b"test content for storage")
            .unwrap();
        runner.run(&repo, &Op::add(&["data.txt"]));

        // Storage directory should exist
        assert!(repo.storage_dir().exists());

        // Should have at least one object
        let objects = repo.list_storage_objects().unwrap();
        assert!(
            !objects.is_empty(),
            "Storage should contain at least one object"
        );
    }

    #[test]
    fn test_content_addressable_deduplication() {
        let repo = TestRepo::new().unwrap();
        let runner = CoreRunner::new();

        runner.run(&repo, &Op::init(".dvs-storage"));

        // Create two files with identical content
        let content = b"identical content for both files";
        repo.write_file("file1.txt", content).unwrap();
        repo.write_file("file2.txt", content).unwrap();

        runner.run(&repo, &Op::add(&["file1.txt"]));
        let objects_after_first = repo.list_storage_objects().unwrap().len();

        runner.run(&repo, &Op::add(&["file2.txt"]));
        let objects_after_second = repo.list_storage_objects().unwrap().len();

        // Should still have same number of objects (content-addressable)
        assert_eq!(
            objects_after_first, objects_after_second,
            "Identical content should be deduplicated"
        );
    }
}
