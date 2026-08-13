param(
    [switch]$Aoc,
    [switch]$Swebench,
    [switch]$Security,
    [switch]$ScaleFs,
    [switch]$SkipDeny,
    # Deprecated alias: offline is now the default; -SkipAoc is a no-op kept for scripts.
    [switch]$SkipAoc
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

Write-Host "Running portability guardrails..."
& "$PSScriptRoot\check-portability.ps1"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$workflowsRoot = Join-Path (Resolve-Path (Join-Path $PSScriptRoot "..")) "workflows"
if (Test-Path -LiteralPath $workflowsRoot -PathType Container) {
    python "$PSScriptRoot\test_detached_workflows.py"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Skipping detached workflow tests (workflows/ not present)."
}

$runAoc = $Aoc -or ($env:RUN_AOC -eq "1")
if ($SkipAoc -and $runAoc) {
    Write-Host "Note: -SkipAoc ignored because -Aoc / RUN_AOC=1 was set."
}
if ($runAoc) {
    Write-Host "Running AoC agent regression (-Aoc / RUN_AOC=1)..."
    & "$PSScriptRoot\aoc-regression.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Skipping live AoC regression (pass -Aoc or set RUN_AOC=1)."
}

$runSwebench = $Swebench -or ($env:RUN_SWEBENCH -eq "1")
if ($runSwebench) {
    Write-Host "Running SWE-bench agent regression (-Swebench / RUN_SWEBENCH=1)..."
    & "$PSScriptRoot\swebench-regression.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Skipping live SWE-bench regression (pass -Swebench or set RUN_SWEBENCH=1)."
}

$runSecurity = $Security -or ($env:RUN_SECURITY -eq "1")
if ($runSecurity) {
    Write-Host "Running security sandbox LLM regression (-Security / RUN_SECURITY=1)..."
    & "$PSScriptRoot\security-regression.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Skipping live security sandbox regression (pass -Security or set RUN_SECURITY=1)."
}

$runScaleFs = $ScaleFs -or ($env:RUN_SCALE_FS -eq "1")
if ($runScaleFs) {
    Write-Host "Running Scale-FS live LLM regression (-ScaleFs / RUN_SCALE_FS=1)..."
    & "$PSScriptRoot\scale-fs-regression.ps1"
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
} else {
    Write-Host "Skipping live Scale-FS regression (pass -ScaleFs or set RUN_SCALE_FS=1)."
}

Write-Host "All checks passed."
