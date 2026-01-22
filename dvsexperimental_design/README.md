# DVS Experimental Design Docs

This directory contains the current, authoritative design documentation for the
DVS experimental codebase. These docs are intentionally aligned to the
sync-first direction (daily usage should be add + sync + status) while also
calling out gaps where the implementation is not yet consistent.

## How to use these docs

- Start with ARCHITECTURE.md to understand the system layers.
- Read COMMANDS.md to see which commands are daily vs advanced.
- Use DATA_MODEL.md and STORAGE.md for file formats and layout.
- Use WORKFLOWS.md for end-to-end behavior.
- Use ROADMAP.md for known gaps and migration to sync-first behavior.

## Current vs target

These docs describe both:
- Current behavior where it is implemented today.
- Target behavior for the sync-first CLI (with explicit notes where the code
  diverges).

If a conflict exists, follow ROADMAP.md and the sync-first decisions in
plans/048-cli-sync.md.

## Document map

- ARCHITECTURE.md: component diagram, crate responsibilities, data flow.
- COMMANDS.md: what each command does and why it exists.
- DATA_MODEL.md: config, metadata, manifest, OID, reflog.
- STORAGE.md: external storage and cache layout, hash addressing.
- WORKFLOWS.md: init/add/sync/status and advanced flows.
- CONFIG.md: repo config, local config, server config, daemon config.
- REMOTE_SERVER.md: CAS server endpoints and auth.
- STATUS_AND_SYNC.md: discrepancy model and sync policy.
- SECURITY_AND_INTEGRITY.md: threat model, integrity checks, auth.
- TESTING.md: test strategy and gaps.
- ROADMAP.md: sync-first migration plan and open issues.
- GLOSSARY.md: terms used throughout the docs.
