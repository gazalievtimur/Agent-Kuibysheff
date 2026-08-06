#Requires -Version 5.1
<#
.SYNOPSIS
  CI-parity gate used by git pre-commit / Cursor hooks.

.DESCRIPTION
  Mirrors the fast CI jobs that keep failing on GitHub, plus local supply-chain:
    - cargo fmt --all -- --check
    - cargo clippy --workspace --all-targets -- -D warnings
    - cargo deny check (skip with -SkipDeny / SKIP_DENY=1)
    - cargo test --workspace
    - cargo +nightly miri test -p sandbox-linux --lib (Linux only; skip with -SkipMiri)

  Set SKIP_PRECOMMIT=1 to bypass callers (emergency only).
#>
param(
    [switch]$SkipMiri,
    [switch]$SkipTests,
    [switch]$SkipDeny
)

$ErrorActionPreference = "Stop"

if ($env:SKIP_PRECOMMIT -eq "1") {
    Write-Host "SKIP_PRECOMMIT=1 - bypassing pre-commit gate."
    exit 0
}

function Invoke-Step([string]$Label, [scriptblock]$Action) {
    Write-Host ""
    Write-Host "==> $Label"
    & $Action
    if ($LASTEXITCODE -ne 0) {
        throw "Step failed: $Label (exit $LASTEXITCODE)"
    }
}

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root

Invoke-Step "cargo fmt --all -- --check" {
    cargo fmt --all -- --check
}

Invoke-Step "cargo clippy --workspace --all-targets -- -D warnings" {
    cargo clippy --workspace --all-targets -- -D warnings
}

if ($SkipDeny -or $env:SKIP_DENY -eq "1") {
    Write-Host "==> Skipping cargo deny (-SkipDeny / SKIP_DENY=1)"
} else {
    cargo deny --version 2>$null | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-deny is not installed. Install with: cargo install --locked cargo-deny"
    }
    Invoke-Step "cargo deny check" {
        cargo deny check
    }
}

if (-not $SkipTests) {
    Invoke-Step "cargo test --workspace" {
        cargo test --workspace
    }
} else {
    Write-Host "==> Skipping cargo test (-SkipTests)"
}

$runMiri = -not $SkipMiri -and $env:SKIP_MIRI -ne "1"
$onLinux = ($IsLinux -eq $true)
if ($runMiri -and $onLinux) {
    & rustup run nightly rustc --version 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Invoke-Step "cargo +nightly miri test -p sandbox-linux --lib" {
            cargo +nightly miri setup
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
            cargo +nightly miri test -p sandbox-linux --lib
        }
    } else {
        Write-Host "==> Skipping Miri (nightly toolchain not installed)"
    }
} elseif ($runMiri) {
    Write-Host "==> Skipping Miri (not Linux)"
} else {
    Write-Host "==> Skipping Miri (-SkipMiri / SKIP_MIRI=1)"
}

Write-Host ""
Write-Host "Pre-commit gate passed."
