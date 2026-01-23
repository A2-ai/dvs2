```

rm -rf ../.dvs ../dvs.toml ../.storage && cargo run --features=cli -- init /home/vincent/Code/a2-ai/dvsexperimental/dvs/.storage
cargo run --features=cli -- add README.md
cargo run --features=cli -- status
rm README.md
cargo run --features=cli -- get README.md
cargo run --features=cli -- status
```