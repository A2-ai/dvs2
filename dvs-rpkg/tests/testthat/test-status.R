test_that("dvs_status on an empty repo returns an empty tibble", {
  new_dvs_test_repo()
  result <- dvs_status()
  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 0L)
})

test_that("dvs_status() defaults to showing ALL statuses (not only 'current')", {
  new_dvs_test_repo()

  write_theoph_shuffled("current.csv", seed = 10)
  write_theoph_shuffled("absent.csv", seed = 20)
  write_theoph_shuffled("unsynced.csv", seed = 30)
  dvs_add(
    paths = file.path(
      getwd(),
      c("current.csv", "absent.csv", "unsynced.csv")
    )
  )

  file.remove("absent.csv")
  writeLines("totally different content", "unsynced.csv")

  result <- dvs_status()

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 3L)
  expect_setequal(result$status, c("current", "absent", "unsynced"))
})

test_that("dvs_status filters by status", {
  new_dvs_test_repo()

  write_theoph_shuffled("a.csv", seed = 1)
  write_theoph_shuffled("b.csv", seed = 2)
  write_theoph_shuffled("c.csv", seed = 3)
  dvs_add(paths = file.path(getwd(), c("a.csv", "b.csv", "c.csv")))

  file.remove("b.csv")
  writeLines("different", "c.csv")

  only_current <- dvs_status(status = "current")
  only_absent <- dvs_status(status = "absent")
  absent_or_unsynced <- dvs_status(status = c("absent", "unsynced"))

  expect_equal(nrow(only_current), 1L)
  expect_equal(only_current$status, "current")

  expect_equal(nrow(only_absent), 1L)
  expect_equal(only_absent$status, "absent")

  expect_equal(nrow(absent_or_unsynced), 2L)
  expect_setequal(absent_or_unsynced$status, c("absent", "unsynced"))
})

test_that("dvs_status ignores untracked files (files never added)", {
  new_dvs_test_repo()

  write_theoph_shuffled("tracked.csv", seed = 5)
  dvs_add(paths = file.path(getwd(), "tracked.csv"))
  write_theoph_shuffled("untracked.csv", seed = 6)

  result <- dvs_status()
  expect_equal(nrow(result), 1L)
  expect_equal(basename(result$path), "tracked.csv")
})

test_that("dvs_status returns POSIXct add_time and dvs_bytes size", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))

  result <- dvs_status()
  expect_s3_class(result$add_time, "POSIXct")
  expect_false(any(is.na(result$add_time)))
  expect_s3_class(result$size, "dvs_bytes")
  expect_true(typeof(unclass(result$size)) %in% c("integer", "double"))
})
