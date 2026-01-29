# Tests for core DVS functionality

test_that("dvs_init creates config file", {
  # Create a temporary directory for testing

  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo (required for DVS)
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory
  storage_dir <- file.path(tmp_dir, ".dvs-storage")

  # Initialize DVS
  result <- dvs_init(storage_dir)

  # Check result

  expect_type(result, "list")
  expect_true("storage_dir" %in% names(result))

  # Check config file was created
  config_file <- file.path(tmp_dir, "dvs.toml")
  expect_true(file.exists(config_file))
})

test_that("dvs_add tracks files", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create a test file
  test_file <- file.path(tmp_dir, "test_data.csv")
  writeLines("a,b,c\n1,2,3\n4,5,6", test_file)

  # Add the file
  result <- dvs_add(test_file)

  # Check result
  expect_s3_class(result, "data.frame")
  expect_true("path" %in% names(result))
  expect_true("outcome" %in% names(result))
  expect_equal(nrow(result), 1)
  expect_equal(result$outcome, "copied")

  # Check metadata file was created (could be .dvs or .dvs.toml depending on config)
  metadata_toml <- paste0(test_file, ".dvs.toml")
  metadata_json <- paste0(test_file, ".dvs")
  expect_true(file.exists(metadata_toml) || file.exists(metadata_json))
})

test_that("dvs_status reports file status", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create and add a test file
  test_file <- file.path(tmp_dir, "test_data.csv")
  writeLines("a,b,c\n1,2,3", test_file)
  dvs_add(test_file)

  # Check status
  result <- dvs_status()

  # Check result
  expect_s3_class(result, "data.frame")
  expect_true("path" %in% names(result))
  expect_true("status" %in% names(result))
  expect_equal(nrow(result), 1)
  expect_equal(result$status, "current")
})

test_that("dvs_get retrieves files", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create and add a test file
  test_file <- file.path(tmp_dir, "test_data.csv")
  original_content <- "a,b,c\n1,2,3\n4,5,6"
  writeLines(original_content, test_file)
  dvs_add(test_file)

  # Delete the original file
  unlink(test_file)
  expect_false(file.exists(test_file))

  # Get the file back
  result <- dvs_get(test_file)

  # Check result
  expect_s3_class(result, "data.frame")
  expect_equal(nrow(result), 1)
  expect_equal(result$outcome, "copied")

  # Check file was restored

  expect_true(file.exists(test_file))
})

test_that("dvs_log returns reflog entries", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create and add a test file (creates a reflog entry)
  test_file <- file.path(tmp_dir, "test_data.csv")
  writeLines("a,b,c\n1,2,3", test_file)
  dvs_add(test_file)

  # Get log
  result <- dvs_log()

  # Check result
  expect_s3_class(result, "data.frame")
  expect_true(nrow(result) >= 1)
  expect_true("index" %in% names(result))
  expect_true("op" %in% names(result))
})

test_that("dvs_status detects unsynced files", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create and add a test file
  test_file <- file.path(tmp_dir, "test_data.csv")
  writeLines("a,b,c\n1,2,3", test_file)
  dvs_add(test_file)

  # Modify the file
  writeLines("a,b,c\n1,2,3\n4,5,6", test_file)

  # Check status - should be unsynced
  result <- dvs_status()

  expect_equal(nrow(result), 1)
  expect_equal(result$status, "unsynced")
})

test_that("dvs_add with message records message in reflog", {
  # Create a temporary directory for testing
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  # Initialize git repo
  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  # Create storage directory and initialize DVS
  storage_dir <- file.path(tmp_dir, ".dvs-storage")
  dvs_init(storage_dir)

  # Create and add a test file with message
  test_file <- file.path(tmp_dir, "test_data.csv")
  writeLines("a,b,c\n1,2,3", test_file)
  test_message <- "Initial data import"
  dvs_add(test_file, message = test_message)

  # Get log and check message
  log_result <- dvs_log()

  expect_true(nrow(log_result) >= 1)
  # The most recent entry should have our message
  expect_true(any(grepl(test_message, log_result$message, fixed = TRUE)))
})

# ============================================================================
# Error Handling Tests
# ============================================================================

test_that("dvs_status fails gracefully without init", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))

  # Should error when not initialized

  expect_error(dvs_status(), "init|not initialized", ignore.case = TRUE)
})

test_that("dvs_add fails for nonexistent file", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  dvs_init(".storage")

  # Adding nonexistent file should return result with error
  result <- dvs_add("nonexistent.csv")
  # DVS returns a data frame with error info, not an R error
  expect_s3_class(result, "data.frame")
})

test_that("dvs_add rejects directories", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  dvs_init(".storage")

  # Create a directory
  dir.create("mydir")

  # Adding directory returns error in result (not an R error)
  result <- dvs_add("mydir")
  expect_s3_class(result, "data.frame")
  expect_equal(result$error, "is_directory")
  expect_true(grepl("directory", result$error_message, ignore.case = TRUE))
})

# ============================================================================
# Rollback Tests
# ============================================================================

test_that("dvs_rollback restores previous state", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  dvs_init(".storage")

  # Add file version 1
  writeLines("version 1", "data.csv")
  dvs_add("data.csv", message = "v1")

  # Add file version 2
  writeLines("version 2", "data.csv")
  dvs_add("data.csv", message = "v2")

  # Check we have 2 log entries
  log <- dvs_log()
  expect_true(nrow(log) >= 2)

  # Rollback to first state (index 1)
  result <- dvs_rollback("1")
  expect_type(result, "list")
  expect_true(result$success)
  expect_true("data.csv" %in% result$restored_files)

  # File should be restored to version 1
  content <- readLines("data.csv")
  expect_equal(content, "version 1")
})

# ============================================================================
# Materialize Tests
# ============================================================================

test_that("dvs_materialize retrieves all tracked files", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  dvs_init(".storage")

  # Add multiple files
  writeLines("file 1", "data1.csv")
  writeLines("file 2", "data2.csv")
  dvs_add(c("data1.csv", "data2.csv"))

  # Delete the working copies
  unlink("data1.csv")
  unlink("data2.csv")
  expect_false(file.exists("data1.csv"))
  expect_false(file.exists("data2.csv"))

  # Materialize all files - returns a list with summary
  result <- dvs_materialize()
  expect_type(result, "list")
  expect_true("materialized" %in% names(result))

  # Files should be restored
  expect_true(file.exists("data1.csv"))
  expect_true(file.exists("data2.csv"))
})

# ============================================================================
# Verify Tests
# ============================================================================

test_that("dvs_verify validates file integrity", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  system2("git", c("config", "user.email", "test@test.com"))
  system2("git", c("config", "user.name", "Test User"))

  dvs_init(".storage")

  # Add a file
  writeLines("test content", "data.csv")
  dvs_add("data.csv")

  # Verify should pass
  result <- dvs_verify()
  expect_type(result, "list")
  expect_equal(result$total, 1)
  expect_equal(result$passed, 1)
  expect_equal(result$errors, 0)
  # Results data frame shows individual file status
  expect_true(all(result$results$ok))
})

# ============================================================================
# Config Tests
# ============================================================================

test_that("dvs_config shows configuration", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))

  dvs_init(".storage")

  # Get config
  result <- dvs_config()
  expect_type(result, "list")
  expect_true("storage_dir" %in% names(result))
})

# ============================================================================
# Edge Cases
# ============================================================================

test_that("dvs_add handles empty file", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  dvs_init(".storage")

  # Create empty file
  file.create("empty.csv")

  # Add should work
  result <- dvs_add("empty.csv")
  expect_s3_class(result, "data.frame")
  expect_equal(result$outcome, "copied")
  expect_equal(result$size, 0)
})

test_that("dvs_add handles binary file", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  dvs_init(".storage")

  # Create binary file with null bytes
  writeBin(as.raw(c(0x00, 0x01, 0x02, 0xFF, 0x00)), "binary.bin")

  # Add should work
  result <- dvs_add("binary.bin")
  expect_s3_class(result, "data.frame")
  expect_equal(result$outcome, "copied")
})

test_that("dvs handles multiple files in batch", {
  tmp_dir <- tempfile("dvs_test_")
  dir.create(tmp_dir)
  on.exit(unlink(tmp_dir, recursive = TRUE), add = TRUE)

  old_wd <- setwd(tmp_dir)
  on.exit(setwd(old_wd), add = TRUE)

  system2("git", c("init", "-q"))
  dvs_init(".storage")

  # Create multiple files
  for (i in 1:5) {
    writeLines(paste("content", i), paste0("file", i, ".csv"))
  }

  # Add all at once
  files <- paste0("file", 1:5, ".csv")
  result <- dvs_add(files)

  expect_s3_class(result, "data.frame")
  expect_equal(nrow(result), 5)
  expect_true(all(result$outcome == "copied"))
})
