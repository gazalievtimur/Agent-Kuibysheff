#Requires -Version 5.1
$ErrorActionPreference = "Stop"
$AgentRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$candidates = @()
if ($env:KUIBYSHEFF_AOC_ROOT) { $candidates += $env:KUIBYSHEFF_AOC_ROOT }
$candidates += (Join-Path (Split-Path -Parent $AgentRoot) "kuibysheff-aoc")
$aocRoot = $null
foreach ($c in $candidates) {
    if ($c -and (Test-Path -LiteralPath (Join-Path $c "scripts\aoc-regression.ps1") -PathType Leaf)) {
        $aocRoot = (Resolve-Path -LiteralPath $c).Path
        break
    }
}
if (-not $aocRoot) {
    throw "AoC example repo not found. Clone https://github.com/gazalievtimur/kuibysheff-aoc or set KUIBYSHEFF_AOC_ROOT."
}
$env:KUIBYSHEFF_SRC = $AgentRoot.Path
Write-Host "Delegating AoC regression to $aocRoot (KUIBYSHEFF_SRC=$($env:KUIBYSHEFF_SRC))"
& (Join-Path $aocRoot "scripts\aoc-regression.ps1") @args
exit $LASTEXITCODE
