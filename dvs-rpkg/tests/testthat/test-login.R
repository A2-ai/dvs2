# Login and logout without a server: http://127.0.0.1:9/ does not answer, so
# a request that went out would fail with server_unavailable.

credentials <- function(xdg, json) {
  dir.create(file.path(xdg, "dvs"), showWarnings = FALSE)
  path <- file.path(xdg, "dvs", "credentials.json")
  writeLines(json, path)
  path
}

test_that("dvs_login() refuses outside an interactive session before it asks the server", {
  new_dvs_test_repo()
  server_project("http://127.0.0.1:9/")
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  cnd <- tryCatch(dvs_login_start_impl(interactive_session = FALSE), error = identity)
  expect_s3_class(cnd, "dvs_error")
  expect_equal(cnd$code, "auth_required")
  expect_match(conditionMessage(cnd), "interactive R session", fixed = TRUE)
  expect_equal(
    error_code(dvs_login_start_impl("http://127.0.0.1:9/", interactive_session = FALSE)),
    "auth_required"
  )
  skip_if(interactive())
  expect_equal(error_code(dvs_login()), "auth_required")
})

test_that("dvs_login() needs a server project or a valid url", {
  new_dvs_test_repo()
  expect_equal(error_code(dvs_login_start_impl(interactive_session = TRUE)), "operation_failed")
  expect_equal(
    error_code(dvs_login_start_impl("not a url", interactive_session = TRUE)),
    "invalid_arguments"
  )
})

test_that("the wait for a login gives up past the CLI's limit", {
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  cnd <- tryCatch(dvs_login_poll_impl("http://127.0.0.1:9/", "code", 121L), error = identity)
  expect_s3_class(cnd, "dvs_error")
  expect_equal(conditionMessage(cnd), "Token will have expired")
  expect_equal(
    error_code(dvs_login_poll_impl("http://127.0.0.1:9/", "code", 120L)),
    "server_unavailable"
  )
})

test_that("dvs_logout() without a token has nothing to revoke", {
  new_dvs_test_repo()
  server_project("http://127.0.0.1:9/")
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  logout <- dvs_logout()
  expect_null(logout$revoked)
  expect_named(logout$warnings, c("code", "message"))
  expect_equal(nrow(logout$warnings), 0)
})

test_that("dvs_logout() deletes the token when the server cannot revoke it", {
  new_dvs_test_repo()
  server_project("http://127.0.0.1:9/")
  xdg <- withr::local_tempdir()
  store <- credentials(xdg, '{"http://127.0.0.1:9/": "tok", "https://other/": "tok2"}')
  withr::local_envvar(XDG_CONFIG_HOME = xdg)
  logout <- dvs_logout()
  expect_false(logout$revoked)
  expect_equal(logout$warnings$code, "revoke_failed")
  expect_match(logout$warnings$message, "cannot reach the dvs server", fixed = TRUE)
  expect_false(dvs_info()$logged_in)
  # Only this server's token goes
  expect_match(readChar(store, file.size(store)), "https://other/", fixed = TRUE)
})

test_that("dvs_logout() resets an unreadable credentials store", {
  new_dvs_test_repo()
  server_project("http://127.0.0.1:9/")
  xdg <- withr::local_tempdir()
  credentials(xdg, "not json")
  withr::local_envvar(XDG_CONFIG_HOME = xdg)
  expect_equal(error_code(dvs_info()), "credentials_unreadable")
  logout <- dvs_logout()
  expect_null(logout$revoked)
  expect_false(dvs_info()$logged_in)
})

test_that("dvs_logout() in a local project fails as the CLI does", {
  new_dvs_test_repo()
  cnd <- tryCatch(dvs_logout(), error = identity)
  expect_s3_class(cnd, "dvs_error")
  expect_equal(cnd$code, "operation_failed")
  expect_equal(conditionMessage(cnd), "This project is not using a dvs server")
})
