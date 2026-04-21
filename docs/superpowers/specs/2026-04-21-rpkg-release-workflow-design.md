# R package release workflow

## Goal

Publish the `dvs` R package as a GitHub Release asset — a single source tarball
with `inst/vendor.tar.xz` embedded — so users install without re-downloading or
re-compiling Rust crates. Two release flavors: real (tag-driven) and dev
(PR-label-driven).

## Motivation

Installing the R package from source today triggers a full `cargo fetch` +
compile of every transitive crate. Users reported this is the dominant cost.
The vendored tarball already exists in the build system (`just vendor`
produces `dvs-rpkg/inst/vendor.tar.xz`), but is gitignored and regenerated per
build. This workflow produces the same tarball shape at release time and
publishes it as a named, durable asset.

## Non-goals

- No precompiled R binaries (`.tgz`/`.zip`). Source tarball + local Rust compile
  only.
- No platform matrix. Source tarball is platform-neutral.
- No CRAN submission automation.
- No handling of fork PRs. `release-dev` label is gated to same-repo PRs.

## Triggers

Single new file: `.github/workflows/release-rpkg.yml`.

| Event                                          | Path            | Output                                      |
| ---------------------------------------------- | --------------- | ------------------------------------------- |
| `push` tag `v*.*.*`                            | real release    | Upload source tarball to Release `v*.*.*`   |
| `pull_request` (labeled/synchronize), label `release-dev` | dev release     | Create/update prerelease `pr-<N>-dev`       |
| `pull_request` (closed)                        | cleanup         | Delete prerelease `pr-<N>-dev` if it exists |

## Version handling

- `DESCRIPTION` on `main` stays at dev version `X.Y.Z.9000` (devtools
  convention). `Cargo.toml [workspace.package] version` stays at `X.Y.Z`.
- **Real release:** before `R CMD build`, workflow rewrites
  `dvs-rpkg/DESCRIPTION` `Version:` line to the tag value (tag stripped of
  leading `v`). Requires tag to be R-version-compliant:
  `^[0-9]+(\.[0-9]+){2,3}(-[0-9]+)?$`. Workflow fails loudly on non-compliant
  tags. Also enforces `Cargo.toml` base version matches the tag's `X.Y.Z`
  prefix — same consistency check pattern as `.miniextendr/.github/workflows/ci.yml`.
- **Dev release:** `DESCRIPTION` is not modified. Tarball filename is renamed
  for the GitHub asset only: `dvs_<ver>.tar.gz` →
  `dvs_<ver>-pr<N>-<sha>.tar.gz`. Installed package reports `<ver>.9000`,
  which correctly identifies it as a dev build.

## Jobs

### `build-rpkg`

Runs on every trigger. Produces the source tarball artifact.

```
runs-on: ubuntu-latest
steps:
  1. checkout@v6 (fetch-depth: 0, lfs: true)
  2. apt-get install -y autoconf
  3. r-lib/actions/setup-r@v2 (r-version: release, use-public-rspm: true)
  4. r-lib/actions/setup-r-dependencies@v2 (working-directory: dvs-rpkg, needs: check)
  5. dtolnay/rust-toolchain@stable
  6. cache ~/.cargo/bin/cargo-revendor (key: runner.os-cargo-revendor-v1)
  7. if cache miss: cargo install --git https://github.com/A2-ai/miniextendr cargo-revendor
  8. just vendor         # regenerates dvs-rpkg/inst/vendor.tar.xz
  9. [real-release path only]
       - extract TAG_VERSION from github.ref_name
       - verify Cargo.toml base matches TAG base (fail otherwise)
       - verify TAG_VERSION matches ^[0-9]+(\.[0-9]+){2,3}(-[0-9]+)?$
       - sed -i "s/^Version: .*/Version: ${TAG_VERSION}/" dvs-rpkg/DESCRIPTION
 10. assert dvs-rpkg/inst/vendor.tar.xz exists and size > 0
 11. R CMD build dvs-rpkg       # produces dvs_<ver>.tar.gz in cwd
 12. offline smoke-install:
       env: NOT_CRAN=false, CARGO_NET_OFFLINE=1, CARGO_HOME=$(mktemp -d)
       R CMD INSTALL --no-test-load --library=$(mktemp -d) dvs_<ver>.tar.gz
       # proves the vendor tarball actually builds without network
 13. sha256sum dvs_<ver>.tar.gz > dvs_<ver>.tar.gz.sha256
 14. actions/upload-artifact@v4: dvs-rpkg-tarball
       (files: dvs_*.tar.gz + .sha256)
```

### `publish-dev`

```
needs: build-rpkg
if: github.event_name == 'pull_request'
    && github.event.action != 'closed'
    && contains(github.event.pull_request.labels.*.name, 'release-dev')
    && github.event.pull_request.head.repo.full_name == github.repository
permissions: { contents: write, pull-requests: write }
runs-on: ubuntu-latest
steps:
  1. actions/download-artifact@v4: dvs-rpkg-tarball
  2. compute:
       PR_NUM = github.event.pull_request.number
       SHA    = short head sha (8 chars)
       TAG    = pr-${PR_NUM}-dev
       ASSET  = rename dvs_<ver>.tar.gz → dvs_<ver>-pr${PR_NUM}-${SHA}.tar.gz
  3. if gh release view "$TAG" succeeds:
       - enumerate existing assets and delete any matching dvs_* (gh
         release view --json assets, loop gh release delete-asset).
         Filenames include the commit sha, so prior-commit assets don't
         collide with the new upload — they'd just accumulate.
       - gh release upload "$TAG" "$ASSET" "$ASSET.sha256"
       - gh release edit "$TAG" --notes "<regenerated install block>"
     else:
       - gh release create "$TAG" "$ASSET" "$ASSET.sha256" \
           --target "${{ github.event.pull_request.head.sha }}" \
           --prerelease \
           --title "Dev build for PR #${PR_NUM}" \
           --notes "<install block>"
  4. PR comment management:
       - search PR comments for marker <!-- dvs-rpkg-dev-release -->
       - if present: update in place via gh pr comment --edit
       - else: post new comment containing the marker + install URL
  5. $GITHUB_STEP_SUMMARY: install URL + sha256
```

### `cleanup-dev`

```
runs-on: ubuntu-latest
if: github.event_name == 'pull_request' && github.event.action == 'closed'
permissions: { contents: write }
steps:
  1. TAG="pr-${{ github.event.pull_request.number }}-dev"
  2. gh release delete "$TAG" --yes --cleanup-tag 2>/dev/null || true
     # tolerate missing release — most closed PRs had no label
```

### `publish-release`

```
needs: build-rpkg
if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')
permissions: { contents: write }
runs-on: ubuntu-latest
steps:
  1. actions/download-artifact@v4: dvs-rpkg-tarball
  2. softprops/action-gh-release@<pinned sha>
       tag_name:  ${{ github.ref_name }}
       name:      ${{ github.ref_name }}
       draft:     true          # matches existing release.yml CLI workflow
       prerelease: ${{ contains(github.ref, '-pre') || contains(github.ref, '-test') }}
       fail_on_unmatched_files: true
       append_body: true        # don't clobber CLI workflow's body
       files: |
         dvs_*.tar.gz
         dvs_*.tar.gz.sha256
       body: |
         **R package install:**
         ```r
         install.packages(
           "https://github.com/A2-ai/dvs2/releases/download/${TAG}/dvs_${VER}.tar.gz",
           repos = NULL, type = "source"
         )
         ```
         Requires: Rust toolchain (rustc >= 1.85.0) on PATH.
```

Two workflows (`release.yml` for CLI, `release-rpkg.yml` for R pkg) both
upload to the same draft Release for a given tag. `softprops/action-gh-release`
is idempotent by tag; `append_body: true` merges bodies.

## README changes

Add new section to the root `README.md`:

```markdown
## Installation — R package

The `dvs` R package ships as a source tarball with Rust dependencies vendored.
Installing does not re-download crates, but does compile the Rust core locally
— you need a Rust toolchain (`rustc >= 1.85.0`) on `PATH`.

### Latest release

    install.packages(
      "https://github.com/A2-ai/dvs2/releases/latest/download/dvs_<version>.tar.gz",
      repos = NULL, type = "source"
    )

See [Releases](https://github.com/A2-ai/dvs2/releases) for specific versions.

### Dev build from a release PR

Open PRs labeled `release-dev` publish a rolling prerelease at tag
`pr-<N>-dev`. The install URL is posted as a comment on the PR. General form:

    install.packages(
      "https://github.com/A2-ai/dvs2/releases/download/pr-<N>-dev/dvs_<version>-pr<N>-<sha>.tar.gz",
      repos = NULL, type = "source"
    )
```

## Failure modes and guards

- **Tag/version mismatch on real release** — fail in build-rpkg step 9 with a
  message referencing `scripts/bump-version.sh` (if it exists) or the exact
  `sed` the author should run.
- **Non-R-compliant tag** — fail with a list of accepted patterns.
- **`just vendor` fails** — job fails, no upload.
- **Offline smoke-install fails** — job fails, no upload. This is the primary
  release gate: guarantees the tarball installs without network.
- **Fork PR labeled `release-dev`** — guard in `publish-dev` skips the job; no
  secret leak, no publish.
- **Race between CLI and R pkg release uploads** — `softprops/action-gh-release`
  re-reads the Release before writing; `append_body: true` prevents body
  clobber. Both jobs write distinct asset filenames.
- **Cleanup job runs on PR that never had the label** — `|| true` on
  `gh release delete` tolerates missing release.

## Testing

- Lint workflow YAML with `actionlint` before merging.
- First real tag push: verify Release contains both CLI tarball and R source
  tarball + sha256.
- First PR labeled `release-dev`: verify prerelease `pr-<N>-dev` exists, PR
  comment posted once, subsequent push edits same comment and replaces asset.
- Close that PR: verify prerelease deleted.
- Install from a dev prerelease on a clean R environment with Rust only —
  should succeed offline (set `CARGO_NET_OFFLINE=1` to prove it).
