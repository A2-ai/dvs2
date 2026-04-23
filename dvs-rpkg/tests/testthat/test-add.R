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
  expect_equal(typeof(unclass(result$size)), "integer")
  expect_equal(typeof(unclass(result$stored_size)), "integer")
  expect_true(result$size > 0L)
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
  expect_true(all(result$size > 0L))
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
