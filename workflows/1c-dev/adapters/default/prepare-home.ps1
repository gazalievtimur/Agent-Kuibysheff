#Requires -Version 5.1
<#
.SYNOPSIS
  Prepare stage home/in and product.json for 1c-dev workflow stages.
#>
param(
    [Parameter(Mandatory = $true)][string]$StageHome,
    [Parameter(Mandatory = $true)][string]$ProductYamlPath,
    [Parameter(Mandatory = $true)][ValidateSet("1", "2", "3", "4")][string]$Stage,
    [string]$IssueKey = "",
    [string]$BriefDir = "",
    [string]$PlanDir = "",
    [string]$CodeDir = ""
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

function Expand-ProductPath {
    param([string]$Template, [hashtable]$Vars)
    $result = $Template
    foreach ($key in $Vars.Keys) {
        $result = $result.Replace("{$key}", [string]$Vars[$key])
    }
    return $result
}

$yaml = Get-Content -LiteralPath $ProductYamlPath -Raw -Encoding UTF8
$productId = Get-YamlScalar $yaml "id" "demo"
$workspaceRoot = Get-YamlScalar $yaml "workspaceRoot"
$productRoot = Get-YamlScalar $yaml "productRoot"
$cfSrc = Get-YamlScalar $yaml "cfSrc" "src/cf"
$cfeSrc = Get-YamlScalar $yaml "cfeSrc" "src/cfe"
$baseline = Get-YamlScalar $yaml "stagingReleaseBranch"
$stagingDb = Get-YamlScalar $yaml "stagingDbPath"
$taskDirPattern = Get-YamlScalar $yaml "taskDirPattern" "{workspaceRoot}/{issueKey}"
$extPattern = Get-YamlScalar $yaml "extensionNamePattern" "Ext_{number}"

$number = ""
if ($IssueKey -match "-(\d+)$") { $number = $Matches[1] }
$vars = @{
    workspaceRoot = $workspaceRoot
    productRoot   = $productRoot
    issueKey      = $IssueKey
    number        = $number
}
$taskDir = Expand-ProductPath $taskDirPattern $vars
$extensionName = Expand-ProductPath $extPattern $vars
$cfRoot = Join-Path $productRoot $cfSrc
$cfeRoot = Join-Path $productRoot $cfeSrc

$inDir = Join-Path $StageHome "in"
$outDir = Join-Path $StageHome "out"
New-Item -ItemType Directory -Force -Path $inDir, $outDir, (Join-Path $StageHome "notes") | Out-Null

$productJson = [ordered]@{
    id                    = $productId
    issueKey              = $IssueKey
    workspaceRoot         = $workspaceRoot
    productRoot           = $productRoot
    cfRoot                = ($cfRoot -replace '\\', '/')
    cfeRoot               = ($cfeRoot -replace '\\', '/')
    stagingReleaseBranch  = $baseline
    stagingDbPath         = $stagingDb
    taskDir               = ($taskDir -replace '\\', '/')
    extensionName         = $extensionName
    stage                 = [int]$Stage
}
$productJsonPath = Join-Path $inDir "product.json"
($productJson | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath $productJsonPath -Encoding UTF8

function Copy-TreeIfPresent {
    param([string]$Src, [string]$Dst)
    if (-not $Src -or -not (Test-Path -LiteralPath $Src)) { return }
    New-Item -ItemType Directory -Force -Path $Dst | Out-Null
    Copy-Item -Path (Join-Path $Src "*") -Destination $Dst -Recurse -Force
}

switch ($Stage) {
    "1" { }
    "2" {
        Copy-TreeIfPresent $BriefDir $inDir
    }
    "3" {
        Copy-TreeIfPresent $PlanDir $inDir
    }
    "4" {
        Copy-TreeIfPresent $PlanDir $inDir
        $coderIn = Join-Path $inDir "coder"
        Copy-TreeIfPresent $CodeDir $coderIn
    }
}

Write-Output $productJsonPath
