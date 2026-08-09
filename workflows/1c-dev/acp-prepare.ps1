#Requires -Version 5.1
# ACP prepare/validate from the 1C copy unit folder.
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "1c-dev-acp-prepare.ps1") -WorkflowRoot $PSScriptRoot @args
exit $LASTEXITCODE
