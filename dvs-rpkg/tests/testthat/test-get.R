# Commit everything in the test repo.
commit_all <- function(message = "commit") {
  stopifnot(system2("git", c("add", "-A")) == 0L)
  stopifnot(system2("git", c("commit", "-q", "-m", shQuote(message))) == 0L)
}

git <- function(...) stopifnot(system2("git", c(...)) == 0L)

# Call in a new_dvs_test_repo(): track t.csv on two branches with different
# content, then switch back, leaving the `feature` version that dvs wrote.
switch_branches <- function() {
  writeLines("main", "t.csv")
  dvs_add("t.csv")
  commit_all("main")
  git("checkout", "-q", "-b", "feature")
  writeLines("feature", "t.csv")
  dvs_add("t.csv")
  commit_all("feature")
  git("checkout", "-q", "-")
}

get_columns <- c(
  "path", "local_path", "before", "action", "satisfied", "size",
  "target_hash", "local_hash", "stored_copy", "code", "message", "hint"
)

test_that("dvs_get restores a deleted file with identical content", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  original <- readLines("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))
  file.remove("t.csv")

  result <- dvs_get(paths = "t.csv")
  expect_s3_class(result, "dvs_get_result")
  expect_s3_class(result, "tbl_df")
  expect_s3_class(result$size, "dvs_bytes")
  expect_equal(result$action, "restored")
  expect_equal(readLines("t.csv"), original)
  expect_equal(dvs_result_info(result)$outcome, "complete")
})

test_that("dvs_get restores multiple files at once", {
  new_dvs_test_repo()
  write_theoph_shuffled("a.csv", seed = 1)
  write_theoph_shuffled("b.csv", seed = 2)
  dvs_add(paths = file.path(getwd(), c("a.csv", "b.csv")))
  file.remove(c("a.csv", "b.csv"))

  result <- dvs_get(paths = c("a.csv", "b.csv"))
  expect_equal(nrow(result), 2L)
  expect_true(all(file.exists(c("a.csv", "b.csv"))))
})

test_that("dvs_get dry_run does not restore files", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))
  file.remove("t.csv")

  result <- dvs_get(paths = "t.csv", dry_run = TRUE)
  expect_equal(result$action, "would_restore")
  expect_true(result$satisfied)
  expect_false(file.exists("t.csv"))
})

test_that("dvs_get on a current file reports 'unchanged'", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))

  result <- dvs_get(paths = "t.csv")
  expect_equal(result$before, "current")
  expect_equal(result$action, "unchanged")
})

test_that("dvs_get(recursive = TRUE) with no paths restores every tracked file at every depth", {
  new_dvs_test_repo()
  write_theoph("top.csv")
  write_theoph_shuffled("data/raw/deep.csv", seed = 1)
  dvs_add(paths = file.path(getwd(), c("top.csv", "data/raw/deep.csv")))
  file.remove(c("top.csv", "data/raw/deep.csv"))

  result <- dvs_get(recursive = TRUE)
  expect_setequal(result$path, c("top.csv", "data/raw/deep.csv"))
  expect_true(all(file.exists(c("top.csv", "data/raw/deep.csv"))))
})

test_that("an untracked path is a row, and holds the batch back by default", {
  new_dvs_test_repo()
  write_theoph("tracked.csv")
  dvs_add(paths = file.path(getwd(), "tracked.csv"))
  file.remove("tracked.csv")

  result <- dvs_get(paths = c("tracked.csv", "untracked.csv"))
  expect_equal(dvs_result_info(result)$outcome, "aborted")
  expect_equal(result$code[result$path == "untracked.csv"], "not_tracked")
  expect_equal(result$code[result$path == "tracked.csv"], "batch_aborted")
  expect_false(file.exists("tracked.csv"))

  result <- dvs_get(paths = c("tracked.csv", "untracked.csv"), on_conflict = "skip")
  expect_equal(result$action[result$path == "tracked.csv"], "restored")
})

test_that("after a branch switch, files dvs wrote are replaced without asking", {
  new_dvs_test_repo()
  switch_branches()
  expect_equal(readLines("t.csv"), "feature")
  result <- dvs_get("t.csv")
  expect_equal(result$before, "clean")
  expect_equal(result$action, "replaced")
  expect_equal(readLines("t.csv"), "main")
})

test_that("local changes abort by default and are kept", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  result <- dvs_get("t.csv")
  expect_equal(result$before, "modified")
  expect_equal(result$action, "not_run")
  expect_equal(result$code, "local_changes")
  expect_false(result$satisfied)
  expect_match(result$hint, "dvs_add", fixed = TRUE)
  expect_equal(readLines("t.csv"), "my edit")
  info <- dvs_result_info(result)
  expect_equal(info$outcome, "aborted")
  expect_equal(info$on_conflict, "abort")
  expect_equal(info$source, "default")
})

test_that("on_conflict = 'skip' and 'overwrite'", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  expect_equal(dvs_get("t.csv", on_conflict = "skip")$action, "skipped")
  expect_equal(readLines("t.csv"), "my edit")
  result <- dvs_get("t.csv", on_conflict = "overwrite")
  expect_equal(result$action, "replaced")
  expect_equal(dvs_result_info(result)$source, "argument")
  expect_equal(readLines("t.csv"), "main")
})

test_that("DVS_GET_ON_CONFLICT is used when the argument is NULL", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  withr::local_envvar(DVS_GET_ON_CONFLICT = "skip")
  result <- dvs_get("t.csv")
  expect_equal(result$action, "skipped")
  expect_equal(dvs_result_info(result)$source, "env")
})

test_that("results keep a fixed schema, even with zero rows outside a project", {
  dir <- withr::local_tempdir()
  withr::local_dir(dir)
  result <- dvs_get()
  expect_equal(names(result), get_columns)
  expect_equal(nrow(result), 0L)
  expect_type(result$satisfied, "logical")
  expect_type(result$code, "character")
  info <- dvs_result_info(result)
  expect_equal(info$outcome, "failed")
  expect_match(info$problem, "not in a dvs project")
})

test_that("on_error controls signalling, and conditions carry the result", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  expect_silent(res <- dvs_get("t.csv"))
  expect_warning(dvs_get("t.csv", on_error = "warn"), class = "dvs_get_incomplete")
  cnd <- tryCatch(dvs_get("t.csv", on_error = "error"), dvs_get_incomplete = identity)
  expect_s3_class(cnd, "dvs_error")
  expect_equal(cnd$result$code, "local_changes")
  withr::local_options(dvs.on_error = "error")
  expect_error(dvs_get("t.csv"), class = "dvs_get_incomplete")
})

test_that("the default return survives options(warn = 2)", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  withr::local_options(warn = 2)
  res <- dvs_get("t.csv")
  expect_equal(res$code, "local_changes")
})

test_that("overwrite in dvs.toml is a configuration error", {
  new_dvs_test_repo()
  cat('\n[get]\non_conflict = "overwrite"\n', file = "dvs.toml", append = TRUE)
  expect_error(dvs_get(), class = "dvs_config_error")
})

test_that("printing shows the outcome header", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  out <- capture.output(print(dvs_get("t.csv")))
  expect_match(out[1], "dvs get: aborted", fixed = TRUE)
  expect_match(out[1], "on_conflict = abort (default)", fixed = TRUE)
  expect_false(grepl("NA", out[1], fixed = TRUE))
  expect_true(is.na(dvs_result_info(dvs_get("t.csv"))$problem))
})

test_that("printing lists what needs attention, and filtered results say so", {
  new_dvs_test_repo()
  switch_branches()
  writeLines("my edit", "t.csv")
  writeLines("u", "u.csv")
  dvs_add("u.csv")
  file.remove("u.csv")
  res <- dvs_get(c("t.csv", "u.csv"))
  out <- capture.output(print(res))
  expect_match(out[1], "1 with local changes or no record", fixed = TRUE)
  expect_true(any(grepl("! t.csv: local changes", out, fixed = TRUE)))
  expect_true(any(grepl('dvs_add("t.csv")', out, fixed = TRUE)))
  expect_false(any(grepl("! u.csv", out, fixed = TRUE)))

  out <- capture.output(print(res[res$path == "t.csv", ]))
  expect_match(out[1], "1 of 2 rows from: dvs get: aborted", fixed = TRUE)

  cnd <- tryCatch(dvs_get(c("t.csv", "u.csv"), on_error = "error"), error = identity)
  expect_match(conditionMessage(cnd), "! t.csv: local changes", fixed = TRUE)
})

test_that("a merge conflict in a .dvs file holds the batch back", {
  new_dvs_test_repo()
  writeLines("a", "a.csv")
  writeLines("b", "b.csv")
  dvs_add(c("a.csv", "b.csv"))
  file.remove("a.csv")
  writeLines(c("<<<<<<< HEAD", "{}", "=======", "{}", ">>>>>>> other"), file.path(".dvs", "b.csv.dvs"))
  res <- dvs_get(c("a.csv", "b.csv"))
  expect_equal(res$code[res$path == "b.csv"], "bad_metadata")
  expect_equal(res$code[res$path == "a.csv"], "batch_aborted")
  expect_false(file.exists("a.csv"))
})
