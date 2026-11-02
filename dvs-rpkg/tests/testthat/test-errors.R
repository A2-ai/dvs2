# The code of a failed command, from a condition that must be a dvs_error.
error_code <- function(expr) {
  cnd <- tryCatch(expr, error = identity)
  expect_s3_class(cnd, "dvs_error")
  cnd$code
}

test_that("a command outside a project fails with not_in_project", {
  withr::local_dir(withr::local_tempdir())
  expect_equal(error_code(dvs_status()), "not_in_project")
  expect_equal(error_code(dvs_add("a.csv")), "not_in_project")
  expect_equal(error_code(dvs_get()), "not_in_project")
})

test_that("an add that selects nothing fails with no_files_selected", {
  new_dvs_test_repo()
  expect_equal(error_code(dvs_add(glob = "*.nothing")), "no_files_selected")
})

test_that("glob with recursive fails with invalid_arguments", {
  new_dvs_test_repo()
  expect_equal(error_code(dvs_status(glob = "*.csv", recursive = TRUE)), "invalid_arguments")
})

test_that("a server failure carries the CLI's code, also with a progress bar", {
  new_dvs_test_repo()
  # No token: dvs stops before it sends a request, so no server is needed.
  writeLines(c("[backend]", 'name = "p"', 'url = "http://127.0.0.1:9/"'), "dvs.toml")
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  writeLines("a", "a.csv")

  expect_equal(error_code(dvs_add("a.csv")), "auth_required")
  expect_equal(error_code(dvs_audit_log("a.csv")), "auth_required")
  # The progress bar runs the add on a worker thread
  expect_equal(
    error_code(dvs_add_impl("a.csv", progress_callback = ProgressBarCallback$new())),
    "auth_required"
  )
  # The CLI's message, with nothing in front
  cnd <- tryCatch(dvs_add("a.csv"), error = identity)
  expect_equal(conditionMessage(cnd), "Could not find access token. Log in first")
})
