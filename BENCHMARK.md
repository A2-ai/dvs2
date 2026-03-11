# Benchmarking dvs1 vs dvs2

## Overview

dvs2 is a rewrite of dvs1's Rust backend, using miniextendr instead of extendr for R bindings. Do **not** benchmark against the dvs1 CLI — that uses a Go backend, which is not a fair comparison. Both dvs1 and dvs2 have Rust backends; the comparison should be R package to R package.

## Setup

### Install rv

You need [rv](https://github.com/A2-ai/rv) to manage R versions and library paths.

### Install dvs1

dvs1 is available from the PRISM repository. Using rv, create an `rv.toml`:

```toml
[project]
name = "cache"
r_version = "4.5"

repositories = [
    {alias = "PRISM", url = "https://prism.dev.a2-ai.cloud/rpkgs/stratus/2026-03-02/" },
]

dependencies = [
    {name = "dvs", repository = "PRISM"},
]
```

Then install:

```bash
rv sync
```

### Install dvs2

```bash
just rpkg-install
```

## What to compare

- `dvs::dvs_add()` (dvs1, extendr) vs `dvs::dvs_add()` (dvs2, miniextendr)
- `dvs::dvs_get()` vs `dvs::dvs_get()`
- `dvs::dvs_status()` vs `dvs::dvs_status()`

Use identical file sets and storage backends for both. Vary file count and file size to capture scaling behavior.

### Serial mode

dvs2 uses rayon for parallel file processing by default. To benchmark single-threaded performance (apples-to-apples with dvs1):

```bash
DVS_NUM_THREADS=1 Rscript bench.R
```
