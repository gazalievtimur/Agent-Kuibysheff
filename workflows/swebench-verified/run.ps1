#Requires -Version 5.1
<#
.SYNOPSIS
  Launch the SWE-bench Verified workflow from the copy unit folder.
#>
param(
    [Parameter(Position = 0)]
    [ValidateSet("preflight", "generate", "grade", "report", "run")]
    [string]$Command = "run",

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs = @(),

    [string]$RepoRoot = "",
    [string]$AgentBin = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$WorkflowDir = $PSScriptRoot

function Import-LocalDotEnv([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return }
    Get-Content -LiteralPath $Path | ForEach-Object {
        $line = $_.Trim()
        if (-not $line -or $line.StartsWith("#") -or $line -notmatch "=") { return }
        $key, $value = $line.Split("=", 2)
        $key = $key.Trim()
        $value = $value.Trim().Trim("'").Trim('"')
        if ($key -and -not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($key))) {
            return
        }
        if ($key) {
            [Environment]::SetEnvironmentVariable($key, $value, "Process")
        }
    }
}

Import-LocalDotEnv (Join-Path $WorkflowDir ".env")
Import-LocalDotEnv (Join-Path (Get-Location) ".env")
$explicitRepoRoot = $RepoRoot
$dotenvRoot = $RepoRoot
if ($dotenvRoot) {
    $dotenvRoot = (Resolve-Path $dotenvRoot).Path
    Import-LocalDotEnv (Join-Path $dotenvRoot ".env")
} elseif (Test-Path -LiteralPath (Join-Path $WorkflowDir "..\..\Cargo.toml")) {
    $dotenvRoot = (Resolve-Path (Join-Path $WorkflowDir "..\..")).Path
    Import-LocalDotEnv (Join-Path $dotenvRoot ".env")
}

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "python not found on PATH"
}

$winStubs = Join-Path $WorkflowDir "win_stubs"
if (Test-Path -LiteralPath $winStubs -PathType Container) {
    $env:PYTHONPATH = if ($env:PYTHONPATH) { "$winStubs;$env:PYTHONPATH" } else { "$winStubs" }
}

$script = Join-Path $WorkflowDir "swebench.py"
$pyArgs = @($script, $Command)
if ($explicitRepoRoot) {
    $pyArgs += @("--repo-root", (Resolve-Path $explicitRepoRoot).Path)
}
if ($AgentBin) { $pyArgs += @("--agent-bin", $AgentBin) }
$pyArgs += $RemainingArgs

Write-Host "Running: $($python.Source) $($pyArgs -join ' ')"
& $python.Source @pyArgs
exit $LASTEXITCODE
