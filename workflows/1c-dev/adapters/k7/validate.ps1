#Requires -Version 5.1
<#
.SYNOPSIS
  Validate stage artifacts for K7 1C workflow.
#>
param(
    [Parameter(Mandatory = $true)][ValidateSet("1", "2", "3", "4")][string]$Stage,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [switch]$RequireTz
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-File {
    param([string]$Path, [string]$Label)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Stage $Stage validation failed: missing $Label ($Path)"
    }
}

function Assert-DirNonEmpty {
    param([string]$Path, [string]$Label)
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Stage $Stage validation failed: missing $Label ($Path)"
    }
    $items = @(Get-ChildItem -LiteralPath $Path -Recurse -File -ErrorAction SilentlyContinue)
    if ($items.Count -eq 0) {
        throw "Stage $Stage validation failed: empty $Label ($Path)"
    }
}

$manifestPath = Join-Path $OutDir "manifest.json"
Assert-File $manifestPath "manifest.json"
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json

switch ($Stage) {
    "1" {
        Assert-File (Join-Path $OutDir "task_brief.md") "task_brief.md"
        Assert-File (Join-Path $OutDir "sources.json") "sources.json"
        if ($manifest.apply_mode -ne "none") {
            throw "Stage 1: apply_mode must be none"
        }
        if ($RequireTz) {
            $brief = Get-Content -LiteralPath (Join-Path $OutDir "task_brief.md") -Raw -Encoding UTF8
            if ($brief -notmatch "(?im)tz_status\s*[:\r\n].*\bok\b" -and
                $brief -notmatch "(?im)##\s*Requirements") {
                throw "Stage 1: -RequireTz failed (tz_status ok or Requirements section expected)"
            }
        }
    }
    "2" {
        foreach ($name in @("prd.md", "architecture.md", "tasks.md", "cfe-scope.md")) {
            Assert-File (Join-Path $OutDir $name) $name
        }
        if ($manifest.apply_mode -ne "none") {
            throw "Stage 2: apply_mode must be none"
        }
    }
    "3" {
        Assert-File (Join-Path $OutDir "code-report.md") "code-report.md"
        Assert-File (Join-Path $OutDir "files-index.md") "files-index.md"
        $src = Join-Path $OutDir "src"
        $report = Get-Content -LiteralPath (Join-Path $OutDir "code-report.md") -Raw -Encoding UTF8
        $hasSrc = (Test-Path -LiteralPath $src) -and (@(Get-ChildItem -LiteralPath $src -Recurse -File -ErrorAction SilentlyContinue).Count -gt 0)
        if (-not $hasSrc -and -not ($report -match "(?im)blocked")) {
            throw "Stage 3: out/src empty and code-report has no blocked reason"
        }
        if ($manifest.apply_mode -ne "none") {
            throw "Stage 3: apply_mode must be none"
        }
    }
    "4" {
        Assert-DirNonEmpty (Join-Path $OutDir "cfe") "cfe/"
        Assert-File (Join-Path $OutDir "implement-report.md") "implement-report.md"
        Assert-File (Join-Path $OutDir "checklist.md") "checklist.md"
        if ($manifest.apply_mode -ne "copy_out") {
            throw "Stage 4: apply_mode must be copy_out"
        }
    }
}

Write-Host "Stage $Stage artifacts OK: $OutDir"
