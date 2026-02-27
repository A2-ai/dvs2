fs_manual_dir_tree <- function(path = ".", recurse = TRUE, ...) {
  files <- fs::dir_ls(path, recurse = recurse, ...)
  by_dir <- split(files, fs::path_dir(files))
  ch <- fs:::box_chars()
  get_coloured_name <- function(x) {
    coloured <- fs:::colourise_fs_path(x)
    sub(x, fs::path_file(x), coloured, fixed = TRUE)
  }
  print_leaf <- function(x, indent) {
    leafs <- by_dir[[x]]
    for (i in seq_along(leafs)) {
      if (i == length(leafs)) {
        cat(
          indent,
          fs:::pc(ch$l, ch$h, ch$h, " "),
          get_coloured_name(leafs[[i]]),
          "\n",
          sep = ""
        )
        print_leaf(leafs[[i]], paste0(indent, "    "))
      } else {
        cat(
          indent,
          fs:::pc(ch$j, ch$h, ch$h, " "),
          get_coloured_name(leafs[[i]]),
          "\n",
          sep = ""
        )
        print_leaf(leafs[[i]], paste0(indent, fs:::pc(ch$v, "   ")))
      }
    }
  }
  cat(fs:::colourise_fs_path(path), "\n", sep = "")
  # print_leaf(fs::path_expand(path), "")
  print_leaf(path, "")
  invisible(files)
}
