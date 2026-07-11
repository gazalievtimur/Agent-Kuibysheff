$ErrorActionPreference = "Stop"

Write-Host "Checking formatting..."
cargo fmt --all -- --check

Write-Host "Running clippy..."
cargo clippy --all-targets -- -D warnings

Write-Host "Running tests..."
cargo test

Write-Host "All checks passed."
