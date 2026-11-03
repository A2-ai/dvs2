# NA is never a value. Whatever the type of the argument, the message says
# that it must not be NA, not that the type or range is wrong.
refuses_na <- function(call, param) {
  expect_argument_error(call, param)
  cnd <- tryCatch(eval(call), error = identity)
  what <- if (param == "paths") "contain" else "be"
  expect_identical(conditionMessage(cnd), sprintf("'%s' must not %s NA", param, what))
}

test_that("an NA in any argument says it must not be NA", {
  new_dvs_test_repo()
  withr::local_options(dvs.num_threads = NULL, dvs.on_error = NULL)
  for (na in list(NA, NA_character_)) {
    refuses_na(bquote(dvs_init(storage_path = .(na))), "storage_path")
    refuses_na(bquote(dvs_init(tempfile(), root_dir = .(na))), "root_dir")
    refuses_na(bquote(dvs_init(tempfile(), group = .(na))), "group")
    refuses_na(bquote(dvs_init(tempfile(), metadata_folder_name = .(na))), "metadata_folder_name")
    refuses_na(bquote(dvs_init(tempfile(), compression = .(na))), "compression")
    refuses_na(bquote(dvs_add(paths = .(na))), "paths")
    refuses_na(bquote(dvs_add("a.csv", message = .(na))), "message")
    refuses_na(bquote(dvs_add(glob = .(na))), "glob")
    refuses_na(bquote(dvs_status(paths = .(na))), "paths")
    refuses_na(bquote(dvs_status(glob = .(na))), "glob")
    refuses_na(bquote(dvs_status(status = .(na))), "status")
    refuses_na(bquote(dvs_get(paths = .(na))), "paths")
    refuses_na(bquote(dvs_get(glob = .(na))), "glob")
    refuses_na(bquote(dvs_get(on_conflict = .(na))), "on_conflict")
    refuses_na(bquote(dvs_get(on_error = .(na))), "on_error")
    refuses_na(bquote(dvs_audit_log(paths = .(na))), "paths")
    refuses_na(bquote(set_dvs_log_level(.(na))), "level")
  }
  for (na in list(NA, NA_real_)) {
    refuses_na(bquote(set_dvs_threads(.(na))), "threads")
    refuses_na(bquote(format_byte_size(.(na))), "size_bytes")
  }
  refuses_na(quote(dvs_add("a.csv", dry_run = NA)), "dry_run")
  refuses_na(quote(dvs_status(recursive = NA)), "recursive")
  refuses_na(quote(dvs_get(recursive = NA)), "recursive")
  refuses_na(quote(dvs_get(dry_run = NA)), "dry_run")
})

test_that("an NA among several values of a choice says it must not be NA", {
  new_dvs_test_repo()
  withr::local_options(dvs.num_threads = NULL, dvs.on_error = NULL)
  refuses_na(quote(dvs_init(tempfile(), compression = c("zstd", NA))), "compression")
  refuses_na(quote(dvs_status(status = c("current", NA))), "status")
  refuses_na(quote(dvs_get(on_conflict = c("skip", NA))), "on_conflict")
  refuses_na(quote(dvs_get(on_error = c("return", NA))), "on_error")
  refuses_na(quote(dvs_get(on_error = factor(c("return", NA)))), "on_error")
  refuses_na(quote(set_dvs_log_level(c("info", NA))), "level")
})

# Several values without an NA are refused for their length.
refuses_length <- function(call, param) {
  expect_argument_error(call, param)
  cnd <- tryCatch(eval(call), error = identity)
  expect_identical(conditionMessage(cnd), sprintf("'%s' must be of length 1", param))
}

test_that("several values of a choice without an NA say it must be of length 1", {
  new_dvs_test_repo()
  withr::local_options(dvs.num_threads = NULL, dvs.on_error = NULL)
  refuses_length(quote(dvs_init(tempfile(), compression = c("none", "zstd"))), "compression")
  refuses_length(quote(dvs_get(on_conflict = c("skip", "abort"))), "on_conflict")
  refuses_length(quote(dvs_get(on_error = c("return", "error"))), "on_error")
  refuses_length(quote(set_dvs_log_level(c("info", "warn"))), "level")

  withr::local_options(dvs.on_error = c("return", "error"))
  cnd <- tryCatch(dvs_get(), error = identity)
  expect_identical(conditionMessage(cnd), "`options(dvs.on_error)` must be of length 1")
})

test_that("an NA in an option says the option must not be NA", {
  new_dvs_test_repo()
  withr::local_options(dvs.num_threads = NA_real_)
  cnd <- tryCatch(dvs_status(), error = identity)
  expect_identical(conditionMessage(cnd), "`options(dvs.num_threads)` must not be NA")

  withr::local_options(dvs.num_threads = NULL, dvs.on_error = NA_character_)
  cnd <- tryCatch(dvs_get(), error = identity)
  expect_identical(conditionMessage(cnd), "`options(dvs.on_error)` must not be NA")

  withr::local_options(dvs.on_error = c("return", NA))
  cnd <- tryCatch(dvs_get(), error = identity)
  expect_identical(conditionMessage(cnd), "`options(dvs.on_error)` must not be NA")
})
