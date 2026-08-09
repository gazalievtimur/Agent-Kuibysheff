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
if ($RepoRoot) {
    $RepoRoot = (Resolve-Path $RepoRoot).Path
    Import-LocalDotEnv (Join-Path $RepoRoot ".env")
} elseif (Test-Path -LiteralPath (Join-Path $WorkflowDir "..\..\Cargo.toml")) {
    $RepoRoot = (Resolve-Path (Join-Path $WorkflowDir "..\..")).Path
    Import-LocalDotEnv (Join-Path $RepoRoot ".env")
}

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "python not found on PATH"
}

$script = Join-Path $WorkflowDir "swebench.py"
$pyArgs = @($script, $Command)
if ($RepoRoot) { $pyArgs += @("--repo-root", $RepoRoot) }
if ($AgentBin) { $pyArgs += @("--agent-bin", $AgentBin) }
$pyArgs += $RemainingArgs

Write-Host "Running: $($python.Source) $($pyArgs -join ' ')"
& $python.Source @pyArgs
exit $LASTEXITCODE
