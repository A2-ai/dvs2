# bootstrap.R - Run before package build (Config/build/bootstrap: TRUE)

if (.Platform$OS.type == "windows") {
  system2("bash", c("-l", "-c", "./configure.win"))
} else {
  system2("./configure")
}
