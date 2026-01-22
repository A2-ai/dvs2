# Configuration

This document describes the configuration files used by DVS.

## Repo config (tracked)

Repo configuration is stored at the repository root and **should be tracked in
Git**.

### File name

The filename depends on build features:

- `dvs.yaml` if the `yaml-config` feature is enabled (default in this repo)
- `dvs.toml` if `toml-config` is enabled and YAML is not
- `dvs.json` as a fallback when only `serde` is enabled

### Fields (dvs-core::Config)

- `storage_dir` (Path): external storage directory for objects.
- `permissions` (u32, optional): Unix file permissions for stored objects.
- `group` (string, optional): Unix group for stored objects.
- `hash_algo` (enum, optional): `blake3`, `sha256`, or `xxh3`.
- `metadata_format` (enum, optional): `json` or `toml`.
- `generated_by` (object, optional): version metadata about the DVS build that
  created the config.

Example (YAML):

```
storage_dir: /mnt/shared/dvs-storage
permissions: 420
hash_algo: blake3
metadata_format: toml
generated_by:
  version: "0.0.0-9000"
  commit: "abc12345"
  tool: "dvs"
```

Notes:
- `permissions` and `group` are only applied on Unix.
- `generated_by` is auto-populated by `dvs init`.

## Local config (per-user, not tracked)

Local config lives under `.dvs/config.toml` and stores user-specific settings.
This file should **not** be committed to Git.

### Fields (dvs-core::LocalConfig)

- `base_url` (string, optional): default remote URL for push/pull/sync.
- `auth.token` (string, optional): bearer token for HTTP CAS auth.
- `cache.max_size` (u64, optional): cache size limit (not enforced yet).

Example (TOML):

```
base_url = "https://dvs.example.com"

[auth]
token = "your-token"

[cache]
max_size = 10737418240
```

## Server config (dvs-server)

The server uses a TOML config file read by `dvs-server`.

### Fields (dvs-server::ServerConfig)

- `host` (string): bind host, default `127.0.0.1`.
- `port` (u16): bind port, default `8080`.
- `storage_root` (Path): CAS storage directory.
- `auth` (object): authentication settings.
- `max_upload_size` (u64): max upload bytes (default 100MB).
- `cors_enabled` (bool)
- `cors_origins` (list of strings)
- `log_level` (string)

### Auth config

`auth` uses API keys with permissions:

- `enabled` (bool)
- `api_keys[]` with fields: `key`, `name`, `permissions`.
- Permissions: `Read`, `Write`, `Delete`, `Admin`.

Example (TOML):

```
host = "0.0.0.0"
port = 8080
storage_root = "/var/dvs/storage"
max_upload_size = 104857600
cors_enabled = true
cors_origins = ["http://localhost:3000"]
log_level = "info"

[auth]
enabled = true

[[auth.api_keys]]
key = "secret-key"
name = "CI"
permissions = ["Read", "Write"]
```

## Environment variables

- `DVS_GIT_BACKEND=cli`: forces the CLI git backend instead of libgit2.

## Config validation

- `dvs init` validates or creates `storage_dir` and checks group existence.
- `dvs-server` validates host, port, storage path, and upload size on startup.
