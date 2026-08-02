param(
    [switch]$SkipAoc
)

$ErrorActionPreference = "Stop"

Write-Host "Checking formatting..."
cargo fmt --all -- --check

Write-Host "Running clippy..."
cargo clippy --workspace --all-targets -- -D warnings

Write-Host "Running tests..."
cargo test --workspace

if ($SkipAoc) {
    Write-Host "Skipping AoC agent regression (-SkipAoc)."
} else {
    Write-Host "Running AoC agent regression..."
    & "$PSScriptRoot\aoc-regression.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

Write-Host "All checks passed."
