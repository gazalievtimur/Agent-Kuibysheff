#Requires -Version 5.1
<#
.SYNOPSIS
  Run security / sandbox-escape LLM regression inside Docker from Windows.

.DESCRIPTION
  Builds kuibysheff-security-lab (or SECURITY_LAB_IMAGE), mounts the repo at /work,
  runs privileged for nested userns, and does NOT mount docker.sock.
  Does NOT set KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP.

.PARAMETER Config
  Forwarded as --config (must be under the repo).

.PARAMETER TaskId
  Optional task id filter(s).

.PARAMETER RequireLimits
  Forward --require-limits (also auto-enabled when config has max_cost).

.PARAMETER RequireCostLimit
  Forward --require-cost-limit (budget_status must be limit_reached).

.PARAMETER Image
  Lab image tag (default kuibysheff-security-lab).
#>
param(
    [string]$Config = "",
    [string[]]$TaskId = @(),
    [switch]$RequireLimits,
    [switch]$RequireCostLimit,
    [string]$Image = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

. (Join-Path $PSScriptRoot "import-dotenv.ps1")
Import-DotEnv (Join-Path $RepoRoot ".env")

$WorkflowDir = Join-Path $RepoRoot "workflows\security-sandbox"
if (-not (Test-Path -LiteralPath $WorkflowDir -PathType Container)) {
    throw @"
workflows/security-sandbox not found (gitignored copy-unit).

Restore from git history for local testing, for example:
  git checkout HEAD~1 -- workflows
  # or: git checkout <commit-before-untrack> -- workflows
"@
}

if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw "docker CLI is required to launch the security lab container."
}

if (-not $Image) {
    if ($env:SECURITY_LAB_IMAGE) {
        $Image = $env:SECURITY_LAB_IMAGE
    } else {
        $Image = "kuibysheff-security-lab"
    }
}

if (-not $Config) {
    $localConfig = Join-Path $RepoRoot "agent-config.local.yaml"
    $exampleConfig = Join-Path $RepoRoot "test-agents\security-probe\agent-config.example.yaml"
    if (Test-Path -LiteralPath $localConfig -PathType Leaf) {
        $Config = $localConfig
    } else {
        $Config = $exampleConfig
    }
}

if (-not (Test-Path -LiteralPath $Config -PathType Leaf)) {
    throw "Security regression config not found: $Config"
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
    throw "Security Linux-docker regression requires $apiKeyEnv in the environment or .env."
}

$baseUrl = "https://api.openai.com/v1"
$bum = [regex]::Match($configText, '(?m)^\s*base_url:\s*"([^"]+)"')
if (-not $bum.Success) {
    $bum = [regex]::Match($configText, "(?m)^\s*base_url:\s*'([^']+)'")
}
if (-not $bum.Success) {
    $bum = [regex]::Match($configText, '(?m)^\s*base_url:\s*([^#\r\n]+)')
}
if ($bum.Success) {
    $baseUrl = $bum.Groups[1].Value.Trim()
}
try {
    $providerHost = ([Uri]$baseUrl).Host
} catch {
    $providerHost = "api.openai.com"
}

$mount = ($RepoRoot.Path -replace '\\', '/')
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

$bankDir = Join-Path $RepoRoot "local\security-bank"
if (-not (Test-Path -LiteralPath $bankDir -PathType Container)) {
    Write-Host "Creating local/security-bank from example..."
    Copy-Item -Recurse (Join-Path $RepoRoot "local\security-bank.example") $bankDir
}

$forwardArgs = @("--config", $configInContainer)
if ($RequireLimits) {
    $forwardArgs += "--require-limits"
}
if ($RequireCostLimit) {
    $forwardArgs += "--require-cost-limit"
}
foreach ($tid in $TaskId) {
    if ($tid) {
        $forwardArgs += @("--task-id", $tid)
    }
}

Write-Host "Security Linux-docker regression via $Image"
Write-Host "  mount=$mount -> /work"
Write-Host "  config=$configInContainer"
Write-Host "  privileged userns lab; NO docker.sock; NO KUIBYSHEFF_ALLOW_UNSANDBOXED_MCP"

Write-Host "Building image..."
docker build -f (Join-Path $RepoRoot "workflows\security-sandbox\Dockerfile") -t $Image $mount
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$dockerArgs = @(
    "run", "--rm", "--privileged",
    "-v", "${mount}:/work",
    "-v", "kuibysheff-security-cargo:/tmp/kuibysheff-security-target",
    "-e", "${apiKeyEnv}=$apiKeyValue",
    "-e", "PROVIDER_EGRESS_HOST=$providerHost",
    "-e", "SECURITY_IN_DOCKER=1",
    "-e", "CARGO_TARGET_DIR=/tmp/kuibysheff-security-target",
    "-w", "/work",
    $Image,
    "bash", "/work/workflows/security-sandbox/entrypoint.sh"
) + $forwardArgs

& docker @dockerArgs
exit $LASTEXITCODE
