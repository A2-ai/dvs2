# AGENTS.md

Commit `Cargo.lock` due to the presence of dependencies on github.

## Development iteration

`just rpkg-dev` is the full development build (`minirextendr::miniextendr_build()`): autoconf, configure, a compile that writes `R/dvs-wrappers.R` from the linked library, roxygen2 (`NAMESPACE`, `man/`), then one install. `devtools::load_all("dvs-rpkg")` also compiles the embedded crate and regenerates the wrappers. Any build of the R package rewrites `R/dvs-wrappers.R` when the Rust side changed.

No development build vendors. Vendoring happens only in the release tarball build.

The recipes that call minirextendr put the rv library first in `R_LIBS`, so they use the minirextendr that `dvs-rpkg/rproject.toml` and `dvs-rpkg/rv.lock` pin. Run `just rpkg-sync` after a fresh checkout or a pin change. They stop unless the `Rscript` on `PATH` is the R version in `rproject.toml` and the rv library holds minirextendr.

## Release tarball

`just rpkg-build-tarball` writes the offline, CRAN-ready `dist/dvs_<version>.tar.gz` (`minirextendr::miniextendr_build_tarball()`). It refreshes the wrappers, `NAMESPACE` and `man/`, seals every crate into `inst/vendor.tar.xz` with cargo-revendor, builds the tarball, and restores `src/rust/Cargo.toml`, `Cargo.lock` and `vendor/`. `just rpkg-check-cran` runs `R CMD check --as-cran` on that tarball. Both need cargo-revendor at the pin: `just install-revendor`.

`just rpkg-reference` builds the single-page HTML reference manual from `man/` and copies it to `dist/reference/dvs.html`. It checks the Rd files first and stops on a problem. It reads no build output and writes nothing the build watches.

Never commit `inst/vendor.tar.xz`. While it exists, `configure` builds in offline tarball mode. The miniextendr pre-commit hook refuses it.

## Bump the miniextendr pin

The pin is the `rev` of `miniextendr-api` and `miniextendr-lint` in `dvs-rpkg/src/rust/Cargo.toml` and the minirextendr `commit` in `dvs-rpkg/rproject.toml`. Keep all three at one commit. The miniextendr pre-commit hook refuses a commit where `rv.lock` and `src/rust/Cargo.lock` name different commits. `just rpkg-update` does not move the pin.

1. See how far main is ahead:

   ```shell
   old=$(grep -o 'rev = "[0-9a-f]*"' dvs-rpkg/src/rust/Cargo.toml | head -1 | cut -d'"' -f2)
   gh api "repos/A2-ai/miniextendr/compare/$old...main" --jq '.ahead_by'
   ```

   Read the commits (`git log --oneline "$old..origin/main"` in a miniextendr clone) for API changes that reach `dvs-rpkg/src/rust/*.rs` and for features that replace hand-written code.
2. Move the three pins. Run `just rpkg-sync` (`rv plan`, then `rv sync`). Check that `rv.lock` and `src/rust/Cargo.lock` name the new commit.
3. Compile with `just rpkg-check`, `just rpkg-check --all-features` and `just rpkg-clippy --all-features`. Port the API drift. This is the pin commit.
4. Diff the two pins' templates: `git diff --stat "$old" <new> -- minirextendr/inst/templates` in a miniextendr clone.
5. Run `just rpkg-upgrade` (`upgrade_miniextendr_package(configure_ac = TRUE)`). It refreshes the build templates, `configure.ac`, `src/Makevars.win`, the agent skills in `.claude/skills/` and the miniextendr sections of the git hooks. `AC_INIT` keeps the version in `DESCRIPTION`. Review every file it lists.
6. Run `just rpkg-dev` and review `git diff dvs-rpkg/R dvs-rpkg/man dvs-rpkg/NAMESPACE`. This is the scaffold commit.
7. Test: `devtools::test("dvs-rpkg")`, `just rpkg-build-tarball` and an offline install of the tarball, `just rpkg-check-cran`, `bash dvs-rpkg/tests/rv`. Outside devtools, run testthat with `NOT_CRAN=true`. The `gctorture` cases skip without it.

The pin bump is one commit, the scaffold refresh another, and each adopted feature its own.

## Run the dvs CLI from any git ref

`just dvs <ref> <args...>` runs a `dvs` binary built from any branch, tag, or commit. Example: `just dvs main status`, or `just dvs HEAD~3 --version`. Use it to test cross-version behavior, such as reading data written by a CLI built from another branch.

- `just dvs-build <ref>` builds if needed and prints the binary path. Progress goes to stderr, so `bin=$(just dvs-build main)` works in scripts.
- The cache is keyed by the resolved commit hash, never the ref name, so a moving branch never serves a stale binary. It is shared across all worktrees at `<main-worktree>/target/dvs-versions/<hash>/bin/dvs`.
- Pass CLI flags directly, as in `just dvs main --help`. Do not insert `--` before the flags. just forwards `--` literally to the binary.
- `just dvs HEAD` builds the committed tree, not your uncommitted changes. Use `just install-cli` for the working tree.
- `just dvs-clean` removes all cached builds, or `just dvs-clean <ref>` removes just one. A plain `cargo clean` in the main worktree also removes the cache, since it lives under `target/`.

## Shared state

If a change adds a DB migration or changes anything on the shared data filesystem (`/data`
by default), read [docs/shared-state.md](docs/shared-state.md) before opening the PR. It
needs the `shared-state` label and a #2-tech-devops post before it deploys anywhere.
