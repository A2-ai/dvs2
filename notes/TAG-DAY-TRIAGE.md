# dvs2 — open PR + issue resolution plan

Goal (revised by owner 2026-06-17): **resolve ALL open PRs and open issues** and get
them merge-ready. Tag/version is deferred. **I prepare; the owner merges** — nothing
is merged unilaterally.

Scope notes from owner:
- **ui/ is out of scope** for now (artifacts + walkthroughs). #205 informational only.
- **Local artifacts skipped/gitignored**: `ui/output`, `target`, `rust-target`,
  `.miniextendr`, `dvs2-demo`, `journal`, `.alexandria-cli`.
- **Done already**: `.miniextendr` pulled `3ab1ea9 → baf9ba86`; `dvs2-demo` → `3dee55d`;
  13 PR worktrees at `~/Documents/a2ai_github/dvs2-pr-worktrees/pr-NNN`.

---

## The two facts that drive everything

1. **main only advanced 1 commit (#206)** past the snapshot. The rpkg PRs (#170/#193/#204)
   "conflict" on exactly one file — `dvs-rpkg/R/dvs-wrappers.R`, which is **generated**
   (CLAUDE.md: never hand-edit). #206 regenerated it. These conflicts **dissolve on
   re-document/re-scaffold**, not by manual merge.
2. **The miniextendr update + a fresh re-scaffold is the linchpin.** It (a) fixes issue
   **#141** (`Option<u64>` → list-column) at the FFI boundary, (b) addresses **#184**
   (packaging), (c) regenerates wrappers so the rpkg-shape PRs rebase clean, and (d)
   supersedes #194. After it lands, the R-side `.dvs_finalize()` coercion in #193 becomes
   dead-but-harmless.

---

## Track 0 — Foundation: re-scaffold dvs-rpkg (gates the whole rpkg track)

Branch `chore/rescaffold-miniextendr-latest` (subagent building it now, off main).
- Sequence: `rpkg-update → rpkg-configure → rpkg-install → rpkg-document` (never skip document).
- **Verify (owner asks):** rust-analyzer works, roxygen2 current, scaffolding current.
- **Verify carries** `tools/lock-shape-check.R` + `src/r_shim.h` (the only hand-maintained
  infra in #194); else cherry-pick from #194 `b4bfa72`. Everything else in #194 is generated.
- Closes/​supersedes **#194**. Resolves **#141** (boundary) + **#184** (packaging).
- → Owner merges. Then the rpkg-shape PRs rebase onto it.

## Track A — recursive-get stack (Rust + rpkg) — independent of rpkg-shape

- **#203** `feat(core+cli): add --recursive` — CHANGES_REQUESTED, rebases clean (1 behind).
  Resolve Keats' 4 (all small, verified against the diff):
  1. `main.rs` — restore `#[clap(required_unless_present="glob")]` on `get` paths, drop the custom `bail!`.
  2. `status.rs` — move `normalize_path` to `paths.rs` (shared by globbing.rs); update both callers.
  3. `globbing.rs:107` — fix the misleading comment (cwd scoping is applied later, not via `None`).
  4. `globbing.rs:109` — drop `Option<Vec<PathBuf>>`; use a plain `Vec` + a `had_paths` bool so
     "paths given but all escaped root" still matches nothing (preserves semantics; Keats: no Option).
  Run `cargo test -p dvs` + `clippy`. Push to update PR. **→ owner merges.**
- **#204** `feat(rpkg): dvs_get(recursive=)` — verified merge-ready (signature/semantics match
  #203 exactly, `recursive=NULL→false` matches CLI, docs complete). After #203 merges, retarget
  base to main, re-document wrappers. **→ owner merges.**
- **#205** ui walkthrough — out of scope; hold for post-tag ui inclusion.
- Spec **#149 L227** (recursive-get in `dvs get --help`) updates once #203 lands.

## Track B — rpkg data-shape — gated by Track 0 on main; order #170 → #193 → #155

These hinge on miniextendr `vec_to_dataframe_split` (shape), not the Option<> fix.
- **#170** `dvs_init` → single-row tibble. No Option<> workaround; clean shape change on old
  miniextendr (68b6b20). Rebase onto main (post-rescaffold), regenerate wrappers (kills the
  generated-file "conflict"), `as_tibble` wrap. **→ owner merges.**
- **#193** event state → df or list-of-df (`vec_to_dataframe_split`, `.dvs_finalize()`). Rebase
  **directly onto main** (NOT onto #192 — see Track D). Keep the essential `is.list` branch;
  the `stored_size/size` coercion is dead-but-harmless post-rescaffold. **→ owner merges.**
- **#155** console summary messages — depends on #193's shape (would break on the list-of-df
  case without it). Rebase after #193. **→ owner merges.**
- Issue **#141**: fixed at the boundary by Track 0; #193's shape verified against acceptance.

## Track C — threads #183 (mergeable, clean rebase)

- Core `dvs/src/utils.rs` priority-chain logic is sound. Keats' open question = the **dual cap**
  `ENVIRONMENT_MAX_THREADS=32` vs `DEFAULT_MAX_THREADS=16` with no documented rationale.
  → **DECISION NEEDED**: unify to one `MAX_THREADS` or justify the asymmetry in a comment.
- env→environment rename already addressed Keats' 2nd thread.
- justfile: keep variadic `ui-publish-only *names`; **drop** the `ui_names += "threads"` (ui coupling).
- `ui/main_threads.sh` defers (ui scope). **→ owner merges after the dual-cap decision.**
- Related spec decision: #149 L415-418 thread **precedence** (env var vs R package) — dpastoor
  said *package should take precedence*; spec currently says env var. Settle alongside #183.

## Track D — paths #192 (likely close)

- `validate_for_add/get` are called **only** from add.rs/get.rs on already-repo-relative paths;
  nothing in dvs-rpkg calls them. #192's `user_path_to_repo_relative` double-converts →
  redundant (matches the author's own "unnecessary mechanism" note from Vincent's review).
  → **DECISION**: close #192; rebase #193 straight onto main. (Confirm.)

## Track E — specs

- **#200** high-level conventions — **merge-ready now**, clean + independent of #149.
- **#149** audit edits — **post-tag**. CHANGES_REQUESTED unresolved: `date→data` typo (L127),
  L75 storage-binding spec-level question, L415-418 precedence (Track C), L227 recursive (Track A).
  Solid mechanical bits (errors section, audit-log JSON, R signatures) could split out, but cheaper
  to finish in one cycle after #200/#203 land.

## Track F — standalone issue fixes (fresh small PRs)

- **#131** lift storage-inside-repo guard into `dvs::init::init()`; drop dup from CLI + rpkg;
  unit test. Clean, self-contained.
- **#135** audit `timestamp: i64 → jiff::Timestamp` via `serde ... second::required` (no wire break).
- **#118** R progress bar only when `interactive()`.
- **#129** re-enable macOS/Windows CI matrix (release gate; uncomment in ci.yml, confirm green).
- **#153** `--compression` flag for `dvs add`, **#152** `--fail-fast` — features needing design;
  discuss before building.

## Dead — close

- **#37** (filters, 121 behind), **#2** (migrate, 143 behind): core was rewritten, `file.rs`
  deleted; not rebaseable. Close. (#2's migration idea → README "migrate from dvs1" TODO, fresh work.)

---

## Execution order (I prepare; owner merges each)

1. **Track 0** re-scaffold → verify env → owner merges → close #194.
2. **#203** (resolve 4 CRs, push) → owner merges → **#204** retarget+document → owner merges.
3. **#170** → **#193** (close #192) → **#155**, each rebased on main → owner merges in turn.
4. **#183** (after dual-cap decision) and **#200** → owner merges.
5. New PRs: **#131**, **#135**, **#118**, **#129**.
6. Post-tag: **#149**, **#205**, features **#153/#152**, migration (**#2** idea).

## Open decisions for the owner
- **#183 dual-cap**: unify to one `MAX_THREADS`, or keep 32/16 with a justifying comment?
- **Thread precedence** (#149/#183): env var vs R package wins? (dpastoor: package.)
- **#192**: confirm close as redundant + rebase #193 straight to main?
- **Scope/order**: start the standalone issue PRs (#131/#135/#118/#129) now, or clear the
  existing PR backlog (Tracks A/B) first?

_Snapshot 2026-06-17. PR diffs read by 5 parallel sub-agents + direct code/issue review._
