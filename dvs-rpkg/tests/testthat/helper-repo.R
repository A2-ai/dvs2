# Create a temp git repo + dvs storage, cd into the repo, and register an
# on.exit handler on the CALLING TEST's frame that restores the working
# directory and removes every file instantiated inside the repo and
# storage (recursively). Equivalent to writing
#
#   on.exit({
#     setwd(<old>)
#     unlink(<repo>, recursive = TRUE, force = TRUE)
#     unlink(<storage>, recursive = TRUE, force = TRUE)
#   }, add = TRUE)
#
# at the top of every test body.
new_dvs_test_repo <- function() {
  repo <- tempfile("dvs-repo-")
  storage <- tempfile("dvs-storage-")
  dir.create(repo)
  old_wd <- getwd()

  cleanup_expr <- bquote({
    setwd(.(old_wd))
    unlink(.(repo), recursive = TRUE, force = TRUE)
    unlink(.(storage), recursive = TRUE, force = TRUE)
  })
  do.call(
    "on.exit",
    list(cleanup_expr, add = TRUE),
    envir = parent.frame()
  )

  setwd(repo)
  stopifnot(system2("git", c("init", "-q")) == 0L)
  stopifnot(system2("git", c("config", "user.email", "test@example.com")) == 0L)
  stopifnot(system2("git", c("config", "user.name", "test")) == 0L)

  dvs_init(storage_path = storage, root_dir = repo)

  list(repo = repo, storage = storage)
}
