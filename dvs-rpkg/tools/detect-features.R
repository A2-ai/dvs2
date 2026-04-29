# Feature detection for dvs-rpkg
# Called by ./configure to auto-detect which Cargo features to enable.
# Output: comma-separated list of Cargo features to enable (printed via cat()).
#
# The upstream miniextendr detect-features.R template is not applicable here:
# it lists miniextendr framework features (rayon, ndarray, vctrs, …) that
# dvs-rpkg does not expose. dvs-rpkg has exactly two optional features:
#
#   nonapi      — enables non-standard R API calls in miniextendr-api.
#                 Causes R CMD check WARNINGS. Never auto-enable.
#   connections — enables R's connection framework in miniextendr-api.
#                 The R connections API is explicitly marked unstable upstream.
#                 Never auto-enable.
#
# Neither feature is used in dvs-rpkg's own lib.rs; they only affect
# miniextendr-api internals. The safe default is no extra features.
#
# To opt in, set CARGO_FEATURES before running configure:
#   CARGO_FEATURES=connections bash ./configure
# (configure guards on `test -z "${CARGO_FEATURES+x}"` so an env var always
# wins over this script — see configure.ac lines ~80-92.)
#
# Alternatively, set DVS_AUTO_FEATURES here as an escape hatch during
# development without touching the configure invocation:
#   DVS_AUTO_FEATURES=connections bash ./configure

features <- character()

# Escape hatch: DVS_AUTO_FEATURES lets developers inject features without
# having to set CARGO_FEATURES (which would prevent this script from running).
auto <- Sys.getenv("DVS_AUTO_FEATURES", unset = "")
if (nzchar(auto)) {
  features <- c(features, strsplit(auto, ",")[[1]])
}

cat(paste(features, collapse = ","))
