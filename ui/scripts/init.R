
if(!exists("dvs_workspace")) dvs_workspace <- getwd() 

dvs_workspace
system2(
  "just", "install-cli", 
)

system2("dvs")

proj_root <- file.path(tempfile(), "projectA")
proj_root
dir.create(proj_root, recursive = TRUE)

setwd(proj_root)

system2(
  "dvs", c("init", "--help")
)

system2(
  "dvs", c("init", proj_root)
)

message("Missing a .git folder, as otherwise `dvs init` does not work")

dir.create(file.path(proj_root, ".git/"))

system2(
  "dvs", c("init", proj_root)
)
# IDEA: Absolute path should be printed, atleast in the R package

fs::dir_tree(proj_root)

readLines(file.path(proj_root, "dvs.toml")) |> cat(sep="\n")

message("now, recreate a project with --no-compression")

proj_root <- file.path(tempfile(), "projectA")
proj_root
dir.create(proj_root, recursive = TRUE)

setwd(proj_root)
dir.create(file.path(proj_root, ".git/"))

system2(
  "dvs", c("init", proj_root, "--no-compression")
)
fs::dir_tree(proj_root)

readLines(file.path(proj_root, "dvs.toml")) |> cat(sep="\n")
