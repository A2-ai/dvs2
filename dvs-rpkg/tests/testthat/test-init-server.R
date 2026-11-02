# init server and attach without a server: http://127.0.0.1:9/ does not
# answer, so a request that went out would fail with server_unavailable.
# Without a token neither asks the server: the impl returns NA, and the R
# function logs in, which a non-interactive session refuses.

url <- "http://127.0.0.1:9/"

test_that("an initialized folder is refused before any login", {
  new_dvs_test_repo()
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  cnd <- tryCatch(dvs_init_server("p", url, "g"), error = identity)
  expect_equal(cnd$code, "operation_failed")
  expect_equal(conditionMessage(cnd), "dvs is already initialized (dvs.toml exists)")
  cnd <- tryCatch(dvs_attach("p", url), error = identity)
  expect_equal(cnd$code, "operation_failed")
  expect_equal(conditionMessage(cnd), "Configuration file already exists")
})

test_that("a metadata folder with files is refused before any login", {
  withr::local_dir(withr::local_tempdir())
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  dir.create(".meta")
  writeLines("x", file.path(".meta", "keep"))
  cnd <- tryCatch(dvs_init_server("p", url, "g", metadata_folder_name = ".meta"), error = identity)
  expect_equal(cnd$code, "operation_failed")
  expect_equal(conditionMessage(cnd), "dvs is already initialized (.meta exists and is not empty)")
})

test_that("with no token, nothing is written and the login comes first", {
  withr::local_dir(withr::local_tempdir())
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  expect_identical(dvs_init_server_impl("p", url, "g"), NA_character_)
  expect_identical(dvs_attach_impl("p", url), NA_character_)
  expect_identical(list.files(all.files = TRUE, no.. = TRUE), character(0))
  skip_if(interactive())
  expect_equal(error_code(dvs_init_server("p", url, "g")), "auth_required")
  expect_equal(error_code(dvs_attach("p", url)), "auth_required")
})

test_that("an unreadable credentials store is an error and stays", {
  withr::local_dir(withr::local_tempdir())
  xdg <- withr::local_tempdir()
  dir.create(file.path(xdg, "dvs"))
  store <- file.path(xdg, "dvs", "credentials.json")
  writeLines("not json", store)
  withr::local_envvar(XDG_CONFIG_HOME = xdg)
  expect_equal(error_code(dvs_init_server("p", url, "g")), "credentials_unreadable")
  expect_equal(error_code(dvs_attach("p", url)), "credentials_unreadable")
  expect_equal(readLines(store), "not json")
})

test_that("a server that does not answer fails with server_unavailable", {
  withr::local_dir(withr::local_tempdir())
  xdg <- withr::local_tempdir()
  dir.create(file.path(xdg, "dvs"))
  writeLines(sprintf('{"%s": "tok"}', url), file.path(xdg, "dvs", "credentials.json"))
  withr::local_envvar(XDG_CONFIG_HOME = xdg)
  expect_equal(error_code(dvs_init_server("p", url, "g")), "server_unavailable")
  expect_equal(error_code(dvs_attach("p", url)), "server_unavailable")
  # A failed attach leaves nothing that blocks a retry
  expect_false(file.exists("dvs.toml"))
})

test_that("arguments are checked before any login", {
  withr::local_dir(withr::local_tempdir())
  withr::local_envvar(XDG_CONFIG_HOME = withr::local_tempdir())
  expect_equal(error_code(dvs_init_server("p", "not a url", "g")), "invalid_arguments")
  expect_equal(error_code(dvs_attach("p", "not a url")), "invalid_arguments")
  expect_argument_error(quote(dvs_init_server("p", url, NA_character_)), "group")
  expect_argument_error(quote(dvs_attach(NA_character_, url)), "name")
  expect_error(dvs_init_server("p", url), "group")
})
