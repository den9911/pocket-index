#!/bin/sh
cd "$(dirname "$0")" || exit 1
command -v cargo >/dev/null 2>&1 || { echo 'Install Rust from https://rustup.rs/'; exit 1; }
cargo run --release
