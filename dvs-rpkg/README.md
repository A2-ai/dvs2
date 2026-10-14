# dvs

R interface to dvs.

## Install

```r
remotes::install_github("A2-ai/dvs2", subdir = "dvs-rpkg")
```

## Server projects

A project on a dvs server is used from R the same way as a local one: attach
a checkout once, then `dvs_add()`, `dvs_get()` and `dvs_status()` work as
usual. A few more functions read what the server knows:

```r
library(dvs)

dvs_login("https://dvs.example.com")      # once: approve in the browser
dvs_attach("pk-study-101", "https://dvs.example.com")

dvs_info()                       # who you are, the token, your role here
dvs_project_info()               # the project: lock (who, when, why), groups
dvs_access()                     # groups with access and their roles
dvs_projects(role = "write")     # projects you can write to
dvs_status(remote = TRUE)        # your files compared with the server
dvs_remote_files("data/", min_size = "1GiB", sort = "-size")
dvs_log("data/derived/pk.csv")   # every version of a file, newest first
dvs_activity(kind = "upload", since = "7d")
dvs_logout()
```

- Results are tibbles, typed even when empty: sizes are `dvs_bytes`, times
  are POSIXct in UTC, hashes are in full.
- `dvs_login()` needs an interactive session. In scripts and scheduled jobs,
  set `DVS_TOKEN` to a token instead.
- Errors are conditions of class `dvs_error_<code>` and `dvs_error`, with the
  fields `code`, `status`, `message` and `request_id` (see `?dvs_error`):

  ```r
  tryCatch(dvs_activity(), dvs_error_token_expired = function(e) dvs_login())
  ```

- When some files fail, `dvs_add()` and `dvs_get()` stop with a
  `dvs_error_files_failed` error whose `results` field holds every file's
  result; `on_error = "collect"` returns the results instead.
- `dvs_get()` makes each file match its `.dvs` pointer, overwriting local
  changes.
- Creating projects, changing access, locking and yanking are done with the
  dvs CLI or in the web UI.
