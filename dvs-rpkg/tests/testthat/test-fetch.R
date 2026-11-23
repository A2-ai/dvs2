test_that("dvs_fetch writes an earlier version and returns where it is", {
  new_dvs_test_repo()
  writeLines("old", "t.csv")
  old <- substr(dvs_add("t.csv")$hash, 1, 12)
  writeLines("new", "t.csv")
  dvs_add("t.csv")

  written <- dvs_fetch("t.csv", old)
  expect_equal(written, file.path(".dvs-scratchpad", old, "t.csv"))
  expect_equal(readLines(written), "old")
  expect_equal(readLines("t.csv"), "new")

  expect_error(dvs_fetch("t.csv", "000000000000"), "has no version", class = "dvs_error")
})
