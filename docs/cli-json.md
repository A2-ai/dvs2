# CLI JSON protocol, version 1

Pass the global `--json` flag before or after a subcommand. A command writes one JSON
document to stdout, including startup, configuration and argument failures. Clients
must check both the result and the exit status. Stderr is reserved for diagnostics;
clients must never parse diagnostic text. Explicit `--help` and `--version` keep
conventional human output and exit 0. A `--json` after the argument terminator `--` is
a literal path, not a request for machine output.

Codes are the API; messages are display text and may change. Clients must tolerate
additional fields and unknown codes without treating them as success. New incompatible
semantics require a new protocol version.

`--emit-events` (instead of `--json`) streams NDJSON events while the command runs and
ends with the same result document; see [Events](#events).

## Envelope

Every command result, error results included, is one object:

| Field | Meaning |
| --- | --- |
| `v` | Protocol version, currently `1` |
| `command` | Command name; `cli` for argument parsing errors |
| `outcome` | `complete`, `partial` or `failed`; `get` also uses `aborted` |
| `dry_run` | Whether this is a requested mutation preview |
| `counts` | `{total, succeeded, failed}`; `get` adds its per-action counts (below) |
| `error` | `null`, or the command-level error `{code, message, hint}` |
| `files` | Per-file (or per-hash, per-entry) rows; empty for commands without rows |
| `started_at`, `finished_at` | RFC 3339 UTC with milliseconds, e.g. `2026-10-21T14:30:00.123Z` |
| `duration_ms` | Integer milliseconds the run took |

Commands add their own fields: `add`/`get` `bytes`, `get` `on_conflict`/`source`,
`init` `project_root`, `logout` `revoked`/`warnings`, `info` its discovery fields.

`error` describes a failure of the whole command; per-file failures stay in `files`. A
`partial` result normally has `error: null`. `hint`, when not `null`, is the next thing
to try, for people.

```json
{"v":1,"command":"add","outcome":"failed","dry_run":false,
 "counts":{"total":0,"succeeded":0,"failed":0},
 "bytes":{"total":0,"to_transfer":0,"already_stored":0,"transferred":0},
 "error":{"code":"no_files_selected","message":"Path not found: missing.txt","hint":null},
 "files":[],"started_at":"2026-10-21T14:30:00.120Z","finished_at":"2026-10-21T14:30:00.123Z","duration_ms":3}
```

### Counts

`total` is the number of rows. `failed` counts rows that failed; `succeeded` the rest.
`history` never fails per row.

`get` reports `total` and `succeeded` next to `restored`, `replaced`, `unchanged`,
`skipped`, `not_run`, `failed`, `would_restore`, `would_replace`. `succeeded` = rows at
their target afterwards (restored + replaced + unchanged; in a dry run also
would_restore + would_replace). `failed` = the `failed` action. Skipped and `not_run`
rows (held back by a conflict) are in neither.

### Bytes

`add` and `get` results (dry runs too) carry
`bytes: {total, to_transfer, already_stored, transferred}`:

- `total`: the sizes of the rows whose size is known.
- `to_transfer`: bytes that need moving: upload for `add`; for `get`, every row whose
  local copy does not match its target (held back, skipped and failed rows included).
- `already_stored`: the rest of `total` (`add`: storage already has the content;
  `get`: the local copy already matches). `to_transfer + already_stored = total`.
- `transferred`: bytes actually moved; `0` in a dry run.

## Rows

Every row of every command has fixed keys: a field that does not apply is `null`, never
missing. Every row carries the same error fields:

| Field | Meaning |
| --- | --- |
| `code` | `null` on success, else a code (below) |
| `message` | `null` on success, else the reason, for people |
| `hint` | `null`, or the next thing to try (only `get` fills it today) |

- `add`: `path`, `outcome` (`copied`, `present`), `hash`, `size`, `stored_size`; all
  `null` on an error row.
- `get`: `path`, `local_path`, `before`, `action`, `satisfied`, `size`, `target_hash`,
  `local_hash`, `stored_copy`.
- `status`: `path`, `status`, `metadata`, `working`, `baseline`.
- `hash`: `path`, `hashes`, `size`, `hash_source` (`cache`, `computed`), and
  `duration_ms`: the time spent hashing that file, `null` for a row that did no work.
- `availability`: `hash`, `state` (`present`, `absent`, `unknown`), `in_history`, and a
  second error pair `history_code`/`history_message` for the history lookup, which is a
  separate request that fails on its own. `in_history` and the history pair are `null`
  without `--path`.
- `history`: the audit entries: `operation_id`, `timestamp`, `user`, `action`, plus the
  common error fields.

All times are RFC 3339 UTC: `started_at`/`finished_at` and history `timestamp` with
milliseconds (the audit log on disk keeps unix seconds), a record's `add_time` as the
record stores it. The only exception is event `ts`, in unix milliseconds.

`path` values are relative to the project root (`info`'s `project_root`); `get` also
gives `local_path`, relative to the current directory.

## Codes

Backend, credentials and record failures share one vocabulary, at the top level
(`error.code`) and per row (`code`), in every command:

| code | when |
| --- | --- |
| `auth_required` | no token / not logged in |
| `auth_expired` | the server said 401 |
| `access_denied` | the server said 403 |
| `server_unavailable` | transport error, timeout, 5xx, unreadable answer |
| `server_too_old` | an endpoint that should exist answered 404/405 |
| `project_not_found` | the project does not exist on the server (404) |
| `bad_metadata` | an unreadable `.dvs` record, e.g. git conflict markers |
| `credentials_unreadable` | the credentials store exists but cannot be read |

Command-level codes: `not_in_project`, `invalid_configuration`, `invalid_arguments`,
`no_files_selected`, `authentication_required` (device login needs an interactive
`dvs login`), `io_error`, and `operation_failed`, the honest fallback when nothing more
specific is known (`get` also uses it when no tracked file matched).

Row codes, besides the shared ones:

- `add`: `file_failed`; the reason is in `message`.
- `get`: `local_changes`, `unknown_local`, `batch_aborted`, `not_tracked`, `not_a_file`,
  `unsafe_path`, `source_missing`, `hash_mismatch`, `destination_changed`,
  `permission_denied`, `file_in_use`, `io_error`.
- `status`: `local_read_failed`.
- `hash`: `local_read_failed` (symlinks), `io_error`.
- `availability`: `backend_read_failed` for an unclassified backend failure.
- `logout` `warnings`: `revoke_failed`.

## Exit status

One table for every command:

| exit | meaning |
| --- | --- |
| `0` | complete |
| `1` | failure, or some rows failed (`partial`/`failed`); `get` failures beat conflicts |
| `2` | usage or configuration: `invalid_arguments`, `invalid_configuration` |
| `3` | `get` only: local conflicts held files back and nothing else failed |

A result written before a nonzero exit is authoritative. No result after the process
was killed means the operation was interrupted: rescan before retrying.

## Events

`--emit-events` writes one JSON object per line. Every event has `v`, `operation_id`
(`"<pid>-<nanos>"`), `seq` (0, 1, 2, ...), `type`, `ts` (unix milliseconds) and
`elapsed_ms` (since the operation started).

| type | fields |
| --- | --- |
| `operation_started` | `command` |
| `waiting_for_auth` | `user_code`, `approval_url` (device login) |
| `transfer_started` | `path`, `total_bytes` |
| `transfer_progress` | `path`, `bytes`, `total_bytes`; at most 10 per second per transfer |
| `transfer_finished` | `path`, `bytes`, `total_bytes`, `success`, `duration_ms` |
| `file_result` | `result`: one row, the same shape as in the result's `files` |
| `operation_finished` | `result` (the full result document), `exit_code`, `started_at`, `finished_at`, `duration_ms`; always last |

`status` streams each `file_result` as that file's local check completes, in completion
order; the final result's `files` keeps its own order. Other commands emit one
`file_result` per row of their result just before `operation_finished`. Transfer events
describe transfer work only: the result is authoritative (a transfer can succeed and the
install still fail).

## Discovery

`dvs info --json` finds the project from the working directory. `project_root` and
`metadata_directory` are absolute native paths. `protocol.versions` lists the supported
result versions; `protocol.capabilities` names features (`json_results`,
`get_conflict_policy`, `connectivity_status`, `status_facts`, `availability`, `history`,
`event_stream`, `device_login_events`).

For a server project, `info` reports `logged_in`, `connection` (`available`,
`unavailable`, or `unknown` when not logged in), `connection_error` (`null`, or
`{code, message}` with `server_unavailable`, `project_not_found` or `server_too_old` when
`connection` is `unavailable`), `user`, `access` and `groups`. `access` is the server's
level (`read`, `write`, `admin`), `none` when the server says 403 (logged in, but no
membership in this project: `connection` is `available`, `connection_error` is `null` and
the exit status is `0`), `not_logged_in` without a token, or `null` when the server could
not say. A rejected login (`auth_expired`, a 401) is an error envelope. A local project
reports `connection: "not_applicable"`.

```json
{"v":1,"command":"info","outcome":"complete","dry_run":false,"counts":{"total":0,"succeeded":0,"failed":0},
 "error":null,"files":[],"version":"0.4.0","backend":"server","project":"proj","server":"https://h/dvs/",
 "compression":"zstd","logged_in":true,"connection":"unavailable",
 "connection_error":{"code":"project_not_found","message":"…"},"user":null,"access":null,"groups":null,
 "project_root":"/work/proj","metadata_directory":"/work/proj/.dvs","protocol":{"versions":[1],"capabilities":["…"]},
 "started_at":"2026-10-21T14:30:00.120Z","finished_at":"2026-10-21T14:30:00.180Z","duration_ms":60}
```

## Authentication

Machine mode never changes stored credentials: a 401 is `auth_expired` and the token
stays; an unparsable store is `credentials_unreadable` and stays. Only `dvs login` and
`dvs logout` change credentials. `login --json` reports `authentication_required`
before starting device login; with `--emit-events` it emits `waiting_for_auth` instead.
Credentials and device secrets are never written to results or events.

`logout --json` deletes the local credential even if remote revocation fails. `revoked`
is `true` for a confirmed revocation, `false` for a failed revoke call, or `null` when
there was no token. `warnings` is always an array of `{code, message}`.
