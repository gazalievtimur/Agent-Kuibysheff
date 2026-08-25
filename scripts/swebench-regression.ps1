#Requires -Version 5.1
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
    throw "SWE-bench example repo not found. Clone https://github.com/gazalievtimur/kuibysheff-swebench or set KUIBYSHEFF_SWEBENCH_ROOT."
}
$env:KUIBYSHEFF_SRC = $AgentRoot.Path
Write-Host "Delegating SWE-bench regression to $sweRoot (KUIBYSHEFF_SRC=$($env:KUIBYSHEFF_SRC))"
& (Join-Path $sweRoot "scripts\swebench-regression.ps1") @args
exit $LASTEXITCODE
