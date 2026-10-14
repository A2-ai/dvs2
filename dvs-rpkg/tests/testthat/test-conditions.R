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
    expect_s3_class(e, "dvs_error_not_a_server_project")
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
  # A folder or a pattern goes alone: the server would AND it with the rest
  e <- catch(dvs_activity(path = c("raw/", "derived/")))
  expect_s3_class(e, "dvs_error_invalid_argument")
  expect_match(conditionMessage(e), "one folder")
  expect_s3_class(catch(dvs_activity(path = c("raw/*.csv", "a.csv"))), "dvs_error_invalid_argument")
})

test_that("arguments are checked in R, naming the function called", {
  new_dvs_test_repo()
  calls <- list(
    quote(dvs_projects(role = "owner")),
    quote(dvs_add(paths = 1)),
    quote(dvs_get(on_error = "x")),
    quote(dvs_status(status = "bogus")),
    quote(dvs_status(remote = NA)),
    quote(dvs_activity(max_rows = 0)),
    quote(dvs_log(NULL)),
    quote(dvs_attach(1, "x")),
    quote(dvs_remote_files(min_size = TRUE)),
    quote(set_dvs_threads(-1)),
    quote(dvs_init(tempfile(), compression = "lz4"))
  )
  for (call in calls) {
    e <- catch(eval(call))
    expect_s3_class(e, "dvs_error_invalid_argument")
    expect_identical(e$call, call)
  }
})

test_that("an interrupted wait for the server is an interrupt, not an error", {
  interrupted <- function() {
    stop(structure(
      list(message = "Interrupted", call = NULL, code = "interrupted", status = NA_integer_, request_id = ""),
      class = c("rust_error", "error", "condition")
    ))
  }
  r <- tryCatch(dvs_call(interrupted()), interrupt = function(e) "interrupt", error = function(e) "error")
  expect_identical(r, "interrupt")
})

test_that("Esc or Ctrl-C stops waiting for a server that does not answer", {
  skip_on_cran()
  skip_on_os("windows")
  helper <- normalizePath(test_path("..", "server", "server.cjs"), mustWork = FALSE)
  skip_if_not(file.exists(helper), "tests/server/server.cjs is missing")
  skip_if(Sys.which("node") == "", "node is not on PATH")
  script <- withr::local_tempfile(fileext = ".R")
  writeLines(c(
    "library(dvs)",
    "Sys.setenv(DVS_TOKEN = 'tok')",
    "r <- tryCatch(dvs_projects(url = commandArgs(TRUE)[1]), interrupt = function(e) 'interrupted')",
    "cat(r, '\\n')"
  ), script)
  rscript <- file.path(R.home("bin"), "Rscript")
  withr::local_envvar(R_LIBS = paste(.libPaths(), collapse = .Platform$path.sep))
  out <- system2(
    Sys.which("node"),
    c(shQuote(helper), "interrupt", "2", "--", shQuote(rscript), shQuote(script)),
    stdout = TRUE, stderr = TRUE
  )
  expect_true("interrupted " %in% out, info = paste(out, collapse = "\n"))
  # Well within the HTTP agent's timeouts
  elapsed <- as.numeric(sub("^elapsed ", "", grep("^elapsed ", out, value = TRUE)))
  expect_lt(elapsed, 5)
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
  expect_identical(
    dvs_time_arg(as.POSIXct("2026-10-01 12:00:00.750", tz = "UTC"), "from"),
    "2026-10-01T12:00:00.750000Z"
  )
  expect_identical(dvs_rfc3339(as.POSIXct("2026-10-13T08:30:01.123456Z", format = "%Y-%m-%dT%H:%M:%OSZ", tz = "UTC")), "2026-10-13T08:30:01.123456Z")
  expect_s3_class(catch(dvs_size_arg(TRUE, "min_size")), "dvs_error_invalid_argument")
  expect_identical(dvs_time_arg("7d", "from"), "7d")
  expect_null(dvs_time_arg(NULL, "from"))
  expect_identical(dvs_size_arg(1048576, "min_size"), "1048576")
  expect_identical(dvs_size_arg(5e9, "min_size"), "5000000000")
  expect_identical(dvs_size_arg("100MiB", "min_size"), "100MiB")
})

test_that("another process holding the credentials lock: dvs gives up, or stops on an interrupt", {
  skip_on_cran()
  skip_on_os("windows")
  helper <- normalizePath(test_path("..", "server", "server.cjs"), mustWork = FALSE)
  skip_if_not(file.exists(helper), "tests/server/server.cjs is missing")
  skip_if(Sys.which("node") == "", "node is not on PATH")
  skip_if(Sys.which("perl") == "", "perl is not on PATH")

  config <- withr::local_tempdir()
  dir.create(file.path(config, "dvs"))
  store <- file.path(config, "dvs", "credentials.json")
  url <- "http://127.0.0.1:9/"
  writeLines(sprintf('{"%s": "tok-old"}', url), store)
  withr::local_envvar(XDG_CONFIG_HOME = config, DVS_TOKEN = NA)

  # Another process takes the lock, as dvs does (flock), until told to stop
  locked <- file.path(config, "locked")
  release <- file.path(config, "release")
  withr::defer(file.create(release))
  system2(Sys.which("perl"), c(
    "-e", shQuote(paste(
      'use Fcntl ":flock"; open(my $f, ">>", $ARGV[0]) or die; flock($f, LOCK_EX) or die;',
      'open(my $m, ">", $ARGV[1]); close($m); my $n = 0;',
      'until (-e $ARGV[2] || $n++ > 600) { select(undef, undef, undef, 0.1) }'
    )),
    shQuote(paste0(store, ".lock")), shQuote(locked), shQuote(release)
  ), wait = FALSE)
  for (i in 1:100) if (!file.exists(locked)) Sys.sleep(0.1)
  expect_true(file.exists(locked))

  # The server cannot be reached, so the logout goes on to forget the token, and
  # waits for the lock: at most 10 seconds
  e <- catch(suppressMessages(dvs_logout(url)))
  expect_s3_class(e, "dvs_error_credentials_busy")
  expect_match(readLines(store), "tok-old", fixed = TRUE)

  # Esc or Ctrl-C stops the wait; the token is kept
  script <- withr::local_tempfile(fileext = ".R")
  writeLines(c(
    "library(dvs)",
    sprintf("r <- tryCatch(dvs_logout('%s'), interrupt = function(e) 'interrupted')", url),
    "cat(r, '\\n')"
  ), script)
  withr::local_envvar(R_LIBS = paste(.libPaths(), collapse = .Platform$path.sep))
  out <- system2(
    Sys.which("node"),
    c(shQuote(helper), "interrupt", "2", "--", shQuote(file.path(R.home("bin"), "Rscript")), shQuote(script)),
    stdout = TRUE, stderr = TRUE
  )
  expect_true("interrupted " %in% out, info = paste(out, collapse = "\n"))
  elapsed <- as.numeric(sub("^elapsed ", "", grep("^elapsed ", out, value = TRUE)))
  expect_lt(elapsed, 5)
  expect_match(readLines(store), "tok-old", fixed = TRUE)
})
