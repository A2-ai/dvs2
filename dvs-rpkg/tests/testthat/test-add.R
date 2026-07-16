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
  expect_equal(result$outcome, "copied")
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
  expect_true(all(result$outcome == "copied"))
  expect_true(all(result$size > 0))
})

test_that("re-adding the same path reports 'present'", {
  new_dvs_test_repo()
  write_theoph("a.csv")

  first <- dvs_add(paths = file.path(getwd(), "a.csv"))
  second <- dvs_add(paths = file.path(getwd(), "a.csv"))

  expect_equal(first$outcome, "copied")
  expect_equal(second$outcome, "present")
  expect_equal(first$hash, second$hash)
})

test_that("two files with identical content share a hash but both 'copied'", {
  new_dvs_test_repo()
  write_theoph("a.csv")
  file.copy("a.csv", "b.csv")

  first <- dvs_add(paths = file.path(getwd(), "a.csv"))
  second <- dvs_add(paths = file.path(getwd(), "b.csv"))

  expect_equal(first$hash, second$hash)
  expect_equal(first$outcome, "copied")
  expect_equal(second$outcome, "copied")
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
  expect_equal(result$outcome, "copied")
})

test_that("dvs_add refuses the whole batch when any path is invalid", {
  new_dvs_test_repo(); write_theoph("ok.csv")
  expect_error(dvs_add(paths = file.path(getwd(), c("ok.csv", "missing.csv"))))
  expect_false(file.exists(file.path(getwd(), ".dvs", "ok.csv.dvs")))
})

test_that("dvs_add reports per-file failures in the result, not as a warning", {
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

  # A per-file runtime failure must not raise an R warning or error. dvs does
  # not turn its own failures into R conditions. The failure is reported only
  # through the returned data frame.
  result <- NULL
  expect_no_warning(
    result <- dvs_add(paths = file.path(getwd(), c("good.csv", "noperm.csv")))
  )

  # The good file is added. The bad file's row carries the error message in the
  # `error` column, with its `outcome` left NA.
  expect_true(file.exists(file.path(getwd(), ".dvs", "good.csv.dvs")))
  good <- result[basename(result$path) == "good.csv", ]
  expect_equal(good$outcome, "copied")
  bad <- result[basename(result$path) == "noperm.csv", ]
  expect_equal(nrow(bad), 1L)
  expect_true(nzchar(bad$error))
  expect_true(is.na(bad$outcome))
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
  expect_equal(result$outcome, "copied")
  # metadata sidecar mirrors the repo-root-relative path
  expect_true(file.exists(
    file.path(repo, ".dvs", "analysis", "run1", "data.csv.dvs")
  ))
})
