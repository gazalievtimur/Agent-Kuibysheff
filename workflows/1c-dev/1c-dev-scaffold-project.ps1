#Requires -Version 5.1
<#
.SYNOPSIS
  Scaffold .kuibysheff/protected/agents + product.yaml into a 1C product folder.
#>
param(
    [Parameter(Mandatory = $true)][string] $ProjectRoot,
    [string] $RepoRoot = "",
    [string] $WorkflowRoot = "",
    [switch] $Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $parent = Join-Path $PSScriptRoot ".."
    $grand = Join-Path $PSScriptRoot "..\.."
    if (Test-Path -LiteralPath (Join-Path $grand "Cargo.toml")) {
        $RepoRoot = Resolve-Path $grand
    } elseif (Test-Path -LiteralPath (Join-Path $parent "Cargo.toml")) {
        $RepoRoot = Resolve-Path $parent
    } else {
        $RepoRoot = ""
    }
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

if ($WorkflowRoot) {
    $workflowRoot = Resolve-Path $WorkflowRoot
} elseif (Test-Path -LiteralPath (Join-Path $PSScriptRoot "prompts") -PathType Container) {
    $workflowRoot = Resolve-Path $PSScriptRoot
} elseif ($RepoRoot) {
    $workflowRoot = Join-Path $RepoRoot "workflows\1c-dev"
} else {
    throw "Specify -WorkflowRoot (1C workflow copy unit with agents/)"
}

$ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
if (-not (Test-Path -LiteralPath $ProjectRoot -PathType Container)) {
    throw "ProjectRoot not found: $ProjectRoot"
}

$kuib = Join-Path $ProjectRoot ".kuibysheff"
$agentsRoot = Join-Path $kuib "protected\agents"
$runsRoot = Join-Path $kuib "runs"
New-Item -ItemType Directory -Force -Path $agentsRoot, $runsRoot | Out-Null

# Prefer agent CLI import when available (canonical protected store + ACL).
$agentBin = $null
foreach ($candidate in @(
        "agent_Kuibysheff",
        $(if ($RepoRoot) { Join-Path $RepoRoot "target\release\agent_Kuibysheff.exe" } else { $null }),
        $(if ($RepoRoot) { Join-Path $RepoRoot "target\debug\agent_Kuibysheff.exe" } else { $null })
    )) {
    if (-not $candidate) { continue }
    if (Get-Command $candidate -ErrorAction SilentlyContinue) {
        $agentBin = $candidate
        break
    }
    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
        $agentBin = $candidate
        break
    }
}

$profiles = @("1c-intake", "1c-analyst", "1c-coder", "1c-implementer")
foreach ($id in $profiles) {
    $src = Join-Path $workflowRoot "agents\$id"
    if (-not (Test-Path -LiteralPath $src -PathType Container)) {
        if ($RepoRoot) {
            $src = Join-Path $RepoRoot "test-agents\$id"
        }
    }
    if (-not (Test-Path -LiteralPath $src -PathType Container)) {
        throw "Missing template profile: $src"
    }

    $dst = Join-Path $agentsRoot $id
    if ((Test-Path -LiteralPath $dst) -and -not $Force) {
        Write-Host "Skip existing $dst (use -Force to overwrite)"
        continue
    }

    if ($agentBin) {
        Write-Host "Importing $id via agent CLI into protected store..."
        & $agentBin init $id --project-root $ProjectRoot --force 2>$null | Out-Null
        & $agentBin config --project-root $ProjectRoot --agent $id import --from $src --force
        if ($LASTEXITCODE -ne 0) {
            throw "config import failed for $id (exit $LASTEXITCODE)"
        }
    } else {
        if (Test-Path -LiteralPath $dst) {
            Remove-Item -LiteralPath $dst -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path $dst | Out-Null
        Copy-Item -LiteralPath (Join-Path $src "*") -Destination $dst -Recurse -Force

        $example = Join-Path $dst "agent-config.example.yaml"
        $local = Join-Path $dst "agent-config.local.yaml"
        $config = Join-Path $dst "agent-config.yaml"
        if (-not (Test-Path -LiteralPath $config)) {
            if (Test-Path -LiteralPath $example) {
                Copy-Item -LiteralPath $example -Destination $config -Force
            } elseif (Test-Path -LiteralPath $local) {
                Copy-Item -LiteralPath $local -Destination $config -Force
            }
        }
    }

    $config = Join-Path $dst "agent-config.yaml"
    # Prefer project-relative CF root (resolved against config parent dir).
    if (Test-Path -LiteralPath $config) {
        $text = Get-Content -LiteralPath $config -Raw -Encoding UTF8
        # protected/agents/<id> → project root is ../../../../ (four levels up from profile)
        $cfRel = "../../../../src/cf"
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

# Fail closed on unresolved machine paths left in generated configs.
$machinePathPattern = '(?i)(?:\b[A-Za-z]:(?:\\|/)|/(?:Users|home)/)'
$requiredLeft = [System.Collections.Generic.List[string]]::new()
Get-ChildItem -LiteralPath $agentsRoot -Recurse -Filter "agent-config.yaml" -ErrorAction SilentlyContinue | ForEach-Object {
    $text = Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8
    if ($text -match $machinePathPattern) {
        throw "Scaffold left machine-local path in $($_.FullName). Replace with project-relative paths or REQUIRED_* placeholders."
    }
    if ($text -match 'REQUIRED_[A-Z0-9_]+') {
        $requiredLeft.Add($_.FullName) | Out-Null
    }
}
if ($requiredLeft.Count -gt 0) {
    Write-Host ""
    Write-Host "WARNING: generated configs still contain REQUIRED_* placeholders:"
    $requiredLeft | ForEach-Object { Write-Host "  $_" }
    Write-Host "Edit these before running agents (SNTX_SEM_CONFIG / BSL indexer / etc)."
}

$productSrc = Join-Path $workflowRoot "vscode\product.yaml.example"
$productDst = Join-Path $kuib "product.yaml"
if ((-not (Test-Path -LiteralPath $productDst)) -or $Force) {
    Copy-Item -LiteralPath $productSrc -Destination $productDst -Force
    Write-Host "Wrote $productDst"
}

$giSrc = Join-Path $workflowRoot "vscode\gitignore.kuibysheff.example"
$giDst = Join-Path $ProjectRoot ".gitignore"
$giSnippet = Get-Content -LiteralPath $giSrc -Raw -Encoding UTF8
if (Test-Path -LiteralPath $giDst) {
    $existing = Get-Content -LiteralPath $giDst -Raw -Encoding UTF8
    if ($existing -notmatch '\.kuibysheff/runs') {
        Add-Content -LiteralPath $giDst -Value "`n$giSnippet" -Encoding UTF8
        Write-Host "Appended .kuibysheff/runs to $giDst"
    }
} else {
    Set-Content -LiteralPath $giDst -Value $giSnippet.TrimEnd() -Encoding UTF8
    Write-Host "Wrote $giDst"
}

$vscodeDir = Join-Path $ProjectRoot ".vscode"
New-Item -ItemType Directory -Force -Path $vscodeDir | Out-Null
$settingsSrc = Join-Path $workflowRoot "vscode\project-settings.acp.example.json"
$settingsDst = Join-Path $vscodeDir "settings.json"
$tasksSrc = Join-Path $workflowRoot "vscode\tasks.acp.example.json"
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
Write-Host "  1. Edit product.yaml and .kuibysheff/protected/agents/*/agent-config.yaml (MCP paths, workspace.root)."
Write-Host "  2. Ensure agent_Kuibysheff is on PATH."
Write-Host "  3. Open the project folder in VS Code and connect ACP agents 1c-intake .. 1c-implementer."
exit 0
