#Requires -Version 5.1
<#
.SYNOPSIS
  Thin wrapper: run SWE-bench regression from sibling / KUIBYSHEFF_SWEBENCH_ROOT.
#>
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RemainingArgs = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$AgentRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$candidates = @()
if ($env:KUIBYSHEFF_SWEBENCH_ROOT) { $candidates += $env:KUIBYSHEFF_SWEBENCH_ROOT }
$candidates += (Join-Path (Split-Path -Parent $AgentRoot) "kuibysheff-swebench")

$sweRoot = $null
foreach ($c in $candidates) {
    if ($c -and (Test-Path -LiteralPath (Join-Path $c "scripts\swebench-regression.ps1") -PathType Leaf)) {
        $sweRoot = (Resolve-Path -LiteralPath $c).Path
        break
    }
}

if (-not $sweRoot) {
    throw @"
SWE-bench example repo not found.

Clone https://github.com/gybson63/kuibysheff-swebench next to this repo, or set:
  KUIBYSHEFF_SWEBENCH_ROOT=C:\path\to\kuibysheff-swebench

Then re-run: .\scripts\check.ps1 -Swebench
"@
}

$env:KUIBYSHEFF_SRC = $AgentRoot.Path
Write-Host "Delegating SWE-bench regression to $sweRoot (KUIBYSHEFF_SRC=$($env:KUIBYSHEFF_SRC))"
& (Join-Path $sweRoot "scripts\swebench-regression.ps1") @RemainingArgs
exit $LASTEXITCODE
