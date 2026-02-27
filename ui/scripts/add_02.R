#' We will make two DVS repositories, choose a common storage directory, but then
#' we will store two files that are distinct, and try to store them again in the same
#' repo, and then inspect the storage directory under these actions/events.
#'

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
proj_root_a <- file.path(tempfile(), "projectA")
proj_root_a
dir.create(proj_root_a, recursive = TRUE)
dir.create(file.path(proj_root_a, ".git/"))
message("define global storage directory")
storage_directory <- file.path(tempdir(), "dvs_data_directory")
storage_directory

message("dvs repository for project A with a storage directory provided")
setwd(proj_root_a)
system2(
  "dvs",
  c("init", storage_directory)
)


proj_root_b <- file.path(tempfile(), "projectB")
dir.create(proj_root_b, recursive = TRUE)
dir.create(file.path(proj_root_b, ".git/"))

message("dvs repository for project B with a storage directory provided")
setwd(proj_root_b)
system2(
  "dvs",
  c("init", storage_directory)
)

# create data directories

fs::dir_create(proj_root_a, "data")
fs::dir_create(proj_root_b, "data")

# store one data file in the two projects

write.table(
  file = file.path(proj_root_a, "data", "theoph_head_15.tab"),
  head(Theoph, 15),
  eol = "\n"
)
write.table(
  file = file.path(proj_root_b, "data", "theoph_head_15.tab"),
  head(Theoph, 15),
  eol = "\n"
)

# store another file in the two projects

write.table(
  file = file.path(proj_root_a, "data", "theoph_head_23.tab"),
  head(Theoph, 23),
  eol = "\n"
)
write.table(
  file = file.path(proj_root_b, "data", "theoph_head_23.tab"),
  head(Theoph, 23),
  eol = "\n"
)


# add two 15/23 files to projects a and b

setwd(proj_root_a)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_a, "data", "theoph_head_23.tab"),
    "--message",
    r"("added head(23) of theoph in tab format")"
  )
)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_a, "data", "theoph_head_15.tab"),
    "--message",
    r"("added head(15) of theoph in tab format")"
  )
)

# <inspection> audit log in common storage

fs::dir_tree(
  tempdir(),
  recurse = TRUE,
  all = TRUE,
  # invert = TRUE,
  # glob = "*.git"
)

readLines(fs::path(storage_directory, "audit.log.jsonl")) |>
  cat(sep = "\n")


# <continue> add the two files to project B

setwd(proj_root_b)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_b, "data", "theoph_head_23.tab"),
    "--message",
    r"("added head(23) of theoph in tab format")"
  )
)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_b, "data", "theoph_head_15.tab"),
    "--message",
    r"("added head(15) of theoph in tab format")"
  )
)

fs::dir_tree(
  tempdir(),
  recurse = TRUE,
  all = TRUE,
  invert = TRUE,
  glob = "*.git"
)


readLines(fs::path(storage_directory, "audit.log.jsonl")) |>
  cat(sep = "\n")

#' Add the Theoph(15) file, with the same message twice.
#'

setwd(proj_root_a)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_a, "data", "theoph_head_15.tab"),
    "--message",
    r"("added head(23) of theoph in tab format")"
  )
)
setwd(proj_root_b)
system2(
  "dvs",
  c(
    "add",
    file.path(proj_root_b, "data", "theoph_head_15.tab"),
    "--message",
    r"("added head(15) of theoph in tab format")"
  )
)

fs::dir_tree(
  tempdir(),
  recurse = TRUE,
  all = TRUE,
  glob = "*/.git",
  invert = TRUE
)

readLines(fs::path(storage_directory, "audit.log.jsonl")) |>
  cat(sep = "\n")


message(
  "open visual studio code in `tempdir()` (session constant)",
  " to have an overview over all file changes"
)
browseURL(url = tempdir(), browser = "code")
