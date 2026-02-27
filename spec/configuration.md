# `dvs.toml`

Every DVS repository has a `dvs.toml` file. Previous version
of [`dvs`](https://github.com/a2-ai/dvs) used YAML format, i.e.
`dvs.yaml`. There is a migration PR [#2](https://github.com/A2-ai/dvs2/pull/2).

Default hashing is `blake3` always, yet we calculate other hashing in case of a non-local backend, in which calculating `blake3` is infeasible.

default initialization should provide:

```toml
compression = "zstd"
metadata_folder_name = ".dvs" # option

[backend]
path = "/path/to/shared/storage
permissions = "664"
group = NULL # default to users default group
```

compression enum - `zstd | none`
hash_algorithms vector of hash algorithms - blake3, md5.

## Storage format

The local backend format follows the structure that files
are stored via their hashing, in which the first two characters determine the sub-directory, and the remainder of the hash denotes the name of the binary instantiation of the tracked datafile.

Note that storage directory is not meant to be manipulated or explored. The binary blobs do not have file extensions, and the compression is discernable from the encoded compression byte in the file.
