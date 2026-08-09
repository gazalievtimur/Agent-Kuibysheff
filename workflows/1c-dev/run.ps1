#Requires -Version 5.1
# Launch 1C workflow from the copy unit folder.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "1c-dev-run.ps1") -WorkflowRoot $PSScriptRoot @args
exit $LASTEXITCODE
