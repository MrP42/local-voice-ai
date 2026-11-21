# Bereitet die Authenticode-Signatur im Release-Lauf vor (release-windows.yml).
#
# Ohne LVA_SIGN_PROVIDER: nichts tun, Warnung, leere Ausgabe -> Release bleibt unsigniert
# (Stand bis 0.27.0). Mit Anbieter: Werkzeug installieren, eine Tauri-Konfiguration mit
# bundle.windows.signCommand nach $OutFile schreiben und deren Pfad als Step-Output
# `config` ausgeben. Der Release-Lauf haengt sie per `--config` an jeden `tauri build`.
param(
    [Parameter(Mandatory = $true)]
    [string]$OutFile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$provider = "$env:LVA_SIGN_PROVIDER".Trim().ToLowerInvariant()
if (-not $provider) {
    Write-Host '::warning::WINDOWS_SIGN_PROVIDER nicht gesetzt: Windows-Artefakte bleiben unsigniert (SmartScreen "Unbekannter Herausgeber").'
    "config=" >> $env:GITHUB_OUTPUT
    exit 0
}

switch ($provider) {
    'azure' {
        cargo install trusted-signing-cli --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo install trusted-signing-cli fehlgeschlagen' }
    }
    'esigner' {
        if (-not $env:CODESIGNTOOL_URL) { throw 'CODESIGNTOOL_URL fehlt (Repo-Variable, Download-Link von SSL.com)' }
        $zip = Join-Path $env:RUNNER_TEMP 'codesigntool.zip'
        $dir = Join-Path $env:RUNNER_TEMP 'codesigntool'
        Invoke-WebRequest -Uri $env:CODESIGNTOOL_URL -OutFile $zip
        Expand-Archive $zip -DestinationPath $dir -Force
        $bat = Get-ChildItem $dir -Recurse -Filter 'CodeSignTool.bat' | Select-Object -First 1
        if (-not $bat) { throw 'CodeSignTool.bat nicht im Archiv' }
        "CODESIGNTOOL_DIR=$($bat.DirectoryName)" >> $env:GITHUB_ENV
    }
    default { throw "Unbekannter WINDOWS_SIGN_PROVIDER '$provider' (azure|esigner)" }
}

$script = (Resolve-Path (Join-Path $PSScriptRoot 'sign-windows.ps1')).Path -replace '\\', '/'
$conf = @{
    bundle = @{
        windows = @{
            signCommand = @{ cmd = 'pwsh'; args = @('-NoProfile', '-File', $script, '%1') }
        }
    }
} | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText($OutFile, $conf)
Write-Host "Signierkonfiguration ($provider): $OutFile"
"config=$($OutFile -replace '\\', '/')" >> $env:GITHUB_OUTPUT
