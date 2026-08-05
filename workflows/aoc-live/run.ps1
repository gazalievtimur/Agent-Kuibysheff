#Requires -Version 5.1
<#
.SYNOPSIS
  Launch the live AoC ACP singleton workflow example.

.PARAMETER Year
  Advent of Code year (e.g. 2024).

.PARAMETER Day
  Puzzle day 1..25.

.PARAMETER Part
  Puzzle part 1 or 2 (default 1).

.PARAMETER MaxAttempts
  Full solve/submit iterations (default 5, hard-capped at 5).

.PARAMETER Config
  Base agent config template.

.PARAMETER SettingsDir
  Agent settings directory.

.PARAMETER RepoRoot
  Repository root.
#>
param(
    [Parameter(Mandatory = $true)][int]$Year,
    [Parameter(Mandatory = $true)][int]$Day,
    [int]$Part = 1,
    [int]$MaxAttempts = 5,
    [string]$Config = "",
    [string]$SettingsDir = "",
    [string]$RepoRoot = "",
    [string]$AgentBin = "",
    [switch]$VerboseLogging
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

$dotenv = Join-Path $RepoRoot "scripts\import-dotenv.ps1"
if (Test-Path -LiteralPath $dotenv -PathType Leaf) {
    . $dotenv
    Import-DotEnv (Join-Path $RepoRoot ".env")
}

if (-not $env:AOC_SESSION) {
    throw "AOC_SESSION is not set. Put your AoC session cookie in the environment or .env."
}

$python = Get-Command python -ErrorAction SilentlyContinue
if (-not $python) {
    $python = Get-Command python3 -ErrorAction SilentlyContinue
}
if (-not $python) {
    throw "python not found on PATH"
}

$script = Join-Path $PSScriptRoot "aoc-singleton.py"
$pyArgs = @(
    $script,
    "--year", "$Year",
    "--day", "$Day",
    "--part", "$Part",
    "--max-attempts", "$MaxAttempts",
    "--repo-root", "$RepoRoot"
)
if ($Config) { $pyArgs += @("--config", $Config) }
if ($SettingsDir) { $pyArgs += @("--settings-dir", $SettingsDir) }
if ($AgentBin) { $pyArgs += @("--agent-bin", $AgentBin) }
if ($VerboseLogging) { $pyArgs += @("-v") }

Write-Host "Running: $($python.Source) $($pyArgs -join ' ')"
& $python.Source @pyArgs
exit $LASTEXITCODE
