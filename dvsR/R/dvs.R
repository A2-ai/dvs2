#' Add Files to DVS
#'
#' Add files to DVS tracking. Computes hashes, creates metadata files,
#' and copies files to storage.
#'
#' @param files Character vector of file paths or glob patterns to add.
#' @param message Optional character string describing this version.
#' @return A data.frame with columns: path, outcome, size, checksum, error, error_message.
#' @export
#' @examples
#' \dontrun{
#' dvs_add("data/large_file.csv")
#' dvs_add(c("data/*.csv", "models/*.rds"), message = "Initial data import")
#' }
dvs_add <- function(files, message = NULL) {
  json <- dvs_add_json(files, message)
  jsonlite::fromJSON(json)
}

#' Get Files from DVS Storage
#'
#' Retrieve files from DVS storage based on metadata.
#'
#' @param files Character vector of file paths or glob patterns to retrieve.
#'   If empty, retrieves all tracked files.
#' @return A data.frame with columns: path, outcome, size, checksum, error, error_message.
#' @export
#' @examples
#' \dontrun{
#' dvs_get("data/large_file.csv")
#' dvs_get(c("data/*.csv"))
#' dvs_get(character(0))  # Get all tracked files
#' }
dvs_get <- function(files = character(0)) {
  json <- dvs_get_json(files)
  jsonlite::fromJSON(json)
}

#' Check DVS File Status
#'
#' Check the status of tracked files by comparing local file hashes
#' with stored metadata.
#'
#' @param files Character vector of file paths or glob patterns to check.
#'   If empty, checks all tracked files.
#' @return A data.frame with columns: path, status, size, checksum, add_time, saved_by, message.
#' @export
#' @examples
#' \dontrun{
#' dvs_status()
#' dvs_status("data/large_file.csv")
#' dvs_status(c("data/*.csv"))
#' }
dvs_status <- function(files = character(0)) {
  json <- dvs_status_json(files)
  jsonlite::fromJSON(json)
}

#' Initialize DVS
#'
#' Initialize DVS for the current project. Creates a configuration
#' file and sets up the storage directory.
#'
#' @param storage_dir Character string specifying the path to the storage directory.
#' @param permissions Optional integer specifying file permissions (octal, e.g., 420 for 0644).
#' @param group Optional character string specifying the Unix group for stored files.
#' @return A list with initialization details (storage_dir, permissions, group, hash_algo, metadata_format).
#' @export
#' @examples
#' \dontrun{
#' dvs_init(".dvs-storage")
#' dvs_init(".dvs-storage", permissions = 420L)
#' dvs_init(".dvs-storage", group = "data-team")
#' }
dvs_init <- function(storage_dir, permissions = NULL, group = NULL) {
  json <- dvs_init_json(storage_dir, permissions, group)
  jsonlite::fromJSON(json)
}
