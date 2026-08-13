#Requires -Version 5.1
<#
.SYNOPSIS
  Run SWE-bench Linux regression inside Docker from a Windows host.

.DESCRIPTION
  Mounts the repo + Docker socket into rust:1-bookworm and executes
  scripts/swebench-regression-linux-docker.sh. Use this when you need to
  verify the Linux ELF path without a native Linux/WSL toolchain.

  Native Windows regression stays: .\scripts\swebench-regression.ps1

.PARAMETER Config
  Forwarded to swebench-regression.sh --config when set.

.PARAMETER InstanceId
  Forwarded to swebench-regression.sh --instance-id (default sympy__sympy-20590).

.PARAMETER Image
  Runner image (default rust:1-bookworm).
#>
param(
    [string]$Config = "",
    [string]$InstanceId = "sympy__sympy-20590",
    [string]$Image = "rust:1-bookworm"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

. (Join-Path $PSScriptRoot "import-dotenv.ps1")
Import-DotEnv (Join-Path $RepoRoot ".env")

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw "docker CLI is required to launch the Linux SWE-bench runner container."
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\swebench-solver\agent-config.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "SWE-bench regression config not found: $Config"
}

$configText = Get-Content -LiteralPath $Config -Raw -Encoding UTF8
$apiKeyEnv = "OPENAI_API_KEY"
$match = [regex]::Match($configText, '(?m)^\s*api_key_env:\s*"([^"]+)"')
if (-not $match.Success) {
    $match = [regex]::Match($configText, "(?m)^\s*api_key_env:\s*'([^']+)'")
}
if (-not $match.Success) {
    $match = [regex]::Match($configText, '(?m)^\s*api_key_env:\s*([A-Za-z_][A-Za-z0-9_]*)')
}
if ($match.Success) {
    $apiKeyEnv = $match.Groups[1].Value.Trim()
}

$apiKeyValue = [Environment]::GetEnvironmentVariable($apiKeyEnv, "Process")
if ([string]::IsNullOrWhiteSpace($apiKeyValue)) {
    throw @"
SWE-bench Linux-docker regression requires $apiKeyEnv in the environment or .env.
"@
}

# Docker Desktop bind mounts need a forward-slash path; keep drive letter.
$mount = ($RepoRoot.Path -replace '\\', '/')
# Config path as seen inside the container (/work/...).
$configRel = $Config
if ([System.IO.Path]::IsPathRooted($Config)) {
    $fullConfig = (Resolve-Path -LiteralPath $Config).Path
    $repoPrefix = $RepoRoot.Path.TrimEnd('\') + '\'
    if ($fullConfig.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        $configRel = ($fullConfig.Substring($repoPrefix.Length) -replace '\\', '/')
    } else {
        throw "Config must live under the repo so it is visible at /work: $Config"
    }
} else {
    $configRel = ($Config -replace '\\', '/')
}
$configInContainer = "/work/$configRel"

$forwardArgs = @(
    "--config", $configInContainer,
    "--instance-id", $InstanceId
)

Write-Host "SWE-bench Linux-docker regression via $Image"
Write-Host "  mount=$mount -> /work"
Write-Host "  config=$configInContainer instance=$InstanceId"
Write-Host "  KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1 (nested Docker has no clone3)"

$dockerArgs = @(
    "run", "--rm",
    "-v", "${mount}:/work",
    "-v", "//var/run/docker.sock:/var/run/docker.sock",
    "-e", "${apiKeyEnv}=$apiKeyValue",
    "-e", "KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP=1",
    $Image,
    "bash", "/work/scripts/swebench-regression-linux-docker.sh"
) + $forwardArgs

& docker @dockerArgs
exit $LASTEXITCODE
