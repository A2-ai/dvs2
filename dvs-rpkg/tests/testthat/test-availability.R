absent <- strrep("0", 64)

test_that("dvs_availability() reports each hash in order, as dvs availability does", {
  repo <- new_dvs_test_repo()
  writeLines("a", "a.csv")
  hash <- dvs_add("a.csv")$hash
  avail <- dvs_availability(c(hash, absent, toupper(hash), hash))
  expect_s3_class(avail, "tbl_df")
  expect_named(avail, c("hash", "state", "code", "message"))
  expect_equal(avail$hash, c(hash, absent, hash, hash))
  expect_equal(avail$state, c("present", "absent", "present", "present"))
  expect_true(all(is.na(avail$code)))
  expect_true(all(is.na(avail$message)))
})

test_that("a storage it cannot read is a row, not an error", {
  repo <- new_dvs_test_repo()
  unlink(repo$storage, recursive = TRUE)
  avail <- dvs_availability(absent)
  expect_equal(avail$state, "unknown")
  expect_equal(avail$code, "io_error")
  expect_type(avail$message, "character")

  server_project("http://127.0.0.1:9/")
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  avail <- dvs_availability(c(absent, absent))
  expect_equal(avail$state, c("unknown", "unknown"))
  expect_equal(avail$code, c("auth_required", "auth_required"))
})

test_that("hashes are checked before any lookup", {
  new_dvs_test_repo()
  cnd <- tryCatch(dvs_availability(c(absent, "abc")), error = identity)
  expect_s3_class(cnd, "dvs_error")
  expect_equal(cnd$code, "invalid_arguments")
  expect_equal(conditionMessage(cnd), "invalid BLAKE3 hash: abc")
  expect_equal(error_code(dvs_availability(character(0))), "invalid_arguments")
  expect_argument_error(quote(dvs_availability(c(absent, NA))), "hashes")
})

test_that("dvs_availability() outside a project fails with not_in_project", {
  withr::local_dir(withr::local_tempdir())
  expect_equal(error_code(dvs_availability(absent)), "not_in_project")
})
