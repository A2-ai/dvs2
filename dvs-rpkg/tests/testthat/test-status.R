test_that("dvs_status on an empty repo returns an empty tibble", {
  new_dvs_test_repo()
  result <- dvs_status()
  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 0L)
})

# Plant one file in each of the three filterable statuses:
#   current.csv  -> Status::Current (added, unchanged)
#   absent.csv   -> Status::Absent (added, then deleted on disk)
#   modified.csv -> Status::Modified (added, then modified on disk)
plant_three_statuses <- function() {
  write_theoph_shuffled("current.csv", seed = 10)
  write_theoph_shuffled("absent.csv", seed = 20)
  write_theoph_shuffled("modified.csv", seed = 30)
  dvs_add(
    paths = file.path(
      getwd(),
      c("current.csv", "absent.csv", "modified.csv")
    )
  )
  file.remove("absent.csv")
  writeLines("totally different content", "modified.csv")
}

test_that("dvs_status() with no filter shows all three statuses", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status()

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 3L)
  expect_setequal(result$status, c("current", "absent", "modified"))
})

test_that("dvs_status(status = 'current') returns only current", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = "current")
  expect_equal(nrow(result), 1L)
  expect_equal(result$status, "current")
})

test_that("dvs_status(status = 'absent') returns only absent", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = "absent")
  expect_equal(nrow(result), 1L)
  expect_equal(result$status, "absent")
})

test_that("dvs_status(status = 'modified') returns only modified", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = "modified")
  expect_equal(nrow(result), 1L)
  expect_equal(result$status, "modified")
})

test_that("dvs_status uses the local baseline to distinguish stale from modified", {
  new_dvs_test_repo()
  writeLines("prior version", "stale.txt")
  dvs_add(paths = file.path(getwd(), "stale.txt"))
  writeLines("new recorded version", "other.txt")
  dvs_add(paths = file.path(getwd(), "other.txt"))
  file.copy(".dvs/other.txt.dvs", ".dvs/stale.txt.dvs", overwrite = TRUE)

  result <- dvs_status(paths = file.path(getwd(), "stale.txt"))
  expect_equal(nrow(result), 1L)
  expect_equal(result$status, "stale")
})

test_that("dvs_status reports unsynced when no local baseline exists", {
  new_dvs_test_repo()
  writeLines("recorded version", "data.txt")
  dvs_add(paths = file.path(getwd(), "data.txt"))
  writeLines("unexplained change", "data.txt")
  unlink(".dvs/.cache", recursive = TRUE)

  result <- dvs_status()
  expect_equal(result$status, "unsynced")
  expect_equal(dvs_status(status = "unsynced")$status, "unsynced")
})

test_that("dvs_status(status = c('current','absent')) returns that pair", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = c("current", "absent"))
  expect_equal(nrow(result), 2L)
  expect_setequal(result$status, c("current", "absent"))
})

test_that("dvs_status(status = c('current','modified')) returns that pair", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = c("current", "modified"))
  expect_equal(nrow(result), 2L)
  expect_setequal(result$status, c("current", "modified"))
})

test_that("dvs_status(status = c('absent','modified')) returns that pair", {
  new_dvs_test_repo()
  plant_three_statuses()

  result <- dvs_status(status = c("absent", "modified"))
  expect_equal(nrow(result), 2L)
  expect_setequal(result$status, c("absent", "modified"))
})

test_that("dvs_status(status = all statuses) == no filter", {
  new_dvs_test_repo()
  plant_three_statuses()

  explicit_all <- dvs_status(status = c("current", "absent", "stale", "modified", "unsynced"))
  default_all <- dvs_status()

  expect_equal(nrow(explicit_all), 3L)
  expect_setequal(explicit_all$status, c("current", "absent", "modified"))
  expect_setequal(explicit_all$path, default_all$path)
  expect_setequal(explicit_all$status, default_all$status)
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

test_that("dvs_status with paths and recursive=TRUE restricts to a subtree", {
  new_dvs_test_repo()
  dir.create("sub")
  write_theoph_shuffled("top.csv", seed = 11)
  write_theoph_shuffled(file.path("sub", "nested.csv"), seed = 12)
  dvs_add(
    paths = c(
      file.path(getwd(), "top.csv"),
      file.path(getwd(), "sub", "nested.csv")
    )
  )

  subtree <- dvs_status(paths = "sub", recursive = TRUE)
  expect_equal(nrow(subtree), 1L)
  expect_equal(basename(subtree$path), "nested.csv")
})

test_that("dvs_status returns POSIXct add_time and dvs_bytes size", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))

  result <- dvs_status()
  expect_s3_class(result$add_time, "POSIXct")
  expect_false(any(is.na(result$add_time)))
  expect_s3_class(result$size, "dvs_bytes")
  expect_equal(typeof(unclass(result$size)), "double")
})

test_that("dvs_status keeps an unreadable record as an error row", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))
  writeLines("broken", file.path(".dvs", "t.csv.dvs"))

  result <- dvs_status(status = "current")
  expect_equal(nrow(result), 1L)
  expect_equal(result$status, "error")
  expect_false(is.na(result$error))
})
