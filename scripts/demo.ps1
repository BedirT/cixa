$ErrorActionPreference = "Stop"
cargo run --quiet --manifest-path "$PSScriptRoot\..\Cargo.toml" -p cixa -- demo
