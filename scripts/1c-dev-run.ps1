#Requires -Version 5.1
<#
.SYNOPSIS
  Orchestrate the 1C Kuibyshev workflow: intake -> analyst -> coder -> implementer.

.DESCRIPTION
  Chains agent_Kuibyshev runs with home handoff, optional -TaskFile (skip intake),
  human gate before coder, and optional CFE apply/build for product adapters.
#>
param(
    [Parameter(Mandatory = $true)]
    [string] $Product,

    [string] $IssueKey = "",

    [Alias("TaskFile")]
    [string] $TaskFilePath = "",

    [ValidateSet("all", "1", "2", "3", "4")]
    [Alias("Stage")]
    [string] $WorkflowStage = "all",

    [string] $FromStage = "",

    [string] $AgentBin = "",
    [string] $RunsRoot = "",
    [string] $RunId = "",
    [string] $RepoRoot = "",
    [string] $ConfigOverride = "",

    [switch] $ApprovePlan,
    [switch] $RequireTz,
    [switch] $RequireSearx,
    [switch] $BuildCfe,
    # Named DoApplyOut so it does not collide with path vars ($applyOut) — PS is case-insensitive.
    [Alias("ApplyOut")]
    [switch] $DoApplyOut,
    [Alias("Force")]
    [switch] $ForceRerun
)

# Back-compat aliases
$TaskFile = $TaskFilePath
$Stage = $WorkflowStage


Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    param([string]$Root, [string]$Relative)
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Relative))
}

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

function Test-StageDone {
    param([string]$MarkerPath)
    return (Test-Path -LiteralPath $MarkerPath -PathType Leaf)
}

function Write-StageMarker {
    param([string]$Path, [string]$StopReason)
    $obj = [ordered]@{ stop_reason = $StopReason; finished_at = (Get-Date).ToString("o") }
    ($obj | ConvertTo-Json) | Set-Content -LiteralPath $Path -Encoding UTF8
}

function Invoke-AgentRun {
    param(
        [string]$Bin,
        [string]$Config,
        [string]$SettingsDir,
        [string]$Prompt,
        [string]$HomeDir,
        [string[]]$Files,
        [string]$StdoutPath,
        [string]$StderrPath
    )

    # Persist full prompt to avoid CLI encoding/length issues; pass a short pointer.
    $promptFile = Join-Path $HomeDir "in\stage_prompt.md"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $promptFile) | Out-Null
    Set-Content -LiteralPath $promptFile -Value $Prompt -Encoding UTF8
    $shortPrompt = "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn."

    $allFiles = @($promptFile) + @($Files | Where-Object { $_ })
    $argList = @(
        "run",
        "--config", $Config,
        "--settings-dir", $SettingsDir,
        "--prompt", $shortPrompt,
        "--home", $HomeDir
    )
    foreach ($f in $allFiles) {
        if ($f -and (Test-Path -LiteralPath $f -PathType Leaf)) {
            $argList += @("--files", $f)
        }
    }

    $argPreview = ($argList | ForEach-Object {
            if ($_ -eq $shortPrompt) { "<short-prompt>" } else { $_ }
        }) -join " "
    Write-Host ">> $Bin $argPreview"

    $stdout = ""
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        $stdoutLines = & $Bin @argList 2> $StderrPath
        if ($null -eq $stdoutLines) { $stdoutLines = @() }
        if ($stdoutLines -is [array]) { $stdout = ($stdoutLines | ForEach-Object { "$_" }) -join "`n" }
        else { $stdout = [string]$stdoutLines }
    } finally {
        $ErrorActionPreference = $oldEap
    }
    $exit = $LASTEXITCODE
    Set-Content -LiteralPath $StdoutPath -Value $stdout -Encoding UTF8

    $stopReason = "error"
    $resultText = ""
    try {
        # Prefer last JSON object in stdout (ignore noise)
        $jsonText = $stdout.Trim()
        if ($jsonText -match '(?s)\{.*"stop_reason".*\}\s*$') {
            $jsonText = $Matches[0]
        }
        $parsed = $jsonText | ConvertFrom-Json
        if ($parsed.stop_reason) { $stopReason = [string]$parsed.stop_reason }
        if ($null -ne $parsed.result) { $resultText = [string]$parsed.result }
    } catch {
        Write-Warning "Failed to parse RunOutput JSON from stdout"
    }

    return [pscustomobject]@{
        ExitCode   = $exit
        StopReason = $stopReason
        Result     = $resultText
        StdoutPath = $StdoutPath
    }
}

function Test-HttpReachable {
    param([string]$Url)
    try {
        $resp = Invoke-WebRequest -Uri $Url -Method Get -TimeoutSec 3 -UseBasicParsing -ErrorAction Stop
        return $true
    } catch {
        # MCP endpoint may reject GET; connection refused vs other errors
        if ($_.Exception.Message -match "refus|unreachable|Unable to connect|timed out") {
            return $false
        }
        return $true
    }
}

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

$dotenv = Join-Path $PSScriptRoot "import-dotenv.ps1"
if (Test-Path -LiteralPath $dotenv) {
    . $dotenv
    Import-DotEnv (Join-Path $RepoRoot ".env")
}

$workflowRoot = Resolve-RepoPath $RepoRoot "workflows/1c-dev"
$productPath = Resolve-RepoPath $RepoRoot "workflows/1c-dev/products/$Product.yaml"
if (-not (Test-Path -LiteralPath $productPath -PathType Leaf)) {
    throw "Product config not found: $productPath (copy zup.yaml.example if needed)"
}
$productYaml = Get-Content -LiteralPath $productPath -Raw -Encoding UTF8
$searxUrl = Get-YamlScalar $productYaml "searxngUrl" "http://127.0.0.1:3000/mcp"

$skipIntake = -not [string]::IsNullOrWhiteSpace($TaskFile)
if ($skipIntake) {
    $TaskFile = [System.IO.Path]::GetFullPath($TaskFile)
    if (-not (Test-Path -LiteralPath $TaskFile -PathType Leaf)) {
        throw "TaskFile not found: $TaskFile"
    }
}
if (-not $skipIntake -and [string]::IsNullOrWhiteSpace($IssueKey)) {
    throw "Provide -IssueKey or -TaskFile"
}
if ($skipIntake -and $Stage -eq "1") {
    throw "-TaskFile skips intake; cannot use -Stage 1"
}
if (-not [string]::IsNullOrWhiteSpace($FromStage) -and $FromStage -notin @("1", "2", "3", "4")) {
    throw "FromStage must be 1, 2, 3, or 4"
}

if (-not $RunsRoot) {
    $RunsRoot = Join-Path $workflowRoot "runs"
} else {
    $RunsRoot = [System.IO.Path]::GetFullPath($RunsRoot)
}

if (-not $RunId) {
    $stamp = Get-Date -Format "yyyyMMdd_HHmmss"
    if ($IssueKey) {
        $RunId = "${IssueKey}_$stamp"
    } else {
        $stem = [System.IO.Path]::GetFileNameWithoutExtension($TaskFile)
        $RunId = "${stem}_$stamp"
    }
}

$runDir = Join-Path $RunsRoot $RunId
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
$artifacts = Join-Path $runDir "artifacts"
$briefDir = Join-Path $artifacts "brief"
$planDir = Join-Path $artifacts "plan"
$codeDir = Join-Path $artifacts "code"
$cfeDir = Join-Path $artifacts "cfe"
$logsDir = Join-Path $runDir "logs"
New-Item -ItemType Directory -Force -Path $briefDir, $planDir, $codeDir, $cfeDir, $logsDir | Out-Null

if (-not $AgentBin) {
    $release = Join-Path $RepoRoot "target\release\agent_Kuibyshev.exe"
    $debugBin = Join-Path $RepoRoot "target\debug\agent_Kuibyshev.exe"
    if (Test-Path -LiteralPath $release) {
        $AgentBin = $release
    } elseif (Test-Path -LiteralPath $debugBin) {
        $AgentBin = $debugBin
    } else {
        $AgentBin = "cargo"
    }
}

$agentProfiles = @{
    "1" = @{ Id = "1c-intake"; Settings = "test-agents/1c-intake"; Config = "test-agents/1c-intake/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage1.intake.md" }
    "2" = @{ Id = "1c-analyst"; Settings = "test-agents/1c-analyst"; Config = "test-agents/1c-analyst/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage2.analysis.md" }
    "3" = @{ Id = "1c-coder"; Settings = "test-agents/1c-coder"; Config = "test-agents/1c-coder/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage3.coder.md" }
    "4" = @{ Id = "1c-implementer"; Settings = "test-agents/1c-implementer"; Config = "test-agents/1c-implementer/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage4.implement.md" }
}

$pathPrepareHome = (Join-Path $workflowRoot "adapters\k7\prepare-home.ps1")
$pathValidate = (Join-Path $workflowRoot "adapters\k7\validate.ps1")
$pathApplyOut = (Join-Path $workflowRoot "adapters\k7\apply-out.ps1")
if ($Product -ne "k7") {
    $altPrep = Join-Path $workflowRoot "adapters\$Product\prepare-home.ps1"
    if (Test-Path -LiteralPath $altPrep) { $pathPrepareHome = $altPrep }
    $altVal = Join-Path $workflowRoot "adapters\$Product\validate.ps1"
    if (Test-Path -LiteralPath $altVal) { $pathValidate = $altVal }
    $altApply = Join-Path $workflowRoot "adapters\$Product\apply-out.ps1"
    if (Test-Path -LiteralPath $altApply) { $pathApplyOut = $altApply }
}

$startStage = 1
if (-not [string]::IsNullOrWhiteSpace($FromStage)) { $startStage = [int]$FromStage }
elseif ($Stage -ne "all") { $startStage = [int]$Stage }
elseif ($skipIntake) { $startStage = 2 }

$endStage = 4
if ($Stage -ne "all") { $endStage = [int]$Stage }

$report = [ordered]@{
    runId          = $RunId
    product        = $Product
    issueKey       = $IssueKey
    taskFile       = $TaskFile
    intake_skipped = $skipIntake
    stages         = @()
    gate           = "none"
    started_at     = (Get-Date).ToString("o")
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

# Brief bootstrap when skipping intake
if ($skipIntake -and $startStage -le 2 -and $endStage -ge 2) {
    if ($ForceRerun -or -not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        Normalize-TaskFileBrief -SrcFile $TaskFile -DestDir $briefDir
        Write-Host "Normalized TaskFile -> $briefDir"
    }
}

function Should-RunStage {
    param([int]$N)
    if ($N -lt $startStage -or $N -gt $endStage) { return $false }
    if ($skipIntake -and $N -eq 1) { return $false }
    $marker = Join-Path $runDir ("stage{0}.done.json" -f $N)
    if ((Test-StageDone $marker) -and -not $ForceRerun) {
        Write-Host "Stage $N already done (use -Force to redo)"
        return $false
    }
    return $true
}

function Run-Stage {
    param([int]$N)

    $profile = $agentProfiles["$N"]
    $settingsDir = Resolve-RepoPath $RepoRoot $profile.Settings
    $configPath = if ($ConfigOverride) { $ConfigOverride } else { Resolve-RepoPath $RepoRoot $profile.Config }
    $promptTpl = Get-Content -LiteralPath (Resolve-RepoPath $RepoRoot $profile.Prompt) -Raw -Encoding UTF8
    $prompt = Expand-Template $promptTpl @{ ISSUE_KEY = $IssueKey; PRODUCT = $Product }

    $stageHome = Join-Path $runDir ("stage{0}\home" -f $N)
    New-Item -ItemType Directory -Force -Path $stageHome | Out-Null

    & $pathPrepareHome `
        -StageHome $stageHome `
        -ProductYamlPath $productPath `
        -Stage "$N" `
        -IssueKey $IssueKey `
        -BriefDir $briefDir `
        -PlanDir $planDir `
        -CodeDir $codeDir | Out-Null

    if ($N -eq 2 -and $RequireSearx) {
        if (-not (Test-HttpReachable $searxUrl)) {
            throw "SearXNG MCP not reachable at $searxUrl (-RequireSearx)"
        }
    } elseif ($N -eq 2) {
        if (-not (Test-HttpReachable $searxUrl)) {
            Write-Warning "SearXNG MCP not reachable at $searxUrl; continuing without hard-fail"
        }
    }

    $files = @()
    switch ($N) {
        1 { }
        2 {
            $files += (Join-Path $briefDir "task_brief.md")
            $files += (Join-Path $stageHome "in\product.json")
        }
        3 {
            $files += (Join-Path $planDir "tasks.md")
            $files += (Join-Path $planDir "architecture.md")
            $files += (Join-Path $planDir "cfe-scope.md")
        }
        4 {
            $files += (Join-Path $planDir "cfe-scope.md")
            $files += (Join-Path $codeDir "files-index.md")
            $files += (Join-Path $codeDir "code-report.md")
        }
    }

    $stdoutPath = Join-Path $logsDir ("stage{0}.stdout.json" -f $N)
    $stderrPath = Join-Path $logsDir ("stage{0}.stderr.txt" -f $N)

    if ($AgentBin -eq "cargo") {
        $promptFile = Join-Path $stageHome "in\stage_prompt.md"
        New-Item -ItemType Directory -Force -Path (Join-Path $stageHome "in") | Out-Null
        Set-Content -LiteralPath $promptFile -Value $prompt -Encoding UTF8
        $shortPrompt = "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn."
        $allFiles = @($promptFile) + @($files | Where-Object { $_ })
        $argList = @(
            "run", "--bin", "agent_Kuibyshev", "--",
            "run",
            "--config", $configPath,
            "--settings-dir", $settingsDir,
            "--prompt", $shortPrompt,
            "--home", $stageHome
        )
        foreach ($f in $allFiles) {
            if ($f -and (Test-Path -LiteralPath $f -PathType Leaf)) {
                $argList += @("--files", $f)
            }
        }
        Write-Host ">> cargo run --bin agent_Kuibyshev -- run ..."
        Push-Location $RepoRoot
        $oldEap = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            $stdoutLines = & cargo @argList 2> $stderrPath
            $stdout = if ($null -eq $stdoutLines) { "" } elseif ($stdoutLines -is [array]) { ($stdoutLines | ForEach-Object { "$_" }) -join "`n" } else { [string]$stdoutLines }
        } finally {
            $ErrorActionPreference = $oldEap
            Pop-Location
        }
        Set-Content -LiteralPath $stdoutPath -Value $stdout -Encoding UTF8
        $stopReason = "error"
        try {
            $parsed = ($stdout.Trim() | ConvertFrom-Json)
            if ($parsed.stop_reason) { $stopReason = [string]$parsed.stop_reason }
        } catch { }
        $runResult = [pscustomobject]@{ ExitCode = $LASTEXITCODE; StopReason = $stopReason; Result = ""; StdoutPath = $stdoutPath }
    } else {
        $runResult = Invoke-AgentRun -Bin $AgentBin -Config $configPath -SettingsDir $settingsDir `
            -Prompt $prompt -HomeDir $stageHome -Files $files -StdoutPath $stdoutPath -StderrPath $stderrPath
    }

    $outDir = Join-Path $stageHome "out"
    $valArgs = @{ Stage = "$N"; OutDir = $outDir }
    if ($RequireTz -and $N -eq 1) { $valArgs.RequireTz = $true }
    & $pathValidate @valArgs

    if ($runResult.StopReason -ne "goal_reached") {
        throw "Stage $N stop_reason=$($runResult.StopReason) (expected goal_reached). See $stdoutPath"
    }

    switch ($N) {
        1 { Copy-DirContents $outDir $briefDir }
        2 { Copy-DirContents $outDir $planDir }
        3 { Copy-DirContents $outDir $codeDir }
        4 { Copy-DirContents $outDir $cfeDir }
    }

    Write-StageMarker -Path (Join-Path $runDir ("stage{0}.done.json" -f $N)) -StopReason $runResult.StopReason
    $script:report.stages += [ordered]@{
        stage       = $N
        agent       = $profile.Id
        stop_reason = $runResult.StopReason
        home        = $stageHome
    }
}

# Stage 1
if (Should-RunStage 1) {
    Run-Stage 1
    if ($RequireTz) {
        & $pathValidate -Stage "1" -OutDir $briefDir -RequireTz
    }
}

# Stage 2
if (Should-RunStage 2) {
    if (-not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        throw "Missing brief at $briefDir (run stage 1 or pass -TaskFile)"
    }
    if ($RequireTz -and $skipIntake) {
        & $pathValidate -Stage "1" -OutDir $briefDir -RequireTz
    }
    Run-Stage 2

    $wf = @"
# Состояние конвейера задачи

| Поле | Значение |
|------|----------|
| режим_conveyor | strict |
| фаза_следующей_работы | 3 |
| ожидается_gate | approve_plan |
| примечание | Plan ready; await -ApprovePlan |
"@
    Set-Content -LiteralPath (Join-Path $planDir "workflow-state.md") -Value $wf -Encoding UTF8
}

# Gate before coder
$willRunCoder = ($startStage -le 3 -and $endStage -ge 3)
if ($willRunCoder) {
    $approvedPath = Join-Path $planDir "APPROVED"
    if ($ApprovePlan) {
        Set-Content -LiteralPath $approvedPath -Value ("approved_at={0}" -f (Get-Date).ToString("o")) -Encoding UTF8
        $report.gate = "approved_via_flag"
    } elseif (Test-Path -LiteralPath $approvedPath) {
        $report.gate = "approved_file"
    } else {
        $report.gate = "waiting"
        $report.finished_at = (Get-Date).ToString("o")
        ($report | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath (Join-Path $runDir "report.json") -Encoding UTF8
        Write-Host ""
        Write-Host "GATE: plan awaiting approval."
        Write-Host "Review: $planDir"
        Write-Host "Then re-run with: -RunId $RunId -FromStage 3 -ApprovePlan"
        exit 2
    }
}

if (Should-RunStage 3) {
    foreach ($req in @("tasks.md", "architecture.md", "cfe-scope.md")) {
        if (-not (Test-Path -LiteralPath (Join-Path $planDir $req))) {
            throw "Missing plan artifact $req in $planDir"
        }
    }
    Run-Stage 3
}

if (Should-RunStage 4) {
    Run-Stage 4
    if ($BuildCfe -or $DoApplyOut) {
        if ([string]::IsNullOrWhiteSpace($IssueKey)) {
            throw "-BuildCfe/-ApplyOut requires -IssueKey for task directory naming"
        }
        $applyArgs = @{
            CfeOutDir        = $cfeDir
            ProductYamlPath  = $productPath
            IssueKey         = $IssueKey
        }
        if ($BuildCfe) { $applyArgs.BuildCfe = $true }
        & $pathApplyOut @applyArgs
    } else {
        Write-Host "Stage 4 artifacts in $cfeDir (pass -ApplyOut and/or -BuildCfe to copy into product task dir)"
    }
}

$report.finished_at = (Get-Date).ToString("o")
($report | ConvertTo-Json -Depth 6) | Set-Content -LiteralPath (Join-Path $runDir "report.json") -Encoding UTF8
Write-Host "Done. report: $(Join-Path $runDir 'report.json')"
exit 0

