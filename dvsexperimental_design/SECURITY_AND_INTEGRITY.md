# Security and integrity

This document describes the security model, integrity guarantees, and current
limitations.

## Threat model (what DVS does and does not protect)

DVS is designed to:

- Keep large data out of Git history.
- Provide content-addressable integrity checks.
- Allow controlled distribution through a remote CAS server.

DVS does **not** provide:

- Encryption at rest or in transit (beyond HTTPS if configured).
- Access control on the local filesystem (relies on OS permissions).
- Fine-grained multi-tenant isolation in the server.

If you need confidentiality, use encrypted storage, encrypted transport, and
proper OS-level permissions.

## Integrity guarantees

### Local add/get

- `dvs add` hashes local files and records the hash in metadata.
- `dvs get` verifies the hash after copying from external storage.

### Remote server

- `dvs-server` verifies that uploaded payloads match the OID in the URL.
- `dvs pull` downloads objects but does not re-verify the hash locally.
- `dvs materialize` copies from cache without re-hashing.

This means remote -> cache -> working tree is currently **not verified**. A
future improvement should validate cached objects against their OIDs during
pull or materialize.

## Authentication and secrets

- Remote auth uses `Authorization: Bearer <token>`.
- Tokens are stored in `.dvs/config.toml` and should not be committed.
- `.dvs/` should be in `.gitignore` (local config, cache, state).

## Data exposure risks

- If `storage_dir` is world-readable, data is exposed even if Git is clean.
- If the remote CAS is public and auth is disabled, objects are publicly
  retrievable by hash.
- Hashes themselves are not secrets; avoid putting sensitive data in DVS without
  additional protections.

## Recommended safeguards

- Store `storage_dir` on a secured shared filesystem with correct permissions.
- Enable auth on the server and use HTTPS.
- Treat `.dvs/config.toml` as secret material (ignore it in Git).
- Consider adding optional client-side verification after pull.

## Future work

- Optional encryption at rest (client-side).
- Signed metadata for provenance and tamper detection.
- End-to-end integrity verification during sync.
