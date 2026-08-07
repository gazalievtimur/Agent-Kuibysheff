#Requires -Version 5.1
<#
.SYNOPSIS
  Launch the SWE-bench Verified workflow (preflight|generate|grade|report|run).

.PARAMETER Command
  Workflow subcommand.

.PARAMETER RepoRoot
  Repository root (default: auto from script location).
#>
param(
    [Parameter(Position = 0)]
    [ValidateSet("preflight", "generate", "grade", "report", "run")]
    [string]$Command = "run",

    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs = @(),

    [string]$RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

$dotenv = Join-Path $RepoRoot "scripts\import-dotenv.ps1"
if (Test-Path -LiteralPath $dotenv -PathType Leaf) {
    . $dotenv
    Import-DotEnv (Join-Path $RepoRoot ".env")
}

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "python not found on PATH"
}

$script = Join-Path $RepoRoot "workflows\swebench-verified\swebench.py"
if (-not (Test-Path -LiteralPath $script -PathType Leaf)) {
    throw "Workflow entry not found: $script"
}

$pyArgs = @($script, $Command, "--repo-root", "$RepoRoot") + $RemainingArgs
Write-Host "Running: $($python.Source) $($pyArgs -join ' ')"
& $python.Source @pyArgs
exit $LASTEXITCODE
