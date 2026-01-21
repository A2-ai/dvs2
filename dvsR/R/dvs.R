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

#' Push Files to Remote Storage
#'
#' Upload objects from local cache to remote storage server.
#'
#' @param remote_url Optional character string specifying the remote URL.
#'   If not provided, uses the URL from local config or manifest.
#' @return A list with push summary (uploaded, present, failed, results).
#' @export
#' @examples
#' \dontrun{
#' dvs_push()
#' dvs_push("https://dvs-server.example.com")
#' }
dvs_push <- function(remote_url = NULL) {
  json <- dvs_push_json(remote_url)
  jsonlite::fromJSON(json)
}

#' Pull Files from Remote Storage
#'
#' Download objects from remote storage server to local cache.
#'
#' @param remote_url Optional character string specifying the remote URL.
#'   If not provided, uses the URL from local config or manifest.
#' @return A list with pull summary (downloaded, cached, failed, results).
#' @export
#' @examples
#' \dontrun{
#' dvs_pull()
#' dvs_pull("https://dvs-server.example.com")
#' }
dvs_pull <- function(remote_url = NULL) {
  json <- dvs_pull_json(remote_url)
  jsonlite::fromJSON(json)
}

#' Materialize Files from Cache
#'
#' Copy cached objects to their working tree locations based on the manifest.
#'
#' @return A list with materialize summary (materialized, up_to_date, failed, results).
#' @export
#' @examples
#' \dontrun{
#' dvs_materialize()
#' }
dvs_materialize <- function() {
  json <- dvs_materialize_json()
  jsonlite::fromJSON(json)
}

#' View DVS Log
#'
#' Display the reflog showing the history of DVS state changes.
#'
#' @param limit Optional integer specifying maximum number of entries to return.
#'   If NULL, returns all entries.
#' @return A data.frame with columns: index, timestamp, actor, op, message,
#'   prev_state, new_state, files.
#' @export
#' @examples
#' \dontrun{
#' dvs_log()
#' dvs_log(limit = 10)
#' }
dvs_log <- function(limit = NULL) {
  json <- dvs_log_json(limit)
  jsonlite::fromJSON(json)
}

#' Rollback to Previous State
#'
#' Restore workspace state to a previous snapshot.
#'
#' @param target Character string specifying the target. Can be either:
#'   - A state ID (hex string)
#'   - A reflog index as a string (e.g., "0" for most recent, "1" for previous)
#' @param force Logical indicating whether to skip dirty working tree check.
#'   Default is FALSE.
#' @param materialize Logical indicating whether to materialize (copy) the
#'   data files to their working locations. Default is TRUE.
#' @return A list with rollback result (success, from_state, to_state,
#'   restored_files, removed_files, error).
#' @export
#' @examples
#' \dontrun{
#' # Rollback to the most recent state
#' dvs_rollback("0")
#'
#' # Rollback to a specific state ID
#' dvs_rollback("abc123def456")
#'
#' # Force rollback without materialization
#' dvs_rollback("1", force = TRUE, materialize = FALSE)
#' }
dvs_rollback <- function(target, force = FALSE, materialize = TRUE) {
  json <- dvs_rollback_json(target, force, materialize)
  jsonlite::fromJSON(json)
}
