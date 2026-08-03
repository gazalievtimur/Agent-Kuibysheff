#Requires -Version 5.1
<#
.SYNOPSIS
  Prepare / promote / validate / approve for the VS Code ACP 1C workflow.

.DESCRIPTION
  Does not run the agent. Sets up {ProjectRoot}/.kuibyshev/runs/vscode-active/
  for ACP agents configured in the product's .vscode/settings.json.

  Modes:
    (default)     Prepare stage home + stage_prompt.md + artifact handoff into in/
    -Promote      Copy stageN/home/out -> artifacts/{brief|plan|code|cfe}
    -Validate     Run adapters validate.ps1
    -ApprovePlan  Write artifacts/plan/APPROVED
#>
param(
    # 1C product folder (VS Code workspace / --project-root)
    [string] $ProjectRoot = "",

    # Optional product id for fallback workflows/1c-dev/products/<id>.yaml
    [string] $Product = "",

    [ValidateSet("1", "2", "3", "4", "")]
    [string] $Stage = "",

    [string] $IssueKey = "",

    [Alias("TaskFile")]
    [string] $TaskFilePath = "",

    [switch] $ApprovePlan,
    [switch] $Promote,
    [switch] $Validate,
    [switch] $RequireTz,

    # Agent Kuibyshev install (prompts + adapters); default = parent of scripts/
    [string] $RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$TaskFile = $TaskFilePath
$ChatStarter = "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn."

$agentProfiles = @{
    "1" = @{ Id = "1c-intake"; Prompt = "workflows/1c-dev/prompts/stage1.intake.md" }
    "2" = @{ Id = "1c-analyst"; Prompt = "workflows/1c-dev/prompts/stage2.analysis.md" }
    "3" = @{ Id = "1c-coder"; Prompt = "workflows/1c-dev/prompts/stage3.coder.md" }
    "4" = @{ Id = "1c-implementer"; Prompt = "workflows/1c-dev/prompts/stage4.implement.md" }
}

function Resolve-RepoPath {
    param([string]$Root, [string]$Relative)
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Relative))
}

function Expand-Template {
    param([string]$Template, [hashtable]$Vars)
    $result = $Template
    foreach ($key in $Vars.Keys) {
        $result = $result.Replace("{{$key}}", [string]$Vars[$key])
        $result = $result.Replace("{$key}", [string]$Vars[$key])
    }
    return $result
}

function Copy-DirContents {
    param([string]$Src, [string]$Dst)
    if (-not (Test-Path -LiteralPath $Src)) { return }
    New-Item -ItemType Directory -Force -Path $Dst | Out-Null
    Copy-Item -Path (Join-Path $Src "*") -Destination $Dst -Recurse -Force -ErrorAction SilentlyContinue
}

function Normalize-TaskFileBrief {
    param([string]$SrcFile, [string]$DestDir)
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
    $raw = Get-Content -LiteralPath $SrcFile -Raw -Encoding UTF8
    $destBrief = Join-Path $DestDir "task_brief.md"
    $looksLikeBrief = $raw -match "(?im)^#\s*Task brief" -or $raw -match "(?im)^##\s*Requirements"
    if ($looksLikeBrief) {
        Copy-Item -LiteralPath $SrcFile -Destination $destBrief -Force
    } else {
        $srcCopy = Join-Path $DestDir "task_source.md"
        Copy-Item -LiteralPath $SrcFile -Destination $srcCopy -Force
        $title = [System.IO.Path]::GetFileNameWithoutExtension($SrcFile)
        $wrapper = @"
# Task brief: $title

## Source
- Origin: task_file
- Path: $SrcFile

## Summary
Provided by operator file (intake skipped). Full text in task_source.md.

## Requirements and acceptance
See task_source.md.

## Related documentation
- (none from intake)

## Images and attachments
| Name | Source | Description / note |
| --- | --- | --- |

## Open questions
- Review operator-provided task_source.md for completeness

## tz_status
partial

## Raw references
- task_source.md
"@
        Set-Content -LiteralPath $destBrief -Value $wrapper -Encoding UTF8
    }

    $sources = [ordered]@{
        origin         = "task_file"
        path           = ($SrcFile -replace '\\', '/')
        skipped_intake = $true
    }
    ($sources | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $DestDir "sources.json") -Encoding UTF8

    $manifest = [ordered]@{
        schema_version = 1
        summary        = "intake skipped: task file provided"
        files_written  = @("task_brief.md", "sources.json")
        patches        = @()
        apply_mode     = "none"
    }
    ($manifest | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $DestDir "manifest.json") -Encoding UTF8
}

function Get-ArtifactDirForStage {
    param([int]$N, [string]$BriefDir, [string]$PlanDir, [string]$CodeDir, [string]$CfeDir)
    switch ($N) {
        1 { return $BriefDir }
        2 { return $PlanDir }
        3 { return $CodeDir }
        4 { return $CfeDir }
        default { throw "Invalid stage $N" }
    }
}

function Write-ActiveMeta {
    param(
        [string]$RunDir,
        [string]$ProjectRoot,
        [string]$Product,
        [string]$IssueKey,
        [string]$TaskFile,
        [string]$LastAction,
        [string]$Stage
    )
    $meta = [ordered]@{
        runId        = "vscode-active"
        projectRoot  = $ProjectRoot
        product      = $Product
        issueKey     = $IssueKey
        taskFile     = $TaskFile
        lastAction   = $LastAction
        stage        = $Stage
        updated_at   = (Get-Date).ToString("o")
    }
    ($meta | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $RunDir "active.json") -Encoding UTF8
}

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

$workflowRoot = Resolve-RepoPath $RepoRoot "workflows/1c-dev"

# Resolve ProjectRoot (required for most modes; can load from active.json)
$activeMetaPathHint = $null
if (-not [string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
} else {
    # Legacy fallback: workflows/1c-dev/runs/vscode-active
    $legacyRun = Join-Path $workflowRoot "runs\vscode-active"
    $legacyMeta = Join-Path $legacyRun "active.json"
    if (Test-Path -LiteralPath $legacyMeta) {
        $prev = Get-Content -LiteralPath $legacyMeta -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($prev.projectRoot) { $ProjectRoot = [string]$prev.projectRoot }
    }
}

if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    throw "Specify -ProjectRoot (1C product folder / VS Code workspace)"
}
if (-not (Test-Path -LiteralPath $ProjectRoot -PathType Container)) {
    throw "ProjectRoot not found: $ProjectRoot"
}

$kuibRoot = Join-Path $ProjectRoot ".kuibyshev"
$runDir = Join-Path $kuibRoot "runs\vscode-active"
$artifacts = Join-Path $runDir "artifacts"
$briefDir = Join-Path $artifacts "brief"
$planDir = Join-Path $artifacts "plan"
$codeDir = Join-Path $artifacts "code"
$cfeDir = Join-Path $artifacts "cfe"
New-Item -ItemType Directory -Force -Path $briefDir, $planDir, $codeDir, $cfeDir | Out-Null

$pathPrepareHome = Join-Path $workflowRoot "adapters\default\prepare-home.ps1"
$pathValidate = Join-Path $workflowRoot "adapters\default\validate.ps1"

$activeMetaPath = Join-Path $runDir "active.json"
if (Test-Path -LiteralPath $activeMetaPath) {
    $prev = Get-Content -LiteralPath $activeMetaPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($IssueKey) -and $prev.issueKey) {
        $IssueKey = [string]$prev.issueKey
    }
    if ([string]::IsNullOrWhiteSpace($Product) -and $prev.product) {
        $Product = [string]$prev.product
    }
    if ([string]::IsNullOrWhiteSpace($TaskFile) -and $prev.taskFile) {
        $TaskFile = [string]$prev.taskFile
    }
}

# Product yaml: project .kuibyshev/product.yaml, else workflows products/<Product>.yaml
$productPath = Join-Path $kuibRoot "product.yaml"
if (-not (Test-Path -LiteralPath $productPath -PathType Leaf)) {
    if ([string]::IsNullOrWhiteSpace($Product)) {
        $Product = "demo"
    }
    $fallback = Resolve-RepoPath $RepoRoot "workflows/1c-dev/products/$Product.yaml"
    if (Test-Path -LiteralPath $fallback -PathType Leaf) {
        $productPath = $fallback
    }
}
if ($Product -and (Test-Path -LiteralPath (Join-Path $workflowRoot "adapters\$Product\prepare-home.ps1"))) {
    $pathPrepareHome = Join-Path $workflowRoot "adapters\$Product\prepare-home.ps1"
}
if ($Product -and (Test-Path -LiteralPath (Join-Path $workflowRoot "adapters\$Product\validate.ps1"))) {
    $pathValidate = Join-Path $workflowRoot "adapters\$Product\validate.ps1"
}

$skipIntake = -not [string]::IsNullOrWhiteSpace($TaskFile)
if ($skipIntake) {
    $TaskFile = [System.IO.Path]::GetFullPath($TaskFile)
    if (-not (Test-Path -LiteralPath $TaskFile -PathType Leaf)) {
        throw "TaskFile not found: $TaskFile"
    }
}

if ($ApprovePlan -and [string]::IsNullOrWhiteSpace($Stage) -and -not $Promote -and -not $Validate) {
    $approvedPath = Join-Path $planDir "APPROVED"
    Set-Content -LiteralPath $approvedPath -Value ("approved_at={0}" -f (Get-Date).ToString("o")) -Encoding UTF8
    Write-Host "Approved plan: $approvedPath"
    Write-ActiveMeta -RunDir $runDir -ProjectRoot $ProjectRoot -Product $Product -IssueKey $IssueKey `
        -TaskFile $TaskFile -LastAction "approve" -Stage ""
    exit 0
}

if ([string]::IsNullOrWhiteSpace($Stage)) {
    throw "Specify -Stage 1|2|3|4 (or -ApprovePlan alone)"
}

$stageNum = [int]$Stage
$stageHome = Join-Path $runDir ("stage{0}\home" -f $stageNum)
$outDir = Join-Path $stageHome "out"
$artifactDir = Get-ArtifactDirForStage -N $stageNum -BriefDir $briefDir -PlanDir $planDir `
    -CodeDir $codeDir -CfeDir $cfeDir
$profile = $agentProfiles["$Stage"]
$productLabel = if ($Product) { $Product } else { Split-Path -Leaf $ProjectRoot }

if ($Promote) {
    if (-not (Test-Path -LiteralPath $outDir)) {
        throw "Nothing to promote: missing $outDir"
    }
    Copy-DirContents $outDir $artifactDir
    if ($stageNum -eq 2) {
        $wf = @"
# Состояние конвейера задачи

| Поле | Значение |
|------|----------|
| режим_conveyor | strict |
| фаза_следующей_работы | 3 |
| ожидается_gate | approve_plan |
| примечание | Plan ready; await -ApprovePlan (VS Code ACP) |
"@
        Set-Content -LiteralPath (Join-Path $planDir "workflow-state.md") -Value $wf -Encoding UTF8
    }
    Write-Host "Promoted stage $Stage out -> $artifactDir"
    Write-ActiveMeta -RunDir $runDir -ProjectRoot $ProjectRoot -Product $Product -IssueKey $IssueKey `
        -TaskFile $TaskFile -LastAction "promote" -Stage $Stage
    if ($Validate) {
        $validateDir = $artifactDir
        $valArgs = @{ Stage = "$Stage"; OutDir = $validateDir }
        if ($RequireTz -and $Stage -eq "1") { $valArgs.RequireTz = $true }
        & $pathValidate @valArgs
        Write-Host "Validated stage $Stage at $validateDir"
    }
    exit 0
}

if ($Validate) {
    $validateDir = $outDir
    if (-not (Test-Path -LiteralPath (Join-Path $outDir "manifest.json"))) {
        $validateDir = $artifactDir
    }
    $valArgs = @{ Stage = "$Stage"; OutDir = $validateDir }
    if ($RequireTz -and $Stage -eq "1") { $valArgs.RequireTz = $true }
    & $pathValidate @valArgs
    Write-Host "Validated stage $Stage at $validateDir"
    Write-ActiveMeta -RunDir $runDir -ProjectRoot $ProjectRoot -Product $Product -IssueKey $IssueKey `
        -TaskFile $TaskFile -LastAction "validate" -Stage $Stage
    exit 0
}

# --- Prepare ---
if (-not (Test-Path -LiteralPath $productPath -PathType Leaf)) {
    throw "Product config not found: $productPath (scaffold .kuibyshev/product.yaml or pass products/<id>.yaml via -Product)"
}

if ($skipIntake -and $Stage -eq "1") {
    throw "-TaskFile skips intake; cannot prepare stage 1"
}
if (-not $skipIntake -and [string]::IsNullOrWhiteSpace($IssueKey)) {
    throw "Provide -IssueKey or -TaskFile"
}

if ($skipIntake -and $stageNum -ge 2) {
    if (-not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        Normalize-TaskFileBrief -SrcFile $TaskFile -DestDir $briefDir
        Write-Host "Normalized TaskFile -> $briefDir"
    }
}

if ($stageNum -eq 2) {
    if (-not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        throw "Missing brief at $briefDir (prepare+chat+promote stage 1, or pass -TaskFile)"
    }
}

if ($stageNum -ge 3) {
    $approvedPath = Join-Path $planDir "APPROVED"
    if ($ApprovePlan) {
        Set-Content -LiteralPath $approvedPath -Value ("approved_at={0}" -f (Get-Date).ToString("o")) -Encoding UTF8
        Write-Host "Approved plan: $approvedPath"
    } elseif (-not (Test-Path -LiteralPath $approvedPath)) {
        Write-Host "GATE: plan awaiting approval at $planDir"
        Write-Host "Then: -ApprovePlan  (or prepare stage $Stage -ApprovePlan)"
        exit 2
    }
    foreach ($req in @("tasks.md", "architecture.md", "cfe-scope.md")) {
        if (-not (Test-Path -LiteralPath (Join-Path $planDir $req))) {
            throw "Missing plan artifact $req in $planDir (promote stage 2 first)"
        }
    }
}

if ($stageNum -eq 4) {
    if (-not (Test-Path -LiteralPath (Join-Path $codeDir "code-report.md"))) {
        throw "Missing coder artifacts in $codeDir (promote stage 3 first)"
    }
}

New-Item -ItemType Directory -Force -Path $stageHome | Out-Null

& $pathPrepareHome `
    -StageHome $stageHome `
    -ProductYamlPath $productPath `
    -Stage "$Stage" `
    -IssueKey $IssueKey `
    -BriefDir $briefDir `
    -PlanDir $planDir `
    -CodeDir $codeDir | Out-Null

$promptTpl = Get-Content -LiteralPath (Resolve-RepoPath $RepoRoot $profile.Prompt) -Raw -Encoding UTF8
$prompt = Expand-Template $promptTpl @{ ISSUE_KEY = $IssueKey; PRODUCT = $productLabel }
$inDir = Join-Path $stageHome "in"
New-Item -ItemType Directory -Force -Path $inDir | Out-Null
$promptFile = Join-Path $inDir "stage_prompt.md"
Set-Content -LiteralPath $promptFile -Value $prompt -Encoding UTF8
Set-Content -LiteralPath (Join-Path $inDir "CHAT_STARTER.txt") -Value $ChatStarter -Encoding UTF8

Write-ActiveMeta -RunDir $runDir -ProjectRoot $ProjectRoot -Product $Product -IssueKey $IssueKey `
    -TaskFile $TaskFile -LastAction "prepare" -Stage $Stage

$agentDir = Join-Path $kuibRoot ("agents\{0}" -f $profile.Id)
Write-Host ""
Write-Host "Prepared stage $Stage ($($profile.Id))"
Write-Host "  project: $ProjectRoot"
Write-Host "  home:    $stageHome"
Write-Host "  prompt:  $promptFile"
Write-Host "  agent:   $($profile.Id)  (settings: $agentDir)"
Write-Host ""
Write-Host "Paste into chat:"
Write-Host "  $ChatStarter"
Write-Host ""
Write-Host "After the agent finishes: -Promote -Stage $Stage -ProjectRoot `"$ProjectRoot`""
exit 0
