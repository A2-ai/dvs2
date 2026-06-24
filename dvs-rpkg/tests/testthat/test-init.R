test_that("dvs_init creates storage and metadata folders", {
  repo <- tempfile("dvs-repo-")
  storage <- tempfile("dvs-storage-")
  dir.create(repo)
  old_wd <- getwd()
  on.exit(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
    },
    add = TRUE
  )
  setwd(repo)

  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "t@e.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "t")) == 0L)

  dvs_init(storage_path = storage, root_dir = repo)
  expect_true(dir.exists(storage))
  expect_true(dir.exists(file.path(repo, ".dvs")))
})

test_that("dvs_init returns a single-row tibble describing the config", {
  repo <- tempfile("dvs-repo-")
  storage <- tempfile("dvs-storage-")
  dir.create(repo)
  old_wd <- getwd()
  on.exit(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
    },
    add = TRUE
  )
  setwd(repo)

  result <- dvs_init(
    storage_path = storage,
    root_dir = repo,
    metadata_folder_name = "custom_meta",
    compression = "none"
  )

  expect_s3_class(result, "tbl_df")
  expect_equal(nrow(result), 1L)
  expect_equal(result$compression, "none")
  expect_equal(result$metadata_folder_name, "custom_meta")
  expect_equal(result$backend_path, storage)
  expect_setequal(
    names(result),
    c("compression", "metadata_folder_name", "backend_path", "backend_group")
  )
})

# Regression guard for miniextendr#307: an unset Option<T> config field must
# come back as a clean scalar NA column, not a list-of-NULL. A list column
# would make init rows non-stackable (rbind/bind_rows would fail).
test_that("dvs_init unset Option field returns a clean NA column and rows are stackable", {
  repo <- tempfile("dvs-repo-")
  storage <- tempfile("dvs-storage-")
  repo2 <- tempfile("dvs-repo-")
  storage2 <- tempfile("dvs-storage-")
  dir.create(repo)
  dir.create(repo2)
  old_wd <- getwd()
  on.exit(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
      unlink(repo2, recursive = TRUE, force = TRUE)
      unlink(storage2, recursive = TRUE, force = TRUE)
    },
    add = TRUE
  )

  # First repo: metadata_folder_name left unset (the Option<String> is None).
  setwd(repo)
  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "t@e.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "t")) == 0L)
  unset <- dvs_init(storage_path = storage, root_dir = repo)

  # The unset field must be a plain NA scalar, not a list column.
  expect_false(is.list(unset$metadata_folder_name))
  expect_true(is.na(unset$metadata_folder_name))

  # Second repo: metadata_folder_name set explicitly.
  setwd(repo2)
  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "t@e.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "t")) == 0L)
  set <- dvs_init(
    storage_path = storage2,
    root_dir = repo2,
    metadata_folder_name = "custom_meta"
  )

  # Both rows must stack cleanly into a single tibble.
  combined <- rbind(unset, set)
  expect_equal(nrow(combined), 2L)
  expect_false(is.list(combined$metadata_folder_name))
  expect_equal(combined$metadata_folder_name, c(NA, "custom_meta"))
})

test_that("dvs_init respects metadata_folder_name", {
  repo <- tempfile("dvs-repo-")
  storage <- tempfile("dvs-storage-")
  dir.create(repo)
  old_wd <- getwd()
  on.exit(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
    },
    add = TRUE
  )
  setwd(repo)

  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "t@e.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "t")) == 0L)

  dvs_init(
    storage_path = storage,
    root_dir = repo,
    metadata_folder_name = "custom_meta"
  )
  expect_true(dir.exists(file.path(repo, "custom_meta")))
})
