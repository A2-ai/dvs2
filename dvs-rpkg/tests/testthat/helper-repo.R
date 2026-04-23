new_dvs_test_repo <- function(envir = parent.frame()) {
  repo <- file.path(tempfile("dvs-repo-"))
  storage <- file.path(tempfile("dvs-storage-"))
  dir.create(repo)

  old_wd <- getwd()
  withr::defer(
    {
      setwd(old_wd)
      unlink(repo, recursive = TRUE, force = TRUE)
      unlink(storage, recursive = TRUE, force = TRUE)
    },
    envir = envir
  )
  setwd(repo)

  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "test@example.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "test")) == 0L)

  dvs_init(storage_path = storage, root_dir = repo)

  list(repo = repo, storage = storage)
}
