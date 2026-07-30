#Requires -Version 5.1
<#
.SYNOPSIS
  Copy stage4 out/cfe into K7 task workdir; optionally BuildCfe.
#>
param(
    [Parameter(Mandatory = $true)][string]$CfeOutDir,
    [Parameter(Mandatory = $true)][string]$ProductYamlPath,
    [Parameter(Mandatory = $true)][string]$IssueKey,
    [switch]$BuildCfe
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-YamlScalar {
    param([string]$Text, [string]$Key, [string]$Default = "")
    $match = [regex]::Match($Text, "(?m)^\s*$([regex]::Escape($Key)):\s*`"([^`"]*)`"")
    if (-not $match.Success) {
        $match = [regex]::Match($Text, "(?m)^\s*$([regex]::Escape($Key)):\s*'([^']*)'")
    }
    if (-not $match.Success) {
        $match = [regex]::Match($Text, "(?m)^\s*$([regex]::Escape($Key)):\s*([^#\r\n]+)")
    }
    if ($match.Success) {
        return $match.Groups[1].Value.Trim()
    }
    return $Default
}

$yaml = Get-Content -LiteralPath $ProductYamlPath -Raw -Encoding UTF8
$workspaceRoot = Get-YamlScalar $yaml "workspaceRoot"
$taskDirPattern = Get-YamlScalar $yaml "taskDirPattern" "{workspaceRoot}/{issueKey}"
$buildScriptTpl = Get-YamlScalar $yaml "buildScript"
$number = ""
if ($IssueKey -match "-(\d+)$") { $number = $Matches[1] }

$taskDir = $taskDirPattern.Replace("{workspaceRoot}", $workspaceRoot).Replace("{issueKey}", $IssueKey).Replace("{number}", $number)
$workSrc = Join-Path $taskDir "_work\src"
$cfeSrc = Join-Path $CfeOutDir "cfe"
if (-not (Test-Path -LiteralPath $cfeSrc -PathType Container)) {
    # allow CfeOutDir itself to be the cfe tree or parent of cfe/
    if (Test-Path -LiteralPath (Join-Path $CfeOutDir "Configuration.xml")) {
        $cfeSrc = $CfeOutDir
    } else {
        throw "CFE sources not found under $CfeOutDir"
    }
}

New-Item -ItemType Directory -Force -Path $workSrc | Out-Null
Write-Host "Copying CFE sources -> $workSrc"
Copy-Item -Path (Join-Path $cfeSrc "*") -Destination $workSrc -Recurse -Force

if ($BuildCfe) {
    $buildScript = $buildScriptTpl.Replace("{workspaceRoot}", $workspaceRoot).Replace("{issueKey}", $IssueKey)
    if (-not $buildScript -or -not (Test-Path -LiteralPath $buildScript -PathType Leaf)) {
        throw "Build script not found: $buildScript"
    }
    Write-Host "BuildCfe via $buildScript -TaskId $IssueKey"
    & powershell.exe -NoProfile -File $buildScript -TaskId $IssueKey -Action BuildCfe
    if ($LASTEXITCODE -ne 0) {
        throw "BuildCfe failed with exit $LASTEXITCODE"
    }
}

Write-Host "apply-out complete: $workSrc"
