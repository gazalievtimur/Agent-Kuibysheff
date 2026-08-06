param(
    [switch]$SkipAoc,
    [switch]$SkipDeny
)

$ErrorActionPreference = "Stop"

function Assert-CargoDeny {
    cargo deny --version 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-deny is not installed. Install with: cargo install --locked cargo-deny"
    }
}

Write-Host "Checking formatting..."
cargo fmt --all -- --check

Write-Host "Running clippy..."
cargo clippy --workspace --all-targets -- -D warnings

if ($SkipDeny -or $env:SKIP_DENY -eq "1") {
    Write-Host "Skipping cargo deny (-SkipDeny / SKIP_DENY=1)."
} else {
    Write-Host "Running cargo deny..."
    Assert-CargoDeny
    cargo deny check
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

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
