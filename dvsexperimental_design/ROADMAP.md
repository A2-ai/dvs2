# Roadmap (sync-first alignment)

This roadmap captures the gaps between the current implementation and the
intended sync-first design. It complements plans/048-cli-sync.md.

## 1) Sync-first CLI

- Implement `dvs sync` in dvs-cli + dvs-core.
- Remove `get`, `push`, and `pull` from CLI help and docs (no aliases).
- Update all user-facing docs to make sync the single entry point.

## 2) Manifest and metadata consistency

- Ensure every `add` updates `dvs.lock` and metadata in lockstep.
- Make `status`/`rollback`/`merge` read TOML and JSON metadata consistently.
- Detect and report manifest/metadata drift.

## 3) Storage path and cache consistency

- Ensure external storage, local cache, and server use identical path layout.
- Decide how `add` populates local cache (or make push read from external
  storage instead).
- Ensure sync can upload newly added objects without a manual cache step.

## 4) Git integration

- Update .gitignore handling so data files are ignored, not metadata.
- Align with the objective: "Git tracks only metadata files."

## 5) Status improvements

- Extend status to report cache and remote availability.
- Clearly report dirty tracked files and missing objects.
- Provide a "what would sync do" summary.

## 6) Integrity verification

- Verify hashes after pull and/or materialize.
- Consider optional checksum verification for cache contents.

## 7) Reflog and rollback

- Include manifest snapshots in reflog state.
- Ensure rollback restores both metadata and manifest consistently.

## 8) R interface

- Align with 4-function surface: init, add, sync, status.
- Implement R runner in dvs-testkit.

## 9) Garbage collection

- Add GC for external storage and local cache.
- Base deletions on references in manifest + metadata + reflog.

## 10) Documentation hygiene

- Deprecate older design docs in `design/` or mark clearly as historical.
- Keep dvsexperimental_design as the authoritative spec.
