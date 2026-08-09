#Requires -Version 5.1
<#
.SYNOPSIS
  Thin forwarder to workflows/swebench-verified/run.ps1 (monorepo UX).
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

$WorkflowLauncher = Join-Path $PSScriptRoot "..\workflows\swebench-verified\run.ps1"
if (-not (Test-Path -LiteralPath $WorkflowLauncher -PathType Leaf)) {
    throw "Workflow launcher not found: $WorkflowLauncher"
}

$forward = @{
    Command = $Command
    RemainingArgs = $RemainingArgs
}
if ($RepoRoot) { $forward.RepoRoot = $RepoRoot }
& $WorkflowLauncher @forward
exit $LASTEXITCODE
