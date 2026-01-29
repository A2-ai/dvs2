# bootstrap.R - Run before package build (Config/build/bootstrap: TRUE)

if (.Platform$OS.type == "windows") {
  system2("bash", c("-l", "-c", "./configure.win"), env = c(NOT_CRAN = "true"))
} else {
  system2("./configure", env = c(NOT_CRAN = "true"))
}
