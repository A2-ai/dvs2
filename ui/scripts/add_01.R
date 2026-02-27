
if (!exists("dvs_workspace")) {
  dvs_workspace <- getwd()
}
source(file.path(dvs_workspace, "ui/scripts/R", "tree.R"), echo = TRUE)

dvs_workspace
withr::with_dir(
  dvs_workspace,
  system2(
    "just",
    "install-cli",
  )
)

system2("dvs")

message("dvs repository where storage directory is different location")
proj_root <- file.path(tempfile(), "projectA")
proj_root
dir.create(proj_root, recursive = TRUE)
setwd(proj_root)
dir.create(file.path(proj_root, ".git/"))
message("define global storage directory")
storage_directory <- file.path(tempdir(), "dvs_data_directory")
list(
  project_root = proj_root,
  storage_root = storage_directory
)

message("storage directory is an ancestor to the project(s)")

message(
  "open visual studio code in `tempdir()` (session constant)",
  " to have an overview over all file changes"
)
browseURL(url = tempdir(), browser = "code")

message("dvs with a storage directory provided")
system2(
  "dvs",
  c("init", storage_directory)
)

cat("# DVS repository\n", file = file.path(proj_root, "README.md"))

# fs::dir_tree(
#   tempdir()
# ) |>
#   print() |>
#   fs::path_rel(file.path(tempdir())) |>
#   fs_manual_dir_tree()

fs::dir_tree(
  tempdir()
)
  tempdir() |> unclass()
  tempdir() |> fs::path_expand() |> unclass()

fs_manual_dir_tree(
  # tempdir()
  tempdir() |> fs::path_expand()
)


#'
#' Let's add something
#'

fs::dir_create(proj_root, "data")

write.table(
  file = file.path(proj_root, "data", "theoph_head.tab"),
  head(Theoph, 15),
  eol = "\n"
)

fs::dir_tree(
  tempdir()
)

message("data/theoph_head.tab:\n")

readLines(
  file.path(proj_root, "data", "theoph_head.tab")
) |>
  cat(sep = "\n")

system2(
  "dvs",
  c("add", "--help")
)

readLines(
  file.path(proj_root, "dvs.toml")
) |>
  cat(sep = "\n")

system2(
  "dvs",
  c(
    "add",
    file.path(proj_root, "data", "theoph_head.tab"),
    "--message",
    r"("added head of theoph in tab format")"
  )
)
# COMMENT: The backtrace need to be removed.

fs::dir_ls(proj_root, recurse = TRUE) |>
  fs::path_rel(start = fs::path(proj_root, ".."))


fs::dir_ls(storage_directory, recurse = TRUE) |>
  fs::path_rel(start = fs::path(storage_directory, ".."))

readLines(fs::path(storage_directory, "audit.log.jsonl")) |>
  cat(sep = "\n")

# Let's add the same file again

system2(
  "dvs",
  c(
    "add",
    file.path(proj_root, "data", "theoph_head.tab"),
    "--message",
    r"("ssadded head of theoph in tab format")"
  )
)
file.edit(fs::path(storage_directory, "audit.log.jsonl"))
readLines(fs::path(storage_directory, "audit.log.jsonl")) |>
  cat(sep = "\n")
#' The audit log has two entries, for files that has the exact same hash, but the time
#' is different.
#'
#'

write.table(
  file = file.path(proj_root, "data", "theoph_head3.tab"),
  head(Theoph, 12),
  eol = "\n"
)

system2(
  "dvs",
  c(
    "add",
    file.path(proj_root, "data", "theoph_head3.tab"),
    "--message",
    r"("added head of theoph in tab format")"
  )
)
