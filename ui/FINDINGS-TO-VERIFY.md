# Findings to verify (fresh session)

These are CLAIMS produced by the `ui/cases_*.sh` scenario sweep. They have NOT
been independently confirmed. A new session must re-verify each from scratch,
treat the descriptions below as hypotheses (not conclusions), and only then
decide: real bug -> file an issue; real + small -> open a PR; not real / spec is
fine -> note and drop.

## Setup (do this first)

1. Install the CLI from a clean `origin/main` (NOT from any feature worktree):
   `git fetch origin && git worktree add ../dvs2-verify origin/main` then
   `cd ../dvs2-verify && just install-cli`. Confirm `dvs --version` and that
   `git rev-parse origin/main` matches what you built.
2. The scenario scripts live on branch `ui-test-review` (commit edc2605, local,
   unpushed): `ui/cases_*.sh`, `ui/SCENARIOS.md`. Use them as a starting point
   but re-run each finding by hand in a pristine repo too.
3. Pristine repo per check: `mktemp -d` a repo, `git init -q`, storage OUTSIDE
   the repo, `dvs init <storage>`. Do not trust a result that reused a dirty
   storage dir (a failed add can poison storage).

## Findings (re-verify each independently)

### Still-present prior issues — confirm they still reproduce on current origin/main
- [ ] #216 — add two byte-identical-content files in ONE invocation under default
  threads. Claim: one fails with a rename TOCTOU on the shared `<hash>.tmp`,
  exit 1. Check `--threads 1` works. Decide: still real?
- [ ] #219 — `dvs add good.bin missing.bin`. Claim: whole add aborts ("Path not
  found"), `good.bin` NOT added (not best-effort, contra spec). Contrast with an
  unreadable (chmod 000) sibling, which IS best-effort. Decide: still real?
- [ ] #220 — `dvs get <tracked> <unknown>`. Claim: unknown path silently dropped,
  exit 0; only all-zero-match exits 1. Decide: spec wording bug or fine?

### Prior issues claimed FIXED — confirm the fix, then close the issue if real
- [ ] #217 — `dvs get <dir> --glob '*.bin'` (positional directory). Claim: now
  walks + filters correctly (was broken). If confirmed fixed, close #217.
- [ ] #218 — `dvs status --glob`/`-g`. Claim: now accepted and globs the metadata
  folder with literal-sep + recursive rules (was rejected exit 2). If confirmed,
  close #218 — BUT see N4 (the flag is undocumented).

### New candidates — verify, then file issue/PR only if real
- [ ] N1 — `dvs get` and absolute / root-relative paths. Claim: `get /abs/path`
  -> "No files to get" exit 1; `get data/x.bin` from a nested cwd -> same; only
  cwd-relative basenames work. Contrast: `add /abs/path` works. So a path copied
  from `status`/`add` output (root-relative) cannot be fed to `get` from a subdir.
  Check what the spec actually requires re: absolute paths before calling it a bug.
- [ ] N2 — glob resolution base differs by command. Claim: from inside `sub/`,
  `get --glob '*.bin'` resolves cwd-relative (globs `sub/`) while
  `status --glob '*.bin'` resolves root/metadata-relative. Spec says resolution is
  "the same way". Likely same root cause as N1. Confirm both, decide which is correct.
- [ ] N3 — `dvs get` over an `unsynced` (locally-modified) file. Claim: silently
  overwrites the local edits with the stored version, no warning/backup, exit 0
  (data loss). Spec is silent. Decide: spec clause, a `--force`/guard, or expected.
- [ ] N4 — `status -g, --glob <GLOB>` has an EMPTY help description and is absent
  from the specs.md `status --help` block, even though it now works. Spec/help sync.
- [ ] N5 — `dvs init` into an already-initialized storage dir errors ("backend
  storage exists"), blocking two projects sharing one storage. Check against the
  glossary's multi-project wording and the cross-project dedup claim — is shared
  storage actually a supported use case, or is this correct?

### Minor / documentation only (verify, likely spec notes not code)
- [ ] trailing slash on a file path is normalized (undocumented).
- [ ] relative storage path is recorded verbatim in `dvs.toml` (resolution base?).
- [ ] `status` zero-match glob -> empty + exit 0, while add/get zero-match -> error
  + exit 1. Intentional (status is read-only) but undocumented.

## Output of the new session
For each item: VERDICT (real-bug / fixed / spec-fix / not-an-issue / wontfix),
the exact repro you ran, and the action taken (issue # filed, PR opened, or
dropped with reason). Do not file until re-verified.
