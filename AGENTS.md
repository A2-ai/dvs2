# AGENTS.md

Commit `Cargo.lock` due to the presence of dependencies on github.

## Local setup

For CLI/UI testing, prefer installing both entrypoints through `just`:

- `just install-cli`
- `just install-rpkg`

## File Deletion Safety

- **Never use `rm` or any permanent deletion command.**
- Always use a safe delete mechanism that moves files to the system trash/recycle bin instead of permanently removing them.
- This ensures files can be recovered if an action was incorrect, unintended, or unsafe.

### Approved approach

- Use a trash command (e.g. `trash`, `gio trash`, `gvfs-trash`, or platform equivalent).
- If no trash utility is available, **stop and ask for guidance** instead of deleting.

### Rationale

Permanent deletion is irreversible and unsafe in automated or agent-driven workflows.  
Using the trash provides a recovery path in case of mistakes or unexpected behavior.

## Development iteration

The R bindings to the embedded rust crate are generated when

- `devtools::load_all("dvs-rpkg")`

this will compile the embedded rust crate, regenerate R wrappers, and replace them if they are different
from previous iteration, and then build the package.

Any invocation that rebuilds the r package, including `just install-rpkg`, will update the R wrappers
for the embedded rust crate.

## Upgrade miniextendr version

Run

```shell
just rpkg-update
```

in order to update miniextendr in the embedded rust crate i.e. `cargo update --manifest-path=dvs-rpkg/src/rust/Cargo.toml`.

The R package scaffolding needs to be updated too. The `minirextendr` R package has a `minirextendr::upgrade_miniextendr_package` that will facilitate the upgrade.
