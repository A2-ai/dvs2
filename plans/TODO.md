# TODO

## Configuration Options to Implement

These are the config fields that need to be wired up:

### dvs.yaml (repository config)

- [ ] `storage_dir` - Path to content-addressable storage
- [ ] `permissions` - File permissions (octal, e.g., 0640)
- [ ] `group` - Unix group for files
- [ ] `hash_algorithm` - Currently blake3, future: sha256
- [ ] `gitignore_data_files` - Auto-add data files to .gitignore

### .dvs sidecar files (per-file metadata)

- [ ] `file_hash` - Blake3 hash of content
- [ ] `file_size` - Size in bytes
- [ ] `saved_by` - Username who last saved
- [ ] `saved_at` - ISO timestamp of last save
- [ ] `message` - Optional commit-like message
- [ ] `original_path` - Path relative to repo root

### Daemon config

- [ ] `watch_paths` - Directories to watch
- [ ] `debounce_ms` - Delay before processing changes
- [ ] `auto_add` - Enable auto-add for new files
- [ ] `auto_add_patterns` - Glob patterns for auto-add
- [ ] `auto_sync` - Enable auto-sync for changes
- [ ] `log_file` - Path to log file
- [ ] `pid_file` - Path to PID file
- [ ] `socket_path` - Unix socket for IPC

### Server config

- [ ] `host` - Bind address
- [ ] `port` - Listen port
- [ ] `storage_root` - Storage directory
- [ ] `max_upload_size` - Upload size limit
- [ ] `cors_enabled` - Enable CORS
- [ ] `cors_origins` - Allowed origins
- [ ] `auth.enabled` - Enable authentication
- [ ] `auth.api_keys` - List of API keys
- [ ] `auth.jwt_secret` - JWT signing secret
- [ ] `tls.enabled` - Enable TLS
- [ ] `tls.cert_path` - Certificate path
- [ ] `tls.key_path` - Private key path

---

## dvs-core

### Types

- [ ] `Config::load()` - Parse dvs.yaml with serde_yaml
- [ ] `Config::save()` - Write dvs.yaml with serde_yaml
- [ ] `Config::validate()` - Check storage_dir exists, permissions valid
- [ ] `Metadata::load()` - Parse .dvs sidecar files
- [ ] `Metadata::save()` - Write .dvs sidecar files
- [ ] `FileInfo::from_path()` - Stat file, get size/mtime

### Operations

- [ ] `init::init()` - Create dvs.yaml, setup storage directory
- [ ] `init::find_git_root()` - Walk up to find .git directory
- [ ] `init::setup_storage_directory()` - Create dirs, set permissions
- [ ] `init::validate_group()` - Check group exists on system
- [ ] `add::add()` - Main add operation orchestration
- [ ] `add::expand_globs()` - Use glob crate to expand patterns
- [ ] `add::add_single_file()` - Hash, copy, create metadata
- [ ] `add::storage_path_for_hash()` - `{storage}/{hash[0:2]}/{hash[2:]}`
- [ ] `add::rollback_add()` - Clean up on failure
- [ ] `get::get()` - Main get operation orchestration
- [ ] `get::expand_globs_tracked()` - Expand globs, filter to tracked files
- [ ] `get::get_single_file()` - Copy from storage, verify hash
- [ ] `get::file_matches_metadata()` - Compare local hash to metadata
- [ ] `get::copy_from_storage()` - Copy and set permissions
- [ ] `status::status()` - Main status operation orchestration
- [ ] `status::find_all_tracked_files()` - Find all .dvs files in repo
- [ ] `status::status_single_file()` - Compare local to stored
- [ ] `status::determine_status()` - Return Current/Modified/Missing/etc

### Helpers

- [ ] `hash::get_file_hash()` - Choose mmap vs read based on size
- [ ] `hash::hash_mmap()` - Memory-mapped blake3 hashing
- [ ] `hash::hash_read()` - Traditional read-based hashing
- [ ] `hash::storage_path_for_hash()` - Build storage path from hash
- [ ] `copy::copy_to_storage()` - Copy with atomic rename
- [ ] `copy::copy_from_storage()` - Copy and verify
- [ ] `copy::set_permissions()` - Unix chmod
- [ ] `copy::set_group()` - Unix chgrp
- [ ] `file::save_metadata()` - Write .dvs JSON file
- [ ] `file::load_metadata()` - Read .dvs JSON file
- [ ] `file::check_meta_files_exist()` - Verify .dvs files present
- [ ] `file::get_current_username()` - whoami equivalent
- [ ] `file::get_file_size()` - File size in bytes
- [ ] `config::find_repo_root()` - Find directory with dvs.yaml
- [ ] `config::load_config()` - Load and validate dvs.yaml
- [ ] `config::save_config()` - Write dvs.yaml
- [ ] `config::validate_storage_dir()` - Check access permissions
- [ ] `parse::expand_globs()` - Glob pattern expansion
- [ ] `parse::matches_glob()` - Single pattern match check
- [ ] `parse::normalize_path()` - Relative to repo root
- [ ] `parse::parse_size()` - "10MB" -> bytes
- [ ] `repo::is_git_repo()` - Check for .git
- [ ] `repo::get_git_root()` - Find .git parent
- [ ] `repo::add_to_gitignore()` - Append pattern to .gitignore
- [ ] `repo::is_git_ignored()` - Check gitignore status
- [ ] `repo::get_current_branch()` - Current git branch name
- [ ] `ignore::load_ignore_patterns()` - Read .dvsignore
- [ ] `ignore::should_ignore()` - Match against patterns
- [ ] `ignore::add_ignore_pattern()` - Append to .dvsignore
- [ ] `cache::load_cache()` - Load hash cache from .dvs/cache
- [ ] `cache::save_cache()` - Write hash cache
- [ ] `cache::get_cached_hash()` - Lookup by mtime+size
- [ ] `cache::update_cache_entry()` - Add/update entry

## dvs-cli

- [ ] Create main.rs with clap App
- [ ] `dvs init` subcommand - calls `dvs_core::init()`
- [ ] `dvs add <files>` subcommand - calls `dvs_core::add()`
- [ ] `dvs get <files>` subcommand - calls `dvs_core::get()`
- [ ] `dvs status [files]` subcommand - calls `dvs_core::status()`
- [ ] `dvs config` subcommand - show/edit configuration
- [ ] `dvs daemon` subcommand - start/stop/status daemon
- [ ] Pretty output formatting (colors, tables)
- [ ] Progress bars for large file operations
- [ ] Error message formatting

## dvs-daemon

- [ ] `start_daemon()` - Initialize and run event loop
- [ ] `stop_daemon()` - Graceful shutdown
- [ ] `is_daemon_running()` - Check PID file
- [ ] `FileWatcher::new()` - Initialize notify watcher
- [ ] `FileWatcher::start()` - Begin watching
- [ ] `FileWatcher::stop()` - Stop watching
- [ ] `FileWatcher::add_path()` - Add watch path
- [ ] `FileWatcher::remove_path()` - Remove watch path
- [ ] `FileWatcher::next_event()` - Async event receiver
- [ ] `EventHandler::new()` - Initialize handler
- [ ] `EventHandler::handle_event()` - Route events
- [ ] `EventHandler::is_tracked()` - Check if file has .dvs
- [ ] `EventHandler::auto_add()` - Auto-add matching files
- [ ] `EventHandler::auto_sync()` - Sync modified files
- [ ] `EventHandler::handle_delete()` - Handle deleted files
- [ ] `DaemonClient::new()` - Create IPC client
- [ ] `DaemonClient::connect()` - Connect to socket
- [ ] `DaemonClient::send_command()` - Send and receive
- [ ] `DaemonServer::new()` - Create IPC server
- [ ] `DaemonServer::listen()` - Start listening
- [ ] `DaemonServer::next_command()` - Receive command
- [ ] `DaemonServer::send_response()` - Send response
- [ ] `DaemonConfig::load()` - Load daemon config
- [ ] `DaemonConfig::save()` - Save daemon config
- [ ] `DaemonConfig::validate()` - Validate config
- [ ] PID file management
- [ ] Logging setup (file + syslog)
- [ ] Signal handling (SIGTERM, SIGHUP)

## dvs-server

- [ ] `start_server()` - Initialize and run HTTP server
- [ ] `create_router()` - Setup axum routes
- [ ] `get_file()` - GET /api/v1/files/:hash
- [ ] `upload_file()` - POST /api/v1/files
- [ ] `check_file()` - HEAD /api/v1/files/:hash
- [ ] `delete_file()` - DELETE /api/v1/files/:hash
- [ ] `get_metadata()` - GET /api/v1/metadata/:hash
- [ ] `upload_metadata()` - POST /api/v1/metadata
- [ ] `health_check()` - Already implemented (returns ok)
- [ ] `server_status()` - GET /api/v1/status
- [ ] `validate_api_key()` - API key authentication
- [ ] `validate_jwt()` - JWT token validation
- [ ] `has_permission()` - Permission checking
- [ ] `auth_middleware()` - Extract auth from headers
- [ ] `LocalStorage::new()` - Initialize storage backend
- [ ] `LocalStorage::exists()` - Check file exists
- [ ] `LocalStorage::get_path()` - Get file path
- [ ] `LocalStorage::store()` - Store file data
- [ ] `LocalStorage::delete()` - Delete file
- [ ] `LocalStorage::stats()` - Storage statistics
- [ ] `ServerConfig::load()` - Load server config
- [ ] `ServerConfig::save()` - Save server config
- [ ] `ServerConfig::validate()` - Validate config
- [ ] CORS configuration
- [ ] Rate limiting
- [ ] Request logging middleware
- [ ] Streaming uploads/downloads for large files

## dvsR (R Package)

- [ ] Wire `dvs_init()` to `dvs_core::init()`
- [ ] Wire `dvs_add()` to `dvs_core::add()`
- [ ] Wire `dvs_get()` to `dvs_core::get()`
- [ ] Wire `dvs_status()` to `dvs_core::status()`
- [ ] R-friendly error handling (convert DvsError to R errors)
- [ ] R-friendly return types (data.frames for status)
- [ ] Progress reporting to R console
- [ ] Vectorized file operations
- [ ] Integration with fs package conventions

## Testing

- [ ] Unit tests for dvs-core types
- [ ] Unit tests for dvs-core helpers
- [ ] Unit tests for dvs-core operations
- [ ] Integration tests with temp directories
- [ ] Integration tests with real git repos
- [ ] dvs-daemon IPC tests
- [ ] dvs-server API tests
- [ ] dvsR testthat tests
- [ ] Property-based tests for hashing
- [ ] Benchmark tests for large files

## Documentation

- [ ] README.md with quick start
- [ ] Architecture documentation
- [ ] API documentation (rustdoc)
- [ ] CLI help text and man pages
- [ ] R package vignettes
- [ ] Examples directory

## CI/CD

- [ ] GitHub Actions for Rust tests
- [ ] GitHub Actions for R CMD check
- [ ] Cross-platform builds (Linux, macOS, Windows)
- [ ] Release automation
- [ ] Changelog generation

## Error Handling & Recovery

- [ ] Atomic file operations (write to temp, rename)
- [ ] Rollback on partial failures
- [ ] Retry logic for transient errors
- [ ] File locking for concurrent access
- [ ] Orphan cleanup (interrupted operations)
- [ ] Corrupt file detection and recovery
- [ ] Network timeout handling (server/client)

## Performance

- [ ] Parallel hashing with rayon
- [ ] Streaming hash computation (no full file in memory)
- [ ] Connection pooling for server client
- [ ] Batch operations (add/get multiple files)
- [ ] Lazy metadata loading
- [ ] Hash cache invalidation strategy
- [ ] Memory-mapped I/O threshold tuning

## Security

- [ ] Input validation (path traversal prevention)
- [ ] Secure file permissions (0600/0640)
- [ ] API key hashing (don't store plaintext)
- [ ] Rate limiting per API key
- [ ] Audit logging
- [ ] TLS configuration for server
- [ ] Secrets management (config file permissions)

## Compatibility

- [ ] Windows path handling (backslashes)
- [ ] Windows permission model
- [ ] Case-insensitive filesystem handling
- [ ] Symlink handling (follow vs ignore)
- [ ] Large file support (>4GB on 32-bit)
- [ ] Unicode filename support
- [ ] Git worktree support
- [ ] Git submodule support

## Future Features

- [ ] Remote storage backends (S3, GCS, Azure)
- [ ] Compression options (zstd, lz4)
- [ ] Encryption at rest
- [ ] Deduplication across repos
- [ ] Web UI for server
- [ ] Garbage collection for orphaned files
- [ ] Hooks (pre-add, post-get)
- [ ] Git LFS migration tool
- [ ] Partial/range downloads
- [ ] Bandwidth throttling
- [ ] Signed URLs for temporary access
- [ ] Quota management per user/project
- [ ] Replication between servers
- [ ] Offline mode with sync queue
- [ ] File type detection and validation
- [ ] Metadata versioning/history

## Misc

- [ ] add a configuration "version", and assume that it is `legacy` if no
    version is provided.
