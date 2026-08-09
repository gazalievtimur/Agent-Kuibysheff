#Requires -Version 5.1
<#
.SYNOPSIS
  Offline portability guardrails: artifact ignore check + static path gate on configs.
#>
param(
    [string] $RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

Push-Location $RepoRoot
try {
    Write-Host "== artifact ignore =="
    $probePaths = @(
        "run/out/manifest.json",
        "demo-home/searxng/out/manifest.json",
        "workflows/aoc-live/__pycache__/runtime.cpython-312.pyc",
        ".cursor/mcp.json"
    )
    foreach ($rel in $probePaths) {
        git check-ignore -q $rel
        if ($LASTEXITCODE -ne 0) {
            throw "Expected gitignore rule for $rel (git check-ignore exit $LASTEXITCODE)"
        }
        Write-Host "ok ignored: $rel"
    }

    Write-Host "== static absolute-path gate (tracked agent/product configs) =="
    # Word-boundary drive roots avoid matching https:// URL schemes.
    $pattern = '(?i)(?:\b[A-Za-z]:(?:\\|/)|/(?:Users|home)/)'
    $hits = @()
    $files = git ls-files -- `
        "**/agent-config*.yaml" `
        "**/agent-config*.yml" `
        "workflows/1c-dev/products/*.yaml.example" `
        ".cursor/mcp.json.example"
    foreach ($file in $files) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { continue }
        $text = Get-Content -LiteralPath $file -Raw -ErrorAction SilentlyContinue
        if ($null -eq $text) { continue }
        if ($text -match $pattern) {
            $hits += $file
        }
    }
    if ($hits.Count -gt 0) {
        Write-Host "Tracked config files with machine-local path patterns:"
        $hits | Select-Object -Unique | ForEach-Object { Write-Host "  $_" }
        throw "Static absolute-path gate failed ($(($hits | Select-Object -Unique).Count) file(s))"
    }
    Write-Host "ok: no disallowed absolute paths in tracked agent/product configs"

    Write-Host "Portability guardrails passed."
} finally {
    Pop-Location
}
