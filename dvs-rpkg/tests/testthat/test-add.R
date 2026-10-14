test_that("dvs_add on a single Theoph CSV returns a tibble with bytes columns", {
  new_dvs_test_repo()
  write_theoph("theoph.csv")

  result <- dvs_add(paths = file.path(getwd(), "theoph.csv"))

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 1L)
  expect_setequal(
    names(result),
    c("path", "outcome", "hash", "size", "stored_size")
  )
  expect_s3_class(result$size, "dvs_bytes")
  expect_s3_class(result$stored_size, "dvs_bytes")
  expect_equal(typeof(unclass(result$size)), "double")
  expect_equal(typeof(unclass(result$stored_size)), "double")
  expect_true(result$size > 0)
  expect_true(nzchar(result$hash))
  expect_equal(result$outcome, "added")
})

test_that("dvs_add on several shuffled Theoph variants produces distinct hashes", {
  new_dvs_test_repo()

  files <- file.path(getwd(), c("theoph_1.csv", "theoph_2.csv", "theoph_3.csv"))
  write_theoph_shuffled(files[1], seed = 1)
  write_theoph_shuffled(files[2], seed = 2)
  write_theoph_shuffled(files[3], seed = 3)

  result <- dvs_add(paths = files)

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 3L)
  expect_equal(length(unique(result$hash)), 3L)
  expect_true(all(result$outcome == "added"))
  expect_true(all(result$size > 0))
})

test_that("re-adding an unchanged path reports 'skipped'", {
  new_dvs_test_repo()
  write_theoph("a.csv")

  first <- dvs_add(paths = file.path(getwd(), "a.csv"))
  second <- dvs_add(paths = file.path(getwd(), "a.csv"))

  expect_equal(first$outcome, "added")
  expect_equal(second$outcome, "skipped")
  expect_equal(first$hash, second$hash)
})

test_that("a copy of added content is recorded as 'present'", {
  new_dvs_test_repo()
  write_theoph("a.csv")
  file.copy("a.csv", "b.csv")

  first <- dvs_add(paths = file.path(getwd(), "a.csv"))
  second <- dvs_add(paths = file.path(getwd(), "b.csv"))

  expect_equal(first$hash, second$hash)
  expect_equal(first$outcome, "added")
  # The content is already stored, so only the path is recorded
  expect_equal(second$outcome, "present")
  expect_true(file.exists(file.path(getwd(), ".dvs", "b.csv.dvs")))
})

test_that("dvs_add dry_run returns a tibble but writes nothing to storage", {
  ctx <- new_dvs_test_repo()
  write_theoph("t.csv")

  storage_before <- list.files(ctx$storage, recursive = TRUE)
  result <- dvs_add(paths = file.path(getwd(), "t.csv"), dry_run = TRUE)
  storage_after <- list.files(ctx$storage, recursive = TRUE)

  expect_s3_class(result, "tbl_df")
  expect_s3_class(result$size, "dvs_bytes")
  expect_equal(storage_before, storage_after)
  expect_false(file.exists(file.path(getwd(), ".dvs", "t.csv.dvs")))
})

test_that("dvs_add on nonexistent path errors", {
  new_dvs_test_repo()
  expect_error(
    dvs_add(paths = file.path(getwd(), "does-not-exist.csv"))
  )
})

test_that("dvs_add with glob picks up matching files", {
  new_dvs_test_repo()
  write_theoph_shuffled("data_a.csv", seed = 7)
  write_theoph_shuffled("data_b.csv", seed = 8)
  writeLines("other", "notes.txt")

  result <- dvs_add(glob = "*.csv")
  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 2L)
  expect_setequal(basename(result$path), c("data_a.csv", "data_b.csv"))
})

test_that("dvs_add accepts a message without error", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  result <- dvs_add(
    paths = file.path(getwd(), "t.csv"),
    message = "added theoph"
  )
  expect_equal(result$outcome, "added")
})

test_that("dvs_add refuses the whole batch when any path is invalid", {
  new_dvs_test_repo(); write_theoph("ok.csv")
  expect_error(dvs_add(paths = file.path(getwd(), c("ok.csv", "missing.csv"))))
  expect_false(file.exists(file.path(getwd(), ".dvs", "ok.csv.dvs")))
})

test_that("dvs_add stops on per-file failures with every file's result, or collects them", {
  # This test forces a per-file failure by revoking all permissions on
  # noperm.csv, so opening it for hashing fails with EACCES. root bypasses
  # Unix permission bits and can still read a mode-000 file, which would make
  # the add succeed and leave no failure to report. Skip under root so the test
  # does not report a false failure (e.g. in CI/Docker running as root).
  skip_if(system("id -un", intern = TRUE) == "root", "cannot revoke read access from root")
  new_dvs_test_repo()
  write_theoph("good.csv")
  write_theoph("noperm.csv")
  Sys.chmod("noperm.csv", "000")
  on.exit(Sys.chmod("noperm.csv", "644"), add = TRUE)

  # By default a failed file stops with a files_failed error that carries
  # every file's result; the good file is still added.
  e <- tryCatch(
    dvs_add(paths = file.path(getwd(), c("good.csv", "noperm.csv"))),
    error = function(e) e
  )
  expect_s3_class(e, c("dvs_error_files_failed", "dvs_error", "error", "condition"), exact = TRUE)
  expect_match(conditionMessage(e), "1 of 2 files failed to add")
  expect_match(conditionMessage(e), "noperm.csv: ")
  expect_true(file.exists(file.path(getwd(), ".dvs", "good.csv.dvs")))
  expect_s3_class(e$results, "tbl_df")
  expect_equal(e$results$outcome[basename(e$results$path) == "noperm.csv"], "failed")

  # Collected, the failure is reported only through the returned data frame,
  # without a warning.
  result <- NULL
  expect_no_warning(
    result <- dvs_add(paths = file.path(getwd(), c("good.csv", "noperm.csv")), on_error = "collect")
  )
  good <- result[basename(result$path) == "good.csv", ]
  expect_equal(good$outcome, "skipped")
  bad <- result[basename(result$path) == "noperm.csv", ]
  expect_equal(nrow(bad), 1L)
  expect_true(nzchar(bad$error))
  expect_equal(bad$outcome, "failed")
})

test_that("dvs_add resolves a bare relative path from a nested working directory", {
  repo <- new_dvs_test_repo()$repo
  nested <- file.path(repo, "analysis", "run1")
  dir.create(nested, recursive = TRUE)
  setwd(nested)
  write_theoph("data.csv")

  # CLI parity: `dvs add data.csv` works from a subdirectory, so must dvs_add.
  result <- dvs_add(paths = "data.csv")

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 1L)
  expect_equal(result$outcome, "added")
  # metadata sidecar mirrors the repo-root-relative path
  expect_true(file.exists(
    file.path(repo, ".dvs", "analysis", "run1", "data.csv.dvs")
  ))
})
