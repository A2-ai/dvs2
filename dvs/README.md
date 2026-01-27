```

rm -rf .dvs dvs.toml .storage && cargo run  -- init /home/vincent/Code/a2-ai/dvs2/.storage
cargo run -- add README.md
cargo run -- status
rm README.md
cargo run -- get README.md
cargo run  -- status
```