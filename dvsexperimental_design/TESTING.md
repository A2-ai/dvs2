# Testing strategy

This document summarizes the existing testing coverage and gaps.

## Existing coverage

### Unit tests (Rust)

- `dvs-core` includes unit tests in helpers and ops (`add`, `get`, `init`,
  `hash`, `config`, etc.).
- `dvs-server` includes unit tests for storage, config, and API helpers.

These focus on file I/O correctness, hashing, and basic operation behavior.

### Conformance testing (dvs-testkit)

`dvs-testkit` provides a framework for running the same scenario across
interfaces and diffing the resulting workspace state.

Implemented runners:

- `CoreRunner` (calls dvs-core directly)
- `CliRunner` (feature-gated, invokes the CLI)
- `ServerRunner` (feature-gated, tests CAS endpoints)

Not implemented:

- `RRunner`
- `DaemonRunner`

## Gaps

- No end-to-end tests for a sync-first workflow (since sync is not implemented).
- Limited tests for TOML metadata vs JSON metadata parity.
- Limited tests for alternate hash algorithms (sha256/xxh3) across all ops.
- No tests validating manifest/metadata consistency.
- No tests for integrity verification after pull/materialize.
- No regression tests for .gitignore integration (data vs metadata ignores).

## Recommended additions for sync-first

1) End-to-end sync scenarios
   - add -> sync -> status
   - sync --pull-only / --push-only
   - sync with local dirty files and --update

2) Manifest + metadata consistency
   - add writes manifest
   - status detects drift

3) Storage path consistency
   - external storage + local cache + server share the same path layout

4) Remote CAS integrity
   - server rejects mismatched payload hashes
   - pull verifies hash on client side (once implemented)

5) R interface parity
   - Use dvs-testkit to assert R bindings produce the same state as core.
