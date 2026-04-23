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
