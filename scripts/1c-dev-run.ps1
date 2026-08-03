#Requires -Version 5.1
<#
.SYNOPSIS
  Оркестратор воркфлоу 1С на Agent Kuibyshev: intake -> analyst -> coder -> implementer.

.DESCRIPTION
  Последовательно запускает агентов, передаёт артефакты через home,
  опционально пропускает intake при -TaskFile, останавливается на gate
  перед кодером, при флагах копирует/собирает CFE через адаптер продукта.
#>
param(
    # Идентификатор продукта (файл products/<Product>.yaml), например demo
    [Parameter(Mandatory = $true)]
    [string] $Product,

    # Ключ задачи Jira (PROJ-123). Обязателен, если нет -TaskFile; нужен для apply/BuildCfe
    [string] $IssueKey = "",

    # Путь к файлу задачи оператора; если задан — этап intake пропускается
    [Alias("TaskFile")]
    [string] $TaskFilePath = "",

    # Какие этапы запускать: all | 1 | 2 | 3 | 4 (алиас -Stage)
    [ValidateSet("all", "1", "2", "3", "4")]
    [Alias("Stage")]
    [string] $WorkflowStage = "all",

    # С какого этапа продолжить (resume): 1..4; пусто = по -Stage / логике skip intake
    [string] $FromStage = "",

    # Путь к бинарнику agent_Kuibyshev, либо "cargo" для сборки на лету
    [string] $AgentBin = "",

    # Корень каталогов прогонов (по умолчанию: ProjectRoot/.kuibyshev/runs или workflows/1c-dev/runs)
    [string] $RunsRoot = "",

    # Папка продукта 1С (VS Code workspace). Когда задана — configs/homes под .kuibyshev/
    [string] $ProjectRoot = "",

    # Идентификатор прогона; если пусто — генерируется из IssueKey/TaskFile + timestamp
    [string] $RunId = "",

    # Корень репозитория Agent Kuibyshev; пусто = родитель каталога scripts/
    [string] $RepoRoot = "",

    # Общий YAML-конфиг агента вместо agent-config.yaml / example профиля этапа
    [string] $ConfigOverride = "",

    # Закрыть human gate: утвердить план и разрешить этап coder
    [switch] $ApprovePlan,

    # Требовать наличие ТЗ / секции requirements в brief (строго)
    [switch] $RequireTz,

    # Жёстко требовать доступность SearXNG MCP на этапе analyst
    [switch] $RequireSearx,

    # После implementer вызвать сборку CFE (BuildCfe) через адаптер продукта
    [switch] $BuildCfe,

    # Скопировать out/cfe в каталог задачи продукта (имя DoApplyOut — без коллизии с путями)
    [Alias("ApplyOut")]
    [switch] $DoApplyOut,

    # Перезапустить уже успешно завершённые этапы (алиас -Force)
    [Alias("Force")]
    [switch] $ForceRerun
)

# --- Совместимые имена параметров ---
# $TaskFile — абсолютный/исходный путь к файлу задачи (из -TaskFile / -TaskFilePath)
$TaskFile = $TaskFilePath
# $Stage — запрошенный диапазон этапов (из -Stage / -WorkflowStage)
$Stage = $WorkflowStage

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    param(
        # Корень, относительно которого строится путь
        [string]$Root,
        # Относительный путь внутри корня
        [string]$Relative
    )
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Relative))
}

function Get-YamlScalar {
    param(
        # Текст YAML-файла
        [string]$Text,
        # Имя ключа верхнего уровня / любое вхождение ключа
        [string]$Key,
        # Значение по умолчанию, если ключ не найден
        [string]$Default = ""
    )
    # $match — результат regex-поиска значения ключа
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
    param(
        # Текст шаблона с плейсхолдерами {{KEY}} или {KEY}
        [string]$Template,
        # Словарь подстановок (имя -> значение)
        [hashtable]$Vars
    )
    # $result — шаблон после последовательных замен
    $result = $Template
    foreach ($key in $Vars.Keys) {
        $result = $result.Replace("{{$key}}", [string]$Vars[$key])
        $result = $result.Replace("{$key}", [string]$Vars[$key])
    }
    return $result
}

function Copy-DirContents {
    param(
        # Исходный каталог
        [string]$Src,
        # Каталог назначения
        [string]$Dst
    )
    if (-not (Test-Path -LiteralPath $Src)) { return }
    New-Item -ItemType Directory -Force -Path $Dst | Out-Null
    Copy-Item -Path (Join-Path $Src "*") -Destination $Dst -Recurse -Force -ErrorAction SilentlyContinue
}

function Test-StageDone {
    param(
        # Путь к маркеру stageN.done.json
        [string]$MarkerPath
    )
    return (Test-Path -LiteralPath $MarkerPath -PathType Leaf)
}

function Write-StageMarker {
    param(
        # Куда записать маркер завершения этапа
        [string]$Path,
        # stop_reason из RunOutput агента
        [string]$StopReason
    )
    # $obj — содержимое маркера этапа
    $obj = [ordered]@{ stop_reason = $StopReason; finished_at = (Get-Date).ToString("o") }
    ($obj | ConvertTo-Json) | Set-Content -LiteralPath $Path -Encoding UTF8
}

function Invoke-AgentRun {
    param(
        # Исполняемый файл агента
        [string]$Bin,
        # Путь к runtime-конфигу (--config)
        [string]$Config,
        # Каталог settings профиля (--settings-dir)
        [string]$SettingsDir,
        # Полный текст промпта этапа
        [string]$Prompt,
        # Корень home агента (--home)
        [string]$HomeDir,
        # Доп. файлы для --files
        [string[]]$Files,
        # Куда сохранить stdout (RunOutput JSON)
        [string]$StdoutPath,
        # Куда сохранить stderr
        [string]$StderrPath
    )

    # Полный промпт пишем в файл (избегаем проблем кодировки/длины CLI)
    # $promptFile — путь к stage_prompt.md внутри home/in
    $promptFile = Join-Path $HomeDir "in\stage_prompt.md"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $promptFile) | Out-Null
    Set-Content -LiteralPath $promptFile -Value $Prompt -Encoding UTF8
    # $shortPrompt — короткий указатель для --prompt
    $shortPrompt = "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn."

    # $allFiles — prompt-файл + входные артефакты этапа
    $allFiles = @($promptFile) + @($Files | Where-Object { $_ })
    # $argList — аргументы командной строки агента
    $argList = @(
        "run",
        "--config", $Config,
        "--settings-dir", $SettingsDir,
        "--prompt", $shortPrompt,
        "--home", $HomeDir
    )
    if ($script:ProjectRootForAgent) {
        $argList += @("--project-root", $script:ProjectRootForAgent)
    }
    foreach ($f in $allFiles) {
        if ($f -and (Test-Path -LiteralPath $f -PathType Leaf)) {
            $argList += @("--files", $f)
        }
    }

    # $argPreview — строка для лога (без длинного промпта)
    $argPreview = ($argList | ForEach-Object {
            if ($_ -eq $shortPrompt) { "<short-prompt>" } else { $_ }
        }) -join " "
    Write-Host ">> $Bin $argPreview"

    # $stdout — собранный stdout процесса агента
    $stdout = ""
    # $oldEap — сохранённый ErrorActionPreference на время вызова
    $oldEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        # $stdoutLines — сырой вывод бинарника (строка или массив строк)
        $stdoutLines = & $Bin @argList 2> $StderrPath
        if ($null -eq $stdoutLines) { $stdoutLines = @() }
        if ($stdoutLines -is [array]) { $stdout = ($stdoutLines | ForEach-Object { "$_" }) -join "`n" }
        else { $stdout = [string]$stdoutLines }
    } finally {
        $ErrorActionPreference = $oldEap
    }
    # $exit — код выхода процесса агента
    $exit = $LASTEXITCODE
    Set-Content -LiteralPath $StdoutPath -Value $stdout -Encoding UTF8

    # $stopReason — причина остановки цикла агента (goal_reached / limit_reached / error)
    $stopReason = "error"
    # $resultText — поле result из RunOutput
    $resultText = ""
    try {
        # $jsonText — фрагмент stdout с JSON RunOutput
        $jsonText = $stdout.Trim()
        if ($jsonText -match '(?s)\{.*"stop_reason".*\}\s*$') {
            $jsonText = $Matches[0]
        }
        # $parsed — десериализованный RunOutput
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
    param(
        # URL MCP/HTTP для проверки доступности
        [string]$Url
    )
    try {
        # $resp — ответ пробного GET (тело не важно)
        $resp = Invoke-WebRequest -Uri $Url -Method Get -TimeoutSec 3 -UseBasicParsing -ErrorAction Stop
        return $true
    } catch {
        # MCP может отклонять GET; «connection refused» считаем недоступностью
        if ($_.Exception.Message -match "refus|unreachable|Unable to connect|timed out") {
            return $false
        }
        return $true
    }
}

# --- Инициализация путей репозитория и окружения ---

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

# $dotenv — путь к helper-скрипту загрузки .env
$dotenv = Join-Path $PSScriptRoot "import-dotenv.ps1"
if (Test-Path -LiteralPath $dotenv) {
    . $dotenv
    Import-DotEnv (Join-Path $RepoRoot ".env")
}

# $workflowRoot — корень пакета воркфлоу workflows/1c-dev
$workflowRoot = Resolve-RepoPath $RepoRoot "workflows/1c-dev"

# $ProjectRootForAgent — передаётся в agent_Kuibyshev --project-root
$script:ProjectRootForAgent = $null
if (-not [string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
    if (-not (Test-Path -LiteralPath $ProjectRoot -PathType Container)) {
        throw "ProjectRoot not found: $ProjectRoot"
    }
    $script:ProjectRootForAgent = $ProjectRoot
}

# $productPath — YAML адаптера: .kuibyshev/product.yaml или products/<Product>.yaml
$productPath = $null
if ($script:ProjectRootForAgent) {
    $inProject = Join-Path $script:ProjectRootForAgent ".kuibyshev\product.yaml"
    if (Test-Path -LiteralPath $inProject -PathType Leaf) {
        $productPath = $inProject
    }
}
if (-not $productPath) {
    $productPath = Resolve-RepoPath $RepoRoot "workflows/1c-dev/products/$Product.yaml"
}
if (-not (Test-Path -LiteralPath $productPath -PathType Leaf)) {
    throw "Product config not found: $productPath (scaffold .kuibyshev/product.yaml or copy products/*.yaml.example)"
}
# $productYaml — сырое содержимое product YAML
$productYaml = Get-Content -LiteralPath $productPath -Raw -Encoding UTF8
# $searxUrl — URL SearXNG MCP для этапа analyst
$searxUrl = Get-YamlScalar $productYaml "searxngUrl" "http://127.0.0.1:3000/mcp"

# $skipIntake — true, если brief берётся из файла, а не из Jira
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
    if ($script:ProjectRootForAgent) {
        $RunsRoot = Join-Path $script:ProjectRootForAgent ".kuibyshev\runs"
    } else {
        $RunsRoot = Join-Path $workflowRoot "runs"
    }
} else {
    $RunsRoot = [System.IO.Path]::GetFullPath($RunsRoot)
}

if (-not $RunId) {
    # $stamp — метка времени для уникального RunId
    $stamp = Get-Date -Format "yyyyMMdd_HHmmss"
    if ($IssueKey) {
        $RunId = "${IssueKey}_$stamp"
    } else {
        # $stem — имя файла задачи без расширения
        $stem = [System.IO.Path]::GetFileNameWithoutExtension($TaskFile)
        $RunId = "${stem}_$stamp"
    }
}

# $runDir — каталог текущего прогона runs/<RunId>
$runDir = Join-Path $RunsRoot $RunId
New-Item -ItemType Directory -Force -Path $runDir | Out-Null
# $artifacts — сводные артефакты всех этапов
$artifacts = Join-Path $runDir "artifacts"
# $briefDir — brief после intake / нормализации TaskFile
$briefDir = Join-Path $artifacts "brief"
# $planDir — план analyst (prd, tasks, cfe-scope, …)
$planDir = Join-Path $artifacts "plan"
# $codeDir — исходники coder (out/src, reports)
$codeDir = Join-Path $artifacts "code"
# $cfeDir — упакованное расширение implementer (out/cfe)
$cfeDir = Join-Path $artifacts "cfe"
# $logsDir — stdout/stderr прогонов агентов
$logsDir = Join-Path $runDir "logs"
New-Item -ItemType Directory -Force -Path $briefDir, $planDir, $codeDir, $cfeDir, $logsDir | Out-Null

if (-not $AgentBin) {
    # $release — release-сборка агента
    $release = Join-Path $RepoRoot "target\release\agent_Kuibyshev.exe"
    # $debugBin — debug-сборка агента (не $debug: конфликт с -Debug)
    $debugBin = Join-Path $RepoRoot "target\debug\agent_Kuibyshev.exe"
    if (Test-Path -LiteralPath $release) {
        $AgentBin = $release
    } elseif (Test-Path -LiteralPath $debugBin) {
        $AgentBin = $debugBin
    } else {
        $AgentBin = "cargo"
    }
}

# $useProjectAgents — профили из {ProjectRoot}/.kuibyshev/agents
$useProjectAgents = $false
if ($script:ProjectRootForAgent) {
    $probe = Join-Path $script:ProjectRootForAgent ".kuibyshev\agents\1c-analyst"
    if (Test-Path -LiteralPath $probe -PathType Container) {
        $useProjectAgents = $true
    }
}

# $agentProfiles — настройки каждого этапа: Id, Settings, Config, Prompt
if ($useProjectAgents) {
    $agentProfiles = @{
        "1" = @{ Id = "1c-intake"; Settings = ".kuibyshev/agents/1c-intake"; Config = ".kuibyshev/agents/1c-intake/agent-config.yaml"; Prompt = "workflows/1c-dev/prompts/stage1.intake.md"; UnderProject = $true }
        "2" = @{ Id = "1c-analyst"; Settings = ".kuibyshev/agents/1c-analyst"; Config = ".kuibyshev/agents/1c-analyst/agent-config.yaml"; Prompt = "workflows/1c-dev/prompts/stage2.analysis.md"; UnderProject = $true }
        "3" = @{ Id = "1c-coder"; Settings = ".kuibyshev/agents/1c-coder"; Config = ".kuibyshev/agents/1c-coder/agent-config.yaml"; Prompt = "workflows/1c-dev/prompts/stage3.coder.md"; UnderProject = $true }
        "4" = @{ Id = "1c-implementer"; Settings = ".kuibyshev/agents/1c-implementer"; Config = ".kuibyshev/agents/1c-implementer/agent-config.yaml"; Prompt = "workflows/1c-dev/prompts/stage4.implement.md"; UnderProject = $true }
    }
} else {
    $agentProfiles = @{
        "1" = @{ Id = "1c-intake"; Settings = "test-agents/1c-intake"; Config = "test-agents/1c-intake/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage1.intake.md"; UnderProject = $false }
        "2" = @{ Id = "1c-analyst"; Settings = "test-agents/1c-analyst"; Config = "test-agents/1c-analyst/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage2.analysis.md"; UnderProject = $false }
        "3" = @{ Id = "1c-coder"; Settings = "test-agents/1c-coder"; Config = "test-agents/1c-coder/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage3.coder.md"; UnderProject = $false }
        "4" = @{ Id = "1c-implementer"; Settings = "test-agents/1c-implementer"; Config = "test-agents/1c-implementer/agent-config.example.yaml"; Prompt = "workflows/1c-dev/prompts/stage4.implement.md"; UnderProject = $false }
    }
}

# Адаптеры: сначала adapters/<Product>/, иначе общий adapters/default/
$pathPrepareHome = Join-Path $workflowRoot "adapters\default\prepare-home.ps1"
$pathValidate = Join-Path $workflowRoot "adapters\default\validate.ps1"
$pathApplyOut = Join-Path $workflowRoot "adapters\default\apply-out.ps1"
$altPrep = Join-Path $workflowRoot "adapters\$Product\prepare-home.ps1"
if (Test-Path -LiteralPath $altPrep) { $pathPrepareHome = $altPrep }
$altVal = Join-Path $workflowRoot "adapters\$Product\validate.ps1"
if (Test-Path -LiteralPath $altVal) { $pathValidate = $altVal }
$altApply = Join-Path $workflowRoot "adapters\$Product\apply-out.ps1"
if (Test-Path -LiteralPath $altApply) { $pathApplyOut = $altApply }

# $startStage — первый этап к выполнению (1..4)
$startStage = 1
if (-not [string]::IsNullOrWhiteSpace($FromStage)) { $startStage = [int]$FromStage }
elseif ($Stage -ne "all") { $startStage = [int]$Stage }
elseif ($skipIntake) { $startStage = 2 }

# $endStage — последний этап к выполнению (1..4)
$endStage = 4
if ($Stage -ne "all") { $endStage = [int]$Stage }

# $report — итоговый JSON-отчёт прогона
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
    param(
        # Исходный файл задачи от оператора
        [string]$SrcFile,
        # Каталог artifacts/brief
        [string]$DestDir
    )
    New-Item -ItemType Directory -Force -Path $DestDir | Out-Null
    # $raw — содержимое файла задачи
    $raw = Get-Content -LiteralPath $SrcFile -Raw -Encoding UTF8
    # $destBrief — целевой task_brief.md
    $destBrief = Join-Path $DestDir "task_brief.md"
    # $looksLikeBrief — файл уже похож на brief по заголовкам
    $looksLikeBrief = $raw -match "(?im)^#\s*Task brief" -or $raw -match "(?im)^##\s*Requirements"
    if ($looksLikeBrief) {
        Copy-Item -LiteralPath $SrcFile -Destination $destBrief -Force
    } else {
        # $srcCopy — копия исходника как task_source.md
        $srcCopy = Join-Path $DestDir "task_source.md"
        Copy-Item -LiteralPath $SrcFile -Destination $srcCopy -Force
        # $title — заголовок brief из имени файла
        $title = [System.IO.Path]::GetFileNameWithoutExtension($SrcFile)
        # $wrapper — обёртка brief вокруг произвольного файла задачи
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

    # $sources — machine-readable список источников brief
    $sources = [ordered]@{
        origin         = "task_file"
        path           = ($SrcFile -replace '\\', '/')
        skipped_intake = $true
    }
    ($sources | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $DestDir "sources.json") -Encoding UTF8

    # $manifest — манифест этапа с apply_mode=none
    $manifest = [ordered]@{
        schema_version = 1
        summary        = "intake skipped: task file provided"
        files_written  = @("task_brief.md", "sources.json")
        patches        = @()
        apply_mode     = "none"
    }
    ($manifest | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $DestDir "manifest.json") -Encoding UTF8
}

# Нормализация TaskFile в artifacts/brief при пропуске intake
if ($skipIntake -and $startStage -le 2 -and $endStage -ge 2) {
    if ($ForceRerun -or -not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        Normalize-TaskFileBrief -SrcFile $TaskFile -DestDir $briefDir
        Write-Host "Normalized TaskFile -> $briefDir"
    }
}

function Should-RunStage {
    param(
        # Номер этапа 1..4
        [int]$N
    )
    if ($N -lt $startStage -or $N -gt $endStage) { return $false }
    if ($skipIntake -and $N -eq 1) { return $false }
    # $marker — файл stageN.done.json (этап уже успешен)
    $marker = Join-Path $runDir ("stage{0}.done.json" -f $N)
    if ((Test-StageDone $marker) -and -not $ForceRerun) {
        Write-Host "Stage $N already done (use -Force to redo)"
        return $false
    }
    return $true
}

function Run-Stage {
    param(
        # Номер этапа 1..4
        [int]$N
    )

    # $profile — запись из $agentProfiles для этапа N
    $profile = $agentProfiles["$N"]
    # $settingsDir — каталог master_prompt / skills / rules
    if ($profile.UnderProject) {
        $settingsDir = Resolve-RepoPath $script:ProjectRootForAgent $profile.Settings
    } else {
        $settingsDir = Resolve-RepoPath $RepoRoot $profile.Settings
    }
    # $configPath — runtime YAML: -ConfigOverride, иначе agent-config.yaml / local / example
    if ($ConfigOverride) {
        $configPath = $ConfigOverride
    } else {
        $projectConfig = Join-Path $settingsDir "agent-config.yaml"
        $localConfig = Join-Path $settingsDir "agent-config.local.yaml"
        if ($profile.UnderProject) {
            $exampleConfig = Resolve-RepoPath $script:ProjectRootForAgent $profile.Config
        } else {
            $exampleConfig = Resolve-RepoPath $RepoRoot $profile.Config
        }
        if (Test-Path -LiteralPath $projectConfig -PathType Leaf) {
            $configPath = $projectConfig
        } elseif (Test-Path -LiteralPath $localConfig -PathType Leaf) {
            $configPath = $localConfig
        } else {
            $configPath = $exampleConfig
        }
    }
    # $promptTpl — шаблон промпта этапа с плейсхолдерами
    $promptTpl = Get-Content -LiteralPath (Resolve-RepoPath $RepoRoot $profile.Prompt) -Raw -Encoding UTF8
    # $prompt — промпт после подстановки ISSUE_KEY / PRODUCT
    $prompt = Expand-Template $promptTpl @{ ISSUE_KEY = $IssueKey; PRODUCT = $Product }

    # $stageHome — изолированный home этапа stageN/home
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

    # $files — список путей для --files (контекст модели)
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

    # $stdoutPath / $stderrPath — логи вывода агента этапа
    $stdoutPath = Join-Path $logsDir ("stage{0}.stdout.json" -f $N)
    $stderrPath = Join-Path $logsDir ("stage{0}.stderr.txt" -f $N)

    # $runResult — итог вызова агента (ExitCode, StopReason, …)
    if ($AgentBin -eq "cargo") {
        # $promptFile — полный промпт на диске для cargo-пути
        $promptFile = Join-Path $stageHome "in\stage_prompt.md"
        New-Item -ItemType Directory -Force -Path (Join-Path $stageHome "in") | Out-Null
        Set-Content -LiteralPath $promptFile -Value $prompt -Encoding UTF8
        # $shortPrompt — короткий --prompt при запуске через cargo
        $shortPrompt = "Execute the stage instructions in the attached file stage_prompt.md (also under in/). Return JSON only on every turn."
        # $allFiles — файлы для --files
        $allFiles = @($promptFile) + @($files | Where-Object { $_ })
        # $argList — argv для `cargo run --bin agent_Kuibyshev -- run …`
        $argList = @(
            "run", "--bin", "agent_Kuibyshev", "--",
            "run",
            "--config", $configPath,
            "--settings-dir", $settingsDir,
            "--prompt", $shortPrompt,
            "--home", $stageHome
        )
        if ($script:ProjectRootForAgent) {
            $argList += @("--project-root", $script:ProjectRootForAgent)
        }
        foreach ($f in $allFiles) {
            if ($f -and (Test-Path -LiteralPath $f -PathType Leaf)) {
                $argList += @("--files", $f)
            }
        }
        Write-Host ">> cargo run --bin agent_Kuibyshev -- run ..."
        Push-Location $RepoRoot
        # $oldEap — сохранённый ErrorActionPreference
        $oldEap = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        try {
            # $stdoutLines / $stdout — вывод cargo/агента
            $stdoutLines = & cargo @argList 2> $stderrPath
            $stdout = if ($null -eq $stdoutLines) { "" } elseif ($stdoutLines -is [array]) { ($stdoutLines | ForEach-Object { "$_" }) -join "`n" } else { [string]$stdoutLines }
        } finally {
            $ErrorActionPreference = $oldEap
            Pop-Location
        }
        Set-Content -LiteralPath $stdoutPath -Value $stdout -Encoding UTF8
        # $stopReason — разобранный stop_reason или error
        $stopReason = "error"
        try {
            # $parsed — RunOutput JSON
            $parsed = ($stdout.Trim() | ConvertFrom-Json)
            if ($parsed.stop_reason) { $stopReason = [string]$parsed.stop_reason }
        } catch { }
        $runResult = [pscustomobject]@{ ExitCode = $LASTEXITCODE; StopReason = $stopReason; Result = ""; StdoutPath = $stdoutPath }
    } else {
        $runResult = Invoke-AgentRun -Bin $AgentBin -Config $configPath -SettingsDir $settingsDir `
            -Prompt $prompt -HomeDir $stageHome -Files $files -StdoutPath $stdoutPath -StderrPath $stderrPath
    }

    # $outDir — home/out текущего этапа
    $outDir = Join-Path $stageHome "out"
    # $valArgs — аргументы для validate.ps1
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

# --- Этап 1: intake ---
if (Should-RunStage 1) {
    Run-Stage 1
    if ($RequireTz) {
        & $pathValidate -Stage "1" -OutDir $briefDir -RequireTz
    }
}

# --- Этап 2: analyst ---
if (Should-RunStage 2) {
    if (-not (Test-Path -LiteralPath (Join-Path $briefDir "task_brief.md"))) {
        throw "Missing brief at $briefDir (run stage 1 or pass -TaskFile)"
    }
    if ($RequireTz -and $skipIntake) {
        & $pathValidate -Stage "1" -OutDir $briefDir -RequireTz
    }
    Run-Stage 2

    # $wf — текст workflow-state.md после плана
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

# --- Gate перед coder ---
# $willRunCoder — в запрошенном диапазоне есть этап 3 (нужно утверждение плана)
$willRunCoder = ($startStage -le 3 -and $endStage -ge 3)
if ($willRunCoder) {
    # $approvedPath — файл-маркер утверждения плана
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

# --- Этап 3: coder ---
if (Should-RunStage 3) {
    foreach ($req in @("tasks.md", "architecture.md", "cfe-scope.md")) {
        if (-not (Test-Path -LiteralPath (Join-Path $planDir $req))) {
            throw "Missing plan artifact $req in $planDir"
        }
    }
    Run-Stage 3
}

# --- Этап 4: implementer (+ optional apply) ---
if (Should-RunStage 4) {
    Run-Stage 4
    if ($BuildCfe -or $DoApplyOut) {
        if ([string]::IsNullOrWhiteSpace($IssueKey)) {
            throw "-BuildCfe/-ApplyOut requires -IssueKey for task directory naming"
        }
        # $applyArgs — параметры для apply-out.ps1
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
