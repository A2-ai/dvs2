# CLI JSON protocol, version 1

Pass the global `--json` flag before or after a subcommand. A command writes one
JSON document to stdout, including startup/configuration failures. Clients must
check both the result and the exit status. Stderr is reserved for diagnostics;
clients must never parse diagnostic text. Explicit `--help` and `--version` retain
conventional human output and exit successfully. A `--json` after the argument
terminator `--` is a literal path, not a request for machine output.

This introduces a breaking change to earlier experimental output: `add` and
`status` now wrap their former result arrays in `files`. `info` keeps its prior
fields and adds the common envelope and discovery fields. The existing `get`
result, conflict policy, per-file facts, reason codes, counts, and exit semantics
are preserved; the new nullable `error` field is additive.

Every command result contains:

| Field | Meaning |
| --- | --- |
| `v` | Protocol version, currently `1` |
| `command` | Command name; `cli` for argument parsing errors |
| `outcome` | `complete`, `partial`, `failed`; get also uses `aborted` |
| `dry_run` | Whether this is a requested mutation preview |
| `counts` | Command-specific summary; add/status use `total`, `succeeded`, `failed` |
| `problem` | Nullable human-readable command-level error (retained for compatibility) |
| `error` | Nullable object with stable `code` and human-readable `message` |
| `files` | Per-file result array; empty for commands without file results |

`error` describes a shared failure; per-file failures remain in `files`. A partial
result can have `error: null`. Messages are display text and may change. Clients
must tolerate additional fields and unknown reason codes without treating them
as success. New incompatible semantics require a new protocol version.

Global error codes are `not_in_project`, `invalid_configuration`,
`invalid_arguments`, `no_files_selected`, `authentication_required`, `io_error`,
and `operation_failed`. The last is the honest fallback when the underlying
operation supplies no typed classification. A valid add selection that finds
no files uses `no_files_selected` and exits `1`; malformed CLI usage uses
`invalid_arguments` and exits `2`. Add/status rows have nullable `code`, with
`file_failed` for failures; their existing nested result fields remain intact.
Get has its existing more specific row codes. Consumers must not infer error
types from messages.

Exit statuses are `0` for success, `1` for operation or per-file failure, `2` for
argument/configuration errors, and `3` for get conflicts without another failure.
A result emitted before a nonzero exit is authoritative. No JSON result after a
process is killed means the operation was interrupted: rescan before retrying.

## Discovery

`dvs info --json` always discovers local project facts without requiring a
separate offline flag. `project_root` and `metadata_directory` are absolute
native paths. Existing file result `path` values are relative to the project
root; get additionally supplies `local_path` for human-facing invocations.
Clients should use `path` with `project_root` and must not assume that every path
is relative to the invoking directory.

`protocol.versions` lists supported result versions; `protocol.capabilities`
currently contains `json_results`, `get_conflict_policy`, and
`connectivity_status`. For a local backend, `connection` is
`not_applicable`. For a server backend, `connection` is `unknown` when no
credential is available, `available` when the access request reaches the
server, and `unavailable` when the server cannot be reached or is failing.
When unavailable, local project facts are still returned, with
`connection_error` describing the failed check and access-specific fields left
unknown. A successful connection check is only a point-in-time observation.

## Authentication

`login --json` reports `authentication_required` before initiating device login.
Use interactive `dvs login` to authenticate. Server init/attach also report this
error before starting login if credentials are absent. Device authorization needs
a streaming protocol to expose approval instructions while waiting; a single
result document cannot carry those instructions in time. Credentials and device
secrets must never be returned in result documents.

`logout --json` emits a complete result after deleting the local credential even
if remote revocation fails. `revoked` is true for confirmed revocation, false for
a failed revoke call, or null when there was no token. `warnings` is always an
array; a revocation failure produces a `revoke_failed` warning. A warning is not
an assertion that the remote token was revoked.
