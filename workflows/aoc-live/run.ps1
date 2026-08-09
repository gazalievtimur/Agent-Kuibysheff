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

.PARAMETER ProjectRoot
  Project owning .kuibysheff/ (default: local/aoc-live-project).

.PARAMETER Agent
  Agent id (default: aoc-live).

.PARAMETER Home
  Relative home under .kuibysheff/ (default: homes/<run-id>).

.PARAMETER ImportFrom
  Template directory imported into the protected profile.

.PARAMETER Config
  Provider config template (import/render source only).

.PARAMETER SettingsDir
  Legacy alias for ImportFrom.

.PARAMETER RepoRoot
  Optional monorepo root (Cargo binary / staged Python fallback).

.PARAMETER AgentBin
  Path to agent_Kuibysheff binary.

.PARAMETER McpJs
  Path to mcp-aoc-tasks.js (default: beside this workflow).
#>
param(
    [Parameter(Mandatory = $true)][int]$Year,
    [Parameter(Mandatory = $true)][int]$Day,
    [int]$Part = 1,
    [int]$MaxAttempts = 5,
    [string]$ProjectRoot = "",
    [string]$Agent = "",
    [string]$Home = "",
    [string]$ImportFrom = "",
    [string]$Config = "",
    [string]$SettingsDir = "",
    [string]$RepoRoot = "",
    [string]$AgentBin = "",
    [string]$McpJs = "",
    [switch]$VerboseLogging
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
    $RepoRoot = Resolve-Path $RepoRoot
    Import-LocalDotEnv (Join-Path $RepoRoot ".env")
} elseif (Test-Path -LiteralPath (Join-Path $WorkflowDir "..\..\.env")) {
    # Optional monorepo .env when running from a checkout.
    Import-LocalDotEnv (Resolve-Path (Join-Path $WorkflowDir "..\..\.env"))
    if (-not $RepoRoot -and (Test-Path -LiteralPath (Join-Path $WorkflowDir "..\..\Cargo.toml"))) {
        $RepoRoot = (Resolve-Path (Join-Path $WorkflowDir "..\..")).Path
    }
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
    "--max-attempts", "$MaxAttempts"
)
if ($RepoRoot) { $pyArgs += @("--repo-root", "$RepoRoot") }
if ($ProjectRoot) { $pyArgs += @("--project-root", $ProjectRoot) }
if ($Agent) { $pyArgs += @("--agent", $Agent) }
if ($Home) { $pyArgs += @("--home", $Home) }
if ($ImportFrom) { $pyArgs += @("--import-from", $ImportFrom) }
elseif ($SettingsDir) { $pyArgs += @("--settings-dir", $SettingsDir) }
if ($Config) { $pyArgs += @("--config", $Config) }
if ($AgentBin) { $pyArgs += @("--agent-bin", $AgentBin) }
if ($McpJs) { $pyArgs += @("--mcp-js", $McpJs) }
if ($VerboseLogging) { $pyArgs += @("-v") }

Write-Host "Running: $($python.Source) $($pyArgs -join ' ')"
& $python.Source @pyArgs
exit $LASTEXITCODE
