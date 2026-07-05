# Plan: switch dvs-rpkg from a git pin to a path dependency on the in-repo dvs core

Status: PLAN ONLY. Not started. Hand this to a fresh session to execute and verify.

## Goal

Stop manually bumping the dvs core in dvs-rpkg. The core lives in the same
monorepo at `dvs/`. dvs-rpkg should depend on it by path, and cargo-revendor
should produce a self-contained `inst/vendor.tar.xz` so the shipped package
still builds offline with no monorepo around it. Dev builds compile the live
`../dvs`. There is no SHA to advance.

## Why this is sound

cargo-revendor is purpose-built for this. Its README states first-class
support for "workspace path dependencies, monorepo siblings, R package
layouts." Plain `cargo vendor` skips path deps. cargo-revendor runs
`cargo package` per local crate, rewrites `path = "../sibling"` to
`vendor/<name>-<version>/`, and `--freeze` rewrites the target manifest so the
whole graph resolves from `vendor/`. That `vendor/` directory ships inside the
package, so it travels with the package into R's temporary build directory.
That is exactly what fixes the relative-path breakage that motivated the git
pin.

This is the cargo-revendor route, not the older `minirextendr::use_vendor_lib`
route. use_vendor_lib edits `configure.ac` (regenerated on every template
upgrade) and ships a separate `inst/dvs-lib.tar.gz`. cargo-revendor needs no
`configure.ac` edit and uses the single existing `inst/vendor.tar.xz`.

## Current state (verified 2026-06-22 on main b3908f6)

- `dvs-rpkg/src/rust/Cargo.toml` line 37:
  `dvs = { git = "https://github.com/A2-ai/dvs2", branch = "main", package = "dvs" }`
  Cargo.lock pins the SHA. Bumping the core means `cargo update -p dvs` plus a PR.
- miniextendr-api, miniextendr-lint, miniextendr-macros are git deps on a
  separate repo. They stay git. Only dvs changes.
- `configure.ac` has no dvs-specific or dvs2-git-url-specific logic. Vendoring
  is generic and gated on `IS_TARBALL_INSTALL`. Confirmed by grep.
- `upgrade_miniextendr_package` does not touch `src/rust/Cargo.toml`. So a
  path-dep line is re-scaffold stable.
- `just vendor` runs `cargo revendor ... --source-root . --freeze --strip-all
  --compress inst/vendor.tar.xz ...`.
- `bootstrap.R` runs `cargo revendor --manifest-path src/rust/Cargo.toml
  --output vendor --compress inst/vendor.tar.xz --blank-md --source-marker
  --force -v`. It does NOT pass `--freeze` or `--source-root`. This is the main
  thing to fix or verify for the path-dep case.
- `.Rbuildignore` ignores `src/rust/.cargo` and `vendor`. The package ships
  vendored sources only through `inst/vendor.tar.xz`.

## Proposed change (on a branch)

1. `dvs-rpkg/src/rust/Cargo.toml`: replace the dvs git line with
   `dvs = { path = "../../../dvs", package = "dvs" }`.
   Remove the now-irrelevant dev `[patch]` guidance comment for dvs. Keep the
   miniextendr git deps and any miniextendr `[patch]` guidance untouched.
2. Delete the stale dvs SHA from `Cargo.lock` by regenerating it
   (`cargo update` or let configure or revendor do it). dvs becomes a local
   crate in the lock, not a git source.
3. Decide and apply the `bootstrap.R` change if testing shows it is needed.
   The likely change is to pass `--freeze --source-root <monorepo-root>` to the
   `cargo revendor` call so the path dep is rewritten to `vendor/`. Resolve the
   in-place manifest rewrite concern first (see Open questions item 1).

## Test matrix (the real deliverable)

Prerequisite. Install the latest cargo-revendor and record its version.
`cargo install --git https://github.com/A2-ai/miniextendr cargo-revendor --locked`
Use R 4.5 for all R steps. rig default reverts to 4.6, so prefix with the 4.5
bin path. See the dvs-rpkg build memory.

### Test 1: dev local install, monorepo present

- `just rpkg-install` from the branch worktree.
- Expect a compile of the live `../dvs` (build log shows a path source, not a
  `git+...dvs2#sha` source), then `* DONE (dvs)`.
- Edit a string in `dvs/src/...`, run `just rpkg-install` again, and confirm the
  edit is reflected with no pin bump. This proves the no-bump goal.

### Test 2: release tarball, clean-room install, no monorepo

This is the critical self-contained proof.

- Produce the vendored tarball. Two candidate flows to compare:
  - `just vendor` then `R CMD build dvs-rpkg`.
  - `devtools::build("dvs-rpkg")` which runs `bootstrap.R`.
- Inspect the built `dvs_<ver>.tar.gz`. Confirm it contains
  `inst/vendor.tar.xz`, and that unpacking that tarball shows
  `vendor/dvs-<ver>/src/...` with the real dvs source.
- Confirm the sealed `src/rust/Cargo.toml` inside the package points dvs at
  `vendor/...`, not at `../../../dvs`.
- Copy the `.tar.gz` to a scratch directory that has no dvs2 checkout anywhere
  above it. Run `R CMD INSTALL dvs_<ver>.tar.gz` there with no network if
  possible.
- Expect tarball mode, an offline build from the vendored dvs source, and
  `* DONE`. Load the package and run `dvs_init` plus `dvs_add` to confirm.

### Test 3: dev install via remotes::install_github, the stated dev path

- Push the branch. Run
  `remotes::install_github("A2-ai/dvs2", subdir = "dvs-rpkg", ref = "<branch>")`.
- Expectation. install_github downloads the whole repo, so the `dvs/` sibling
  is present at build time and the path resolves. pkgbuild honors
  `Config/build/bootstrap`, so `bootstrap.R` runs, configure plus cargo-revendor
  seal `inst/vendor.tar.xz`, then the tarball installs.
- Confirm the install succeeds end to end and the package loads.

## Open questions to resolve during testing

Read-only verification 2026-06-22 (read cargo-revendor src + README, bootstrap.R,
configure.ac, justfile, lock-shape-check.R). Verdicts inline. No repo changes made.

1. Freeze rewrites the manifest in place. CONFIRMED REAL for a path dep.
   `freeze_manifest` does an unconditional `std::fs::write(manifest_path, ...)`
   (cargo-revendor `src/vendor.rs:1109`), NOT wrapped by ManifestGuard
   (ManifestGuard only covers the transient `[patch.crates-io]` added during the
   `cargo package` step and restores BEFORE freeze runs). With dvs as a local
   path-dep crate, freeze step 1 rewrites `dvs = { path = "../../../dvs" }` to
   `dvs = { path = "../../vendor/dvs-<ver>" }` and step 3 adds a
   `[patch.crates-io]` block. Both persist. `just vendor` has NO restore step.
   This is benign TODAY only because dvs is a git (external) dep, so freeze
   touches nothing in the committed manifest. The migration converts a no-op
   freeze into a manifest-mutating one. REQUIRED change: add
   `git checkout dvs-rpkg/src/rust/Cargo.toml dvs-rpkg/src/rust/Cargo.lock` after
   `just vendor`. In throwaway build trees (the install_github bundle, the
   R CMD build copy) the rewrite is harmless and in fact necessary, so only the
   dev/monorepo invocations need the restore.
2. Does bootstrap.R need `--freeze` (plus maybe `--source-root`). CONFIRMED YES.
   Path deps are NOT source-replaceable (configure's `[source] replace-with` and
   the `.cargo/config.toml` apply only to crates.io and git sources). Freeze is
   the ONLY thing that rewrites the target manifest's path dep to `vendor/`.
   Without it the sealed `src/rust/Cargo.toml` keeps `../../../dvs` and a
   clean-room install cannot resolve dvs. bootstrap.R currently omits `--freeze`,
   so it MUST gain it. `--source-root` auto-detects from `cargo metadata` at
   produce time (the path still resolves then), so it is likely optional for the
   produce pass; pass `--source-root ..` to match `just vendor` if testing shows
   drift. Adding `--freeze` to bootstrap.R re-triggers item 1 for the dev
   `devtools::build()` case (it runs bootstrap.R in the tracked tree) — restore
   needed there too, or run release vendoring only on a throwaway/CI checkout.
3. Does remotes::install_github bring the `dvs/` sibling. RESOLVED 2026-06-22 by
   empirical test (`remotes::install_git` from a local `file://` repo = identical
   machinery to install_github, only transport differs; branch had BOTH the
   path-dep manifest and the bootstrap.R `--freeze --source-root ..` change).
   YES on all counts: remotes cloned the whole repo to a temp bundle WITH the
   `dvs/` sibling, built the subdir IN-PLACE (sibling present at build time),
   honored `Config/build/bootstrap` (bootstrap.R ran), revendor froze and sealed
   ("bootstrapped 2 workspace crate(s)", "vendored 1 local + 135 external",
   "Frozen: ... resolves from vendor/ only"), R CMD build produced
   `dvs_0.3.0.9000.tar.gz`, then tarball-mode offline install → `* DONE (dvs)`.
   Installed package loads and runs (`dvs_version()` = 0.3.0, `dvs_init()` =
   "DVS Initialized"). The caveat still stands for OTHER consumers (`rv sync` /
   `install.packages()` extracting `dvs-rpkg/` alone with no sibling and no
   bundled tarball) — those would break at source-mode build, exactly as today's
   Cargo.toml comment warns. install_github itself is proven fine.
   Required: bootstrap.R MUST carry `--freeze --source-root ..` (without it the
   sealed manifest keeps `../../../dvs` and the install fails). Tested with it.
4. Provenance. User decision (unchanged). Path dep drops the dvs SHA from
   Cargo.lock; the vendored source in the release tarball becomes the record.
   Reproducible release requires vendoring from a clean checkout at a tag.
5. CI / lock-shape-check. RESOLVED 2026-06-22 by empirical probe (scratch copy,
   path-dep manifest, full `just vendor`-equivalent revendor with `--freeze`).
   dvs lands SOURCELESS in the frozen Cargo.lock (a local crate has no `source =`
   line), so NO `source = "path+..."` appears. The real `tools/lock-shape-check.R`
   exits 0 in tarball mode. miniextendr crates land as `git+...#<sha>` (correct);
   120 `checksum =` lines (allowed). The sealed `inst/vendor.tar.xz` contains
   `vendor/dvs/{Cargo.toml,src/lib.rs}` with workspace inheritance resolved
   (literal `version = "0.3.0"`, no `.workspace = true`). Freeze rewrote the
   manifest to `dvs = { path = "../../vendor/dvs", version = "*" }` + a
   `[patch.crates-io] dvs = { path = "../../vendor/dvs" }` block; flat `vendor/dvs`
   (not versioned) for the local crate. NO carve-out needed. The guard accepts
   the path-dep migration as-is.
6. Mixed sources in one pass. CONFIRMED OK. Freeze rewrites dvs (local to
   `vendor/` path + `[patch.crates-io]`) and leaves miniextendr as a git dep
   (logged as a remaining-git warning, resolved offline at install via configure's
   `[source."git+<url>"]` scan of Cargo.toml). configure's git-URL scan stops
   emitting a dvs entry once dvs is no longer `git =` (correct). Path math checks
   out: `vendor_rel = pathdiff(output, manifest_parent)` = `../../vendor` from
   `src/rust/`, and configure unpacks the tarball to `dvs-rpkg/vendor`, so the
   frozen `../../vendor/dvs-<ver>` resolves to the unpacked dir. Self-consistent.

## Re-scaffold interaction

Confirmed favorable. `configure.ac` and `Makevars.in` are generic, and
`upgrade_miniextendr_package` does not touch `Cargo.toml`. So the only
divergence from the template is the one dependency line in `Cargo.toml`, which
the template never overwrites. Re-verify this after any future miniextendr
template upgrade. If a future template starts managing the dvs line, revisit.

## Rollback

If the in-place freeze rewrite or the install_github path proves fragile,
revert to the git pin and automate the bump instead. A small CI job can run
`cargo update -p dvs` on core main pushes and open the bump PR. The bump still
lands through review, which keeps reproducibility, but removes the manual step.

## Relevant files

- `dvs-rpkg/src/rust/Cargo.toml` dependency declaration
- `dvs-rpkg/src/rust/Cargo.lock` dvs source entry
- `dvs-rpkg/bootstrap.R` cargo-revendor invocation
- `justfile` vendor recipe
- `dvs-rpkg/configure.ac` generic vendoring, no change expected
- cargo-revendor README in the local miniextendr checkout at
  `/Users/elea/Documents/GitHub/miniextendr/cargo-revendor/README.md`
