# `dvs track` / `dvs_track`

Goal: Purpose is to specify which files we ought to follow in dvs.

User journey:

- [ ] All the .csv files underneath a specific directory.
- [ ] All the .csv files that are less than 25 MB

- File type
- Size filters

<!-- FIXME: Cloned repositories do not have hooks. should we address this in `dvs status` or `dvs follow` -->

<!-- - [ ] MOSSA: Filtering **/* but only the tracked files by dvs! -->

## CLI

```shell
$ dvs follow --help
Files that are followed by dvs when untracked.

Usage:
  dvs follow [COMMANDS] [OPTIONS]

Commands:
  add
  list
  audit

Options:
  -h, --help    Show help for a command
```

`add` command:
<!-- TODO -->

`list` command:
<!-- TODO -->

`add` command:
<!-- TODO -->

## R package

Support the following

- `ext` which are following-filters based on file extensions, e.g. `"csx"`.
- `glob`: a glob that can enable matching files through their paths and file extension
- `regex`: a regular expression to match files through their full paths

Provide diagnostics in case users accidentally write `.csv` instead of the correct `csv`.

The follow filter must support

- `glob`, `ext`, `regex` field
- an optional `label` that can be used to identify which follow-filter matched a file
- file size qualifiers: 
  `file_size_gt` (file size greater than mask),
  `file_size_lt` (file size less than mask)

Example:

```toml
[[follow]]
{ ext = "parquet" }
[[follow]]
{ glob = "data/**/*.csv", label? = "optional label" }
[[follow]]
{  regex = ".+tab[0-9].+", file_size_gt ="5MB" } # match all nonmem tab files  sdtab001 patab001 .... over 5MB
[[follow]]
{ glob = "model/nonmem/**/*", file_size_gt = "10MB" }
```

## Matcher audit

A helpful utility for end users is a way to figure out why a given file was followed
by dvs. To that end, the dvs track ought to display the matching filter next to every
followed file.
