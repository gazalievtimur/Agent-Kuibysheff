#Requires -Version 5.1
<#
.SYNOPSIS
  Run Referent (or another settings profile) against local AoC bank tasks and
  compare RunOutput.result to expected answers.

.DESCRIPTION
  Task bank and run artifacts stay outside git (local/aoc-bank, local/aoc-runs).
  This script is the eval harness — it is not cargo test / CI.

.PARAMETER TaskId
  Optional task id filter. Repeat or omit to run all bank tasks.

.PARAMETER BankDir
  Path to JSON task bank. Default: ./local/aoc-bank

.PARAMETER Config
  Agent runtime config. Default: ./test-agents/referent/agent-config.aoc.example.yaml

.PARAMETER SettingsDir
  Agent settings directory. Default: ./test-agents/referent

.PARAMETER RepoRoot
  Repository root (for cargo run and relative MCP paths). Default: parent of scripts/
#>
param(
    [string[]]$TaskId = @(),
    [string]$BankDir = "",
    [string]$Config = "",
    [string]$SettingsDir = "",
    [string]$RepoRoot = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
    param([string]$Root, [string]$Relative)
    return [System.IO.Path]::GetFullPath((Join-Path $Root $Relative))
}

if (-not $RepoRoot) {
    $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
} else {
    $RepoRoot = Resolve-Path $RepoRoot
}

. (Join-Path $PSScriptRoot "import-dotenv.ps1")
Import-DotEnv (Join-Path $RepoRoot ".env")

if (-not $BankDir) {
    $BankDir = Resolve-RepoPath $RepoRoot "local/aoc-bank"
} else {
    $BankDir = [System.IO.Path]::GetFullPath($BankDir)
}

if (-not $Config) {
    $Config = Resolve-RepoPath $RepoRoot "test-agents/referent/agent-config.aoc.example.yaml"
} else {
    $Config = [System.IO.Path]::GetFullPath($Config)
}

if (-not $SettingsDir) {
    $SettingsDir = Resolve-RepoPath $RepoRoot "test-agents/referent"
} else {
    $SettingsDir = [System.IO.Path]::GetFullPath($SettingsDir)
}

if (-not (Test-Path -LiteralPath $BankDir -PathType Container)) {
    throw "AoC bank not found: $BankDir`nCopy local/aoc-bank.example to local/aoc-bank and fill tasks."
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "Config not found: $Config"
}

if (-not (Test-Path -LiteralPath $SettingsDir -PathType Container)) {
    throw "Settings dir not found: $SettingsDir"
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

$baseConfigText = Get-Content -LiteralPath $Config -Raw -Encoding UTF8
$providerBaseUrl = Get-YamlScalar $baseConfigText "base_url" "https://polza.ai/api/v1"
$providerModel = Get-YamlScalar $baseConfigText "model" "deepseek/deepseek-v4-flash"
$providerApiKeyEnv = Get-YamlScalar $baseConfigText "api_key_env" "POLZA_API_KEY"
$providerApiKey = Get-YamlProviderApiKey $baseConfigText
$providerTimeoutMs = Get-YamlScalar $baseConfigText "timeout_ms" "180000"
$maxIterations = Get-YamlScalar $baseConfigText "max_iterations" "40"
$maxTokens = Get-YamlScalar $baseConfigText "max_tokens" "500000"
$maxDurationSec = Get-YamlScalar $baseConfigText "max_duration_sec" "900"

function Resolve-PythonForSandbox {
    $candidates = @()
    foreach ($name in @("python", "python3")) {
        $cmd = Get-Command $name -ErrorAction SilentlyContinue
        if ($null -ne $cmd -and $cmd.Source) {
            $candidates += [string]$cmd.Source
        }
    }
    try {
        $pyOut = & py -3 -c "import sys; print(sys.executable)" 2>$null
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($pyOut)) {
            $candidates += ([string]$pyOut).Trim()
        }
    } catch {
        # py launcher optional
    }

    foreach ($candidate in $candidates) {
        if ([string]::IsNullOrWhiteSpace($candidate)) { continue }
        if ($candidate -match '(?i)\\WindowsApps\\') { continue }
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    throw "Could not resolve a real python.exe for sandboxed home.run (avoid WindowsApps stub)."
}

function Ensure-AoCPythonRuntime {
    param(
        [Parameter(Mandatory = $true)][string]$RepoRoot,
        [Parameter(Mandatory = $true)][string]$SourceExe
    )

    # Host installs like C:\Python312 often reject AppContainer ACL inheritance
    # (win32=87). Stage a user-writable copy under local/ that we can grant.
    $destRoot = Join-Path $RepoRoot "local\aoc-sandbox-runtime\python"
    $marker = Join-Path $destRoot ".ak-source"
    $srcRoot = Split-Path -Parent $SourceExe
    $destExe = Join-Path $destRoot "python.exe"
    $needSync = $true
    if ((Test-Path -LiteralPath $destExe -PathType Leaf) -and (Test-Path -LiteralPath $marker -PathType Leaf)) {
        $prev = (Get-Content -LiteralPath $marker -Raw -Encoding UTF8).Trim()
        if ($prev -eq $srcRoot) {
            $needSync = $false
        }
    }
    if ($needSync) {
        New-Item -ItemType Directory -Force -Path $destRoot | Out-Null
        Write-Host "Staging sandboxed Python runtime from $srcRoot -> $destRoot"
        & robocopy $srcRoot $destRoot /E `
            /XD Doc Docs tcl tk include Test tests Tools `
            /XF *.pdb *.htm *.chm `
            /NFL /NDL /NJH /NJS /nc /ns /np | Out-Null
        if ($LASTEXITCODE -ge 8) {
            throw "robocopy python runtime failed with code $LASTEXITCODE"
        }
        Set-Content -LiteralPath $marker -Value $srcRoot -Encoding UTF8
    }
    if (-not (Test-Path -LiteralPath $destExe -PathType Leaf)) {
        throw "Staged python runtime missing executable: $destExe"
    }
    return (Resolve-Path -LiteralPath $destRoot).Path
}

$hostPythonExe = Resolve-PythonForSandbox
$pythonRoot = Ensure-AoCPythonRuntime -RepoRoot $RepoRoot -SourceExe $hostPythonExe
$pythonExe = Join-Path $pythonRoot "python.exe"
$pythonExePosix = ($pythonExe -replace '\\', '/')
$pythonRootPosix = ($pythonRoot -replace '\\', '/')
$isWindowsHost = $env:OS -eq "Windows_NT"
if ($isWindowsHost) {
    $pythonInheritEnvYaml = '["SYSTEMROOT", "SystemRoot"]'
} else {
    # On Linux, use the host interpreter directly (namespace mounts handle roots).
    $pythonExe = $hostPythonExe
    $pythonRoot = Split-Path -Parent $hostPythonExe
    $pythonExePosix = ($pythonExe -replace '\\', '/')
    $pythonRootPosix = ($pythonRoot -replace '\\', '/')
    $pythonInheritEnvYaml = "[]"
}

Write-Host "sandbox python=$pythonExe root=$pythonRoot"

$taskFiles = @(Get-ChildItem -LiteralPath $BankDir -Filter "*.json" | Sort-Object Name)
if ($taskFiles.Count -eq 0) {
    throw "No JSON tasks in $BankDir"
}

$tasks = @()
foreach ($file in $taskFiles) {
    $raw = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8
    $obj = $raw | ConvertFrom-Json
    if (-not $obj.id) {
        throw "Task file missing id: $($file.FullName)"
    }
    if ($null -eq $obj.expected) {
        throw "Task $($obj.id) missing expected"
    }
    if ($TaskId.Count -gt 0 -and ($TaskId -notcontains [string]$obj.id)) {
        continue
    }
    $tasks += [pscustomobject]@{
        Id       = [string]$obj.id
        Expected = ([string]$obj.expected).Trim()
        Path     = $file.FullName
    }
}

if ($tasks.Count -eq 0) {
    throw "No tasks matched the requested TaskId filter."
}

$runId = Get-Date -Format "yyyyMMdd-HHmmss"
$runsRoot = Resolve-RepoPath $RepoRoot "local/aoc-runs/$runId"
New-Item -ItemType Directory -Force -Path $runsRoot | Out-Null

$env:AOC_BANK_DIR = $BankDir

$results = @()
$passed = 0
$failed = 0

Write-Host "AoC eval run=$runId bank=$BankDir tasks=$($tasks.Count)"
Write-Host "config=$Config settings=$SettingsDir model=$providerModel"

Push-Location $RepoRoot
try {
    foreach ($task in $tasks) {
        $homeDir = Join-Path $runsRoot $task.Id
        New-Item -ItemType Directory -Force -Path $homeDir | Out-Null
        New-Item -ItemType Directory -Force -Path (Join-Path $homeDir "in") | Out-Null
        New-Item -ItemType Directory -Force -Path (Join-Path $homeDir "out") | Out-Null
        $logDir = Join-Path $homeDir "logs"
        New-Item -ItemType Directory -Force -Path $logDir | Out-Null

        # Point AoC MCP at this run home so aoc_get_input materializes input.txt
        # instead of inlining the full puzzle payload into the model context.
        $env:AOC_HOME_DIR = $homeDir
        $env:AOC_BANK_DIR = $BankDir

        # Explicit env in a per-run config (MCP subprocess may not see parent env
        # reliably across shells); paths use forward slashes for YAML safety.
        $bankPosix = ($BankDir -replace '\\', '/')
        $homePosix = ($homeDir -replace '\\', '/')
        $logDirPosix = ($logDir -replace '\\', '/')
        $runConfigPath = Join-Path $homeDir "agent-config.yaml"
        $providerApiKeyLine = ""
        if (-not [string]::IsNullOrWhiteSpace($providerApiKey)) {
            $escapedApiKey = $providerApiKey.Replace('"', '\"')
            $providerApiKeyLine = "  api_key: `"$escapedApiKey`"`n"
        }
        $runConfigBody = @"
provider:
  base_url: "$providerBaseUrl"
  model: "$providerModel"
$providerApiKeyLine  api_key_env: "$providerApiKeyEnv"
  timeout_ms: $providerTimeoutMs
  max_retries: 3
  retry_base_delay_ms: 500

mcp:
  - name: "aoc"
    command: "node"
    args:
      - "./mcp-aoc-tasks.js"
      - "--bank-dir=$bankPosix"
      - "--home-dir=$homePosix"
    env:
      AOC_BANK_DIR: "$bankPosix"
      AOC_HOME_DIR: "$homePosix"
    timeout_ms: 30000

limits:
  max_iterations: $maxIterations
  max_tokens: $maxTokens
  max_duration_sec: $maxDurationSec

logging:
  enable_ai_log: true
  enable_mcp_log: true
  enable_chat_history: true
  output_dir: "$logDirPosix"

# Fail-closed OS sandbox for home.run (AppContainer / Linux namespaces).
# Required since 0.2.0 (omitting access is an error). mode defaults to strict.
access:
  mode: strict
  tools:
    builtins:
      - home.list
      - home.read
      - home.write
      - home.run
  filesystem:
    home:
      # AoC solutions and input.txt live at home root; in/out kept for artifacts.
      read: [".", "in", "out"]
      write: [".", "out"]
  run:
    programs:
      - name: python
        executable: "$pythonExePosix"
        runtime_read_roots: ["$pythonRootPosix"]
        inherit_env: $pythonInheritEnvYaml
        allow_children: false
    max_args: 32
    max_arg_chars: 4096
    max_output_chars: 200000
    max_timeout_ms: 120000
"@
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText($runConfigPath, $runConfigBody, $utf8NoBom)

        # Also seed input.txt from the bank so the solver can run even if the
        # model skips aoc_get_input.
        $taskObj = (Get-Content -LiteralPath $task.Path -Raw -Encoding UTF8) | ConvertFrom-Json
        $seedInput = [string]$taskObj.input
        if (-not $seedInput.EndsWith("`n")) { $seedInput += "`n" }
        $utf8NoBom = New-Object System.Text.UTF8Encoding $false
        [System.IO.File]::WriteAllText((Join-Path $homeDir "input.txt"), $seedInput, $utf8NoBom)

        $prompt = "Solve AoC task $($task.Id). Work one turn at a time: each reply must be exactly one JSON object (never multiple JSON objects). Do not pre-emit future turns. Steps across turns: 1) Fetch statement with aoc_get_task and call aoc_get_input (writes/confirm home/input.txt; do not paste the full input into thoughts). input.txt is already present under home. 2) Write solution.py that reads input.txt, then home.run with program=python. Debug until stdout shows the correct answer. 3) Final response: done=true with result equal to only the final answer string. Do not guess. Return JSON only on every turn."

        Write-Host ""
        Write-Host "=== $($task.Id) ==="

        $stdoutPath = Join-Path $homeDir "agent.stdout.json"
        $stderrPath = Join-Path $homeDir "agent.stderr.txt"

        $agentExe = Join-Path $RepoRoot "target\release\agent_Kuibyshev.exe"
        $effectiveConfig = $runConfigPath

        $entry = [ordered]@{
            id           = $task.Id
            expected     = $task.Expected
            pass         = $false
            stop_reason  = $null
            result       = $null
            usage        = $null
            error        = $null
            home         = $homeDir
            log_dir      = $logDir
            logs         = $null
            elapsed_ms   = $null
        }

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        try {
            # Native stderr (tracing) must not become terminating errors under
            # $ErrorActionPreference=Stop.
            $prevEap = $ErrorActionPreference
            $ErrorActionPreference = "Continue"
            try {
                # Always prefer a freshly built release binary so sandbox/access
                # changes are exercised (do not silently reuse a stale exe).
                if (-not (Test-Path -LiteralPath $agentExe -PathType Leaf)) {
                    throw "Release binary missing: $agentExe (run cargo build --release first)"
                }
                $allOutput = & $agentExe `
                    run `
                    --config $effectiveConfig `
                    --settings-dir $SettingsDir `
                    --prompt $prompt `
                    --home $homeDir `
                    --save-chat-history `
                    2>&1
                $exitCode = $LASTEXITCODE
            } finally {
                $ErrorActionPreference = $prevEap
            }

            $sw.Stop()
            $entry.elapsed_ms = $sw.ElapsedMilliseconds

            $stdoutParts = @()
            $stderrParts = @()
            foreach ($item in @($allOutput)) {
                if ($item -is [System.Management.Automation.ErrorRecord]) {
                    $stderrParts += $item.ToString()
                } else {
                    $stdoutParts += [string]$item
                }
            }

            $stdoutText = ($stdoutParts -join "`n")
            $stderrText = ($stderrParts -join "`n")
            Set-Content -LiteralPath $stdoutPath -Value $stdoutText -Encoding UTF8
            Set-Content -LiteralPath $stderrPath -Value $stderrText -Encoding UTF8

            if ($exitCode -ne 0) {
                $entry.error = "agent exited with code ${exitCode}: $stderrText"
                $failed += 1
                Write-Host "FAIL $($task.Id): $($entry.error)"
                $results += [pscustomobject]$entry
                continue
            }

            $jsonText = $stdoutText.Trim()
            # Extract the first top-level JSON object (RunOutput), ignoring any noise.
            $start = $jsonText.IndexOf("{")
            if ($start -ge 0) {
                $depth = 0
                $end = -1
                for ($i = $start; $i -lt $jsonText.Length; $i++) {
                    $ch = $jsonText[$i]
                    if ($ch -eq "{") { $depth++ }
                    elseif ($ch -eq "}") {
                        $depth--
                        if ($depth -eq 0) {
                            $end = $i
                            break
                        }
                    }
                }
                if ($end -ge $start) {
                    $jsonText = $jsonText.Substring($start, $end - $start + 1)
                }
            }
            $output = $jsonText | ConvertFrom-Json

            $actual = if ($null -eq $output.result) { "" } else { ([string]$output.result).Trim() }
            $stopReason = [string]$output.stop_reason
            $entry.result = $actual
            $entry.stop_reason = $stopReason
            $entry.usage = $output.usage
            if ($null -ne $output.logs) {
                $entry.logs = $output.logs
            }

            if ($stopReason -eq "goal_reached" -and $actual -eq $task.Expected) {
                $entry.pass = $true
                $passed += 1
                Write-Host "PASS $($task.Id) result=$actual"
                Write-Host "Logs dir: $logDir"
                if ($null -ne $output.logs) {
                    if ($output.logs.system_log) { Write-Host "  system: $($output.logs.system_log)" }
                    if ($output.logs.ai_log) { Write-Host "  ai:     $($output.logs.ai_log)" }
                    if ($output.logs.mcp_log) { Write-Host "  mcp:    $($output.logs.mcp_log)" }
                    if ($output.logs.chat_log) { Write-Host "  chat:   $($output.logs.chat_log)" }
                }
            } else {
                $failed += 1
                Write-Host "FAIL $($task.Id) stop=$stopReason result='$actual' expected='$($task.Expected)'"
            }
        }
        catch {
            $sw.Stop()
            $entry.elapsed_ms = $sw.ElapsedMilliseconds
            $entry.error = $_.Exception.Message
            $failed += 1
            Write-Host "FAIL $($task.Id): $($entry.error)"
        }

        $results += [pscustomobject]$entry
    }
}
finally {
    Pop-Location
}

$report = [ordered]@{
    run_id     = $runId
    bank_dir   = $BankDir
    config     = $Config
    settings   = $SettingsDir
    passed     = $passed
    failed     = $failed
    total      = $results.Count
    tasks      = $results
}

$reportPath = Join-Path $runsRoot "report.json"
($report | ConvertTo-Json -Depth 8) | Set-Content -LiteralPath $reportPath -Encoding UTF8

Write-Host ""
Write-Host "Report: $reportPath"
Write-Host "Summary: passed=$passed failed=$failed total=$($results.Count)"

if ($failed -gt 0) {
    exit 1
}
exit 0
