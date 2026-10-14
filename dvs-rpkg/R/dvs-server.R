# Functions that ask a dvs server. Changing access, creating projects and
# yanking stay in the dvs CLI.

dvs_interactive <- function() {
  interactive()
}

dvs_login_prompt <- function(uri, code) {
  cli::cli_inform(c(
    "Open {.url {uri}} and check that it shows the code {.strong {code}}.",
    "i" = "Waiting for approval; press Esc or Ctrl-C to stop waiting."
  ))
}

#' Log in to a dvs server
#'
#' Starts a login in the browser: prints the page where you approve it and the
#' code that page must show, then waits for the approval and stores the token
#' for later sessions. You can stop waiting with Esc or Ctrl-C; a code approved
#' after that still logs you in.
#'
#' Logging in needs a browser, so it works only in interactive sessions. In
#' scripts and scheduled jobs, set the `DVS_TOKEN` environment variable to a
#' token instead; when it is set, dvs uses it rather than a stored login.
#'
#' @param url The server's URL, such as `"https://dvs.example.com"`. `NULL`
#'   (the default) uses the server of the project in the working directory.
#' @return `TRUE`, invisibly, once the login is approved.
#' @seealso [dvs_logout()], [dvs_info()]
#' @examples
#' \dontrun{
#' dvs_login("https://dvs.example.com")
#' }
#' @export
dvs_login <- function(url = NULL) {
  if (!dvs_interactive()) {
    dvs_abort("not_interactive", c(
      "dvs_login() approves a login in a browser, so it needs an interactive session.",
      "In scripts and jobs, set the DVS_TOKEN environment variable to a token instead."
    ))
  }
  if (nzchar(trimws(Sys.getenv("DVS_TOKEN")))) {
    cli::cli_inform(c("!" = "DVS_TOKEN is set, so dvs will keep using it instead of this login."))
  }
  handle <- dvs_call(dvs_login_start_impl(url))
  shown <- FALSE
  repeat {
    state <- dvs_call(dvs_login_poll_impl(handle))
    if (!shown && !is.null(state$uri)) {
      dvs_login_prompt(state$uri, state$code)
      shown <- TRUE
    }
    if (isTRUE(state$done)) {
      break
    }
    Sys.sleep(0.2)
  }
  until <- dvs_format_time(dvs_time(state$expires_at))
  cli::cli_inform("You are now logged in to {.url {state$url}} until {until}.")
  invisible(TRUE)
}

#' Log out of a dvs server
#'
#' Revokes the stored token on the server and forgets it here. A token in
#' `DVS_TOKEN` is not revoked: unset the variable to stop using it.
#'
#' @inheritParams dvs_login
#' @return `TRUE` invisibly if a stored token was forgotten, `FALSE` if there
#'   was none.
#' @export
dvs_logout <- function(url = NULL) {
  result <- dvs_call(dvs_logout_impl(url))
  for (note in unlist(result$notes)) {
    cli::cli_inform(c("!" = "{note}"))
  }
  if (isTRUE(result$logged_out)) {
    cli::cli_inform("Logged out of {.url {result$url}}.")
  } else {
    cli::cli_inform("You were not logged in to {.url {result$url}}.")
  }
  invisible(isTRUE(result$logged_out))
}

#' Who you are on a dvs server
#'
#' Asks the server who the token belongs to, and reports where the token comes
#' from, when it expires, the server's and this package's versions, and your
#' role on the project in the working directory. Without a token, the
#' identity fields are `NA`.
#'
#' @inheritParams dvs_login
#' @return A `dvs_info` list with `server`, `server_version`, `api_versions`,
#'   `client_version`, `user`, `email`, `global_admin`, `groups`,
#'   `token_source` (`"DVS_TOKEN"`, `"dvs_login"` or `NA`), `token_expires_at`
#'   (POSIXct UTC), `project`, `project_role` and `project_locked`.
#' @examples
#' \dontrun{
#' info <- dvs_info()
#' info$project_role
#' }
#' @export
dvs_info <- function(url = NULL) {
  x <- dvs_call(dvs_info_impl(url))
  structure(
    list(
      server = dvs_scalar(x$server, "chr"),
      server_version = dvs_scalar(x$server_version, "chr"),
      api_versions = as.integer(unlist(x$api_versions)),
      client_version = dvs_scalar(x$client_version, "chr"),
      user = dvs_scalar(x$user, "chr"),
      email = dvs_scalar(x$email, "chr"),
      global_admin = dvs_scalar(x$global_admin, "lgl"),
      groups = as.character(unlist(x$groups)),
      token_source = dvs_scalar(x$token_source, "chr"),
      token_expires_at = dvs_scalar(x$token_expires_at, "time"),
      project = dvs_scalar(x$project, "chr"),
      project_role = dvs_scalar(x$project_role, "chr"),
      project_locked = dvs_scalar(x$project_locked, "lgl")
    ),
    class = "dvs_info"
  )
}

#' @export
print.dvs_info <- function(x, ...) {
  cat("Server:  ", x$server, " (dvs-server ", x$server_version, ", API ",
    paste(x$api_versions, collapse = ", "), ")\n",
    sep = ""
  )
  cat("Client:  dvs ", x$client_version, "\n", sep = "")
  if (is.na(x$user)) {
    cat("User:    not logged in; run dvs_login(), or set DVS_TOKEN\n")
  } else {
    cat("User:    ", x$user, if (!is.na(x$email)) paste0(" <", x$email, ">"),
      if (isTRUE(x$global_admin)) " (global admin)", "\n",
      sep = ""
    )
    cat("Token:   from ", x$token_source,
      if (!is.na(x$token_expires_at)) paste0(", expires ", dvs_format_time(x$token_expires_at)),
      "\n",
      sep = ""
    )
  }
  if (!is.na(x$project_role)) {
    cat("Project: ", x$project, ", your role: ", x$project_role,
      if (isTRUE(x$project_locked)) " (locked: uploads are refused)", "\n",
      sep = ""
    )
  }
  invisible(x)
}

new_dvs_project <- function(x) {
  lock <- if (isTRUE(x$locked) && !is.null(x$locked_by)) {
    list(
      by = dvs_scalar(x$locked_by, "chr"),
      at = dvs_scalar(x$locked_at, "time"),
      reason = dvs_scalar(x$lock_reason, "chr")
    )
  }
  grants <- dvs_frame(
    list(
      group = x$grant_group, role = x$grant_role, removed = x$grant_removed,
      group_id = x$grant_group_id
    ),
    dvs_spec$grants
  )
  structure(
    list(
      name = dvs_scalar(x$name, "chr"),
      server = dvs_scalar(x$server, "chr"),
      your_role = dvs_scalar(x$your_role, "chr"),
      locked = dvs_scalar(x$locked, "lgl"),
      lock = lock,
      compression = dvs_scalar(x$compression, "chr"),
      created_at = dvs_scalar(x$created_at, "time"),
      created_by = dvs_scalar(x$created_by, "chr"),
      repo_commit_url = dvs_scalar(x$repo_commit_url, "chr"),
      files = dvs_scalar(x$files, "int"),
      versions = dvs_scalar(x$versions, "int"),
      uploads = dvs_scalar(x$uploads, "int"),
      size = dvs_scalar(x$size, "bytes"),
      stored_size = dvs_scalar(x$stored_size, "bytes"),
      last_upload_at = dvs_scalar(x$last_upload_at, "time"),
      last_activity_at = dvs_scalar(x$last_activity_at, "time"),
      grants = grants
    ),
    class = "dvs_project"
  )
}

#' Attach the working directory to a server project
#'
#' Checks that the project exists on the server and that you can read it, then
#' writes `dvs.toml` and creates the metadata folder. Metadata files already in
#' the folder, such as those of a fresh clone, are kept. Without a token for
#' the server, an interactive session logs in first with [dvs_login()].
#'
#' Creating a project on a server is done with the dvs CLI.
#'
#' @param name The project's name on the server.
#' @param url The server's URL.
#' @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
#' @return The project, as [dvs_project_info()] describes it, invisibly.
#' @examples
#' \dontrun{
#' dvs_attach("pk-study-101", "https://dvs.example.com")
#' }
#' @export
dvs_attach <- function(name, url, metadata_folder_name = NULL) {
  if (!dvs_call(dvs_has_token_impl(url)) && dvs_interactive()) {
    dvs_login(url)
  }
  project <- dvs_call(dvs_attach_impl(name, url, metadata_folder_name))
  project <- new_dvs_project(project)
  cli::cli_inform(paste0(
    "Attached {project$name} (your role: {project$your_role})",
    if (isTRUE(project$locked)) ", locked: uploads are refused" else "",
    "."
  ))
  invisible(project)
}

#' The server project in the working directory
#'
#' @return A `dvs_project` list: `name`, `server`, `your_role`, `locked`,
#'   `lock` (`NULL`, or a list of `by`, `at` and `reason`), `compression`,
#'   `created_at`, `created_by`, `repo_commit_url`, what the project holds
#'   (`files`, `versions`, `uploads`, `size` and `stored_size` of the latest
#'   versions, `last_upload_at`, `last_activity_at`), and `grants`, a tibble of
#'   the groups with access and their roles.
#' @examples
#' \dontrun{
#' project <- dvs_project_info()
#' project$lock$reason
#' project$grants
#' }
#' @export
dvs_project_info <- function() {
  project <- dvs_call(dvs_project_info_impl())
  new_dvs_project(project)
}

#' @export
print.dvs_project <- function(x, ...) {
  cat("Project:   ", x$name, "\n", sep = "")
  cat("Your role: ", x$your_role, "\n", sep = "")
  if (!is.null(x$lock)) {
    cat("Locked:    by ", x$lock$by, " on ", dvs_format_time(x$lock$at),
      if (!is.na(x$lock$reason)) paste0(": ", x$lock$reason),
      "; uploads are refused, downloads still work\n",
      sep = ""
    )
  } else if (isTRUE(x$locked)) {
    cat("Locked:    uploads are refused, downloads still work\n")
  } else {
    cat("Locked:    no\n")
  }
  cat("Created:   ", dvs_format_time(x$created_at), " by ", x$created_by, "\n", sep = "")
  if (!is.na(x$files)) {
    cat("Holds:     ", x$files, if (x$files == 1L) " file, " else " files, ",
      format_byte_size(x$size), " in their latest versions, ",
      format_byte_size(x$stored_size), " of distinct content stored\n",
      sep = ""
    )
  }
  if (!is.na(x$last_activity_at)) {
    cat("Activity:  last on ", dvs_format_time(x$last_activity_at), "\n", sep = "")
  }
  cat("Groups:\n")
  for (i in seq_len(nrow(x$grants))) {
    cat("  ", x$grants$group[i], " ", x$grants$role[i],
      if (isTRUE(x$grants$removed[i])) " (deleted from Arx: gives nobody access)", "\n",
      sep = ""
    )
  }
  invisible(x)
}

#' The projects you can read on a dvs server
#'
#' @param role Only projects where your role is at least this: `"read"`,
#'   `"write"` or `"admin"`. `NULL` (the default) for every project you can
#'   read.
#' @param locked `TRUE` for locked projects only, `FALSE` for unlocked ones
#'   only, `NULL` (the default) for both.
#' @param url The server's URL. `NULL` (the default) uses the server of the
#'   project in the working directory.
#' @return A tibble, one row per project: `name`, `your_role`, `locked`,
#'   `locked_by`, `locked_at`, `lock_reason`, `files`, `size`, `stored_size`,
#'   `last_activity_at`, `created_at`, `created_by`.
#' @examples
#' \dontrun{
#' dvs_projects(role = "write")
#' }
#' @export
dvs_projects <- function(role = NULL, locked = NULL, url = NULL) {
  if (!is.null(role)) {
    role <- match.arg(role, c("read", "write", "admin"))
  }
  pager <- dvs_call(dvs_projects_impl(role = role, locked = locked, url = url))
  dvs_collect(pager, dvs_spec$projects)
}

#' Who has access to the server project in the working directory
#'
#' Access is given to directory groups. Changing it is done with the dvs CLI
#' or on the project's Access page.
#'
#' @return A tibble, one row per group: `group`, `role` (`"read"`, `"write"`
#'   or `"admin"`), `removed` (the group was deleted from the directory, so the
#'   grant gives nobody access) and `group_id`.
#' @export
dvs_access <- function() {
  grants <- dvs_call(dvs_access_impl())
  dvs_frame(grants, dvs_spec$grants)
}

#' The files uploaded to the server project in the working directory
#'
#' Every path ever uploaded, with its latest upload. The server knows uploads,
#' not which of them a Git branch uses.
#'
#' @param prefix Only paths starting with this, from the project's top folder,
#'   such as `"data/derived/"`.
#' @param glob Only paths matching this pattern: `*` within a folder, `**`
#'   across folders.
#' @param min_size,max_size Only files at least / at most this large: a number
#'   of bytes, or a size such as `"100MiB"`.
#' @param user Only files whose latest version these people uploaded: their
#'   usernames or emails, or `"me"`.
#' @param since Only files uploaded in the last `"7d"`, `"12h"`, `"2w"`, ….
#' @param from,to Only files uploaded from / until this time: a Date, a
#'   POSIXct, or a string such as `"2026-10-01"`. A date as `to` includes that
#'   day.
#' @param branch Only files whose latest upload came from these branches.
#' @param hash Only files whose latest version has this hash, or a hash
#'   starting with these (at least 8) characters.
#' @param yanked `TRUE` for files whose latest version is yanked only.
#' @param sort `"path"`, `"uploaded_at"`, `"size"`, `"versions"` or `"user"`;
#'   put `-` first for descending, as in `"-size"`.
#' @param max_rows The most rows to return; a warning says when there are
#'   more.
#' @return A tibble, one row per path: `path`, `size`, `stored_size`, `hash`,
#'   `uploaded_at`, `uploaded_by`, `branch`, `commit`, `git_state`,
#'   `versions`, `first_uploaded_at`, `yanked`.
#' @examples
#' \dontrun{
#' dvs_remote_files("data/", min_size = "1GiB", sort = "-size")
#' }
#' @export
dvs_remote_files <- function(
  prefix = NULL,
  glob = NULL,
  min_size = NULL,
  max_size = NULL,
  user = NULL,
  since = NULL,
  from = NULL,
  to = NULL,
  branch = NULL,
  hash = NULL,
  yanked = NULL,
  sort = NULL,
  max_rows = 1e5
) {
  min_size <- dvs_size_arg(min_size, "min_size")
  max_size <- dvs_size_arg(max_size, "max_size")
  from <- dvs_time_arg(from, "from")
  to <- dvs_time_arg(to, "to")
  pager <- dvs_call(dvs_remote_files_impl(
    prefix = prefix,
    glob = glob,
    min_size = min_size,
    max_size = max_size,
    user = dvs_chr_arg(user),
    since = since,
    from = from,
    to = to,
    branch = dvs_chr_arg(branch),
    hash = hash,
    yanked = yanked,
    sort = sort
  ))
  dvs_collect(pager, dvs_spec$files, max_rows = max_rows)
}

#' Every version of a file on the server
#'
#' The contents uploaded at a path of the server project in the working
#' directory, newest first.
#'
#' @param path The file, relative to the working directory.
#' @return A tibble, one row per version: `hash`, `latest` (the newest),
#'   `size`, `stored_size`, `first_uploaded_at`, `last_uploaded_at`,
#'   `uploads` (times this content was uploaded at the path), `downloads`,
#'   `yanked`, and `other_paths` (other paths in the project holding the same
#'   content).
#' @examples
#' \dontrun{
#' dvs_log("data/derived/pk.csv")
#' }
#' @export
dvs_log <- function(path) {
  pager <- dvs_call(dvs_log_impl(path))
  versions <- dvs_collect(pager, dvs_spec$versions)
  versions$latest <- seq_len(nrow(versions)) == 1L
  versions
}

#' The activity of the server project in the working directory
#'
#' Uploads, downloads and project changes (locks, access changes, yanks, …),
#' newest first. Every page is read, up to `max_rows`.
#'
#' @param kind `"upload"`, `"download"`, or a project change: `"lock"`,
#'   `"unlock"`, `"access_change"`, `"yank"`, `"unyank"`, `"project_create"`,
#'   `"settings_change"`, `"project_rename"`. Several are allowed.
#' @param user Only what these people did: their usernames or emails, or
#'   `"me"`.
#' @param since Only the last `"7d"`, `"12h"`, `"2w"`, ….
#' @param from,to From / until this time: a Date, a POSIXct, or a string such
#'   as `"2026-10-01"`. A date as `to` includes that day.
#' @param path Files, folders ending in `/`, or a pattern with `*`, relative to
#'   the working directory.
#' @param hash A content hash, or its first 8 or more characters.
#' @param branch The branches the person's checkout was on.
#' @param commit The commit the person's checkout was on: its first 7 or more
#'   characters.
#' @param batch One run of `dvs_add()` or `dvs_get()`, by the id in the `batch`
#'   column.
#' @param max_rows The most rows to return; a warning says when there are
#'   more.
#' @return A tibble, one row per event: `at`, `kind`, `user`, `global_admin`
#'   (a change made through global admin rather than a grant), `path`, `hash`,
#'   `size`, `stored_size`, `branch`, `commit`, `git_state`, `message` (of
#'   `dvs_add()`), `reason` (of a lock or yank), `batch`, `client`, `detail`
#'   (a project change's before and after, as JSON) and `id`.
#' @examples
#' \dontrun{
#' dvs_activity(kind = "upload", since = "7d")
#' dvs_activity(path = "data/", user = "me")
#' }
#' @export
dvs_activity <- function(
  kind = NULL,
  user = NULL,
  since = NULL,
  from = NULL,
  to = NULL,
  path = NULL,
  hash = NULL,
  branch = NULL,
  commit = NULL,
  batch = NULL,
  max_rows = 1e5
) {
  from <- dvs_time_arg(from, "from")
  to <- dvs_time_arg(to, "to")
  pager <- dvs_call(dvs_activity_impl(
    kind = dvs_chr_arg(kind),
    user = dvs_chr_arg(user),
    since = since,
    from = from,
    to = to,
    path = dvs_chr_arg(path),
    hash = hash,
    branch = dvs_chr_arg(branch),
    commit = commit,
    batch = batch
  ))
  dvs_collect(pager, dvs_spec$activity, max_rows = max_rows)
}
