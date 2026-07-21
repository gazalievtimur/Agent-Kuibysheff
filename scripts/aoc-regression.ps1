#Requires -Version 5.1
<#
.SYNOPSIS
  Prerequisites check + AoC single-agent regression eval.

.DESCRIPTION
  Fails if the local bank, config, or API key are missing. Intended to run from
  scripts/check.ps1 on every local quality gate.

.PARAMETER Config
  Runtime config. Defaults to agent-config.local.yaml when present, otherwise
  test-agents/referent/agent-config.aoc.example.yaml.

.PARAMETER TaskId
  Optional task filter. When omitted, runs the full local bank.
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

$bankDir = Join-Path $RepoRoot "local\aoc-bank"
if (-not (Test-Path -LiteralPath $bankDir -PathType Container)) {
    throw @"
AoC regression bank not found: $bankDir

Copy the example and fill real tasks (gitignored):
  Copy-Item -Recurse .\local\aoc-bank.example .\local\aoc-bank
"@
}

$taskCount = @(Get-ChildItem -LiteralPath $bankDir -Filter "*.json").Count
if ($taskCount -eq 0) {
    throw "AoC regression bank is empty: $bankDir"
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\referent\agent-config.aoc.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "AoC regression config not found: $Config"
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
AoC regression requires a provider API key.

Set one of:
  - provider.api_key in $Config
  - environment variable $apiKeyEnv
  - $apiKeyEnv in $(Join-Path $RepoRoot '.env')
"@
}

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "AoC regression requires Node.js on PATH (mcp-aoc-tasks.js)."
}
if (-not (Get-Command python -ErrorAction SilentlyContinue)) {
    throw "AoC regression requires python on PATH (home.run solutions)."
}

Write-Host "AoC regression: bank=$bankDir tasks=$taskCount config=$Config"
Write-Host "Building release agent (sandboxed home.run)..."
& cargo build --release
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$evalArgs = @{
    Config  = $Config
    BankDir = $bankDir
}
if ($TaskId.Count -gt 0) {
    $evalArgs.TaskId = $TaskId
}

& (Join-Path $PSScriptRoot "aoc-eval.ps1") @evalArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host "AoC regression passed."
