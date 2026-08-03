#Requires -Version 5.1
<#
.SYNOPSIS
  Scaffold .kuibyshev/ agents + product.yaml into a 1C product folder.
#>
param(
    [Parameter(Mandatory = $true)][string] $ProjectRoot,
    [string] $RepoRoot = "",
    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}
$ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
if (-not (Test-Path -LiteralPath $ProjectRoot -PathType Container)) {
    throw "ProjectRoot not found: $ProjectRoot"
}

$kuib = Join-Path $ProjectRoot ".kuibyshev"
$agentsRoot = Join-Path $kuib "agents"
$runsRoot = Join-Path $kuib "runs"
New-Item -ItemType Directory -Force -Path $agentsRoot, $runsRoot | Out-Null

$profiles = @("1c-intake", "1c-analyst", "1c-coder", "1c-implementer")
foreach ($id in $profiles) {
    $src = Join-Path $RepoRoot "test-agents\$id"
    if (-not (Test-Path -LiteralPath $src -PathType Container)) {
        throw "Missing template profile: $src"
    }
    $dst = Join-Path $agentsRoot $id
    if ((Test-Path -LiteralPath $dst) -and -not $Force) {
        Write-Host "Skip existing $dst (use -Force to overwrite)"
        continue
    }
    if (Test-Path -LiteralPath $dst) {
        Remove-Item -LiteralPath $dst -Recurse -Force
    }
    Copy-Item -LiteralPath $src -Destination $dst -Recurse -Force

    $example = Join-Path $dst "agent-config.example.yaml"
    $local = Join-Path $dst "agent-config.local.yaml"
    $config = Join-Path $dst "agent-config.yaml"
    if (Test-Path -LiteralPath $example) {
        Copy-Item -LiteralPath $example -Destination $config -Force
    } elseif (Test-Path -LiteralPath $local) {
        Copy-Item -LiteralPath $local -Destination $config -Force
    }

    # Prefer project-relative CF root (resolved against config parent dir).
    if (Test-Path -LiteralPath $config) {
        $text = Get-Content -LiteralPath $config -Raw -Encoding UTF8
        $cfRel = "../../../src/cf"
        $text = [regex]::Replace($text, '(?m)^(\s*root:\s*)"[^"]*"', "`${1}`"$cfRel`"")
        $text = [regex]::Replace(
            $text,
            '("serve",\s*"--path",\s*")[^"]*(")',
            "`${1}$cfRel`${2}"
        )
        Set-Content -LiteralPath $config -Value $text -Encoding UTF8
    }

    Write-Host "Scaffolded $dst"
}

$productSrc = Join-Path $RepoRoot "workflows\1c-dev\vscode\product.yaml.example"
$productDst = Join-Path $kuib "product.yaml"
if ((-not (Test-Path -LiteralPath $productDst)) -or $Force) {
    Copy-Item -LiteralPath $productSrc -Destination $productDst -Force
    Write-Host "Wrote $productDst"
}

$giSrc = Join-Path $RepoRoot "workflows\1c-dev\vscode\gitignore.kuibyshev.example"
$giDst = Join-Path $ProjectRoot ".gitignore"
$giSnippet = Get-Content -LiteralPath $giSrc -Raw -Encoding UTF8
if (Test-Path -LiteralPath $giDst) {
    $existing = Get-Content -LiteralPath $giDst -Raw -Encoding UTF8
    if ($existing -notmatch '\.kuibyshev/runs') {
        Add-Content -LiteralPath $giDst -Value "`n$giSnippet" -Encoding UTF8
        Write-Host "Appended .kuibyshev/runs to $giDst"
    }
} else {
    Set-Content -LiteralPath $giDst -Value $giSnippet.TrimEnd() -Encoding UTF8
    Write-Host "Wrote $giDst"
}

$vscodeDir = Join-Path $ProjectRoot ".vscode"
New-Item -ItemType Directory -Force -Path $vscodeDir | Out-Null
$settingsSrc = Join-Path $RepoRoot "workflows\1c-dev\vscode\project-settings.acp.example.json"
$settingsDst = Join-Path $vscodeDir "settings.json"
$tasksSrc = Join-Path $RepoRoot "workflows\1c-dev\vscode\tasks.acp.example.json"
$tasksDst = Join-Path $vscodeDir "tasks.json"

if ((-not (Test-Path -LiteralPath $settingsDst)) -or $Force) {
    Copy-Item -LiteralPath $settingsSrc -Destination $settingsDst -Force
    Write-Host "Wrote $settingsDst (merge manually if you already had VS Code settings)"
} else {
    Write-Host "Keep existing $settingsDst - merge acp.agents from project-settings.acp.example.json"
}

if ((-not (Test-Path -LiteralPath $tasksDst)) -or $Force) {
    Copy-Item -LiteralPath $tasksSrc -Destination $tasksDst -Force
    Write-Host "Wrote $tasksDst"
}

Write-Host ""
Write-Host "Next:"
Write-Host "  1. Edit product.yaml and agents/*/agent-config.yaml (MCP paths, workspace.root)."
Write-Host "  2. Ensure agent_Kuibyshev is on PATH."
Write-Host "  3. Open the project folder in VS Code and connect ACP agents 1c-intake .. 1c-implementer."
exit 0
