# https://just.systems

rpkg_dir := "dvs-rpkg"
rpkg_manifest := rpkg_dir / "src/rust/Cargo.toml"

default:
    @just --list

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
    cargo install --force --locked --path=dvs-cli {{args}}

# ============================================================================
# R package ({dvs})
# ============================================================================

# FIXME: decide if we want to add NOT_CRAN (dev mode) here

rpkg-configure:
    cd {{quote(rpkg_dir)}} && NOT_CRAN=true ./configure

# Re-vendor dependencies from git (miniextendr + dvs)
rpkg-vendor:
    cd {{quote(rpkg_dir)}} && NOT_CRAN=true FORCE_VENDOR=true ./configure

rpkg-build *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} {{args}}

rpkg-build-release *args:
    cargo build --manifest-path={{quote(rpkg_manifest)}} --release {{args}}

rpkg-test *args:
    cargo test --manifest-path={{quote(rpkg_manifest)}} {{args}}

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
    NOT_CRAN=true Rscript -e 'install.packages("{{rpkg_dir}}", repos = NULL, type = "source")'
alias install-rpkg := rpkg-install

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

ci: fmt-check clippy check-std-fs test
    @echo "All CI checks passed!"
