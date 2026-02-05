```

rm -rf .dvs dvs.toml .storage && cargo run  -- init /home/vincent/Code/a2-ai/dvs2/.storage
cargo run -- add README.md
cargo run -- status
rm README.md
cargo run -- get README.md
cargo run  -- status
```

- --glob for CLI with expansion inside CLI
- pretty print .dvs metadata
- add some filters for dvs status --absent/--current
- add dvs history that looks in the audit log and presents all the changes
- version number for config