AC_INIT([dvs], [0.0.0.9000])

dnl Verify we're running from the R package directory
if test ! -f "DESCRIPTION" || test ! -f "NAMESPACE"; then
  AC_MSG_ERROR([configure must be run from the dvsR package directory (where DESCRIPTION exists).])
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

dnl No features by default
CARGO_FEATURES_FLAG=""
CARGO_FEATURE_CPPFLAGS=""
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
AS_IF([test "x$CARGO" = "xno"], [AC_MSG_ERROR([`cargo` not found])])
AS_IF([test "x$RUSTC" = "xno"], [AC_MSG_ERROR([`rustc` not found])])
AS_IF([test "x$SED" = "xno"], [AC_MSG_ERROR([`sed` not found])])
AC_SUBST([CARGO])
AC_SUBST([RUSTC])
AC_SUBST([SED])
AC_SUBST([CYGPATH])

dnl Use native paths for Cargo on Windows/MSYS
abs_rpkg_src_cargo="$abs_rpkg_src"
if test "x$CYGPATH" != "xno"; then
  abs_rpkg_src_cargo="$($CYGPATH -m "$abs_rpkg_src" 2>/dev/null || echo "$abs_rpkg_src")"
fi
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

dnl ---- Cargo configuration ----
AC_ARG_VAR([CARGO_TARGET_DIR], [Cargo target directory (default: src/rust/target)])
: ${CARGO_TARGET_DIR="$abs_rpkg_src/rust/target"}
AC_SUBST([CARGO_TARGET_DIR])

CARGO_TARGET_DIR_CARGO="$abs_rpkg_src_cargo/rust/target"
AC_SUBST([CARGO_TARGET_DIR_CARGO])

AC_ARG_VAR([CARGO_PROFILE], [Cargo build profile (debug or release)])
: ${CARGO_PROFILE="release"}
AC_SUBST([CARGO_PROFILE])

dnl Static library name - will be set after reading Cargo.toml
CARGO_STATICLIB_NAME="dvs"
AC_SUBST([CARGO_STATICLIB_NAME])

AC_ARG_VAR([ENV_RUSTFLAGS], [Rust compiler flags passed as RUSTFLAGS])
: ${ENV_RUSTFLAGS="$RUSTFLAGS"}
AC_SUBST([ENV_RUSTFLAGS])

AC_ARG_VAR([CARGO_BUILD_TARGET], [Cargo build target triple (e.g. aarch64-apple-darwin)])
: ${CARGO_BUILD_TARGET=""}
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

dnl ---- package name variants ----
dnl PACKAGE_NAME from AC_INIT preserves case (e.g., dvsR)
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
AC_CONFIG_FILES([src/entrypoint.c:src/entrypoint.c.in])
AC_CONFIG_FILES([src/mx_abi.c:src/mx_abi.c.in])

dnl ---- output files ----
dnl Create .cargo directory
AS_MKDIR_P([src/rust/.cargo])

dnl Generate config files from templates
AC_CONFIG_FILES([src/rust/.cargo/config.toml:src/rust/cargo-config.toml.in])

dnl ---- AC_CONFIG_COMMANDS run after AC_CONFIG_FILES generates files ----

dnl 1) Run cargo vendor (dev mode only, after Cargo.toml is generated)
AC_CONFIG_COMMANDS([cargo-vendor],
[
  if test "$NOT_CRAN" != "true"; then
    dnl CRAN/offline mode - check for vendor directory or tarball
    if test ! -d "$VENDOR_OUT" || test -z "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
      if test -f "$abs_rpkg_dir/inst/vendor.tar.xz"; then
        echo "configure: CRAN build - unpacking inst/vendor.tar.xz"
        tar -xJf "$abs_rpkg_dir/inst/vendor.tar.xz" -C "$abs_rpkg_src"
        if test $? -ne 0; then
          echo "configure: error: failed to unpack vendor.tar.xz" >&2
          exit 1
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
        for _mx_crate in miniextendr-api miniextendr-macros miniextendr-lint; do
          dnl Try both layouts: flat (miniextendr-api/) and nested (crates/miniextendr-api/)
          _mx_src="$MINIEXTENDR_SOURCE_DIR/$_mx_crate"
          if test ! -d "$_mx_src"; then
            _mx_src="$MINIEXTENDR_SOURCE_DIR/crates/$_mx_crate"
          fi
          _mx_dst="$VENDOR_OUT/$_mx_crate"
          if test -d "$_mx_src"; then
            rm -rf "$_mx_dst"
            cp -R "$_mx_src" "$_mx_dst"
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
            dnl Replace workspace inheritance with actual values
            $SED -i.bak \
              -e 's/^version\.workspace = true$/version = "0.1.0"/' \
              -e 's/^edition\.workspace = true$/edition = "2024"/' \
              -e 's/^license\.workspace = true$/license = "MIT"/' \
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

      dnl Temporarily move .cargo/config.toml to avoid circular vendored-sources reference
      _cargo_cfg="src/rust/.cargo/config.toml"
      _cargo_cfg_bak="src/rust/.cargo/config.toml.bak"
      if test -f "$_cargo_cfg"; then
        mv "$_cargo_cfg" "$_cargo_cfg_bak"
      fi

      $CARGO $RUST_TOOLCHAIN vendor \
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
      $CARGO $RUST_TOOLCHAIN generate-lockfile --manifest-path "$MANIFEST"
      if test $? -ne 0; then
        echo "configure: error: cargo generate-lockfile failed" >&2
        exit 1
      fi
    fi

    dnl Update vendor stamp
    _new="$(cksum < "$LOCKFILE" | awk '{print $1":"$2}')"
    echo "$_new" > "$STAMP"

    dnl Clear checksums for tarball compatibility
    echo "configure: clearing vendor checksums"
    for _crate_dir in "$VENDOR_OUT"/*/; do
      if test -d "$_crate_dir"; then
        echo '{"files":{}}' > "${_crate_dir}.cargo-checksum.json"
      fi
    done

    echo "configure: vendor complete"
  fi
],
[CARGO="$CARGO" RUST_TOOLCHAIN="$RUST_TOOLCHAIN" NOT_CRAN="$NOT_CRAN" VENDOR_OUT="$VENDOR_OUT" abs_rpkg_src="$abs_rpkg_src" abs_rpkg_dir="$abs_top_srcdir" FORCE_VENDOR="$FORCE_VENDOR" MINIEXTENDR_SOURCE_DIR="$MINIEXTENDR_SOURCE_DIR"])

dnl 2) Compress vendor directory for CRAN tarballs
AC_CONFIG_COMMANDS([compress-vendor],
[
  if test "$NOT_CRAN" = "true" && test -d "$VENDOR_OUT" && test -n "`ls -A \"$VENDOR_OUT\" 2>/dev/null`"; then
    VENDOR_TARBALL="$abs_rpkg_dir/inst/vendor.tar.xz"
    VENDOR_STAMP="$abs_rpkg_dir/inst/.vendor.tar.xz.cksum"

    _lock_ck=""
    if test -f "src/rust/Cargo.lock"; then
      _lock_ck="$(cksum < "src/rust/Cargo.lock" | awk '{print $1":"$2}')"
    fi

    _new_fingerprint="lock:$_lock_ck"
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

      dnl Clear checksums in staging copy
      for _crate_dir in "$_staging_tmp/vendor"/*/; do
        test -d "$_crate_dir" && echo '{"files":{}}' > "${_crate_dir}.cargo-checksum.json"
      done

      dnl Remove unnecessary directories to reduce size
      find "$_staging_tmp/vendor" -type d -name 'tests' -exec rm -rf {} + 2>/dev/null || true
      find "$_staging_tmp/vendor" -type d -name 'benches' -exec rm -rf {} + 2>/dev/null || true
      find "$_staging_tmp/vendor" -type d -name 'examples' -exec rm -rf {} + 2>/dev/null || true
      find "$_staging_tmp/vendor" -type d -name '.github' -exec rm -rf {} + 2>/dev/null || true
      dnl Truncate .md files (keep file but remove content)
      find "$_staging_tmp/vendor" -name '*.md' -type f | while read _md; do : > "$_md"; done

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
[NOT_CRAN="$NOT_CRAN" VENDOR_OUT="$VENDOR_OUT" abs_rpkg_src="$abs_rpkg_src" abs_rpkg_dir="$abs_top_srcdir"])

dnl 3) Extract CARGO_STATICLIB_NAME from generated Cargo.toml
AC_CONFIG_COMMANDS([patch-staticlib-name],
[
  if test "$NOT_CRAN" = "true"; then
    _pkgid_flags="--color=never"
  else
    _pkgid_flags="--offline --color=never"
  fi
  _pkgid_output="$( $CARGO $RUST_TOOLCHAIN pkgid $_pkgid_flags --manifest-path src/rust/Cargo.toml 2>/dev/null )"
  _pkgid_status=$?
  if test $_pkgid_status -ne 0; then
    dnl Fallback: extract from Cargo.toml directly
    REAL_STATICLIB_NAME="$($SED -n 's/^name = "\(.*\)"/\1/p' src/rust/Cargo.toml | head -1 | tr '-' '_')"
  else
    REAL_STATICLIB_NAME="$(echo "$_pkgid_output" | "$SED" 's/.*#\([^@]*\)@.*/\1/' | tr '-' '_')"
  fi
  if test -z "$REAL_STATICLIB_NAME"; then
    echo "configure: warning: Could not extract package name, using default: $CARGO_STATICLIB_NAME" >&2
    REAL_STATICLIB_NAME="$CARGO_STATICLIB_NAME"
  fi
  echo "configure: CARGO_STATICLIB_NAME = $REAL_STATICLIB_NAME"

  dnl Patch generated files
  for f in src/Makevars src/rust/document.rs; do
    if test -f "$f"; then
      "$SED" "s/__CARGO_STATICLIB_NAME_PLACEHOLDER__/$REAL_STATICLIB_NAME/g" "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    fi
  done
],
[CARGO="$CARGO" RUST_TOOLCHAIN="$RUST_TOOLCHAIN" NOT_CRAN="$NOT_CRAN" SED="$SED" CARGO_STATICLIB_NAME="$CARGO_STATICLIB_NAME"])

AC_OUTPUT
