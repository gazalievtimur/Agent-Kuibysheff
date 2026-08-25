#Requires -Version 5.1
<#
.SYNOPSIS
  Live A2A regression gate (Agent Card, Bearer, SendMessage + LLM).

.DESCRIPTION
  Builds release kbshff, ensures the local task bank, runs scripts/a2a-eval.py.

.PARAMETER Config
  Import/render source. Defaults to agent-config.local.yaml when present.

.PARAMETER TaskId
  Optional task id filter(s).
#>
param(
    [string]$Config = "",
    [string[]]$TaskId = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

. (Join-Path $PSScriptRoot "import-dotenv.ps1")
Import-DotEnv (Join-Path $RepoRoot ".env")

$bankExample = Join-Path $RepoRoot "local\a2a-bank.example"
$bankDir = Join-Path $RepoRoot "local\a2a-bank"
if (-not (Test-Path -LiteralPath $bankDir -PathType Container)) {
    if (Test-Path -LiteralPath $bankExample -PathType Container) {
        Write-Host "Copying A2A bank example -> local/a2a-bank"
        Copy-Item -Recurse -Force $bankExample $bankDir
    } else {
        throw "A2A bank not found: $bankDir"
    }
}

$taskCount = @(Get-ChildItem -LiteralPath $bankDir -Filter "*.json").Count
if ($taskCount -eq 0) {
    throw "A2A bank is empty: $bankDir"
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\a2a-probe\agent-config.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "A2A regression config not found: $Config"
}

$configText = Get-Content -LiteralPath $Config -Raw -Encoding UTF8
if (-not (Test-ProviderApiKeyAvailable $configText)) {
    $hasSend = $false
    foreach ($file in Get-ChildItem -LiteralPath $bankDir -Filter "*.json") {
        $obj = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
        if (($obj.kind -eq $null -or $obj.kind -eq "send") -and ($TaskId.Count -eq 0 -or ($TaskId -contains [string]$obj.id))) {
            $hasSend = $true
            break
        }
    }
    if ($hasSend) {
        throw "A2A send tasks require a provider API key (set api_key_env or use agent-config.local.yaml)."
    }
}

Write-Host "Building release agent..."
cargo build --release -p agent_Kuibysheff --bin kbshff
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$pyArgs = @(
    (Join-Path $RepoRoot "scripts\a2a-eval.py"),
    "--repo-root", $RepoRoot,
    "--bank-dir", $bankDir,
    "--config", $Config
)
foreach ($id in @($TaskId)) {
    if ($id) {
        $pyArgs += @("--task-id", $id)
    }
}

Write-Host "A2A regression: bank=$bankDir tasks=$taskCount config=$Config"
$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "A2A regression requires python on PATH."
}

& $python.Source @pyArgs
exit $LASTEXITCODE
