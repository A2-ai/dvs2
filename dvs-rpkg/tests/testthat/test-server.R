# The server functions against a real dvs-server. Run with one started by
# tests/server/server.cjs, which sets DVS_TEST_SERVER:
#
#   node tests/server/server.cjs serve -- Rscript -e 'testthat::test_local()'
#
# The tests run in order and share a project. The server has the personas
# alice (dvs-admins, proj-x), bob (proj-x) and carol (other), with the tokens
# tok-<name>.

server_url <- Sys.getenv("DVS_TEST_SERVER")
helper <- normalizePath(test_path("..", "server", "server.cjs"), mustWork = FALSE)

skip_without_server <- function() {
  skip_if(server_url == "", "set DVS_TEST_SERVER (tests/server/server.cjs serve)")
  skip_if_not(file.exists(helper), "tests/server/server.cjs is missing")
  skip_if(Sys.which("node") == "", "node is not on PATH")
}

shared <- new.env()
shared$project <- paste0("rpkg-", paste(sample(letters, 8), collapse = ""))
shared$config <- tempfile("dvs-config-")
dir.create(shared$config)

node <- function(...) {
  out <- system2(Sys.which("node"), c(shQuote(helper), vapply(c(...), shQuote, "")), stdout = TRUE, stderr = TRUE)
  status <- attr(out, "status")
  if (!is.null(status) && status != 0) stop(paste(out, collapse = "\n"))
  invisible(out)
}

# Act as `user` (a seeded token in DVS_TOKEN, or none), with this file's own
# credentials store, until the calling test ends.
local_user <- function(user, env = parent.frame()) {
  token <- if (is.null(user)) NA else paste0("tok-", user)
  withr::local_envvar(DVS_TOKEN = token, XDG_CONFIG_HOME = shared$config, .local_envir = env)
}

local_checkout <- function(env = parent.frame()) {
  withr::local_dir(shared$checkout, .local_envir = env)
}

catch <- function(expr) tryCatch(expr, error = function(e) e)

is_utc_time <- function(x) inherits(x, "POSIXct") && identical(attr(x, "tzone"), "UTC")

test_that("dvs_info: who you are, from DVS_TOKEN, and nothing without a token", {
  skip_without_server()
  local_user("bob")
  info <- dvs_info(server_url)
  expect_s3_class(info, "dvs_info")
  expect_identical(info$user, "bob")
  expect_identical(info$email, "bob@example.com")
  expect_identical(info$token_source, "DVS_TOKEN")
  expect_true("proj-x" %in% info$groups)
  expect_false(info$global_admin)
  expect_identical(info$client_version, dvs_version())
  expect_true(1L %in% info$api_versions)
  expect_true(is.na(info$project_role))
  expect_output(print(info), "User: +bob <bob@example.com>")

  local_user(NULL)
  info <- dvs_info(server_url)
  expect_true(is.na(info$user))
  expect_true(is.na(info$token_source))
  expect_output(print(info), "not logged in; run dvs_login\\(\\)")
})

test_that("dvs_attach: a project created on the server, attached here", {
  skip_without_server()
  local_user("bob")
  node("create", server_url, "bob", shared$project, "proj-x")
  shared$checkout <- tempfile("dvs-checkout-")
  dir.create(shared$checkout)
  local_checkout()
  system2("git", c("init", "-q", "--initial-branch=main"))

  expect_message(project <- dvs_attach(shared$project, server_url), "Attached .* \\(your role: admin\\)")
  expect_s3_class(project, "dvs_project")
  expect_identical(project$name, shared$project)
  expect_true(file.exists("dvs.toml"))
  expect_true(dir.exists(".dvs"))
})

test_that("dvs_add and dvs_status(remote = TRUE): what the server has", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  dir.create("data")
  writeLines(c("x", "1"), "data/a.csv")
  writeBin(as.raw(rep(7L, 3000)), "data/big.bin")
  writeLines("notes", "notes.txt")

  added <- dvs_add(c("data/a.csv", "data/big.bin", "notes.txt"), message = "first")
  expect_setequal(added$outcome, "added")

  status <- dvs_status(remote = TRUE)
  expect_true(all(c("remote_status", "on_server", "newer_uploads", "latest_at") %in% names(status)))
  expect_setequal(status$remote_status, "latest")
  expect_true(all(status$on_server))
  expect_true(is_utc_time(status$latest_at))
  expect_identical(status$latest_hash, status$hash)
})

test_that("dvs_log: every version of a file, newest first, with full hashes", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  shared$first <- dvs_status("data/a.csv")$hash
  writeLines(c("x", "2"), "data/a.csv")
  dvs_add("data/a.csv", message = "second")
  shared$second <- dvs_status("data/a.csv")$hash

  log <- dvs_log("data/a.csv")
  expect_s3_class(log, "tbl_df")
  expect_identical(names(log), names(dvs_spec$versions))
  expect_identical(log$hash, c(shared$second, shared$first))
  expect_identical(nchar(log$hash), c(64L, 64L))
  expect_identical(log$latest, c(TRUE, FALSE))
  expect_s3_class(log$size, "dvs_bytes")
  expect_true(is_utc_time(log$first_uploaded_at))

  # Relative to the working directory
  withr::local_dir("data")
  expect_identical(dvs_log("a.csv")$hash, log$hash)

  empty <- dvs_log("never.csv")
  expect_identical(nrow(empty), 0L)
  expect_identical(names(empty), names(dvs_spec$versions))
})

test_that("dvs_remote_files: filters, sorting, sizes and an empty result", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  files <- dvs_remote_files()
  expect_setequal(files$path, c("data/a.csv", "data/big.bin", "notes.txt"))
  expect_s3_class(files$size, "dvs_bytes")
  expect_true(is_utc_time(files$uploaded_at))
  expect_identical(files$uploaded_by, rep("bob", 3))
  expect_identical(files$versions[files$path == "data/a.csv"], 2L)

  expect_setequal(dvs_remote_files("data/")$path, c("data/a.csv", "data/big.bin"))
  expect_identical(dvs_remote_files(glob = "**/*.bin")$path, "data/big.bin")
  expect_identical(dvs_remote_files(min_size = "2KiB")$path, "data/big.bin")
  expect_identical(dvs_remote_files(min_size = 2048)$path, "data/big.bin")
  expect_identical(dvs_remote_files(sort = "-size")$path[1], "data/big.bin")
  expect_identical(nrow(dvs_remote_files(user = "carol")), 0L)
  expect_identical(nrow(dvs_remote_files(since = "1h")), 3L)
  expect_identical(nrow(dvs_remote_files(from = Sys.time() + 3600)), 0L)
  expect_identical(nrow(dvs_remote_files(to = Sys.time() - 3600)), 0L)

  empty <- dvs_remote_files("nothing/")
  expect_s3_class(empty, "tbl_df")
  expect_identical(names(empty), names(dvs_spec$files))
  expect_s3_class(empty$size, "dvs_bytes")

  expect_warning(two <- dvs_remote_files(max_rows = 2), "stopped at max_rows = 2 rows")
  expect_identical(nrow(two), 2L)
})

test_that("dvs_activity: every page, filters, max_rows and an empty result", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  activity <- dvs_activity()
  expect_s3_class(activity, "tbl_df")
  expect_identical(names(activity), names(dvs_spec$activity))
  expect_true(is_utc_time(activity$at))
  expect_identical(sum(activity$kind == "upload"), 4L)
  expect_true("project_create" %in% activity$kind)
  expect_identical(activity$at, sort(activity$at, decreasing = TRUE))

  uploads <- dvs_activity(kind = "upload", path = "data/a.csv")
  expect_identical(uploads$hash, c(shared$second, shared$first))
  expect_identical(uploads$message, c("second", "first"))
  expect_identical(uploads$user, c("bob", "bob"))
  expect_identical(nrow(dvs_activity(kind = "upload", path = "data/")), 3L)
  expect_identical(nrow(dvs_activity(user = "carol")), 0L)
  batch <- dvs_activity(batch = uploads$batch[1])
  expect_identical(batch$path, "data/a.csv")

  expect_warning(three <- dvs_activity(max_rows = 3), "dvs_activity\\(\\) stopped at max_rows = 3 rows")
  expect_identical(nrow(three), 3L)

  empty <- dvs_activity(kind = "lock")
  expect_identical(nrow(empty), 0L)
  expect_identical(names(empty), names(dvs_spec$activity))
})

test_that("dvs_access and dvs_projects", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  access <- dvs_access()
  expect_identical(access$group, "proj-x")
  expect_identical(access$role, "admin")
  expect_false(access$removed)

  projects <- dvs_projects()
  expect_true(shared$project %in% projects$name)
  mine <- projects[projects$name == shared$project, ]
  expect_identical(mine$your_role, "admin")
  expect_identical(mine$files, 3L)
  expect_s3_class(mine$size, "dvs_bytes")
  expect_true(shared$project %in% dvs_projects(role = "admin", url = server_url)$name)
  expect_false(shared$project %in% dvs_projects(locked = TRUE)$name)

  local_user("carol")
  expect_false(shared$project %in% dvs_projects()$name)
})

test_that("a locked project: who, when and why, in the project and its activity", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  node("lock", server_url, "bob", shared$project, "results submitted")

  project <- dvs_project_info()
  expect_true(project$locked)
  expect_identical(project$lock$by, "bob")
  expect_identical(project$lock$reason, "results submitted")
  expect_true(is_utc_time(project$lock$at))
  expect_identical(project$grants$group, "proj-x")
  expect_identical(project$files, 3L)
  expect_output(print(project), "Locked: +by bob on .*: results submitted")

  lock <- dvs_activity(kind = "lock")
  expect_identical(lock$reason, "results submitted")
  expect_identical(lock$user, "bob")
  expect_true(shared$project %in% dvs_projects(locked = TRUE)$name)

  writeLines("more", "more.txt")
  e <- catch(dvs_add("more.txt"))
  expect_s3_class(e, "dvs_error_project_locked")
  expect_false(is.na(e$status))
  expect_match(conditionMessage(e), "uploads are refused")
})

test_that("server errors: their code, status and request id, and hints that name R functions", {
  skip_without_server()
  local_checkout()

  local_user("carol")
  e <- catch(dvs_project_info())
  expect_s3_class(e, c("dvs_error_project_not_found", "dvs_error", "error", "condition"), exact = TRUE)
  expect_identical(e$status, 404L)
  expect_false(is.na(e$request_id))
  expect_identical(deparse(e$call), "dvs_project_info()")

  withr::local_envvar(DVS_TOKEN = "not-a-token")
  e <- catch(dvs_activity())
  expect_s3_class(e, "dvs_error_unauthenticated")
  expect_identical(e$status, 401L)
  expect_match(conditionMessage(e), "DVS_TOKEN")
})

test_that("a yanked file fails dvs_get with the per-file results", {
  skip_without_server()
  local_user("bob")
  local_checkout()
  node("yank", server_url, "bob", shared$project, shared$second, "wrong units")
  expect_identical(dvs_log("data/a.csv")$yanked, c(TRUE, FALSE))
  expect_true(dvs_remote_files("data/a.csv")$yanked)
  expect_identical(dvs_status("data/a.csv", remote = TRUE)$yank_reason, "wrong units")

  file.remove("data/a.csv")
  e <- catch(dvs_get(c("data/a.csv", "notes.txt")))
  expect_s3_class(e, "dvs_error_files_failed")
  failed <- e$results[e$results$path == "data/a.csv", ]
  expect_identical(failed$code, "yanked")
  expect_match(failed$error, "wrong units")
  expect_match(conditionMessage(e), "1 of 2 files failed to get")

  collected <- dvs_get("data/a.csv", on_error = "collect")
  expect_identical(collected$code, "yanked")
})

test_that("dvs_login, dvs_info and dvs_logout with a stored token", {
  skip_without_server()
  local_user(NULL)
  local_mocked_bindings(
    dvs_interactive = function() TRUE,
    # Approve in the browser's place, as bob, while dvs_login() waits
    dvs_login_prompt = function(uri, code) {
      system2(Sys.which("node"), c(shQuote(helper), "approve", shQuote(uri), "bob"), wait = FALSE)
    }
  )
  expect_message(expect_true(dvs_login(server_url)), "You are now logged in to")
  info <- dvs_info(server_url)
  expect_identical(info$user, "bob")
  expect_identical(info$token_source, "dvs_login")
  expect_true(is_utc_time(info$token_expires_at))
  expect_true(info$token_expires_at > Sys.time())

  # A stored token the server rejects: the hint names dvs_login()
  credentials <- file.path(shared$config, "dvs", "credentials.json")
  saved <- tempfile()
  file.copy(credentials, saved)
  expect_message(expect_true(dvs_logout(server_url)), "Logged out of")
  file.copy(saved, credentials, overwrite = TRUE)
  e <- catch(dvs_info(server_url))
  expect_s3_class(e, "dvs_error")
  expect_true(e$code %in% c("unauthenticated", "token_expired"))
  expect_match(conditionMessage(e), "dvs_login()", fixed = TRUE)
  expect_no_match(conditionMessage(e), "dvs login", fixed = TRUE)

  # The rejected token was forgotten
  expect_message(expect_false(dvs_logout(server_url)), "not logged in")
})
