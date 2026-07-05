# Branch Audit — unmerged local work (handoff for next session)

Generated during PR-backlog triage. Goal: surface local branches that may carry
commits never landed on `main`, so they can be investigated (recovered or pruned).

## How to read this

- **`git cherry origin/main <branch>`** counts commits whose *patch-id* is not in
  `main` (the `unmerged` column). **This massively over-reports.** Squash-merges and
  rebases change patch-ids, so a branch whose content fully landed via a squashed PR
  still shows a high `unmerged` count. A 2026-02 branch with `unmerged=45` is almost
  certainly merged-by-squash, not lost work.
- **`last=` (last-commit date) is the real triage signal.** Recent branches with
  unmerged patches that are *not* open-PR heads are the ones worth a look first.
- Open-PR head branches are listed separately — they are accounted for (they land via
  their PRs) and need no investigation.

## Investigation method (next session)

For any candidate, confirm whether its work is truly absent from main:
```
git log --oneline --cherry-mark --right-only origin/main...<branch>   # '>' = only on branch
git range-diff origin/main...<branch>                                  # content-level compare
git log -1 --format='%cs %s' <branch>                                  # date + subject
```
If every commit shows as already-in-main (squash), the branch is safe to delete.
If real unmerged work exists, collate into a recovery PR off `main`.

---

## Candidate orphan branches (NOT open-PR heads), newest first
feat/verify-thread-priority-chain-base             last=2026-06-17 ahead=6    unmerged=6
validate/170-on-207                                last=2026-06-17 ahead=2    unmerged=2
error-state-as-dataframe-base                      last=2026-06-17 ahead=2    unmerged=2
feat/dvs-init-returns-tibble-base                  last=2026-06-17 ahead=1    unmerged=1
rescaffold/miniextendr-latest                      last=2026-05-17 ahead=1    unmerged=1
fix/resolve-paths-pass-through                     last=2026-05-17 ahead=1    unmerged=1
docs/internals                                     last=2026-05-14 ahead=1    unmerged=1
fix/dvs-init-compression-vector                    last=2026-05-05 ahead=3    unmerged=3
rpkg-document-sync                                 last=2026-05-04 ahead=2    unmerged=1
docs/specs-cli-help-refresh                        last=2026-05-04 ahead=1    unmerged=1
feat/rpkg-log-routing-and-worker-pump              last=2026-05-01 ahead=5    unmerged=5
chore/refresh-rpkg-scaffolding                     last=2026-04-30 ahead=4    unmerged=4
feat/rpkg-log-to-r-console-pr                      last=2026-04-30 ahead=1    unmerged=1
feat/rpkg-log-to-r-console                         last=2026-04-30 ahead=1    unmerged=1
chore/ui-rollup                                    last=2026-04-29 ahead=5    unmerged=5
docs/rpkg-stale-cargo-pin                          last=2026-04-29 ahead=1    unmerged=1
chore/track-alx-config                             last=2026-04-29 ahead=1    unmerged=1
backup/spec-review-april-before-specs-md-restore   last=2026-04-23 ahead=2    unmerged=1
rpkg-release-workflow                              last=2026-04-21 ahead=9    unmerged=9
fix/get-absolute-canonicalize                      last=2026-04-21 ahead=5    unmerged=4
chore/upgrade-miniextendr-scaffolding              last=2026-04-19 ahead=1    unmerged=1
fix-windows-unused-anyhow-import                   last=2026-04-17 ahead=2    unmerged=2
demo-work-changes                                  last=2026-04-16 ahead=8    unmerged=8
update-miniextendr                                 last=2026-04-15 ahead=7    unmerged=7
investigate/windows-ci-vendor                      last=2026-04-15 ahead=7    unmerged=7
enum-match-arg                                     last=2026-04-15 ahead=10   unmerged=10
fix/vendor-offline-build                           last=2026-04-15 ahead=1    unmerged=1
remove-lfs                                         last=2026-04-14 ahead=3    unmerged=3
dvs-rpkg-cleanup                                   last=2026-04-14 ahead=198  unmerged=16
dvs_rpkg_parallel                                  last=2026-04-14 ahead=189  unmerged=8
progress-rpkg-cli                                  last=2026-04-10 ahead=224  unmerged=45
update-miniextendr-scaffolding                     last=2026-04-10 ahead=184  unmerged=4
progress-rpkg                                      last=2026-04-08 ahead=188  unmerged=9
origin-progress                                    last=2026-04-07 ahead=181  unmerged=2
rpkg-status-metadata                               last=2026-04-03 ahead=180  unmerged=2
progress                                           last=2026-04-03 ahead=179  unmerged=1
miniextendr-update                                 last=2026-04-02 ahead=181  unmerged=4
improve-rpkg-claude-md                             last=2026-04-02 ahead=178  unmerged=1
dvs_rpkg_tibble                                    last=2026-04-02 ahead=168  unmerged=5
audit-log-perms                                    last=2026-03-24 ahead=176  unmerged=1
perms                                              last=2026-03-24 ahead=172  unmerged=2
ui_cli_rpkg                                        last=2026-03-21 ahead=172  unmerged=12
status-subfolder                                   last=2026-03-20 ahead=164  unmerged=1
dvs-rpkg-document                                  last=2026-03-19 ahead=162  unmerged=3
dvs-rpkg-rv-test                                   last=2026-03-19 ahead=162  unmerged=1
status-update                                      last=2026-03-18 ahead=161  unmerged=1
warn_storage_in_repo                               last=2026-03-18 ahead=160  unmerged=3
bump_version                                       last=2026-03-18 ahead=159  unmerged=3
thread_pool_v2                                     last=2026-03-18 ahead=157  unmerged=7
init-noop                                          last=2026-03-18 ahead=157  unmerged=1
uphaul_dvs_rpkg                                    last=2026-03-17 ahead=159  unmerged=8
fix/cleanup-scaffolding                            last=2026-03-17 ahead=154  unmerged=1
audit-fixes                                        last=2026-03-17 ahead=152  unmerged=1
add-fixes                                          last=2026-03-16 ahead=154  unmerged=4
rpkg_update                                        last=2026-03-15 ahead=152  unmerged=2
vvv                                                last=2026-03-12 ahead=202  unmerged=52
verbose-all                                        last=2026-03-11 ahead=158  unmerged=10
verbose_add                                        last=2026-03-11 ahead=153  unmerged=5
spec_status                                        last=2026-03-11 ahead=153  unmerged=5
spec_init                                          last=2026-03-11 ahead=153  unmerged=5
spec_get                                           last=2026-03-11 ahead=152  unmerged=4
spec_delete                                        last=2026-03-11 ahead=152  unmerged=4
spec_audit                                         last=2026-03-11 ahead=152  unmerged=4
spec_add                                           last=2026-03-11 ahead=151  unmerged=3
initial_spec_misc                                  last=2026-03-11 ahead=151  unmerged=3
spec_sync                                          last=2026-03-11 ahead=150  unmerged=2
fix/clippy-too-many-args                           last=2026-03-11 ahead=149  unmerged=1
fix/canonicalize-outside-project                   last=2026-03-11 ahead=149  unmerged=1
bench_cache2                                       last=2026-03-06 ahead=159  unmerged=12
ui_test_scripts                                    last=2026-03-04 ahead=140  unmerged=1
single-out-project-check                           last=2026-02-27 ahead=140  unmerged=2
spec_update_01                                     last=2026-02-27 ahead=138  unmerged=3
parallel_sqlite                                    last=2026-02-26 ahead=143  unmerged=8
parallel                                           last=2026-02-26 ahead=141  unmerged=6
out_of_tree                                        last=2026-02-25 ahead=137  unmerged=2
finish-add                                         last=2026-02-23 ahead=138  unmerged=5
dvs_get_story                                      last=2026-02-22 ahead=160  unmerged=31
selective-hash                                     last=2026-02-22 ahead=135  unmerged=2
backup-update_miniextendr-pre-squash-20260218-154806 last=2026-02-18 ahead=160  unmerged=32
dvs_get_story_no_spec                              last=2026-02-18 ahead=157  unmerged=28
design                                             last=2026-02-16 ahead=140  unmerged=14
hashes-cache                                       last=2026-02-16 ahead=127  unmerged=1
rpkg_audit2                                        last=2026-02-13 ahead=134  unmerged=9
new-things                                         last=2026-02-05 ahead=113  unmerged=3
rpkg_audit_log                                     last=2026-02-04 ahead=111  unmerged=1
rpkg_audit                                         last=2026-02-04 ahead=111  unmerged=1
rpkg_many_files                                    last=2026-02-03 ahead=117  unmerged=7
arg_multipaths2                                    last=2026-02-01 ahead=116  unmerged=8
rpkg-build-improvements                            last=2026-02-01 ahead=113  unmerged=5
glob_as_arg                                        last=2026-02-01 ahead=111  unmerged=2
allow_no_message                                   last=2026-01-30 ahead=115  unmerged=8
as_data_frame                                      last=2026-01-30 ahead=114  unmerged=8
rpkg_do_dvs_init                                   last=2026-01-29 ahead=126  unmerged=23
add_dvs-rpkg                                       last=2026-01-29 ahead=125  unmerged=22
impl_rpackage_bindings                             last=2026-01-29 ahead=118  unmerged=16
rpkg_dvs_add                                       last=2026-01-29 ahead=111  unmerged=8
rpkg_do_dvs_init_clean                             last=2026-01-29 ahead=105  unmerged=2

## Open-PR head branches (accounted for — will land via their PRs)

ui/threads                                         last=2026-06-17 ahead=7    unmerged=7
feat/verify-thread-priority-chain                  last=2026-06-17 ahead=7    unmerged=7
ui/events                                          last=2026-06-17 ahead=3    unmerged=3
ui/init                                            last=2026-06-17 ahead=2    unmerged=2
feat/dvs-get-recursive-core                        last=2026-06-17 ahead=2    unmerged=2
fix/progress-bar-interactive-only                  last=2026-06-17 ahead=1    unmerged=1
fix/lift-storage-guard-into-core                   last=2026-06-17 ahead=1    unmerged=1
fix/audit-timestamp-jiff                           last=2026-06-17 ahead=1    unmerged=1
docs/specs-high-level-conventions                  last=2026-06-17 ahead=1    unmerged=1
chore/rescaffold-miniextendr-latest                last=2026-06-17 ahead=1    unmerged=1
spec-review-april-2                                last=2026-05-17 ahead=5    unmerged=5
feat/ui-main-recursive                             last=2026-05-17 ahead=3    unmerged=3
fix/r-pkg-warnings                                 last=2026-05-17 ahead=2    unmerged=2
feat/dvs-rpkg-get-recursive                        last=2026-05-17 ahead=2    unmerged=2
feat/dvs-init-returns-tibble                       last=2026-05-17 ahead=2    unmerged=2
error-state-as-dataframe                           last=2026-05-17 ahead=2    unmerged=2
filters                                            last=2026-02-18 ahead=128  unmerged=1
migrate                                            last=2026-01-30 ahead=108  unmerged=3
