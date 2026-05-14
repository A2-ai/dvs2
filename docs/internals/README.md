# dvs2 internals

These docs cover how the three crates and packages in this workspace fit together internally. They are aimed at someone reading the code for the first time and trying to build a mental model. They are not aimed at end users.

## Workspaces

The repository contains two cargo workspaces. The root workspace at `/Cargo.toml` has two members: `dvs` (the core library) and `dvs-cli` (the binary). The R-package workspace at `/dvs-rpkg/src/rust/Cargo.toml` is a standalone workspace. It depends on `dvs` either by git (for CRAN and release builds) or by local path (for monorepo development).

## Layered architecture

```
                     ┌─────────────────────────────┐
                     │   dvs-cli (binary `dvs`)    │   clap, indicatif, tabled
                     └──────────────┬──────────────┘
                                    │
       ┌────────────────────────────┴──────────────────────────────┐
       │                                                            │
       │                  ┌─────────────────────────┐               │
       │                  │   dvs (core lib crate)  │ ◀─────────────┤
       │                  └─────────────────────────┘               │
       │                                                            │
       │                                                            │
       │      ┌──────────────────────┐      ┌──────────────────────┐│
       │      │  dvs-rpkg R wrappers │─────▶│ dvs-rpkg FFI crate   ││
       │      │  (R/*.R)             │      │ (src/rust/lib.rs)    │┘
       │      └──────────────────────┘      └──────────────────────┘
       │                                              │
       │                                              │ via
       │                                              │ miniextendr
       └──────────────────────────────────────────────┘
```

## Files

[dvs.md](dvs.md) covers the core library. It explains the storage model, hashing, the backend trait, the batch flows for init, add, get, and status, the cache, and the audit log.

[dvs-cli.md](dvs-cli.md) covers the binary that wraps the core crate. Its main topics are clap, progress bars, and table or JSON output.

[dvs-rpkg.md](dvs-rpkg.md) covers the R package. It walks through the miniextendr FFI layer, the R wrappers, type marshalling, and pillar formatting.

## Recommended reading order

Start with section 1 (the mental model) and section 4 (the core flows) of [dvs.md](dvs.md). Everything else is in service of those flows.

Then skim [dvs-cli.md](dvs-cli.md). The CLI is a thin layer over the core, so once you understand the core, the CLI reads quickly.

Read [dvs-rpkg.md](dvs-rpkg.md) only if you need to touch the R side. The architecture overview and the FFI boundary section are the load-bearing parts.
