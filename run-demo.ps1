param(
    [string]$ProjectRoot = ".",
    [string]$AgentId = "demo",
    [string]$ImportFrom = ".\settings",
    [string]$ConfigTemplate = ".\agent-config.local-demo.yaml",
    [string]$Prompt = @"
Summarize the attached README in 5-8 bullet points.

Required steps:
1. First response: done=false, one home.write call creating out/summary.md
2. Second response: done=false, one home.write call creating out/manifest.json
3. Third response: done=true with a short result

Return JSON only on every turn.
"@,
    [string[]]$Files = @(".\README.md")
)

$ErrorActionPreference = "Stop"

function Assert-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' is not available in PATH."
    }
}

try {
    Assert-Command "cargo"

    if ([string]::IsNullOrWhiteSpace($env:OPENAI_API_KEY)) {
        throw "OPENAI_API_KEY is not set. Example: `$env:OPENAI_API_KEY = 'your_api_key'"
    }

    $ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
    Write-Host "Ensuring protected agent profile `$AgentId under $ProjectRoot"

    cargo run --bin kbshff -- init $AgentId --project-root $ProjectRoot --force
    if ($LASTEXITCODE -ne 0) { throw "init failed" }

    if (Test-Path -LiteralPath $ImportFrom) {
        cargo run --bin kbshff -- config --project-root $ProjectRoot --agent $AgentId import --from $ImportFrom --force
        if ($LASTEXITCODE -ne 0) { throw "config import failed" }
    }

    if (Test-Path -LiteralPath $ConfigTemplate -PathType Leaf) {
        cargo run --bin kbshff -- config --project-root $ProjectRoot --agent $AgentId import --from $ConfigTemplate --force
        if ($LASTEXITCODE -ne 0) { throw "config template import failed" }
    }

    Write-Host "Running kbshff agent=$AgentId"
    cargo run --bin kbshff -- run `
        --project-root $ProjectRoot `
        --agent $AgentId `
        --prompt $Prompt `
        --files $Files
} catch {
    Write-Error $_
    exit 1
}
