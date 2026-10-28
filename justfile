# https://just.systems

rpkg_dir := "dvs-rpkg"
rpkg_manifest := rpkg_dir / "src/rust/Cargo.toml"

default:
    @just --list

# Download sample datasets for UI tests
datasets:
    mkdir -p datasets
    curl -fL \
    -o datasets/theoph.csv https://vincentarelbundock.github.io/Rdatasets/csv/datasets/Theoph.csv \
    -o datasets/indometh.csv https://vincentarelbundock.github.io/Rdatasets/csv/datasets/Indometh.csv \
    -o datasets/chickweight.csv https://vincentarelbundock.github.io/Rdatasets/csv/datasets/ChickWeight.csv \
    -o datasets/orange.csv https://vincentarelbundock.github.io/Rdatasets/csv/datasets/Orange.csv

# ============================================================================
# dvs crate
# ============================================================================

build *args:
    cargo build {{args}}

build-release *args:
    cargo build --release {{args}}

test *args:
    cargo test {{args}}

clippy *args:
    cargo clippy {{args}}

fmt *args:
    cargo fmt {{args}}

check *args:
    cargo check {{args}}

doc *args:
    cargo doc --no-deps {{args}}

# Check for std::fs usage (allow Permissions/Metadata types)
check-std-fs:
    @! rg -nP -g '*.rs' -e 'std::fs(?!::(Permissions|Metadata)\b)' -e 'std::\s*\{[^}]*\bfs\b[^}]*\}' dvs

# Install the dvs CLI binary
install-cli *args:
    cargo install --profile dev-cli --force --locked --path=dvs-cli {{args}}

# Progress goes to stderr, so `bin=$(just dvs-build <ref>)` works. Cache is
# keyed by resolved commit hash (never the ref name, which could go stale) and
# shared across all worktrees: <main-worktree>/target/dvs-versions/<hash>/bin/dvs.
# Build & cache the dvs CLI for any git ref (branch/tag/hash); prints the binary path
dvs-build ref:
    #!/usr/bin/env bash
    set -euo pipefail
    ref={{quote(ref)}}
    hash="$(git rev-parse --verify --quiet "$ref^{commit}")" \
        || { echo "error: cannot resolve '$ref' to a commit" >&2; exit 1; }
    root="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
    cache="$root/target/dvs-versions/$hash"
    bin="$cache/bin/dvs"
    if [[ ! -x "$bin" ]]; then
        echo "Building dvs-cli @ $hash ($ref) ..." >&2
        mkdir -p "$root/target/dvs-versions" "$cache/bin"
        work="$(mktemp -d "$root/target/dvs-versions/.work-$hash-XXXXXX")"
        trap 'rm -rf "$work"' EXIT
        git archive "$hash" | tar -x -C "$work"
        cargo build --release --locked \
            --manifest-path "$work/dvs-cli/Cargo.toml" \
            --target-dir "$work/target" 1>&2
        mv -f "$work/target/release/dvs" "$bin"
    fi
    echo "$bin"

# Dash args forward as-is (`just dvs main --help`); do NOT insert `--`,
# just would pass it through literally to the binary.
# Run the dvs CLI built from any git ref: `just dvs main status`
[positional-arguments]
dvs ref *args:
    #!/usr/bin/env bash
    set -euo pipefail
    bin="$({{quote(just_executable())}} --justfile {{quote(justfile())}} dvs-build "$1")"
    exec "$bin" "${@:2}"

# Remove cached dvs CLI builds (all, or one ref's)
[positional-arguments]
dvs-clean *ref:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
    if [[ $# -ge 1 ]]; then
        rm -rf "$root/target/dvs-versions/$(git rev-parse --verify "$1^{commit}")"
    else
        rm -rf "$root/target/dvs-versions"
    fi

# ============================================================================
# R package (dvsR)
# ============================================================================

# Rscript with the rv library first: minirextendr comes from the commit that
# rproject.toml and rv.lock pin, and devtools and roxygen2 from the user library.
rpkg_rscript := 'R_LIBS="$(cd ' + rpkg_dir + ' && pwd)/$(cd ' + rpkg_dir + ' && rv library)" Rscript'

# The rv library is built for the R version in rproject.toml. Another R
# fails to load it with dlopen errors, and an rv library without
# minirextendr would leave R_LIBS pointing nowhere, so an older minirextendr
# from the user library would run instead. Stop before either happens.
[private]
rpkg-r-guard:
    #!/usr/bin/env bash
    set -euo pipefail
    want="$(sed -n 's/^r_version = "\(.*\)"$/\1/p' {{quote(rpkg_dir / "rproject.toml")}})"
    have="$(Rscript -e 'cat(R.version$major, sub("[.].*", "", R.version$minor), sep = ".")')"
    if [ "$want" != "$have" ]; then
      echo "error: Rscript on PATH is R $have, rproject.toml wants R $want." >&2
      echo "  Put R $want first on PATH (rig default $want), then run just rpkg-sync." >&2
      exit 1
    fi
    lib="$(cd {{quote(rpkg_dir)}} && pwd)/$(cd {{quote(rpkg_dir)}} && rv library)"
    if [ ! -f "$lib/minirextendr/DESCRIPTION" ]; then
      echo "error: minirextendr is not in the rv library $lib. Run: just rpkg-sync" >&2
      exit 1
    fi

# Sync the rv project library (rv reads rproject.toml from the cwd, in dvs-rpkg/)
[working-directory: 'dvs-rpkg']
rpkg-sync *args:
    rv sync {{args}}

[working-directory: 'dvs-rpkg']
rpkg-configure:
    if command -v autoconf >/dev/null 2>&1; then autoconf; else echo "autoconf not found; using existing configure"; fi
    bash ./configure

rpkg-build *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-build-release *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} --release {{args}}

# cargo test, the testthat suite against the checkout (devtools sets
# NOT_CRAN, so the gctorture cases run), then the rv install from a local
# path.
[doc('Run the rpkg cargo tests, the testthat suite and the rv install test.')]
rpkg-test *args:
    cargo test --manifest-path={{quote(rpkg_manifest)}} {{args}}
    Rscript -e 'devtools::test("{{rpkg_dir}}", stop_on_failure = TRUE)'
    bash {{quote(rpkg_dir / "tests/rv")}}

rpkg-clippy *args:
    cargo clippy --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-check *args:
    cargo check --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-fmt *args:
    cargo fmt --manifest-path={{quote(rpkg_manifest)}} {{args}}

# Updates the registry dependencies. The miniextendr crates are pinned by
# `rev` and do not move: see AGENTS.md, "Bump the miniextendr pin".
[doc('Update the registry dependencies in the rpkg Cargo.lock (not the miniextendr pin).')]
rpkg-update *args:
    cargo update --manifest-path={{quote(rpkg_manifest)}} {{args}}

# minirextendr's development build: autoconf, configure, a compile that
# writes R/dvs-wrappers.R from the linked library, roxygen2, then one
# install. Nothing in it vendors.
[doc('Build with minirextendr: configure, wrappers, roxygen2, install. No vendoring.')]
rpkg-dev: rpkg-r-guard
    {{rpkg_rscript}} -e 'minirextendr::miniextendr_build("{{rpkg_dir}}")'

rpkg-document:
    Rscript -e 'devtools::document("{{rpkg_dir}}")'

# R CMD INSTALL exits non-zero when the install fails. install.packages()
# only warns, so the old recipe reported success for a failed build.
[doc('R CMD INSTALL the package from the checkout.')]
rpkg-install:
    R CMD INSTALL {{quote(rpkg_dir)}}
alias install-rpkg := rpkg-install

# The one recipe that vendors. miniextendr_build_tarball() refreshes the
# wrappers, NAMESPACE and man/, seals every crate into inst/vendor.tar.xz
# with cargo-revendor, builds dist/dvs_<version>.tar.gz, and restores the
# source tree (Cargo.toml, Cargo.lock, vendor/, the archive) on exit.
# Needs cargo-revendor at the pin: `just install-revendor`. dist/ must
# exist: pkgbuild writes to a dest_path that does not exist as a file.
[doc('Build the vendored, offline release tarball into dist/.')]
rpkg-build-tarball: rpkg-r-guard
    mkdir -p dist
    {{rpkg_rscript}} -e 'minirextendr::miniextendr_build_tarball("{{rpkg_dir}}", dest_path = "dist")'

# R CMD check points TMPDIR at a directory of its own and deletes it at the
# end. An sccache server that the check's first compile starts keeps that
# TMPDIR, and every later Rust build on the machine then fails with
# "sccache: Failed to create temp dir" until the server stops. Start the
# server here, outside the check. A server that is already running is left
# as it is: stopping it would break builds in progress elsewhere. A stale one
# needs `sccache --stop-server`.
[private]
rpkg-sccache-server:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v sccache >/dev/null 2>&1; then
      sccache --start-server >/dev/null 2>&1 || true
    fi

# R CMD check --as-cran --no-manual on that vendored tarball. Fails on a
# WARNING. The remote part of the CRAN incoming check is off: CRAN has a
# package named DVS, and "Conflicting package names" is an ERROR on every run.
[doc('R CMD check --as-cran the vendored release tarball.')]
rpkg-check-cran: rpkg-r-guard rpkg-sccache-server
    _R_CHECK_CRAN_INCOMING_REMOTE_=false {{rpkg_rscript}} -e 'minirextendr::miniextendr_check("{{rpkg_dir}}")'

[doc('Diagnose the package with minirextendr::miniextendr_doctor().')]
rpkg-doctor: rpkg-r-guard
    {{rpkg_rscript}} -e 'minirextendr::miniextendr_doctor("{{rpkg_dir}}")'

# The single-page HTML reference manual, built from man/ with base R's
# tools::pkg2HTML() (R >= 4.4). It runs checkRd over man/ first and stops on a
# finding, so it also lints the Rd files after rpkg-document. The script
# writes dvs-rpkg/rust-target/doc/r/, which a clean install removes, so the
# recipe copies the result to dist/reference/, next to the release tarball.
[doc('Build the HTML reference manual and copy it to dist/reference.')]
rpkg-reference:
    #!/usr/bin/env bash
    set -euo pipefail
    Rscript {{quote(rpkg_dir / "tools/build-html-reference.R")}}
    rm -rf dist/reference
    mkdir -p dist/reference
    cp -R {{quote(rpkg_dir / "rust-target/doc/r")}}/. dist/reference/
    echo "copied to dist/reference/dvs.html"

# Refresh the scaffold, configure.ac, the agent skills and the git hook
# sections from the pinned minirextendr. configure.ac's AC_INIT takes the
# version from DESCRIPTION.
[doc('Refresh the scaffold, skills and git hooks from the pinned minirextendr.')]
rpkg-upgrade: rpkg-r-guard
    {{rpkg_rscript}} -e 'minirextendr::upgrade_miniextendr_package("{{rpkg_dir}}", configure_ac = TRUE)'

# Run the four install-* integration tests sequentially.
# install-tarball requires cargo-revendor; it is skipped if not installed.
[doc('Run the install-* integration tests.')]
rpkg-test-install:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-revendor >/dev/null \
      && bash {{quote(rpkg_dir / "tests/install-tarball")}} \
      || echo "skipping install-tarball: cargo-revendor not installed"
    bash {{quote(rpkg_dir / "tests/install-github")}}
    bash {{quote(rpkg_dir / "tests/install-devtools")}}
    bash {{quote(rpkg_dir / "tests/install-rv-github")}}

# Install cargo-revendor at the miniextendr commit Cargo.toml pins. It is
# what vendors the release tarball.
[doc('Install cargo-revendor at the pinned miniextendr commit.')]
install-revendor:
    cargo install --locked --git https://github.com/A2-ai/miniextendr \
      --rev "$(grep -o 'rev = "[0-9a-f]*"' {{quote(rpkg_manifest)}} | head -1 | cut -d'"' -f2)" \
      cargo-revendor

# ============================================================================
# Combined
# ============================================================================

build-all: build rpkg-build

# Install both the dvs CLI binary and the dvs R package from the current branch
install-all: install-cli rpkg-install

test-all: test rpkg-test

check-all: check rpkg-check

clippy-all: clippy rpkg-clippy

fmt-all: fmt rpkg-fmt

fmt-check:
    cargo fmt -- --check
    cargo fmt --manifest-path={{quote(rpkg_manifest)}} -- --check

# Generate Rust bindings for cli's C progress API (reference only, not compiled)
rpkg-bindgen-cli:
    #!/usr/bin/env bash
    set -euo pipefail
    R_INCLUDE="$(Rscript -e 'cat(R.home("include"))')"
    CLI_INCLUDE="$(Rscript -e 'cat(system.file("include", package = "cli"))')"
    WRAPPER="$(mktemp /tmp/cli_bindgen_XXXXXX).h"
    printf '#include <Rinternals.h>\n#include <cli/progress.h>\n' > "$WRAPPER"
    bindgen \
      --merge-extern-blocks \
      --no-layout-tests \
      --no-doc-comments \
      --wrap-static-fns \
      --wrap-static-fns-path /tmp/cli_static_wrappers.c \
      --allowlist-file '.*/cli/progress\.h' \
      --blocklist-type 'SEXPREC' \
      --blocklist-type 'SEXP' \
      --raw-line 'use miniextendr_api::ffi::SEXP;' \
      "$WRAPPER" \
      -- \
      -I"$R_INCLUDE" \
      -I"$CLI_INCLUDE"
    echo ""
    echo "# Wrapper header: $WRAPPER"
    echo "# C shims written to /tmp/cli_static_wrappers.c"

ci: fmt-check clippy check-std-fs test
    @echo "All CI checks passed!"

# ============================================================================
# UI test outputs (publish to alx project in .alx/config.yaml)
# ============================================================================

# Names correspond to ui/main_<NAME>.sh (and ui/main.sh for "main").
# Each name has matching ui/output/ui-<NAME>.html and alx topic ui-<NAME>.
ui_names := "main status progress parallel log cli_help threads init"

# Run all ui/main*.sh scripts and capture each log into /tmp/ui-<NAME>.log
ui-run:
    #!/usr/bin/env bash
    set -uo pipefail
    bash ui/cleanup.sh >/dev/null 2>&1 || true
    for name in {{ui_names}}; do
        if [[ "$name" == "main" ]]; then script="ui/main.sh"; else script="ui/main_${name}.sh"; fi
        log="/tmp/ui-${name}.log"
        echo "Running ${script} → ${log}"
        bash "$script" > "$log" 2>&1 || echo "  WARN: ${script} exited nonzero (log captured anyway)"
    done

# Wrap each /tmp/ui-<NAME>.log into ui/output/ui-<NAME>.html
ui-render:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p ui/output
    for name in {{ui_names}}; do
        log="/tmp/ui-${name}.log"
        out="ui/output/ui-${name}.html"
        if [[ ! -f "$log" ]]; then echo "skipping ${name} (no ${log})"; continue; fi
        {
            printf '<!DOCTYPE html>\n<html><head><meta charset="utf-8"><title>ui-%s</title>\n' "$name"
            printf '<style>body{font-family:ui-monospace,Menlo,Consolas,monospace;background:#1e1e1e;color:#e6e6e6;padding:1rem;margin:0}pre{white-space:pre-wrap;word-wrap:break-word;font-size:20px;line-height:1.45}h1{color:#9ecbff;font-family:system-ui,sans-serif;font-size:1.5rem;margin:0 0 1rem}</style></head><body>\n'
            printf '<h1>ui-%s</h1>\n<pre>' "$name"
            sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g' "$log"
            printf '</pre></body></html>\n'
        } > "$out"
        echo "wrote ${out}"
    done

# Publish ui/output/ui-<NAME>.html to alx with each script as source attachment.
# No args → publish all ui_names. Args → publish only those names.
# Examples: `just ui-publish-only`            (all)
#           `just ui-publish-only threads`    (one)
#           `just ui-publish-only threads parallel`  (subset)
ui-publish-only *names:
    #!/usr/bin/env bash
    set -uo pipefail
    targets="{{names}}"
    if [[ -z "$targets" ]]; then targets="{{ui_names}}"; fi
    for name in $targets; do
        if [[ "$name" == "main" ]]; then script="ui/main.sh"; else script="ui/main_${name}.sh"; fi
        out="ui/output/ui-${name}.html"
        if [[ ! -f "$out" ]]; then echo "skipping ${name} (no ${out})"; continue; fi
        echo "--- alx publish ui-${name} ---"
        sources=(-S "$script")
        if grep -q 'helpers\.sh' "$script"; then sources+=(-S ui/helpers.sh); fi
        alx publish "$out" "${sources[@]}" -t "ui-${name}" \
            --overwrite --skip-warnings --no-prompt 2>&1 | tail -8
    done

# Full pipeline: run scripts → render HTML → publish to alx
ui-publish: ui-run ui-render ui-publish-only
