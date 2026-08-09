#Requires -Version 5.1
# Thin forwarder to workflows/1c-dev/run.ps1 (monorepo UX).
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
& (Join-Path $PSScriptRoot "..\workflows\1c-dev\run.ps1") @args
exit $LASTEXITCODE
