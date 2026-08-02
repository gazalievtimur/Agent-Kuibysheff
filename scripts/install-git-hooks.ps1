#Requires -Version 5.1
<#
.SYNOPSIS
  Point this repo at .githooks (shared pre-commit CI gate).
#>
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $root
git config core.hooksPath .githooks
Write-Host "Configured core.hooksPath=.githooks"
git config --get core.hooksPath
