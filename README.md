# `dvs`

`dvs` (Data Version System) versions large or sensitive files under Git without storing their content in Git.
`dvs add` copies a file into storage and writes a small metadata file that you commit. `dvs get` restores the file from storage on any checkout.

This is a rewrite of A2-AI's [dvs](https://github.com/A2-ai/dvs) R package. It has a Rust core, a `dvs` CLI, and an R package built on the same core.

## Installation

### CLI (`dvs` binary)

Direct from git:

```sh
cargo install --git https://github.com/A2-ai/dvs2 --locked --force --all-features dvs-cli
```

From a clone:

```sh
git clone https://github.com/A2-ai/dvs2
cargo install --path dvs2/dvs-cli --force --locked --all-features
```

### R package (`dvs`)

Direct from git, with [rv](https://github.com/A2-ai/rv):

```sh
rv init
rv add dvs --git https://github.com/A2-ai/dvs2 --branch main --directory dvs-rpkg
```

## Usage

Create a project in a Git repository, choosing where the data is stored:

```sh
# In a directory, such as a shared drive
dvs init local /shared/data/my-project

# On a dvs server
dvs init server my-project https://dvs.example.com --group my-team

# To join a project that already exists on a dvs server, use attach instead
dvs attach my-project https://dvs.example.com
```

Then track, check and restore files:

```sh
dvs add data/raw.csv -m "initial extract"   # store the file, then commit its metadata and dvs.toml
dvs status                                  # which tracked files are current, absent or changed
dvs get data/raw.csv                        # restore it, e.g. in a fresh clone
dvs history data/raw.csv                    # its upload history
```

`dvs init server` and `dvs attach` log you in to the server if needed. When a login expires, run `dvs login` (and `dvs logout` to sign out).
`dvs info` shows the project's configuration.
Every command accepts `--json` for machine-readable output. See `dvs --help` and [specs.md](specs.md) for details.

## TODOs

- Azure backend
- GC?
- dvs remove?
- integrity check? would need to read the file again after saving it
- migrate from dvs1
