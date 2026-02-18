# AGENTS.md

Commit `Cargo.lock` due to the presence of dependencies on github.

## Development iteration

The R bindings to the embedded rust crate are generated when

- `devtools::load_all({"dvs-rpkg")`

this will compile the embedded rust crate, regenerate R wrappers, and replace them if they are different
from previous iteration, and then build the package.

Any invocation that rebuilds the r package will update the R wrappers for the embedded rust crate.

## Upgrade miniextendr version

Run

```shell
just rpkg-update
```

in order to update miniextendr in the embedded rust crate i.e. `cargo update --manifest-path=dvs-rpkg/src/rust/Cargo.toml`.

The R package scaffolding needs to be updated too. The `minirextendr` R package has a `minirextendr::upgrade_miniextendr_package` that will facilitate the upgrade.
