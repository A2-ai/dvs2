test_that("dvs_audit_log returns the init and add entries", {
  new_dvs_test_repo()
  write_theoph("theoph.csv")
  dvs_add(paths = file.path(getwd(), "theoph.csv"))

  log <- dvs_audit_log()

  expect_s3_class(log, "tbl_df")
  # new_dvs_test_repo() runs dvs_init, then one dvs_add: an init entry plus an
  # add entry.
  expect_gte(nrow(log), 2L)
})

test_that("dvs_audit_log filters to the add entry for the given path", {
  new_dvs_test_repo()
  write_theoph("theoph.csv")
  dvs_add(paths = file.path(getwd(), "theoph.csv"))

  # Filter by the repo-root-relative path recorded at add time. A non-empty
  # filter keeps only the matching add entry and drops the init entry.
  filtered <- dvs_audit_log(paths = "theoph.csv")

  expect_s3_class(filtered, "tbl_df")
  expect_equal(nrow(filtered), 1L)
})

test_that("dvs_audit_log with a non-matching path returns zero rows without erroring", {
  new_dvs_test_repo()
  write_theoph("theoph.csv")
  dvs_add(paths = file.path(getwd(), "theoph.csv"))

  bogus <- expect_no_error(dvs_audit_log(paths = "does-not-exist.csv"))

  expect_s3_class(bogus, "tbl_df")
  expect_equal(nrow(bogus), 0L)
})

test_that("dvs_audit_log on a server project points to the server's history", {
  repo <- tempfile("dvs-repo-")
  dir.create(repo)
  old_wd <- setwd(repo)
  on.exit({
    setwd(old_wd)
    unlink(repo, recursive = TRUE, force = TRUE)
  }, add = TRUE)
  stopifnot(system2("git", c("init", "-q")) == 0L)
  writeLines(c("[backend]", 'name = "demo"', 'url = "http://127.0.0.1:9/"'), "dvs.toml")
  dir.create(".dvs")

  # Its history lives on the server; nothing is read locally or over the network
  expect_error(dvs_audit_log(), "dvs_log\\(\\)")
})
