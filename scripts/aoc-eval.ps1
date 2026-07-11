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

$taskFiles = Get-ChildItem -LiteralPath $BankDir -Filter "*.json" | Sort-Object Name
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
Write-Host "config=$Config settings=$SettingsDir"

Push-Location $RepoRoot
try {
    foreach ($task in $tasks) {
        $homeDir = Join-Path $runsRoot $task.Id
        New-Item -ItemType Directory -Force -Path $homeDir | Out-Null

        $prompt = @"
Solve AoC task $($task.Id).

Required steps:
1. Fetch the task statement with aoc_get_task and the input with aoc_get_input.
2. Write a Python solution under home with home.write.
3. Run it with home.run (program=python). Debug using stdout/stderr until correct.
4. Final response: done=true with result equal to only the final answer string.

Return JSON only on every turn.
"@

        Write-Host ""
        Write-Host "=== $($task.Id) ==="

        $stdoutPath = Join-Path $homeDir "agent.stdout.json"
        $stderrPath = Join-Path $homeDir "agent.stderr.txt"

        $cargoArgs = @(
            "run", "--release", "--",
            "--config", $Config,
            "--settings-dir", $SettingsDir,
            "--prompt", $prompt,
            "--home", $homeDir
        )

        $entry = [ordered]@{
            id           = $task.Id
            expected     = $task.Expected
            pass         = $false
            stop_reason  = $null
            result       = $null
            usage        = $null
            error        = $null
            home         = $homeDir
            elapsed_ms   = $null
        }

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        try {
            $p = Start-Process -FilePath "cargo" `
                -ArgumentList $cargoArgs `
                -WorkingDirectory $RepoRoot `
                -NoNewWindow `
                -Wait `
                -PassThru `
                -RedirectStandardOutput $stdoutPath `
                -RedirectStandardError $stderrPath

            $sw.Stop()
            $entry.elapsed_ms = $sw.ElapsedMilliseconds

            if ($p.ExitCode -ne 0) {
                $errText = ""
                if (Test-Path -LiteralPath $stderrPath) {
                    $errText = (Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue)
                }
                $entry.error = "cargo exited with code $($p.ExitCode): $errText"
                $failed += 1
                Write-Host "FAIL $($task.Id): $($entry.error)"
                $results += [pscustomobject]$entry
                continue
            }

            $stdout = Get-Content -LiteralPath $stdoutPath -Raw -Encoding UTF8
            # Agent prints a single JSON document; tolerate leading/trailing whitespace.
            $jsonText = $stdout.Trim()
            $output = $jsonText | ConvertFrom-Json

            $actual = if ($null -eq $output.result) { "" } else { ([string]$output.result).Trim() }
            $stopReason = [string]$output.stop_reason
            $entry.result = $actual
            $entry.stop_reason = $stopReason
            $entry.usage = $output.usage

            if ($stopReason -eq "goal_reached" -and $actual -eq $task.Expected) {
                $entry.pass = $true
                $passed += 1
                Write-Host "PASS $($task.Id) result=$actual"
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
