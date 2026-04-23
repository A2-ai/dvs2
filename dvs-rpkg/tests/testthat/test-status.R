test_that("dvs_status returns add_time as POSIXct", {
  repo <- file.path(tempfile("dvs-repo-"))
  storage <- file.path(tempfile("dvs-storage-"))
  dir.create(repo)

  old_wd <- getwd()
  on.exit(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
    },
    add = TRUE
  )
  setwd(repo)

  # Git repo is required for dvs init (DvsPaths walks up to find .git).
  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "test@example.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "test")) == 0L)

  dvs_init(storage_path = storage, root_dir = repo)

  data_file <- file.path(repo, "data.txt")
  writeLines("hello dvs", data_file)
  dvs_add(paths = data_file)

  result <- dvs_status()
  expect_true("add_time" %in% names(result$ok))
  expect_s3_class(result$ok$add_time, "POSIXct")
  expect_false(any(is.na(result$ok$add_time)))
  expect_s3_class(result$ok$size, "dvs_bytes")

  # Remove the tracked file so dvs_get has something to retrieve.
  # dvs_get resolves paths relative to the repo root (unlike dvs_add,
  # which accepts absolute paths), so feed it the repo-relative basename.
  file.remove(data_file)
  got <- dvs_get(paths = basename(data_file))
  expect_s3_class(got$ok$size, "dvs_bytes")
})

test_that("dvs_status returns $ok only when nothing fails", {
  tmp <- withr::local_tempdir()
  withr::local_dir(tmp)
  dir.create(".git")
  storage <- withr::local_tempdir()
  dvs::dvs_init(storage_path = storage)

  writeLines("x", "a.csv")
  dvs::dvs_add("a.csv")

  res <- dvs::dvs_status()
  expect_true(!is.null(res$ok))
  expect_null(res$err)
  expect_equal(nrow(res$ok), 1L)
  expect_equal(res$ok$path, "a.csv")
})
