@echo off
cd /d "%~dp0"
where cargo >nul 2>&1 || (echo Install Rust from https://rustup.rs/ & exit /b 1)
cargo run --release
pause
