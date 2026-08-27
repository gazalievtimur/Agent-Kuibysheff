#Requires -Version 5.1
<#
.SYNOPSIS
  Live Scale-FS LLM regression gate.

.DESCRIPTION
  Builds release agent, ensures the local task bank, runs workflows/scale-fs-live/eval.py
  against a real provider, then assert_regression.py.

.PARAMETER Config
  Import/render source. Defaults to agent-config.local.yaml when present, otherwise
  test-agents/scale-fs-probe/agent-config.example.yaml.

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

$WorkflowDir = Join-Path $RepoRoot "workflows\scale-fs-live"
if (-not (Test-Path -LiteralPath $WorkflowDir -PathType Container)) {
    throw @"
workflows/scale-fs-live not found (gitignored copy-unit).

Restore from git history for local testing, for example:
  git checkout HEAD~1 -- workflows
  # or: git checkout <commit-before-untrack> -- workflows
"@
}

$bankExample = Join-Path $RepoRoot "local\scale-fs-bank.example"
$bankDir = Join-Path $RepoRoot "local\scale-fs-bank"
if (-not (Test-Path -LiteralPath $bankDir -PathType Container)) {
    if (Test-Path -LiteralPath $bankExample -PathType Container) {
        Write-Host "Copying scale-fs bank example -> local/scale-fs-bank"
        Copy-Item -Recurse -Force $bankExample $bankDir
    } else {
        throw "Scale-FS bank not found: $bankDir (and no example at $bankExample)"
    }
}

$taskCount = @(Get-ChildItem -LiteralPath $bankDir -Filter "*.json").Count
if ($taskCount -eq 0) {
    throw "Scale-FS bank is empty: $bankDir"
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\scale-fs-probe\agent-config.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "Scale-FS regression config not found: $Config"
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
Scale-FS regression requires a provider API key via environment.

Set:
  - environment variable $apiKeyEnv
  - $apiKeyEnv in $(Join-Path $RepoRoot '.env')
"@
}

Write-Host "Building release agent..."
cargo build --release -p agent_Kuibysheff --bin kbshff
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$settingsDir = Join-Path $RepoRoot "test-agents\scale-fs-probe"
$runsRoot = Join-Path $RepoRoot "local\scale-fs-runs"
New-Item -ItemType Directory -Force -Path $runsRoot | Out-Null

$pyArgs = @(
    (Join-Path $RepoRoot "workflows\scale-fs-live\eval.py"),
    "--repo-root", $RepoRoot,
    "--bank-dir", $bankDir,
    "--config", $Config,
    "--settings-dir", $settingsDir,
    "--runs-root", $runsRoot
)
foreach ($id in @($TaskId)) {
    if ($id) {
        $pyArgs += @("--task-id", $id)
    }
}

Write-Host "Scale-FS regression: bank=$bankDir tasks=$taskCount config=$Config"
$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "Scale-FS regression requires python/python3 on PATH."
}
& $python.Source @pyArgs
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$latestPtr = Join-Path $runsRoot "LATEST"
if (-not (Test-Path -LiteralPath $latestPtr -PathType Leaf)) {
    throw "Scale-FS regression: LATEST pointer missing under local/scale-fs-runs"
}
$runDir = (Get-Content -LiteralPath $latestPtr -Raw -Encoding UTF8).Trim()
$report = Join-Path $runDir "report.json"
if (-not (Test-Path -LiteralPath $report -PathType Leaf)) {
    throw "Scale-FS regression: report.json not found: $report"
}

& $python.Source (Join-Path $RepoRoot "workflows\scale-fs-live\assert_regression.py") $report
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

Write-Host "Scale-FS regression passed."
exit 0
