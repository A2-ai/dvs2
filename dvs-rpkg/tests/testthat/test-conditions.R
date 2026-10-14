catch <- function(expr) tryCatch(expr, error = function(e) e)

test_that("a dvs error has its code's class and the fields code, status, message and request_id", {
  e <- new_dvs_error("project_locked", "locked", status = 409L, request_id = "r1")
  expect_s3_class(e, c("dvs_error_project_locked", "dvs_error", "error", "condition"), exact = TRUE)
  expect_identical(e$code, "project_locked")
  expect_identical(e$status, 409L)
  expect_identical(conditionMessage(e), "locked")
  expect_identical(e$request_id, "r1")
})

test_that("an error from Rust becomes a dvs error raised from the calling function", {
  rust <- function() {
    stop(structure(
      list(message = "token expired", call = NULL, code = "token_expired", status = 401L, request_id = "abc"),
      class = c("dvs_rust_error", "rust_error", "simpleError", "error", "condition")
    ))
  }
  caller <- function() dvs_call(rust())
  e <- catch(caller())
  expect_s3_class(e, c("dvs_error_token_expired", "dvs_error", "error", "condition"), exact = TRUE)
  expect_identical(e$status, 401L)
  expect_identical(e$request_id, "abc")
  expect_identical(deparse(e$call), "caller()")

  # A Rust error without a code (a panic) is a dvs_error_error with no status
  panic <- function() stop(structure(list(message = "boom", call = NULL), class = c("rust_error", "error", "condition")))
  e <- catch(dvs_call(panic()))
  expect_s3_class(e, "dvs_error_error")
  expect_identical(e$status, NA_integer_)
  expect_identical(e$request_id, NA_character_)
})

test_that("outside a repository the error is not_in_repository, with no status", {
  dir <- withr::local_tempdir()
  withr::local_dir(dir)
  e <- catch(dvs_status())
  expect_s3_class(e, "dvs_error_not_in_repository")
  expect_identical(e$status, NA_integer_)
  expect_identical(deparse(e$call), "dvs_status()")
  expect_s3_class(catch(dvs_project_info()), "dvs_error_not_in_repository")
  expect_match(conditionMessage(catch(dvs_info())), "give the server's `url`")
})

test_that("on a local project the server functions say they are for server projects only", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  dvs_add("t.csv")
  calls <- list(
    quote(dvs_project_info()),
    quote(dvs_access()),
    quote(dvs_remote_files()),
    quote(dvs_log("t.csv")),
    quote(dvs_activity()),
    quote(dvs_status(remote = TRUE)),
    quote(dvs_projects()),
    quote(dvs_info()),
    quote(dvs_logout())
  )
  for (call in calls) {
    e <- catch(eval(call))
    expect_s3_class(e, "dvs_error_server_projects_only")
    expect_identical(e$call, call)
  }
})

test_that("dvs_login needs an interactive session and points at DVS_TOKEN", {
  local_mocked_bindings(dvs_interactive = function() FALSE)
  e <- catch(dvs_login("http://127.0.0.1:9/"))
  expect_s3_class(e, "dvs_error_not_interactive")
  expect_match(conditionMessage(e), "DVS_TOKEN")
})

test_that("bad arguments are invalid_argument errors", {
  new_dvs_test_repo()
  expect_s3_class(catch(dvs_remote_files(from = 3)), "dvs_error_invalid_argument")
  expect_s3_class(catch(dvs_activity(from = c("2026-01-01", "2026-02-01"))), "dvs_error_invalid_argument")
  expect_s3_class(catch(dvs_info("not a url")), "dvs_error_invalid_argument")
})

test_that("a server that does not answer gives a dvs error with no status", {
  repo <- withr::local_tempdir()
  withr::local_dir(repo)
  system2("git", c("init", "-q"))
  writeLines(c("[backend]", 'name = "demo"', 'url = "http://127.0.0.1:9/"'), "dvs.toml")
  dir.create(".dvs")
  withr::local_envvar(DVS_TOKEN = "tok", XDG_CONFIG_HOME = withr::local_tempdir())
  e <- catch(dvs_project_info())
  expect_s3_class(e, "dvs_error")
  expect_identical(e$status, NA_integer_)
})

test_that("failed files stop dvs_get with the results, unless they are collected", {
  repo <- new_dvs_test_repo()
  writeLines("a", "a.txt")
  writeLines("b", "b.txt")
  added <- dvs_add(c("a.txt", "b.txt"))
  blob <- list.files(repo$storage, recursive = TRUE, full.names = TRUE)
  unlink(blob[grepl(substr(added$hash[added$path == "a.txt"], 3, 64), blob)])
  file.remove(c("a.txt", "b.txt"))

  e <- catch(dvs_get(c("a.txt", "b.txt")))
  expect_s3_class(e, c("dvs_error_files_failed", "dvs_error", "error", "condition"), exact = TRUE)
  expect_identical(e$code, "files_failed")
  expect_match(conditionMessage(e), "1 of 2 files failed to get")
  expect_match(conditionMessage(e), "a.txt: ")
  expect_s3_class(e$results, "tbl_df")
  expect_setequal(e$results$path, c("a.txt", "b.txt"))
  expect_identical(is.na(e$results$error[match(c("a.txt", "b.txt"), e$results$path)]), c(FALSE, TRUE))

  collected <- dvs_get(c("a.txt", "b.txt"), on_error = "collect")
  expect_s3_class(collected, "tbl_df")
  expect_identical(is.na(collected$error[match(c("a.txt", "b.txt"), collected$path)]), c(FALSE, TRUE))
})

test_that("empty results are typed zero-row tibbles", {
  spec <- dvs_spec$activity
  empty <- dvs_frame(data.frame(), spec)
  expect_s3_class(empty, "tbl_df")
  expect_identical(nrow(empty), 0L)
  expect_identical(names(empty), names(spec))
  expect_s3_class(empty$at, "POSIXct")
  expect_identical(attr(empty$at, "tzone"), "UTC")
  expect_s3_class(empty$size, "dvs_bytes")
  expect_type(empty$global_admin, "logical")
  expect_type(empty$kind, "character")

  for (name in names(dvs_spec)) {
    frame <- dvs_frame(list(), dvs_spec[[name]])
    expect_identical(nrow(frame), 0L)
    expect_identical(names(frame), names(dvs_spec[[name]]))
  }
})

test_that("columns are given their types, and an unset column is NA", {
  rows <- data.frame(
    at = c("2026-10-13T08:30:00Z", "2026-10-13T08:30:01.5Z"),
    kind = "upload", size = c(1536, NA), global_admin = NA,
    stringsAsFactors = FALSE
  )
  frame <- dvs_frame(rows, dvs_spec$activity)
  expect_identical(nrow(frame), 2L)
  expect_identical(format(frame$at, "%Y-%m-%d %H:%M:%OS1", tz = "UTC"), c("2026-10-13 08:30:00.0", "2026-10-13 08:30:01.5"))
  expect_s3_class(frame$size, "dvs_bytes")
  expect_identical(unclass(frame$size), c(1536, NA))
  expect_identical(frame$global_admin, c(NA, NA))
  expect_identical(frame$path, c(NA_character_, NA_character_))
})

test_that("date and size arguments are written as the server reads them", {
  expect_identical(dvs_time_arg(as.Date("2026-10-01"), "from"), "2026-10-01")
  expect_identical(
    dvs_time_arg(as.POSIXct("2026-10-01 12:00:00", tz = "America/New_York"), "from"),
    "2026-10-01T16:00:00Z"
  )
  expect_identical(dvs_time_arg("7d", "from"), "7d")
  expect_null(dvs_time_arg(NULL, "from"))
  expect_identical(dvs_size_arg(1048576, "min_size"), "1048576")
  expect_identical(dvs_size_arg(5e9, "min_size"), "5000000000")
  expect_identical(dvs_size_arg("100MiB", "min_size"), "100MiB")
})
