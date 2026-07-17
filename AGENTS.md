# AGENTS.md

Commit `Cargo.lock` due to the presence of dependencies on github.

## Development iteration

The R bindings to the embedded rust crate are generated when

- `devtools::load_all({"dvs-rpkg")`

this will compile the embedded rust crate, regenerate R wrappers, and replace them if they are different
from previous iteration, and then build the package.

Any invocation that rebuilds the r package will update the R wrappers for the embedded rust crate.

## Upgrade miniextendr version

Run

```shell
just rpkg-update
```

in order to update miniextendr in the embedded rust crate i.e. `cargo update --manifest-path=dvs-rpkg/src/rust/Cargo.toml`.

The R package scaffolding needs to be updated too. The `minirextendr` R package has a `minirextendr::upgrade_miniextendr_package` that will facilitate the upgrade.

## Run the dvs CLI from any git ref

`just dvs <ref> <args...>` runs a `dvs` binary built from any branch, tag, or commit. Example: `just dvs main status`, or `just dvs HEAD~3 --version`. Use it to test cross-version behavior, such as reading data written by a CLI built from another branch.

- `just dvs-build <ref>` builds if needed and prints the binary path. Progress goes to stderr, so `bin=$(just dvs-build main)` works in scripts.
- The cache is keyed by the resolved commit hash, never the ref name, so a moving branch never serves a stale binary. It is shared across all worktrees at `<main-worktree>/target/dvs-versions/<hash>/bin/dvs`.
- Pass CLI flags directly, as in `just dvs main --help`. Do not insert `--` before the flags. just forwards `--` literally to the binary.
- `just dvs HEAD` builds the committed tree, not your uncommitted changes. Use `just install-cli` for the working tree.
- `just dvs-clean` removes all cached builds, or `just dvs-clean <ref>` removes just one. A plain `cargo clean` in the main worktree also removes the cache, since it lives under `target/`.
