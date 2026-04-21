# `dvs`

## note: this is a "develop-in-the-open" active rewrite targetting stabilization in ~March 2026, for the moment expect major breakage/instability as we
## consolidate the feature set we want to support in dvs for this next release.
## please contact us if these ideas or discussion points are of interest, we'd love to get more collaborative discussions going

Rewrite of `dvs`, the data-version-control system made by A2-AI.

DVS (Data Version System) is a tool for versioning large or sensitive files under Git without tracking the file content directly.


## Installation — R package

The `dvs` R package ships as a source tarball with Rust dependencies vendored
in — installing does not re-download crates. You still need a Rust toolchain
(`rustc >= 1.85.0`) on `PATH`, since the Rust core compiles locally on install.

### Latest release

```r
install.packages(
  "https://github.com/A2-ai/dvs2/releases/latest/download/dvs_<version>.tar.gz",
  repos = NULL, type = "source"
)
```

Substitute `<version>` for the current release. See [Releases][releases] for
available versions.

### Dev build from an open PR

When a PR is labeled `release-dev`, CI publishes a rolling prerelease at tag
`pr-<N>-dev` and posts the install URL as a comment on the PR. General form:

```r
install.packages(
  "https://github.com/A2-ai/dvs2/releases/download/pr-<N>-dev/dvs_<version>-pr<N>-<sha>.tar.gz",
  repos = NULL, type = "source"
)
```

The prerelease is deleted when the PR closes.

[releases]: https://github.com/A2-ai/dvs2/releases

## TODOs

- Azure backend
- GC?
- dvs remove?
- integrity check? would need to read the file again after saving it
- compression?
- migrate from dvs1
