test_that("dvs_get returns only $ok after successful add + local delete", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("data", "a.csv")
  dvs::dvs_add("a.csv")
  file.remove("a.csv")

  res <- dvs::dvs_get("a.csv")
  expect_equal(nrow(res$ok), 1L)
  expect_null(res$err)
})

test_that("dvs_get returns $err with kind=not_tracked for untracked path", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "untracked.csv")
  res <- dvs::dvs_get("untracked.csv")

  expect_equal(nrow(res$ok), 0L)
  expect_equal(nrow(res$err), 1L)
  expect_equal(res$err$kind, "not_tracked")
})
