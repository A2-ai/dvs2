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

rpkg-test *args:
    cargo test --manifest-path={{quote(rpkg_manifest)}} {{args}}
    bash {{quote(rpkg_dir / "tests/rv")}}

rpkg-clippy *args:
    cargo clippy --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-check *args:
    cargo check --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-fmt *args:
    cargo fmt --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-update *args:
    cargo update --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-document:
    Rscript -e 'devtools::document("{{rpkg_dir}}")'

rpkg-install:
    Rscript -e 'install.packages("{{rpkg_dir}}", repos = NULL, type = "source")'
alias install-rpkg := rpkg-install

# Run the four install-* integration tests sequentially.
# install-tarball requires cargo-revendor; it is skipped if not installed.
rpkg-test-install:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v cargo-revendor >/dev/null \
      && bash {{quote(rpkg_dir / "tests/install-tarball")}} \
      || echo "skipping install-tarball: cargo-revendor not installed"
    bash {{quote(rpkg_dir / "tests/install-github")}}
    bash {{quote(rpkg_dir / "tests/install-devtools")}}
    bash {{quote(rpkg_dir / "tests/install-rv-github")}}

# Install cargo-revendor (required for vendoring)
install-revendor:
    cargo install --git https://github.com/A2-ai/miniextendr cargo-revendor

# Vendor all dependencies for offline/CRAN builds
# Vendors deps, strips fat, freezes manifest, compresses into inst/vendor.tar.xz
#
# --force bypasses cargo-revendor's cache: source-only edits to workspace
# crates leave Cargo.lock untouched, so without --force the cache check
# skips re-vendoring and ships a stale tarball.
vendor:
    cargo revendor \
      --manifest-path {{rpkg_dir}}/src/rust/Cargo.toml \
      --source-root . \
      --output {{rpkg_dir}}/vendor \
      --strip-all \
      --freeze \
      --compress {{rpkg_dir}}/inst/vendor.tar.xz \
      --blank-md \
      --source-marker \
      --force \
      -v

# Vendor monorepo library crates for CRAN (use_vendor_lib)
# Call this for each monorepo crate that your R package depends on.
# Example: just vendor-lib dvs ../../../dvs
vendor-lib crate dev_path:
    Rscript -e 'args <- commandArgs(trailingOnly = TRUE); minirextendr::use_vendor_lib(args[[1]], dev_path = args[[2]], path = args[[3]])' "{{crate}}" "{{dev_path}}" "{{rpkg_dir}}"
    cd {{rpkg_dir}} && autoconf -vif

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
