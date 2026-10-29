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

test_that("dvs_audit_log takes paths = NULL as no filter", {
  new_dvs_test_repo()
  write_theoph("theoph.csv")
  dvs_add(paths = "theoph.csv")
  expect_equal(dvs_audit_log(paths = NULL), dvs_audit_log())
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

test_that("dvs_audit_log flattens the action into columns with a POSIXct timestamp", {
  new_dvs_test_repo()
  writeLines("a", "a.csv")
  dvs_add(paths = file.path(getwd(), "a.csv"), message = "why")

  log <- dvs_audit_log()
  expect_s3_class(log$timestamp, "POSIXct")
  expect_equal(log$action_variant, c("init", "add"))
  expect_equal(log$action_file_path, c(NA, "a.csv"))
  expect_equal(log$action_message, c(NA, "why"))
  expect_false("action" %in% names(log))
})

test_that("dvs_audit_log raises an NA path as a dvs_error_argument", {
  new_dvs_test_repo()
  expect_argument_error(quote(dvs_audit_log(paths = NA_character_)), "paths")
})
