test_that("dvs_get restores a deleted file with identical content", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  original <- readLines("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))
  file.remove("t.csv")
  expect_false(file.exists("t.csv"))

  result <- dvs_get(paths = "t.csv")
  expect_s3_class(result, "tbl_df")
  expect_s3_class(result$size, "dvs_bytes")
  expect_true(file.exists("t.csv"))
  expect_equal(readLines("t.csv"), original)
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
  expect_s3_class(result, "tbl_df")
  expect_false(file.exists("t.csv"))
})

test_that("dvs_get when file is already present reports 'present'", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add(paths = file.path(getwd(), "t.csv"))

  result <- dvs_get(paths = "t.csv")
  expect_equal(result$outcome, "present")
})

test_that("dvs_get on never-added path errors", {
  new_dvs_test_repo()
  expect_error(dvs_get(paths = "never-added.csv"))
})

test_that("dvs_get() with no args restores every tracked file at every depth", {
  new_dvs_test_repo()
  write_theoph("top.csv")
  write_theoph_shuffled("data/raw/deep.csv", seed = 1)
  dvs_add(paths = file.path(getwd(), c("top.csv", "data/raw/deep.csv")))
  file.remove(c("top.csv", "data/raw/deep.csv"))

  result <- dvs_get()
  expect_s3_class(result, "tbl_df")
  expect_setequal(result$path, c("top.csv", "data/raw/deep.csv"))
  expect_true(all(file.exists(c("top.csv", "data/raw/deep.csv"))))
})
