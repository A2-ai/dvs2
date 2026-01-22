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
