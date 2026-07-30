param(
    [string]$ConfigPath = ".\agent-config.local-demo.yaml",
    [string]$SettingsDir = ".\settings",
    [string]$Prompt = @"
Summarize the attached README in 5-8 bullet points.

Required steps:
1. First response: done=false, one home.write call creating out/summary.md
2. Second response: done=false, one home.write call creating out/manifest.json
3. Third response: done=true with a short result

Return JSON only on every turn.
"@,
    [string]$HomePath = ".\demo-home",
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

    Write-Host "Running agent_Kuibyshev with home: $HomePath"
    cargo run --bin agent_Kuibyshev -- run `
        --config $ConfigPath `
        --settings-dir $SettingsDir `
        --prompt $Prompt `
        --home $HomePath `
        --files $Files
} catch {
    Write-Error $_
    exit 1
}
