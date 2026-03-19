# https://just.systems

export NOT_CRAN := "true"

rpkg_dir := "dvs-rpkg"
rpkg_manifest := rpkg_dir / "src/rust/Cargo.toml"

default:
    @just --list

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
