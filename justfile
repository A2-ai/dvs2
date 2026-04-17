# https://just.systems

export NOT_CRAN := "true"

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

# ============================================================================
# R package (dvsR)
# ============================================================================

rpkg-configure:
    cd {{quote(rpkg_dir)}} && \
    if command -v autoconf >/dev/null 2>&1; then autoconf; else echo "autoconf not found; using existing configure"; fi && \
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

# Install cargo-revendor (required for vendoring)
install-revendor:
    cargo install --git https://github.com/CGMossa/miniextendr cargo-revendor

# Vendor all dependencies for offline/CRAN builds
# Vendors deps, strips fat, freezes manifest, compresses into inst/vendor.tar.xz
#
# --force bypasses cargo-revendor's cache: source-only edits to workspace
# crates leave Cargo.lock untouched, so without --force the cache check
# skips re-vendoring and ships a stale tarball.
vendor:
    cargo revendor \
      --manifest-path {{rpkg_dir}}/src/rust/Cargo.toml \
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
