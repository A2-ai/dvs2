AC_INIT([dvs], [0.0.0.9000])

dnl Use tools/ directory for auxiliary build files (config.guess, config.sub)
dnl This avoids the CRAN NOTE about non-standard top-level files
AC_CONFIG_AUX_DIR([tools])

dnl Canonicalize build/host triples for cross-compilation support
AC_CANONICAL_HOST

dnl Verify we're running from the R package directory
if test ! -f "DESCRIPTION" || test ! -f "NAMESPACE"; then
  AC_MSG_ERROR([configure must be run from the dvs-rpkg package directory (where DESCRIPTION exists).])
fi

dnl Ensure R_HOME is set and valid
if test -z "$R_HOME"; then
  R_HOME="$(R RHOME 2>/dev/null)"
  if test $? -ne 0 || test -z "$R_HOME"; then
    AC_MSG_ERROR([R not found. Please ensure R is installed and on PATH, or set R_HOME.])
  fi
fi
if test ! -d "$R_HOME"; then
  AC_MSG_ERROR([R_HOME directory does not exist: $R_HOME])
fi

dnl NOT_CRAN handling:
dnl - CRAN builds: NOT_CRAN is unset or empty -> default to false (offline mode)
dnl - Dev builds: set NOT_CRAN=true in environment to enable network
: ${NOT_CRAN:=false}
case "$NOT_CRAN" in
  true|TRUE|1) NOT_CRAN=true ;;
  *)           NOT_CRAN=false ;;
esac
export NOT_CRAN

dnl Set cargo offline flag based on NOT_CRAN
if test "$NOT_CRAN" = "true"; then
  CARGO_OFFLINE_FLAG=""
  CARGO_LOCKED_FLAG="--locked"
else
  CARGO_OFFLINE_FLAG="--offline"
  CARGO_LOCKED_FLAG=""
fi
AC_SUBST([CARGO_OFFLINE_FLAG])
AC_SUBST([CARGO_LOCKED_FLAG])
AC_SUBST([NOT_CRAN])

dnl Set cargo features flag
AC_ARG_VAR([DVS_FEATURES], [Comma-separated cargo features (e.g., "nonapi")])
: ${DVS_FEATURES:=""}

if test -n "$DVS_FEATURES"; then
  CARGO_FEATURES_FLAG="--features=$DVS_FEATURES"
  CARGO_FEATURE_CPPFLAGS=""
  # Auto-generate C preprocessor flags for each feature
  IFS=','
  for _feature in $DVS_FEATURES; do
    # Convert feature name to uppercase and replace hyphens with underscores
    _feature_upper=$(echo "$_feature" | tr 'a-z-' 'A-Z_')
    CARGO_FEATURE_CPPFLAGS="$CARGO_FEATURE_CPPFLAGS -DCARGO_FEATURE_$_feature_upper"
  done
  unset IFS
else
  CARGO_FEATURES_FLAG=""
  CARGO_FEATURE_CPPFLAGS=""
fi
AC_SUBST([CARGO_FEATURES_FLAG])
AC_SUBST([CARGO_FEATURE_CPPFLAGS])

dnl ---- canonical paths ----
abs_top_srcdir="$(cd "$srcdir" && pwd)"
AC_SUBST([ABS_TOP_SRCDIR], ["$abs_top_srcdir"])
abs_rpkg_src="$abs_top_srcdir/src"
AC_SUBST([ABS_RPKG_SRCDIR], ["$abs_rpkg_src"])
RUST_SRC_DIR="$abs_rpkg_src/rust"

dnl ---- tool discovery ----
AC_PATH_TOOL([CARGO],[cargo],[no])
AC_PATH_TOOL([RUSTC],[rustc],[no])
AC_PATH_PROG([SED],[sed],[no])
AC_PATH_PROG([CYGPATH],[cygpath],[no])
AC_PATH_PROG([RSYNC],[rsync],[no])
AS_IF([test "x$CARGO" = "xno"], [AC_MSG_ERROR([`cargo` not found])])
AS_IF([test "x$RUSTC" = "xno"], [AC_MSG_ERROR([`rustc` not found])])
AS_IF([test "x$SED" = "xno"], [AC_MSG_ERROR([`sed` not found])])
AC_SUBST([CARGO])
AC_SUBST([RUSTC])
AC_SUBST([SED])
AC_SUBST([CYGPATH])
AC_SUBST([RSYNC])

dnl Use native paths for Cargo on Windows/MSYS to avoid path mangling
abs_rpkg_src_cargo="$abs_rpkg_src"
case "$host_os" in
  *msys*|*cygwin*|*mingw*)
    if test "x$CYGPATH" != "xno"; then
      abs_rpkg_src_cargo="$($CYGPATH -m "$abs_rpkg_src")"
    fi
    ;;
esac
AC_SUBST([ABS_RPKG_SRC_CARGO], ["$abs_rpkg_src_cargo"])

dnl Report rustc version
RUSTC_VERSION="$($RUSTC --version)"
AC_MSG_NOTICE([using $RUSTC_VERSION])

dnl Check minimum rustc version (1.85+)
RUSTC_VERSION_NUM=`echo "$RUSTC_VERSION" | "$SED" 's/rustc \(@<:@0-9@:>@*\.@<:@0-9@:>@*\.@<:@0-9@:>@*\).*/\1/'`
RUSTC_MAJOR=`echo "$RUSTC_VERSION_NUM" | cut -d. -f1`
RUSTC_MINOR=`echo "$RUSTC_VERSION_NUM" | cut -d. -f2`
MIN_RUSTC_MAJOR=1
MIN_RUSTC_MINOR=85
if test "$RUSTC_MAJOR" -lt "$MIN_RUSTC_MAJOR" || \
   (test "$RUSTC_MAJOR" -eq "$MIN_RUSTC_MAJOR" && test "$RUSTC_MINOR" -lt "$MIN_RUSTC_MINOR"); then
  AC_MSG_ERROR([rustc $MIN_RUSTC_MAJOR.$MIN_RUSTC_MINOR+ required, found $RUSTC_VERSION_NUM])
fi

dnl Allow explicit toolchain
AC_ARG_VAR([RUST_TOOLCHAIN],[Rust toolchain selector like '+stable' or '+nightly'])
: ${RUST_TOOLCHAIN=""}
AC_SUBST([RUST_TOOLCHAIN])

dnl Convenience: cargo command that includes optional toolchain selector
CARGO_CMD="$CARGO $RUST_TOOLCHAIN"
AC_SUBST([CARGO_CMD])

dnl ---- Cargo configuration ----
AC_ARG_VAR([CARGO_TARGET_DIR], [Cargo target directory (default: src/rust/target)])
: ${CARGO_TARGET_DIR="$abs_rpkg_src/rust/target"}
AC_SUBST([CARGO_TARGET_DIR])

CARGO_TARGET_DIR_CARGO="$abs_rpkg_src_cargo/rust/target"
AC_SUBST([CARGO_TARGET_DIR_CARGO])

AC_ARG_VAR([CARGO_PROFILE], [Cargo build profile (debug or release)])
: ${CARGO_PROFILE="release"}
AC_SUBST([CARGO_PROFILE])

dnl Static library name - extract from Cargo.toml.in and convert hyphens to underscores
CARGO_STATICLIB_NAME="$($SED -n 's/^name = "\(.*\)"/\1/p' "$RUST_SRC_DIR/Cargo.toml.in" | head -1 | tr '-' '_')"
if test -z "$CARGO_STATICLIB_NAME"; then
  AC_MSG_ERROR([Could not extract crate name from $RUST_SRC_DIR/Cargo.toml.in])
fi
AC_SUBST([CARGO_STATICLIB_NAME])

AC_ARG_VAR([ENV_RUSTFLAGS], [Rust compiler flags passed as RUSTFLAGS])
: ${ENV_RUSTFLAGS="$RUSTFLAGS"}
AC_SUBST([ENV_RUSTFLAGS])

AC_ARG_VAR([CARGO_BUILD_TARGET], [Cargo build target triple (e.g. aarch64-apple-darwin)])
if test -z "$CARGO_BUILD_TARGET"; then
  if test -n "$host" && test -n "$build" && test "$host" != "$build"; then
    CARGO_BUILD_TARGET="$host"
    AC_MSG_NOTICE([CARGO_BUILD_TARGET not set; using host triple $host])
  fi
fi
AC_SUBST([CARGO_BUILD_TARGET])

dnl Compute Cargo libdir path
if test -n "$CARGO_BUILD_TARGET"; then
  CARGO_LIBDIR="$CARGO_TARGET_DIR/$CARGO_BUILD_TARGET/$CARGO_PROFILE"
else
  CARGO_LIBDIR="$CARGO_TARGET_DIR/$CARGO_PROFILE"
fi
AC_SUBST([CARGO_LIBDIR])

dnl ---- user feedback ----
AC_MSG_NOTICE([R_HOME                = $R_HOME])
AC_MSG_NOTICE([NOT_CRAN              = $NOT_CRAN])
AC_MSG_NOTICE([CARGO_TARGET_DIR      = $CARGO_TARGET_DIR])
AC_MSG_NOTICE([CARGO_PROFILE         = $CARGO_PROFILE])
AC_MSG_NOTICE([CARGO_STATICLIB_NAME  = $CARGO_STATICLIB_NAME])
AC_MSG_NOTICE([CARGO_LIBDIR          = $CARGO_LIBDIR])
AS_IF([test -n "$RUST_TOOLCHAIN"],
      [AC_MSG_NOTICE([RUST_TOOLCHAIN        = $RUST_TOOLCHAIN])])
AS_IF([test -n "$CARGO_BUILD_TARGET"],
      [AC_MSG_NOTICE([CARGO_BUILD_TARGET    = $CARGO_BUILD_TARGET])])
AS_IF([test -n "$ENV_RUSTFLAGS"],
      [AC_MSG_NOTICE([ENV_RUSTFLAGS         = $ENV_RUSTFLAGS])])

dnl ---- package name variants ----
dnl PACKAGE_NAME from AC_INIT preserves case (e.g., dvs-rpkg)
dnl PACKAGE_TARNAME is always lowercase (e.g., dvsr)
dnl Use PACKAGE_NAME for R_init function, PACKAGE_TARNAME_RS for Rust module

dnl Rust-safe name (lowercase, underscore for hyphens)
pkg_rs="$(echo "$PACKAGE_TARNAME" | $SED 's/-/_/g')"
AC_SUBST([PACKAGE_TARNAME_RS], [$pkg_rs])
pkg_rs_upper=`printf '%s' "$pkg_rs" | tr 'a-z' 'A-Z'`
AC_SUBST([PACKAGE_TARNAME_RS_UPPERCASE], [$pkg_rs_upper])

dnl ---- optional cargo deps ----
OPTIONAL_CARGO_DEPS=
for f in \
  "$RUST_SRC_DIR/build.rs" \
  "$RUST_SRC_DIR/.cargo/config.toml" \
  "$RUST_SRC_DIR/rust-toolchain.toml" \
  "$RUST_SRC_DIR/rust-toolchain"
do
  test -f "$f" && OPTIONAL_CARGO_DEPS="$OPTIONAL_CARGO_DEPS $f"
done
AC_SUBST([OPTIONAL_CARGO_DEPS])

dnl ---- vendoring ----
VENDOR_OUT="$abs_rpkg_src/vendor"
AC_SUBST([VENDOR_OUT])
VENDOR_OUT_CARGO="$abs_rpkg_src_cargo/vendor"
AC_SUBST([VENDOR_OUT_CARGO])

dnl FORCE_VENDOR: Set to true to force re-vendoring even if stamp file exists
: ${FORCE_VENDOR:=false}
case "$FORCE_VENDOR" in
  true|TRUE|1) FORCE_VENDOR=true ;;
  *)           FORCE_VENDOR=false ;;
esac
AC_SUBST([FORCE_VENDOR])

dnl MINIEXTENDR_SOURCE_DIR: Optional path to upstream miniextendr repo for staleness detection
dnl If set, vendoring will re-run when miniextendr sources are newer than vendor stamp
AC_ARG_VAR([MINIEXTENDR_SOURCE_DIR], [Path to upstream miniextendr repo (optional, for staleness detection)])
: ${MINIEXTENDR_SOURCE_DIR=""}
AC_SUBST([MINIEXTENDR_SOURCE_DIR])

dnl Source replacement config for .cargo/config.toml
CARGO_SOURCE_REPLACE="$(cat <<EOF
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "$VENDOR_OUT_CARGO"
EOF
)"
AC_SUBST([CARGO_SOURCE_REPLACE])


dnl Generate Cargo.toml and document.rs from templates
AC_CONFIG_FILES([src/rust/Cargo.toml:src/rust/Cargo.toml.in])
AC_CONFIG_FILES([src/rust/document.rs:src/rust/document.rs.in])

dnl Generate Makevars, entrypoint.c, and mx_abi.c from templates
AC_CONFIG_FILES([src/Makevars:src/Makevars.in])
AC_CONFIG_FILES([src/Makevars.win:src/Makevars.win.in])
AC_CONFIG_FILES([src/entrypoint.c:src/entrypoint.c.in])
AC_CONFIG_FILES([src/mx_abi.c:src/mx_abi.c.in])

dnl ---- output files ----
dnl Create .cargo directory
AS_MKDIR_P([src/rust/.cargo])

dnl Generate config files from templates
AC_CONFIG_FILES([src/rust/.cargo/config.toml:src/rust/cargo-config.toml.in])

dnl ---- AC_CONFIG_COMMANDS run after AC_CONFIG_FILES generates files ----

dnl 1) Sync cargo config to monorepo root (for vendored builds in monorepo context)
AC_CONFIG_COMMANDS([sync-root-cargo-config],
[
  dnl Sync .cargo/config.toml to monorepo root if in dev mode and monorepo detected
  ROOT_WORKSPACE="$abs_rpkg_dir/.."
  ROOT_CFG="$ROOT_WORKSPACE/.cargo/config.toml"
  RPKG_CFG="src/rust/.cargo/config.toml"

  if test "$NOT_CRAN" = "true" && test -d "$ROOT_WORKSPACE" && test -f "$RPKG_CFG"; then
    dnl Check if this looks like a monorepo (has Cargo.toml at root)
    if test -f "$ROOT_WORKSPACE/Cargo.toml"; then
      mkdir -p "$ROOT_WORKSPACE/.cargo"

      if test -f "$ROOT_CFG"; then
        dnl Warn about overwrite
        echo "======================================================================" >&2
        echo "configure: WARNING: Overwriting $ROOT_CFG with rpkg cargo config" >&2
        echo "configure:          This enables vendored sources for the entire workspace" >&2
        echo "configure:" >&2
        echo "configure:          If you have custom settings, back them up first." >&2
        echo "======================================================================" >&2
        sleep 2
      fi

      cp "$RPKG_CFG" "$ROOT_CFG"
      echo "configure: synced cargo config to monorepo root"
    fi
  fi
],
[NOT_CRAN="$NOT_CRAN" abs_rpkg_dir="$abs_top_srcdir"])

dnl 2) Check cargo lockfile compatibility and regenerate if needed
AC_CONFIG_COMMANDS([cargo-lockfile-compat],
[
  LOCKFILE_PATH="src/rust/Cargo.lock"
  if test -f "$LOCKFILE_PATH"; then
    LOCKFILE_VERSION="$("$SED" -n 's/^version = //p' "$LOCKFILE_PATH" | head -n1)"
  else
    LOCKFILE_VERSION=""
  fi

  CARGO_VERSION="$($CARGO_CMD --version | "$SED" 's/cargo \(@<:@0-9@:>@*\.@<:@0-9@:>@*\.@<:@0-9@:>@*\).*/\1/')"
  CARGO_MAJOR="$(echo "$CARGO_VERSION" | cut -d. -f1)"
  CARGO_MINOR="$(echo "$CARGO_VERSION" | cut -d. -f2)"

  if test -n "$LOCKFILE_VERSION" && test "$LOCKFILE_VERSION" -ge 4; then
    if test "$CARGO_MAJOR" -lt 1 || \
       (test "$CARGO_MAJOR" -eq 1 && test "$CARGO_MINOR" -lt 78); then
      echo "configure: WARNING: cargo $CARGO_VERSION cannot read Cargo.lock v$LOCKFILE_VERSION; regenerating" >&2
      rm -f "$LOCKFILE_PATH"
      $CARGO_CMD generate-lockfile --manifest-path src/rust/Cargo.toml $CARGO_OFFLINE_FLAG
      if test $? -ne 0; then
        echo "configure: error: cargo generate-lockfile failed" >&2
        exit 1
      fi
    fi
  fi
],
[CARGO_CMD="$CARGO_CMD" CARGO_OFFLINE_FLAG="$CARGO_OFFLINE_FLAG" SED="$SED"])

dnl 3) Run cargo vendor (dev mode only, after Cargo.toml is generated)
AC_CONFIG_COMMANDS([cargo-vendor],
[
  dnl Check if we should use pre-vendored sources:
  dnl 1. CRAN/offline mode (NOT_CRAN != true), OR
  dnl 2. Vendor directory is empty/missing BUT vendor.tar.xz exists
  dnl    (This handles R CMD check running on an extracted tarball with NOT_CRAN=true)
  _use_prevendored=0
  if test "$NOT_CRAN" != "true"; then
    _use_prevendored=1
  elif test ! -d "$VENDOR_OUT" || test -z "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
    if test -f "$abs_rpkg_dir/inst/vendor.tar.xz"; then
      echo "configure: vendor directory empty but vendor.tar.xz exists"
      echo "configure: falling back to pre-vendored sources (R CMD check context)"
      _use_prevendored=1
    fi
  fi

  if test $_use_prevendored -eq 1; then
    dnl CRAN/offline mode - check for vendor directory or tarball
    if test ! -d "$VENDOR_OUT" || test -z "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
      if test -f "$abs_rpkg_dir/inst/vendor.tar.xz"; then
        echo "configure: CRAN build - unpacking inst/vendor.tar.xz"
        tar -xJf "$abs_rpkg_dir/inst/vendor.tar.xz" -C "$abs_rpkg_src"
        if test $? -ne 0; then
          echo "configure: error: failed to unpack vendor.tar.xz" >&2
          exit 1
        fi
        dnl Strip checksums from Cargo.lock (vendored crates have empty checksums)
        if test -f "$abs_rpkg_src/rust/Cargo.lock"; then
          $SED -i.bak '/^checksum = /d' "$abs_rpkg_src/rust/Cargo.lock" && rm -f "$abs_rpkg_src/rust/Cargo.lock.bak"
          echo "configure: stripped checksums from Cargo.lock for vendor compatibility"
        fi
        dnl Mark that we unpacked from tarball (to skip compress-vendor later)
        touch "$abs_rpkg_src/.unpacked-from-tarball"
        dnl Remove --locked from Makevars since vendored sources have cleared checksums
        if test -f "src/Makevars"; then
          $SED -i.bak 's/--locked//g' src/Makevars && rm -f src/Makevars.bak
          echo "configure: removed --locked from Makevars for tarball compatibility"
        fi
        if test -f "src/Makevars.win"; then
          $SED -i.bak 's/--locked//g' src/Makevars.win && rm -f src/Makevars.win.bak
          echo "configure: removed --locked from Makevars.win for tarball compatibility"
        fi
      else
        echo "configure: error: CRAN/offline build but vendor directory is missing: $VENDOR_OUT" >&2
        echo "configure:        Run configure with NOT_CRAN=true first, or use a release tarball." >&2
        exit 1
      fi
    fi
    echo "configure: CRAN build - using pre-vendored sources"
  else
    dnl Dev mode - run cargo vendor
    MANIFEST="src/rust/Cargo.toml"
    LOCKFILE="src/rust/Cargo.lock"
    STAMP="$VENDOR_OUT/.vendor.lock.cksum"

    _need_vendor=0
    if test "$FORCE_VENDOR" = "true"; then
      echo "configure: FORCE_VENDOR=true, will re-vendor"
      _need_vendor=1
    elif test ! -f "$STAMP"; then
      echo "configure: vendor stamp missing, will vendor"
      _need_vendor=1
    elif test ! -d "$VENDOR_OUT" || test -z "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
      echo "configure: vendor directory missing or empty, will vendor"
      _need_vendor=1
    fi

    dnl Check if miniextendr sources are newer than vendor stamp (optional)
    if test $_need_vendor -eq 0 && test -n "$MINIEXTENDR_SOURCE_DIR" && test -d "$MINIEXTENDR_SOURCE_DIR"; then
      dnl Find the newest .rs file in miniextendr source (try both flat and nested layouts)
      _mx_newest=""
      for _mx_name in miniextendr-api miniextendr-macros miniextendr-lint; do
        _mx_crate="$MINIEXTENDR_SOURCE_DIR/$_mx_name"
        if test ! -d "$_mx_crate"; then
          _mx_crate="$MINIEXTENDR_SOURCE_DIR/crates/$_mx_name"
        fi
        if test -d "$_mx_crate"; then
          _crate_newest=$(find "$_mx_crate" -name "*.rs" -type f -newer "$STAMP" 2>/dev/null | head -1)
          if test -n "$_crate_newest"; then
            _mx_newest="$_crate_newest"
            break
          fi
        fi
      done
      if test -n "$_mx_newest"; then
        echo "configure: miniextendr source newer than vendor stamp: $_mx_newest"
        _need_vendor=1
      fi
    fi

    if test $_need_vendor -eq 1; then
      echo "configure: running cargo vendor"
      mkdir -p "$VENDOR_OUT"

      dnl Copy miniextendr crates from source if MINIEXTENDR_SOURCE_DIR is set
      if test -n "$MINIEXTENDR_SOURCE_DIR" && test -d "$MINIEXTENDR_SOURCE_DIR"; then
        echo "configure: copying miniextendr crates from $MINIEXTENDR_SOURCE_DIR"

        dnl Extract workspace.package values from miniextendr root Cargo.toml
        _mx_root_toml="$MINIEXTENDR_SOURCE_DIR/Cargo.toml"
        if test -f "$_mx_root_toml"; then
          WORKSPACE_VERSION=$(grep -A20 'workspace.package' "$_mx_root_toml" | grep '^version' | head -1 | $SED 's/^version *= *"\(.*\)"/\1/' | $SED "s/^version *= *'\\(.*\\)'/\\1/")
          WORKSPACE_EDITION=$(grep -A20 'workspace.package' "$_mx_root_toml" | grep '^edition' | head -1 | $SED 's/^edition *= *"\(.*\)"/\1/' | $SED "s/^edition *= *'\\(.*\\)'/\\1/")
          WORKSPACE_LICENSE=$(grep -A20 'workspace.package' "$_mx_root_toml" | grep '^license' | head -1 | $SED 's/^license *= *"\(.*\)"/\1/' | $SED "s/^license *= *'\\(.*\\)'/\\1/")
          echo "configure:   extracted workspace values: version=$WORKSPACE_VERSION edition=$WORKSPACE_EDITION license=$WORKSPACE_LICENSE"
        else
          dnl Fallback to defaults if root Cargo.toml not found
          WORKSPACE_VERSION="0.1.0"
          WORKSPACE_EDITION="2024"
          WORKSPACE_LICENSE="MIT"
          echo "configure:   warning: miniextendr root Cargo.toml not found, using defaults" >&2
        fi

        for _mx_crate in miniextendr-api miniextendr-macros miniextendr-lint; do
          dnl Try both layouts: flat (miniextendr-api/) and nested (crates/miniextendr-api/)
          _mx_src="$MINIEXTENDR_SOURCE_DIR/$_mx_crate"
          if test ! -d "$_mx_src"; then
            _mx_src="$MINIEXTENDR_SOURCE_DIR/crates/$_mx_crate"
          fi
          _mx_dst="$VENDOR_OUT/$_mx_crate"
          if test -d "$_mx_src"; then
            rm -rf "$_mx_dst"
            mkdir -p "$_mx_dst"
            dnl Use rsync to copy, excluding tests/benches/examples
            $RSYNC -a --exclude='/tests' --exclude='/benches' --exclude='/examples' "$_mx_src/" "$_mx_dst/"
            dnl Create checksum file for vendored source compatibility
            echo '{"files":{}}' > "$_mx_dst/.cargo-checksum.json"
            echo "configure:   copied $_mx_crate"
          else
            echo "configure: warning: $_mx_crate not found in $MINIEXTENDR_SOURCE_DIR" >&2
          fi
        done

        dnl Patch Cargo.toml files to remove workspace inheritance
        echo "configure: patching miniextendr Cargo.toml files"
        for _mx_crate in miniextendr-api miniextendr-macros miniextendr-lint; do
          _toml="$VENDOR_OUT/$_mx_crate/Cargo.toml"
          if test -f "$_toml"; then
            dnl Replace workspace inheritance with extracted values
            $SED -i.bak \
              -e "s/^version\.workspace = true$/version = \"$WORKSPACE_VERSION\"/" \
              -e "s/^edition\.workspace = true$/edition = \"$WORKSPACE_EDITION\"/" \
              -e "s/^license\.workspace = true$/license = \"$WORKSPACE_LICENSE\"/" \
              -e '/^repository\.workspace = true$/d' \
              -e '/^homepage\.workspace = true$/d' \
              -e '/^keywords\.workspace = true$/d' \
              -e '/^categories\.workspace = true$/d' \
              -e 's/miniextendr-macros = { workspace = true }/miniextendr-macros = { path = "..\/miniextendr-macros" }/' \
              -e 's/miniextendr-engine = { workspace = true }/miniextendr-engine = { path = "..\/miniextendr-engine" }/' \
              -e 's/miniextendr-lint = { workspace = true }/miniextendr-lint = { path = "..\/miniextendr-lint" }/' \
              -e 's/miniextendr-api = { workspace = true }/miniextendr-api = { path = "..\/miniextendr-api" }/' \
              "$_toml"
            dnl Handle features arrays separately (brackets need special handling)
            $SED -i.bak \
              -e 's/proc-macro2 = { workspace = true }/proc-macro2 = { version = "1.0", features = LBRACKET"span-locations"RBRACKET }/' \
              -e 's/quote = { workspace = true }/quote = "1.0"/' \
              -e 's/syn = { workspace = true }/syn = { version = "2.0", features = LBRACKET"full", "extra-traits"RBRACKET }/' \
              "$_toml"
            dnl Replace LBRACKET/RBRACKET placeholders with actual brackets
            dnl Use @<:@ and @:>@ which are autoconf's escaped [ and ]
            $SED -i.bak -e 's/LBRACKET/@<:@/g' -e 's/RBRACKET/@:>@/g' "$_toml"
            rm -f "${_toml}.bak"
          fi
        done
      fi

      dnl Copy dvs from parent directory (it's a path dependency, not on crates.io)
      dnl This must happen BEFORE cargo vendor since dvs is referenced in Cargo.toml
      _dvs_core_src="$abs_rpkg_dir/../dvs"
      _dvs_core_dst="$VENDOR_OUT/dvs"
      if test -d "$_dvs_core_src"; then
        echo "configure: copying dvs from $_dvs_core_src"
        rm -rf "$_dvs_core_dst"
        mkdir -p "$_dvs_core_dst"
        dnl Use rsync to copy, excluding tests/benches/examples
        $RSYNC -a --exclude='/tests' --exclude='/benches' --exclude='/examples' "$_dvs_core_src/" "$_dvs_core_dst/"

        dnl Extract workspace values from monorepo root Cargo.toml
        _workspace_toml="$abs_rpkg_dir/../Cargo.toml"
        if test -f "$_workspace_toml"; then
          DVS_WS_VERSION=$(grep -A20 'workspace.package' "$_workspace_toml" | grep '^version' | head -1 | $SED 's/^version *= *"\(.*\)"/\1/' | $SED "s/^version *= *'\\(.*\\)'/\\1/")
          DVS_WS_EDITION=$(grep -A20 'workspace.package' "$_workspace_toml" | grep '^edition' | head -1 | $SED 's/^edition *= *"\(.*\)"/\1/' | $SED "s/^edition *= *'\\(.*\\)'/\\1/")
          DVS_WS_RUSTVERSION=$(grep -A20 'workspace.package' "$_workspace_toml" | grep '^rust-version' | head -1 | $SED 's/^rust-version *= *"\(.*\)"/\1/' | $SED "s/^rust-version *= *'\\(.*\\)'/\\1/")
          DVS_WS_LICENSE=$(grep -A20 'workspace.package' "$_workspace_toml" | grep '^license' | head -1 | $SED 's/^license *= *"\(.*\)"/\1/' | $SED "s/^license *= *'\\(.*\\)'/\\1/")
          echo "configure:   extracted dvs workspace values: version=$DVS_WS_VERSION edition=$DVS_WS_EDITION rust-version=$DVS_WS_RUSTVERSION"
        else
          dnl Fallback to defaults if workspace Cargo.toml not found
          DVS_WS_VERSION="0.0.0-9000"
          DVS_WS_EDITION="2021"
          DVS_WS_RUSTVERSION="1.86"
          DVS_WS_LICENSE="MIT"
          echo "configure:   warning: workspace Cargo.toml not found, using defaults" >&2
        fi

        dnl Patch dvs Cargo.toml to remove workspace inheritance
        _dvs_toml="$_dvs_core_dst/Cargo.toml"
        if test -f "$_dvs_toml"; then
          echo "configure: patching dvs Cargo.toml (removing workspace inheritance)"
          $SED -i.bak \
            -e "s/^version\.workspace = true$/version = \"$DVS_WS_VERSION\"/" \
            -e "s/^edition\.workspace = true$/edition = \"$DVS_WS_EDITION\"/" \
            -e "s/^rust-version\.workspace = true$/rust-version = \"$DVS_WS_RUSTVERSION\"/" \
            -e "s/^license\.workspace = true$/license = \"$DVS_WS_LICENSE\"/" \
            -e "s/^authors\.workspace = true$/authors = @<:@@:>@/" \
            -e '/^description\.workspace = true$/d' \
            -e '/^publish\.workspace = true$/d' \
            -e "s/^publish = true$/publish = false/" \
            "$_dvs_toml"

          dnl Patch workspace dependency references with hardcoded versions
          dnl Note: Versions are taken from workspace Cargo.toml at time of writing
          $SED -i.bak \
            -e 's/serde = { workspace = true, optional = true }/serde = { version = "1", features = @<:@"derive"@:>@, optional = true }/' \
            -e 's/serde_yaml = { workspace = true, optional = true }/serde_yaml = { version = "0.9", optional = true }/' \
            -e 's/serde_json = { workspace = true, optional = true }/serde_json = { version = "1", optional = true }/' \
            -e 's/toml = { workspace = true, optional = true }/toml = { version = "0.9", optional = true }/' \
            -e 's/memmap2 = { workspace = true, optional = true }/memmap2 = { version = "0.9", optional = true }/' \
            -e 's/blake3 = { workspace = true, optional = true }/blake3 = { version = "1.8", features = @<:@"rayon"@:>@, optional = true }/' \
            -e 's/xxhash-rust = { workspace = true, optional = true }/xxhash-rust = { version = "0.8", features = @<:@"xxh3"@:>@, optional = true }/' \
            -e 's/sha2 = { workspace = true, optional = true }/sha2 = { version = "0.10", optional = true }/' \
            -e 's/fs-err\.workspace = true/fs-err = "3"/' \
            -e 's/walkdir = { workspace = true, optional = true }/walkdir = { version = "2.5", optional = true }/' \
            -e 's/glob\.workspace = true/glob = "0.3"/' \
            -e 's/jiff\.workspace = true/jiff = { version = "0.2", features = @<:@"serde"@:>@ }/' \
            "$_dvs_toml"

          rm -f "${_dvs_toml}.bak"
        fi
        echo '{"files":{}}' > "$_dvs_core_dst/.cargo-checksum.json"
        echo "configure:   copied and patched dvs"
      else
        echo "configure: warning: dvs not found at $_dvs_core_src" >&2
      fi

      dnl Temporarily move .cargo/config.toml to avoid circular vendored-sources reference
      _cargo_cfg="src/rust/.cargo/config.toml"
      _cargo_cfg_bak="src/rust/.cargo/config.toml.bak"
      if test -f "$_cargo_cfg"; then
        mv "$_cargo_cfg" "$_cargo_cfg_bak"
      fi

      dnl Use --locked if Cargo.lock exists to respect existing dependency versions
      _vendor_locked_flag=""
      if test -f "$LOCKFILE" && test -n "$CARGO_LOCKED_FLAG"; then
        _vendor_locked_flag="$CARGO_LOCKED_FLAG"
      fi
      $CARGO_CMD vendor $_vendor_locked_flag \
        --manifest-path "$MANIFEST" \
        "$VENDOR_OUT" > /dev/null
      _vendor_status=$?

      dnl Restore cargo config
      if test -f "$_cargo_cfg_bak"; then
        mv "$_cargo_cfg_bak" "$_cargo_cfg"
      fi

      if test $_vendor_status -ne 0; then
        echo "configure: error: cargo vendor failed" >&2
        exit 1
      fi
    fi

    dnl Generate Cargo.lock if missing
    if test ! -f "$LOCKFILE"; then
      echo "configure: generating Cargo.lock"
      $CARGO_CMD generate-lockfile --manifest-path "$MANIFEST"
      if test $? -ne 0; then
        echo "configure: error: cargo generate-lockfile failed" >&2
        exit 1
      fi
    fi

    dnl Update vendor stamp
    _new="$(cksum < "$LOCKFILE" | cut -d' ' -f1,2 | tr ' ' ':')"
    echo "$_new" > "$STAMP"

    dnl Keep vendor checksums for dev builds - they match Cargo.lock and enable --locked
    dnl Checksums are cleared only in the tarball compression step (for CRAN compatibility)

    echo "configure: vendor complete"
  fi
],
[CARGO_CMD="$CARGO_CMD" NOT_CRAN="$NOT_CRAN" VENDOR_OUT="$VENDOR_OUT" abs_rpkg_src="$abs_rpkg_src" abs_rpkg_dir="$abs_top_srcdir" FORCE_VENDOR="$FORCE_VENDOR" MINIEXTENDR_SOURCE_DIR="$MINIEXTENDR_SOURCE_DIR" CARGO_LOCKED_FLAG="$CARGO_LOCKED_FLAG"])

dnl 4) Compress vendor directory for CRAN tarballs
AC_CONFIG_COMMANDS([compress-vendor],
[
  dnl Skip if we unpacked from tarball (R CMD check context with NOT_CRAN=true)
  if test -f "$abs_rpkg_src/.unpacked-from-tarball"; then
    rm -f "$abs_rpkg_src/.unpacked-from-tarball"
    echo "configure: skipping vendor compression (using unpacked tarball)"
  elif test "$NOT_CRAN" = "true" && test -d "$VENDOR_OUT" && test -n "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
    VENDOR_TARBALL="$abs_rpkg_dir/inst/vendor.tar.xz"
    VENDOR_STAMP="$abs_rpkg_dir/inst/.vendor.tar.xz.cksum"

    dnl Calculate fingerprint: Cargo.lock + miniextendr source checksums + dvs checksum
    _lock_ck=""
    if test -f "src/rust/Cargo.lock"; then
      _lock_ck="$(cksum < "src/rust/Cargo.lock" | cut -d' ' -f1,2 | tr ' ' ':')"
    fi

    dnl Add miniextendr crate checksums if source available
    _mx_ck=""
    if test -n "$MINIEXTENDR_SOURCE_DIR" && test -d "$MINIEXTENDR_SOURCE_DIR"; then
      for _mx_crate in miniextendr-api miniextendr-macros miniextendr-lint; do
        _mx_src="$MINIEXTENDR_SOURCE_DIR/$_mx_crate"
        if test ! -d "$_mx_src"; then
          _mx_src="$MINIEXTENDR_SOURCE_DIR/crates/$_mx_crate"
        fi
        if test -d "$_mx_src"; then
          _crate_ck=$(find "$_mx_src" -name "*.rs" -type f -exec cksum {} \; 2>/dev/null | cksum | cut -d' ' -f1)
          _mx_ck="${_mx_ck}:${_crate_ck}"
        fi
      done
    fi

    dnl Add dvs checksum if source available
    _dvs_core_ck=""
    _dvs_core_src="$abs_rpkg_dir/../dvs"
    if test -d "$_dvs_core_src"; then
      _dvs_core_ck=$(find "$_dvs_core_src" \( -name "*.rs" -o -name "Cargo.toml" \) -type f -exec cksum {} \; 2>/dev/null | cksum | cut -d' ' -f1)
    fi

    _new_fingerprint="lock:$_lock_ck:mx:$_mx_ck:dvs:$_dvs_core_ck"
    _old_fingerprint="$(cat "$VENDOR_STAMP" 2>/dev/null || true)"

    _need_compress=0
    if test ! -f "$VENDOR_TARBALL"; then
      _need_compress=1
      echo "configure: vendor.tar.xz does not exist"
    elif test "x$_new_fingerprint" != "x$_old_fingerprint"; then
      _need_compress=1
      echo "configure: vendor contents changed"
    fi

    if test $_need_compress -eq 1; then
      echo "configure: compressing vendor directory to inst/vendor.tar.xz"
      mkdir -p "$abs_rpkg_dir/inst"

      dnl Create temp staging area
      _staging_tmp="$abs_rpkg_src/.vendor-tarball-staging"
      rm -rf "$_staging_tmp"
      mkdir -p "$_staging_tmp"
      cp -R "$VENDOR_OUT" "$_staging_tmp/vendor"

      dnl Clear checksums in staging copy for tarball compatibility
      dnl Note: When unpacking tarball, we also strip checksums from Cargo.lock to match
      for _crate_dir in "$_staging_tmp/vendor"/*/; do
        test -d "$_crate_dir" && echo '{"files":{}}' > "${_crate_dir}.cargo-checksum.json"
      done

      dnl Remove unnecessary directories to reduce size
      dnl Note: Do NOT remove 'examples' - some crates use include_str! on example files
      find "$_staging_tmp/vendor" -type d -name 'tests' -exec rm -rf {} + 2>/dev/null || true
      find "$_staging_tmp/vendor" -type d -name 'benches' -exec rm -rf {} + 2>/dev/null || true
      find "$_staging_tmp/vendor" -type d -name '.github' -exec rm -rf {} + 2>/dev/null || true
      dnl Truncate .md files (keep file but remove content)
      find "$_staging_tmp/vendor" -name '*.md' -type f -exec cp /dev/null {} \;

      dnl Create tarball
      tar -cJf "$VENDOR_TARBALL" -C "$_staging_tmp" vendor
      _tar_status=$?

      rm -rf "$_staging_tmp"

      if test $_tar_status -ne 0; then
        echo "configure: warning: failed to compress vendor directory" >&2
      else
        echo "$_new_fingerprint" > "$VENDOR_STAMP"
        echo "configure: created inst/vendor.tar.xz"
      fi
    else
      echo "configure: skipping vendor compression (unchanged)"
    fi
  fi
],
[NOT_CRAN="$NOT_CRAN" VENDOR_OUT="$VENDOR_OUT" abs_rpkg_src="$abs_rpkg_src" abs_rpkg_dir="$abs_top_srcdir" MINIEXTENDR_SOURCE_DIR="$MINIEXTENDR_SOURCE_DIR" RSYNC="$RSYNC"])

AC_OUTPUT
