# `dvs.toml`

default initialization should provide:

```
compression = "zstd"
metadata_folder_name = ".dvs" # option

[backend]
path = "/path/to/shared/storage
permissions = "664"
group = NULL # default to users default group
```

compression enum - zstd | none
hash_algorithms vector of hash algorithms - blake3, md5