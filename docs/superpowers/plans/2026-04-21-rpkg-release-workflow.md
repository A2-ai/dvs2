# R package release workflow — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `dvs` R package as a source tarball (with vendored Rust deps embedded) via GitHub Releases, on two paths: tag push (real release) and PR label (rolling dev release).

**Architecture:** One new workflow file `.github/workflows/release-rpkg.yml` that shares four jobs: `build-rpkg` (always), `publish-dev` (PR labeled `release-dev`), `cleanup-dev` (PR closed), `publish-release` (tag push). Source tarball produced by `R CMD build dvs-rpkg` after `just vendor` regenerates `dvs-rpkg/inst/vendor.tar.xz`. Real path rewrites `DESCRIPTION` version from the tag; dev path keeps `X.Y.Z.9000` and renames only the GitHub asset filename. Both paths smoke-install the built tarball with `CARGO_NET_OFFLINE=1` as the release gate.

**Tech Stack:** GitHub Actions, `r-lib/actions`, `dtolnay/rust-toolchain`, `actions/cache`, `actions/upload-artifact`, `actions/download-artifact`, `softprops/action-gh-release`, `gh` CLI, `actionlint`.

**Spec:** `docs/superpowers/specs/2026-04-21-rpkg-release-workflow-design.md`

---

## File Structure

Files created:
- `.github/workflows/release-rpkg.yml` — new workflow, single file, all four jobs.

Files modified:
- `README.md` — add **Installation — R package** section.

Files NOT touched:
- `.github/workflows/release.yml` — leave the existing CLI release workflow alone.
- `.github/workflows/ci.yml` — unchanged.
- `dvs-rpkg/DESCRIPTION` — never committed with a rewritten version; workflow rewrites in-place for the real-release build only.

---

## Conventions for this plan

- All paths are relative to repo root `dvs2/`.
- Every YAML step below uses pinned action SHAs where the existing `release.yml` pins them; otherwise pin major versions matching existing `ci.yml` usage.
- "Smoke-install" means: `R CMD INSTALL --no-test-load` against a mktemp library with `CARGO_NET_OFFLINE=1` + isolated `CARGO_HOME`.
- The label name is exactly `release-dev` (lowercase, single hyphen).
- Tag pattern for dev: `pr-<PR_NUMBER>-dev`.
- Commit sha is the 8-char short form of `github.event.pull_request.head.sha`.

---

### Task 1: Scaffold the workflow file with triggers, permissions, env

**Files:**
- Create: `.github/workflows/release-rpkg.yml`

**Rationale:** Start with a syntactically valid, minimal workflow so `actionlint` passes. Jobs get filled in next tasks.

- [ ] **Step 1: Create the workflow skeleton**

Write this file exactly:

```yaml
name: release-rpkg

on:
  push:
    tags: ["v*.*.*"]
  pull_request:
    types: [labeled, synchronize, closed]

permissions:
  contents: read

concurrency:
  # Separate by event + tag/PR so a tag push and a PR push don't cancel
  # each other, but pushes to the same PR do cancel in-flight dev builds.
  group: release-rpkg-${{ github.event_name }}-${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true

jobs:
  # Filled in by subsequent tasks.
  noop:
    if: false
    runs-on: ubuntu-latest
    steps:
      - run: "placeholder — replaced in later tasks"
```

- [ ] **Step 2: Lint the workflow**

```bash
cd /Users/elea/Documents/a2ai_github/dvs2
# Install actionlint if missing
if ! command -v actionlint >/dev/null; then
  bash <(curl -s https://raw.githubusercontent.com/rhysd/actionlint/main/scripts/download-actionlint.bash)
  export PATH="$PWD:$PATH"
fi
actionlint .github/workflows/release-rpkg.yml
```

Expected: no output (success). If actionlint warns about the `noop` job never running, that's acceptable — it disappears in Task 2.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-rpkg.yml
git commit -m "ci: scaffold release-rpkg workflow (triggers only)"
```

---

### Task 2: Add the `build-rpkg` job (produces the source tarball artifact)

**Files:**
- Modify: `.github/workflows/release-rpkg.yml` (replace `noop` with `build-rpkg`)

**Rationale:** This job is shared by both the dev and real-release paths. Producing and validating the artifact is the core of the workflow.

- [ ] **Step 1: Replace the `noop` job with `build-rpkg`**

Replace the entire `jobs:` section of `.github/workflows/release-rpkg.yml` with:

```yaml
jobs:
  build-rpkg:
    name: Build R package source tarball
    runs-on: ubuntu-latest
    timeout-minutes: 30
    if: |
      (github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v'))
      || (
        github.event_name == 'pull_request'
        && github.event.action != 'closed'
        && contains(github.event.pull_request.labels.*.name, 'release-dev')
        && github.event.pull_request.head.repo.full_name == github.repository
      )
    permissions:
      contents: read
    env:
      CARGO_TERM_COLOR: always
    steps:
      - name: Checkout
        uses: actions/checkout@v6
        with:
          fetch-depth: 0
          lfs: true
          # For PR events, check out the PR head (not the default merge ref)
          # so DESCRIPTION/Cargo.toml reflect the PR branch state.
          ref: ${{ github.event.pull_request.head.sha || github.ref }}

      - name: Install system deps
        run: |
          sudo apt-get update
          sudo apt-get install -y autoconf

      - name: Install Rust toolchain
        uses: dtolnay/rust-toolchain@master
        with:
          toolchain: stable

      - name: Install just
        uses: taiki-e/install-action@just

      - name: Setup R
        uses: r-lib/actions/setup-r@v2
        with:
          r-version: release
          use-public-rspm: true

      - name: Setup R dependencies
        uses: r-lib/actions/setup-r-dependencies@v2
        with:
          working-directory: dvs-rpkg
          needs: check

      - name: Cache cargo-revendor
        id: revendor-cache
        uses: actions/cache@v4
        with:
          path: ~/.cargo/bin/cargo-revendor
          key: ${{ runner.os }}-cargo-revendor-v1

      - name: Install cargo-revendor
        if: steps.revendor-cache.outputs.cache-hit != 'true'
        run: cargo install --git https://github.com/A2-ai/miniextendr cargo-revendor

      - name: Vendor rpkg deps
        run: just vendor

      - name: Stamp DESCRIPTION version (real release only)
        if: startsWith(github.ref, 'refs/tags/v')
        env:
          REF_NAME: ${{ github.ref_name }}
        run: |
          set -euo pipefail
          TAG_VERSION="${REF_NAME#v}"

          # Validate R-compliant version syntax: X.Y.Z, X.Y.Z.N, X.Y.Z-N.
          if ! [[ "$TAG_VERSION" =~ ^[0-9]+(\.[0-9]+){2,3}(-[0-9]+)?$ ]]; then
            echo "::error::Tag '$REF_NAME' is not an R-compliant version." >&2
            echo "Accepted patterns: vX.Y.Z, vX.Y.Z.N, vX.Y.Z-N (N numeric)." >&2
            exit 1
          fi

          # Enforce Cargo.toml base == tag base (same check .miniextendr ci uses).
          CARGO_VERSION=$(sed -n '/\[workspace\.package\]/,/^\[/{ s/^version = "\(.*\)"/\1/p; }' Cargo.toml | head -1)
          TAG_BASE=$(echo "$TAG_VERSION" | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+')
          CARGO_BASE=$(echo "$CARGO_VERSION" | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+')
          if [ "$TAG_BASE" != "$CARGO_BASE" ]; then
            echo "::error::Tag base ($TAG_BASE) != Cargo.toml base ($CARGO_BASE)." >&2
            echo "Bump Cargo.toml [workspace.package] version before tagging." >&2
            exit 1
          fi

          sed -i "s/^Version: .*/Version: ${TAG_VERSION}/" dvs-rpkg/DESCRIPTION
          echo "Rewrote DESCRIPTION Version: -> ${TAG_VERSION}"

      - name: Assert vendor tarball exists
        run: |
          test -s dvs-rpkg/inst/vendor.tar.xz \
            || { echo "::error::inst/vendor.tar.xz missing or empty after just vendor"; exit 1; }

      - name: Build R source tarball
        run: R CMD build dvs-rpkg

      - name: Locate built tarball
        id: tarball
        run: |
          set -euo pipefail
          TARBALL=$(ls -1 dvs_*.tar.gz | head -1)
          test -n "$TARBALL" || { echo "::error::R CMD build did not produce a tarball"; exit 1; }
          echo "path=$TARBALL" >> "$GITHUB_OUTPUT"
          echo "Built: $TARBALL"

      - name: Offline smoke-install
        env:
          NOT_CRAN: "false"
          CARGO_NET_OFFLINE: "1"
        run: |
          set -euo pipefail
          CARGO_HOME=$(mktemp -d)
          export CARGO_HOME
          LIB=$(mktemp -d)
          R CMD INSTALL --no-test-load --library="$LIB" "${{ steps.tarball.outputs.path }}"
          echo "Offline install OK (library: $LIB)"

      - name: Generate sha256
        run: |
          sha256sum "${{ steps.tarball.outputs.path }}" > "${{ steps.tarball.outputs.path }}.sha256"
          cat "${{ steps.tarball.outputs.path }}.sha256"

      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: dvs-rpkg-tarball
          path: |
            dvs_*.tar.gz
            dvs_*.tar.gz.sha256
          if-no-files-found: error
          retention-days: 7
```

Remove the initial guard-step draft from Step 1 — the final file has only the job-`if` version above (no `noop`, no `guard` step).

- [ ] **Step 2: Lint**

```bash
actionlint .github/workflows/release-rpkg.yml
```

Expected: no output.

- [ ] **Step 3: Sanity-check the YAML parses**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-rpkg.yml'))"
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-rpkg.yml
git commit -m "ci(release-rpkg): add build-rpkg job producing source tarball artifact"
```

---

### Task 3: Add the `publish-dev` job (PR-label rolling prerelease + PR comment)

**Files:**
- Modify: `.github/workflows/release-rpkg.yml` (append new job under `jobs:`)

- [ ] **Step 1: Append the `publish-dev` job**

Append this under `jobs:` in `.github/workflows/release-rpkg.yml` (after `build-rpkg`):

```yaml
  publish-dev:
    name: Publish dev prerelease
    needs: build-rpkg
    runs-on: ubuntu-latest
    if: |
      github.event_name == 'pull_request'
      && github.event.action != 'closed'
      && contains(github.event.pull_request.labels.*.name, 'release-dev')
      && github.event.pull_request.head.repo.full_name == github.repository
    permissions:
      contents: write
      pull-requests: write
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      PR_NUM: ${{ github.event.pull_request.number }}
      HEAD_SHA: ${{ github.event.pull_request.head.sha }}
    steps:
      - name: Download artifact
        uses: actions/download-artifact@v4
        with:
          name: dvs-rpkg-tarball

      - name: Compute names
        id: names
        run: |
          set -euo pipefail
          TAG="pr-${PR_NUM}-dev"
          SHORT_SHA="${HEAD_SHA:0:8}"
          SRC=$(ls -1 dvs_*.tar.gz | head -1)
          # dvs_0.3.0.9000.tar.gz -> dvs_0.3.0.9000-pr<N>-<sha>.tar.gz
          ASSET="${SRC%.tar.gz}-pr${PR_NUM}-${SHORT_SHA}.tar.gz"
          mv "$SRC" "$ASSET"
          mv "${SRC}.sha256" "${ASSET}.sha256"
          # Regenerate sha256 (path inside file changed).
          sha256sum "$ASSET" > "${ASSET}.sha256"

          echo "tag=$TAG" >> "$GITHUB_OUTPUT"
          echo "asset=$ASSET" >> "$GITHUB_OUTPUT"
          echo "short_sha=$SHORT_SHA" >> "$GITHUB_OUTPUT"

      - name: Create or refresh prerelease
        env:
          TAG: ${{ steps.names.outputs.tag }}
          ASSET: ${{ steps.names.outputs.asset }}
        run: |
          set -euo pipefail
          INSTALL_URL="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/${ASSET}"

          NOTES=$(cat <<EOF
          Dev build for PR #${PR_NUM} at commit ${HEAD_SHA}.

          **Install:**
          \`\`\`r
          install.packages(
            "${INSTALL_URL}",
            repos = NULL, type = "source"
          )
          \`\`\`

          Requires: Rust toolchain (\`rustc >= 1.85.0\`) on \`PATH\`.

          _This prerelease is replaced on every push to the PR and deleted when the PR closes._
          EOF
          )

          if gh release view "$TAG" >/dev/null 2>&1; then
            # Delete existing dvs_* assets so old-commit assets don't pile up.
            mapfile -t OLD < <(gh release view "$TAG" --json assets -q '.assets[].name' | grep '^dvs_' || true)
            for name in "${OLD[@]}"; do
              gh release delete-asset "$TAG" "$name" --yes || true
            done
            gh release upload "$TAG" "$ASSET" "${ASSET}.sha256"
            gh release edit "$TAG" \
              --target "$HEAD_SHA" \
              --title "Dev build for PR #${PR_NUM}" \
              --notes "$NOTES" \
              --prerelease
          else
            gh release create "$TAG" "$ASSET" "${ASSET}.sha256" \
              --target "$HEAD_SHA" \
              --prerelease \
              --title "Dev build for PR #${PR_NUM}" \
              --notes "$NOTES"
          fi

          echo "install_url=$INSTALL_URL" >> "$GITHUB_OUTPUT"
        id: publish

      - name: Comment on PR (create or update)
        env:
          TAG: ${{ steps.names.outputs.tag }}
          ASSET: ${{ steps.names.outputs.asset }}
        run: |
          set -euo pipefail
          INSTALL_URL="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/${ASSET}"
          MARKER="<!-- dvs-rpkg-dev-release -->"

          BODY=$(cat <<EOF
          ${MARKER}
          ### Dev R package build

          Tarball: [\`${ASSET}\`](${INSTALL_URL})

          \`\`\`r
          install.packages(
            "${INSTALL_URL}",
            repos = NULL, type = "source"
          )
          \`\`\`

          Requires: Rust toolchain (\`rustc >= 1.85.0\`) on \`PATH\`.
          Built from commit \`${HEAD_SHA}\`. Updated on every push.
          EOF
          )

          # Find existing comment by marker.
          COMMENT_ID=$(gh api \
            "repos/${GITHUB_REPOSITORY}/issues/${PR_NUM}/comments" \
            --jq ".[] | select(.body | contains(\"${MARKER}\")) | .id" \
            | head -1)

          if [ -n "$COMMENT_ID" ]; then
            gh api --method PATCH \
              "repos/${GITHUB_REPOSITORY}/issues/comments/${COMMENT_ID}" \
              -f body="$BODY" >/dev/null
            echo "Updated PR comment ${COMMENT_ID}"
          else
            gh pr comment "$PR_NUM" --body "$BODY"
            echo "Posted new PR comment"
          fi

      - name: Step summary
        env:
          TAG: ${{ steps.names.outputs.tag }}
          ASSET: ${{ steps.names.outputs.asset }}
        run: |
          INSTALL_URL="https://github.com/${GITHUB_REPOSITORY}/releases/download/${TAG}/${ASSET}"
          {
            echo "## Dev prerelease published"
            echo ""
            echo "- Tag: \`$TAG\`"
            echo "- Asset: \`$ASSET\`"
            echo "- Install URL: $INSTALL_URL"
          } >> "$GITHUB_STEP_SUMMARY"
```

- [ ] **Step 2: Lint**

```bash
actionlint .github/workflows/release-rpkg.yml
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-rpkg.yml'))"
```

Expected: no output from either.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-rpkg.yml
git commit -m "ci(release-rpkg): add publish-dev job (rolling prerelease + PR comment)"
```

---

### Task 4: Add the `cleanup-dev` job (delete prerelease on PR close)

**Files:**
- Modify: `.github/workflows/release-rpkg.yml` (append new job)

- [ ] **Step 1: Append the `cleanup-dev` job**

Append under `jobs:`:

```yaml
  cleanup-dev:
    name: Cleanup dev prerelease
    runs-on: ubuntu-latest
    if: github.event_name == 'pull_request' && github.event.action == 'closed'
    permissions:
      contents: write
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
      PR_NUM: ${{ github.event.pull_request.number }}
    steps:
      - name: Checkout
        uses: actions/checkout@v6

      - name: Delete prerelease and tag if present
        run: |
          set -euo pipefail
          TAG="pr-${PR_NUM}-dev"
          if gh release view "$TAG" >/dev/null 2>&1; then
            gh release delete "$TAG" --yes --cleanup-tag
            echo "Deleted release and tag $TAG"
          else
            echo "No release $TAG to clean up"
          fi
```

- [ ] **Step 2: Lint**

```bash
actionlint .github/workflows/release-rpkg.yml
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-rpkg.yml'))"
```

Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-rpkg.yml
git commit -m "ci(release-rpkg): add cleanup-dev job (delete prerelease on PR close)"
```

---

### Task 5: Add the `publish-release` job (tag push → real release)

**Files:**
- Modify: `.github/workflows/release-rpkg.yml` (append new job)

- [ ] **Step 1: Append the `publish-release` job**

Append under `jobs:`:

```yaml
  publish-release:
    name: Publish R package to tag release
    needs: build-rpkg
    runs-on: ubuntu-latest
    if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')
    permissions:
      contents: write
    steps:
      - name: Download artifact
        uses: actions/download-artifact@v4
        with:
          name: dvs-rpkg-tarball

      - name: Locate tarball
        id: tarball
        run: |
          set -euo pipefail
          TARBALL=$(ls -1 dvs_*.tar.gz | head -1)
          VERSION="${TARBALL#dvs_}"
          VERSION="${VERSION%.tar.gz}"
          echo "path=$TARBALL" >> "$GITHUB_OUTPUT"
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"

      - name: Append to GitHub Release
        uses: softprops/action-gh-release@c062e08bd532815e2082a85e87e3ef29c3e6d191
        with:
          tag_name: ${{ github.ref_name }}
          name: ${{ github.ref_name }}
          draft: true
          prerelease: ${{ contains(github.ref, '-pre') || contains(github.ref, '-test') }}
          fail_on_unmatched_files: true
          append_body: true
          files: |
            dvs_*.tar.gz
            dvs_*.tar.gz.sha256
          body: |
            **R package install:**
            ```r
            install.packages(
              "https://github.com/${{ github.repository }}/releases/download/${{ github.ref_name }}/${{ steps.tarball.outputs.path }}",
              repos = NULL, type = "source"
            )
            ```
            Requires: Rust toolchain (`rustc >= 1.85.0`) on `PATH`.
          token: ${{ secrets.GITHUB_TOKEN }}
```

Note on the action SHA: `c062e08bd532815e2082a85e87e3ef29c3e6d191` is the exact pinned SHA already used by `.github/workflows/release.yml`. Reuse it so both workflows move together when bumped.

- [ ] **Step 2: Lint**

```bash
actionlint .github/workflows/release-rpkg.yml
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-rpkg.yml'))"
```

Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/release-rpkg.yml
git commit -m "ci(release-rpkg): add publish-release job (tag push → GH Release)"
```

---

### Task 6: Update README.md with installation instructions

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Insert the new section**

Read current `README.md` first to find the right insertion point (before the `## TODOs` section).

Insert this block immediately above `## TODOs`:

```markdown
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

```

(Keep one blank line after the closing fence and before `## TODOs`.)

- [ ] **Step 2: Verify the section renders as expected**

```bash
grep -n "## Installation — R package" README.md
grep -n "## TODOs" README.md
```

Expected: the Installation heading is line-numbered *before* TODOs.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: add R package install instructions for release tarballs"
```

---

### Task 7: Final workflow lint + plan self-check

**Files:**
- None (validation only)

- [ ] **Step 1: Run actionlint on the entire workflows directory**

```bash
actionlint .github/workflows/*.yml
```

Expected: no output. Fix any issues actionlint reports in `release-rpkg.yml` only — do not touch other workflow files.

- [ ] **Step 2: Cross-check job dependencies**

```bash
grep -nE '^\s+(build-rpkg|publish-dev|cleanup-dev|publish-release):' .github/workflows/release-rpkg.yml
grep -n 'needs: build-rpkg' .github/workflows/release-rpkg.yml
```

Expected: 4 job definitions; both `publish-dev` and `publish-release` declare `needs: build-rpkg`; `cleanup-dev` does NOT need `build-rpkg` (it runs on PR close only).

- [ ] **Step 3: Verify all commits are on the feature branch**

```bash
git log --oneline origin/main..HEAD
```

Expected: at least 5 commits (scaffold, build, publish-dev, cleanup-dev, publish-release, README, optionally spec). No changes on `main`.

- [ ] **Step 4: Push the branch**

```bash
git push -u origin rpkg-release-workflow
```

This intentionally does not open a PR — the orchestrating agent will open the PR after review.

---

## Out-of-scope / deferred

- Testing the workflow end-to-end requires a real tag push and a real labeled PR; those happen after merge, not during implementation.
- `r-lib/actions/check-r-package` is intentionally NOT run here — `ci.yml` already runs `R CMD check --as-cran` on every PR; release is downstream of green CI.
- No cross-fork support. If/when external PRs need dev builds, switch to `pull_request_target` with careful guards (separate task).
