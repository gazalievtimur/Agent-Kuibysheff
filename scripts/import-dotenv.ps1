Set-StrictMode -Version Latest

function Import-DotEnv {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return
    }

    Get-Content -LiteralPath $Path -Encoding UTF8 | ForEach-Object {
        $line = $_.Trim()
        if ($line -eq "" -or $line.StartsWith("#")) {
            return
        }

        $equalsIndex = $line.IndexOf("=")
        if ($equalsIndex -lt 1) {
            return
        }

        $name = $line.Substring(0, $equalsIndex).Trim()
        $value = $line.Substring($equalsIndex + 1).Trim()
        if ($value.StartsWith('"') -and $value.EndsWith('"') -and $value.Length -ge 2) {
            $value = $value.Substring(1, $value.Length - 2)
        } elseif ($value.StartsWith("'") -and $value.EndsWith("'") -and $value.Length -ge 2) {
            $value = $value.Substring(1, $value.Length - 2)
        }

        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, "Process"))) {
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }
}

function Get-YamlProviderApiKey {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ConfigText
    )

    foreach ($pattern in @(
            '(?m)^\s*api_key:\s*"([^"]*)"',
            "(?m)^\s*api_key:\s*'([^']*)'",
            '(?m)^\s*api_key:\s*([^#\r\n]+)'
        )) {
        $match = [regex]::Match($ConfigText, $pattern)
        if ($match.Success) {
            $value = $match.Groups[1].Value.Trim()
            if (-not [string]::IsNullOrWhiteSpace($value)) {
                return $value
            }
        }
    }

    return ""
}

function Test-ProviderApiKeyAvailable {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ConfigText
    )

    $apiKeyEnv = "OPENAI_API_KEY"
    foreach ($pattern in @(
            '(?m)^\s*api_key_env:\s*"([^"]+)"',
            "(?m)^\s*api_key_env:\s*'([^']+)'",
            '(?m)^\s*api_key_env:\s*([A-Za-z_][A-Za-z0-9_]*)'
        )) {
        $match = [regex]::Match($ConfigText, $pattern)
        if ($match.Success) {
            $apiKeyEnv = $match.Groups[1].Value.Trim()
            break
        }
    }

    foreach ($scope in @("Process", "User", "Machine")) {
        $value = [Environment]::GetEnvironmentVariable($apiKeyEnv, $scope)
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            return $true
        }
    }

    return $false
}
