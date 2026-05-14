# `dvs-cli` internals

The `dvs-cli` crate lives at `/dvs-cli/`. The entire crate is a single source file, `dvs-cli/src/main.rs`, roughly 17 KB and 474 lines.

## Purpose and shape

`dvs-cli` is a thin CLI binary that wraps the `dvs` core crate. It owns no storage logic, hashing, or compression: those all live in `dvs/`. Its responsibilities are argument parsing, TTY detection, progress-bar lifecycle, table and JSON rendering, and exit-code discipline.

The crate uses clap 4 with the `derive` feature (`Parser` plus `Subcommand`) and exposes four subcommands: `init`, `add`, `status`, and `get`. The compiled binary is named `dvs` (declared via `[[bin]] name = "dvs"` in `Cargo.toml`), not `dvs-cli`.

## Command inventory

| Subcommand | Args and flags | `dvs::` call (`main.rs` line, then core symbol) |
|---|---|---|
| `init` | `<path>` (positional), `--root-dir`, `--metadata-folder-name`, `--group`, `--no-compression` | `main.rs:225`, calls `dvs::init::init` (`dvs/src/init.rs:13`) |
| `add` | `[paths…]` (required unless `--glob`), `--glob`, `-m/--message`, `--dry-run` | `main.rs:251`, calls `dvs::add_files` (`dvs/src/files/add.rs:90`) |
| `status` | `[paths…]`, `-r/--recursive`, `--current`, `--absent`, `--unsynced`, `--with-metadata` | `main.rs:323`, calls `dvs::get_status` (`dvs/src/files/status.rs:131`) |
| `get` | `[paths…]` (required unless `--glob`), `--glob`, `--dry-run` | `main.rs:423`, calls `dvs::get_files` (`dvs/src/files/get.rs:115`) |

Several supporting core calls are used across commands. `dvs::globbing::resolve_paths_for_add` is at `dvs/src/globbing.rs:28`, and `dvs::globbing::resolve_paths_for_get` is at `dvs/src/globbing.rs:96`. `dvs::set_num_threads` is at `dvs/src/utils.rs:16`, and `dvs::format_size` is at `dvs/src/utils.rs:35`.

## Argument parsing

The top-level struct is defined at `main.rs:91-104`:

```rust
#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true)]
pub struct Cli {
    /// Output results as JSON
    #[clap(long, global = true)]
    pub json: bool,

    /// Number of threads for parallel operations (0 = auto-detect)
    #[clap(long, global = true)]
    pub threads: Option<usize>,

    #[clap(subcommand)]
    pub command: Command,
}
```

A few non-obvious clap details are worth pointing out. `subcommand_negates_reqs = true` suppresses the "required argument missing" error when only `--version` or `--help` is passed at the top level. Without it, the required `command` field would make `dvs --version` fail. The `required_unless_present = "glob"` attribute on `Add.paths` and `Get.paths` (`main.rs:46, 81`) means either positional paths or `--glob` must be given. `global = true` on `--json` and `--threads` makes those flags acceptable before or after the subcommand name on the command line. There are no custom value parsers in use; the only types in play are `PathBuf`, `String`, `bool`, and `Option<usize>`, all of which clap handles out of the box. No env-var bindings are declared in clap attributes either. The only environment variable consumed is `RUST_LOG`, via `env_logger::init()` at `main.rs:184`.

## Output and UX layer

Normal output goes to stdout via `println!`, while errors (both per-file and fatal) go to stderr via `eprintln!`. This stream split is consistent across all four commands.

In JSON mode (`--json`), each command serializes its result type with `serde_json`. The `add` and `get` commands serialize the full `Vec<AddResult>` or `Vec<GetResult>` from the core crate. The `status` command serializes `Vec<FileStatus>`. The `init` command emits `{"status": "initialized"}`, hand-built with the `json!` macro. When JSON mode is on, it entirely replaces the table and per-line output, and progress bars are also suppressed.

Table output is used only by the `status` command. It uses `tabled` 0.20, with two row variants: `StatusRow` (path, status, size) for the default view, and `StatusRowFull<'a>` (which adds hash, created_by, add_time, compression, and message) when `--with-metadata` is passed. Both tables right-align the size column and left-align the header row.

Progress bars are driven by `indicatif 0.18`, using a `MultiProgress` wrapping individual `ProgressBar`s. Progress is enabled only when all three of the following conditions hold (`main.rs:248, 420`):

```rust
let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();
```

The bar template is `{bar:40} {pos_fmt}/{len_fmt} ({percent}%) | {msg}`, where `pos_fmt` and `len_fmt` are custom keys that call `format_size` so the bar shows human-readable byte counts like `45 MiB / 1.2 GiB`. Each file gets its own `ProgressBar` added to the `MultiProgress`. Bars for successful files call `finish_and_clear()`; abandoned files call `pb.abandon()`.

Progress is also gated on a per-file size threshold from `Config::progress_bytes_threshold()` (`main.rs:250, 422`). The threshold defaults to 500 MiB (524,288,000 bytes) and can be overridden in the `[cli]` section of `dvs.toml` (see `dvs/src/config.rs:15, 221`). Files below the threshold get a no-op `FileProgress` whose callbacks are both empty closures.

Logging is set up by an unconditional `env_logger::init()` at startup (`main.rs:184`). The `RUST_LOG` environment variable controls level and filter. There is no explicit verbosity flag on the CLI.

For exit codes, `main()` at `main.rs:468-473` calls `try_main()`. On `Err` it prints `Error: {e}` to stderr and exits with code 1, and on success it exits 0. Partial failures (some files in a batch failing) are tracked in a `has_errors` bool; if that bool is set, `try_main` returns `Err(anyhow!("Some files failed to ..."))` after printing all per-file results, which produces exit code 1 plus a generic summary line on top of the per-file errors that were already printed.

## CLI-specific details not in the core crate

TTY detection uses `std::io::stderr().is_terminal()` (`main.rs:248, 420`) from the standard library `IsTerminal` trait (imported at `main.rs:1`). It is used solely to gate progress bars on stderr. There is no TTY detection for stdout, no color handling, and no terminal-width querying.

The CLI has no explicit color handling and no `colored`, `termcolor`, or `owo-colors` dependency. Neither `indicatif` nor `tabled` is configured to use color.

The CLI does not handle signals: there is no `ctrlc` crate and no `signal-hook`. Ctrl-C therefore causes an unclean exit out of whatever blocking I/O is in progress.

The only environment variable read by the CLI layer itself is `RUST_LOG` (via `env_logger`). No `DVS_*` env vars are consumed at the CLI layer.

The `init` command does its own path canonicalization at `main.rs:213-219`. The storage path may not yet exist, so the CLI applies a three-way resolution: if the full path exists, it calls `canonicalize`; if only the parent exists, it canonicalizes the parent and appends the leaf; otherwise it uses `std::path::absolute` (which does not resolve symlinks). The root directory is always fully canonicalized, since it must exist. The subsequent guard `abs_storage.starts_with(&abs_root)` prevents initializing a repository whose storage lives inside its own working tree.

## Quirks and gotchas

When a file is already present in storage, the `add` command reports it as `Outcome::Present` but prints nothing for it (`main.rs:294`, an empty match arm). Only newly copied files produce output. This is correct behavior but can confuse users who expect per-path confirmation.

The `make_progress_callback` factory at `main.rs:157-181` captures `MultiProgress` by move. The `MultiProgress` is constructed inside the closure factory and lives for the lifetime of the callback. It is passed into the core functions as `Option<&on_file_start>`, so the `MultiProgress` drop happens after `add_files` or `get_files` returns. That ordering is correct.

Because per-file errors are printed during result iteration, the final `return Err(...)` in `try_main` causes `main()` to print a second, generic line of the form `Error: Some files failed to add/get/get status` on stderr. This double-error pattern is intentional (it ensures a non-zero exit code carries an explicit summary), but it does produce slightly redundant output.

`StatusRowFull` borrows from `FileMetadata` with lifetime `'a`. That is a non-trivial lifetime in a struct that derives `Tabled`. The `From<&'a FileMetadata>` impl at `main.rs:125-137` fills string slices directly from metadata fields, and the `..Default::default()` pattern fills in the `path` and `status` fields, which are not derivable from `FileMetadata` alone (they are patched after construction at `main.rs:362-363`).

The `status` command's filter logic deliberately never hides errors. If none of `--current`, `--absent`, or `--unsynced` is passed, `show_all = true` and no filtering is applied. If any one is passed, errors are unconditionally retained in the output even if the user didn't ask for them (`main.rs:324-333`). The intent is to avoid silently hiding errors, but it does mean `dvs status --current` will still print error rows.

Finally, `walkdir` is listed in `[dependencies]` but not imported in `main.rs`. The CLI uses `walkdir` transitively through `dvs::globbing`, which is a public module. The direct entry in `dvs-cli/Cargo.toml` is likely a leftover from an earlier implementation in which the CLI walked directories itself.
