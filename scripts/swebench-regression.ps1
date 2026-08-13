#Requires -Version 5.1
<#
.SYNOPSIS
  Opt-in SWE-bench Verified capability regression (one fixed instance).

.DESCRIPTION
  Preflight-lite (Docker Linux, Python deps, API key) → release build →
  generate+grade+report for sympy__sympy-20590 → assert harness_resolved.
  Intended for scripts/check.ps1 -Swebench / RUN_SWEBENCH=1 (not PR CI).

.PARAMETER Config
  Provider config template (import/render only). Defaults to
  agent-config.local.yaml when present, otherwise
  test-agents/swebench-solver/agent-config.example.yaml.

.PARAMETER InstanceId
  Fixed Verified instance (default: sympy__sympy-20590).

.NOTES
  For a Linux ELF regression from Windows (no WSL toolchain), use:
    .\scripts\swebench-regression-linux-docker.ps1
#>
param(
    [string]$Config = "",
    [string]$InstanceId = "sympy__sympy-20590"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

. (Join-Path $PSScriptRoot "import-dotenv.ps1")
Import-DotEnv (Join-Path $RepoRoot ".env")

$WorkflowDir = Join-Path $RepoRoot "workflows\swebench-verified"
$Requirements = Join-Path $WorkflowDir "requirements.txt"
$AssertScript = Join-Path $WorkflowDir "assert_regression.py"

if (-not (Test-Path -LiteralPath $WorkflowDir -PathType Container)) {
    throw @"
workflows/swebench-verified not found (gitignored copy-unit).

Restore from git history for local testing, for example:
  git checkout HEAD~1 -- workflows
  # or: git checkout <commit-before-untrack> -- workflows
"@
}
if (-not (Test-Path -LiteralPath $AssertScript -PathType Leaf)) {
    throw "assert_regression.py not found: $AssertScript"
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\swebench-solver\agent-config.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "SWE-bench regression config not found: $Config"
}

$configText = Get-Content -LiteralPath $Config -Raw -Encoding UTF8
if (-not (Test-ProviderApiKeyAvailable $configText)) {
    $apiKeyEnv = "OPENAI_API_KEY"
    $match = [regex]::Match($configText, '(?m)^\s*api_key_env:\s*"([^"]+)"')
    if (-not $match.Success) {
        $match = [regex]::Match($configText, "(?m)^\s*api_key_env:\s*'([^']+)'")
    }
    if (-not $match.Success) {
        $match = [regex]::Match($configText, '(?m)^\s*api_key_env:\s*([A-Za-z_][A-Za-z0-9_]*)')
    }
    if ($match.Success) {
        $apiKeyEnv = $match.Groups[1].Value.Trim()
    }

    throw @"
SWE-bench regression requires a provider API key via environment.

Set:
  - environment variable $apiKeyEnv
  - $apiKeyEnv in $(Join-Path $RepoRoot '.env')

Inline provider.api_key in config is rejected by ConfigSafetyValidator.
"@
}

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "SWE-bench regression requires python on PATH."
}

$winStubs = Join-Path $WorkflowDir "win_stubs"
if (Test-Path -LiteralPath $winStubs -PathType Container) {
    $env:PYTHONPATH = if ($env:PYTHONPATH) { "$winStubs;$env:PYTHONPATH" } else { $winStubs }
}

Write-Host "Checking Python deps (pip install -r workflows/swebench-verified/requirements.txt)..."
& $python.Source -c "import importlib.metadata, docker, mcp; v=importlib.metadata.version('swebench'); print('swebench', v)"
if ($LASTEXITCODE -ne 0) {
    throw @"
SWE-bench Python deps missing or broken.

Install pinned deps:
  pip install -r $Requirements
"@
}

Write-Host "Checking Docker Linux engine..."
& $python.Source -c @"
import sys
sys.path.insert(0, r'$($WorkflowDir -replace '\\','/')')
from runtime import check_docker_linux
check_docker_linux()
print('Docker Linux OK')
"@
if ($LASTEXITCODE -ne 0) {
    throw "SWE-bench regression requires a Docker Linux engine (Docker Desktop / Linux)."
}

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw "SWE-bench regression requires docker CLI on PATH."
}

$runId = "regression-" + (Get-Date -Format "yyyyMMdd-HHmmss")
$reportPath = Join-Path $WorkflowDir "runs\$runId\report.json"

Write-Host "SWE-bench regression: instance=$InstanceId run_id=$runId config=$Config"
Write-Host "Building release agent..."
& cargo build --release
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$configAbs = (Resolve-Path -LiteralPath $Config).Path
& (Join-Path $PSScriptRoot "swebench-verified-run.ps1") -Command run -RemainingArgs @(
    "--instance-id", $InstanceId,
    "--run-id", $runId,
    "--workers", "1",
    "--config", $configAbs,
    "--repo-root", $RepoRoot.Path
)
$runExit = $LASTEXITCODE
if ($runExit -ne 0) {
    Write-Host "SWE-bench run exited with code $runExit (still asserting report if present)."
}

& $python.Source $AssertScript $reportPath --instance-id $InstanceId
$assertExit = $LASTEXITCODE
if ($assertExit -ne 0) {
    exit $assertExit
}
if ($runExit -ne 0) {
    exit $runExit
}

Write-Host "SWE-bench regression passed."
