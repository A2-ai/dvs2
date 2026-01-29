# bootstrap.R - Run before package build (Config/build/bootstrap: TRUE)
# Sets NOT_CRAN=true so cargo vendor runs during GitHub installs

# Build environment with NOT_CRAN set
env <- c(Sys.getenv(), NOT_CRAN = "true", FORCE_VENDOR = "true")
env_strings <- paste0(names(env), "=", env)

if (.Platform$OS.type == "windows") {
  system2("bash", c("-l", "-c", "./configure.win"), env = env_strings)
} else {
  system2("./configure", env = env_strings)
}
