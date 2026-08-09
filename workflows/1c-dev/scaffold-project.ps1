#Requires -Version 5.1
# Scaffold from the 1C copy unit folder.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "1c-dev-scaffold-project.ps1") -WorkflowRoot $PSScriptRoot @args
exit $LASTEXITCODE
