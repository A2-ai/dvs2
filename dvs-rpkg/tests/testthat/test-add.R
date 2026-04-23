test_that("dvs_add returns only $ok when all paths succeed", {
  skip_if_not(nzchar(Sys.which("Rscript")))
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("hello", "a.csv")
  res <- dvs::dvs_add("a.csv")

  expect_true(!is.null(res$ok))
  expect_null(res$err)
  expect_equal(nrow(res$ok), 1L)
  expect_equal(res$ok$path, "a.csv")
})

test_that("dvs_add returns $ok and $err with kind column on mixed inputs", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "ok.csv")
  res <- dvs::dvs_add(c("ok.csv", "missing.csv"))

  expect_equal(nrow(res$ok), 1L)
  expect_equal(nrow(res$err), 1L)
  expect_true("kind" %in% names(res$err))
  expect_equal(res$err$kind, "not_found")
})

test_that("dvs_add returns zero-row $ok and populated $err when all fail", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  res <- dvs::dvs_add(c("nope1.csv", "nope2.csv"))

  expect_equal(nrow(res$ok), 0L)
  expect_equal(nrow(res$err), 2L)
  expect_true(all(res$err$kind == "not_found"))
})
