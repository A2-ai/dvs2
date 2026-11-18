test_that("dvs_fetch writes an earlier version to the scratchpad", {
  new_dvs_test_repo()
  writeLines("old", "t.csv")
  old <- dvs_add("t.csv")$hash
  writeLines("new", "t.csv")
  dvs_add("t.csv")

  result <- dvs_fetch("t.csv", substr(old, 1, 8))
  expect_equal(
    names(result),
    c(
      "path", "local_path", "before", "action", "satisfied", "size",
      "target_hash", "local_hash", "stored_copy", "code", "message", "hint",
      "duration_ms", "written_to", "hash"
    )
  )
  expect_equal(result$action, "restored")
  expect_equal(result$hash, old)
  expect_equal(result$written_to, file.path(".dvs-scratchpad", substr(old, 1, 8), "t.csv"))
  expect_equal(readLines(result$written_to), "old")
  expect_equal(readLines("t.csv"), "new")
})
