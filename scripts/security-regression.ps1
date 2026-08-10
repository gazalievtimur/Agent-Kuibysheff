#Requires -Version 5.1
<#
.SYNOPSIS
  Security / sandbox-escape LLM regression gate (Windows entrypoint).

.DESCRIPTION
  On Windows this forwards to the Linux Docker lab (OS sandbox is Linux namespaces
  for the live LLM bank). Native Linux should use scripts/security-regression.sh.

.PARAMETER Config
  Import/render source config (api_key_env + provider). Defaults to
  agent-config.local.yaml or test-agents/security-probe/agent-config.example.yaml.

.PARAMETER TaskId
  Optional task filter(s).

.PARAMETER RequireLimits
  Forward --require-limits to the lab (also auto when config has max_cost).

.PARAMETER RequireCostLimit
  Forward --require-cost-limit (cost budget must stop the run).
#>
param(
    [string]$Config = "",
    [string[]]$TaskId = @(),
    [switch]$RequireLimits,
    [switch]$RequireCostLimit
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

Write-Host "Windows host: forwarding security regression to Linux Docker lab..."
$forward = @{
    Config = $Config
}
if ($TaskId -and $TaskId.Count -gt 0) {
    $forward["TaskId"] = $TaskId
}
if ($RequireLimits) {
    $forward["RequireLimits"] = $true
}
if ($RequireCostLimit) {
    $forward["RequireCostLimit"] = $true
}
& (Join-Path $PSScriptRoot "security-regression-linux-docker.ps1") @forward
exit $LASTEXITCODE
