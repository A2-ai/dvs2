# `dvs`

## note: this is a "develop-in-the-open" active rewrite targetting stabilization in ~March 2026, for the moment expect major breakage/instability as we
## consolidate the feature set we want to support in dvs for this next release.
## please contact us if these ideas or discussion points are of interest, we'd love to get more collaborative discussions going

Rewrite of `dvs`, the data-version-control system made by A2-AI.

DVS (Data Version System) is a tool for versioning large or sensitive files under Git without tracking the file content directly.

## Installation

`Cargo.lock` is checked in, so `--locked` builds are reproducible.

### CLI (`dvs` binary)

From a clone:

```sh
git clone https://github.com/A2-ai/dvs2
cd dvs2/dvs-cli
cargo install --path . --force --locked --all-features
```

Direct from git, no clone:

```sh
cargo install --git https://github.com/A2-ai/dvs2 --locked --force --all-features dvs-cli
```

### R package (`dvs`)

From a clone, with [rv](https://github.com/A2-ai/rv):

```sh
git clone https://github.com/A2-ai/dvs2
cd dvs2/dvs-rpkg
rv sync
Rscript -e 'install.packages(".", repos = NULL, type = "source")'
```

Direct from git, no clone, with `rv`:

```sh
rv init
rv add dvs --git https://github.com/A2-ai/dvs2 --branch main --directory dvs-rpkg
```

`rv add` requires one of `--branch`, `--tag`, or `--commit`.

## TODOs

- Azure backend
- GC?
- dvs remove?
- integrity check? would need to read the file again after saving it
- compression?
- migrate from dvs1
