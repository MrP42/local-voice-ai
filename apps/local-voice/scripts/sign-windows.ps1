# Authenticode-Signatur fuer eine Windows-Datei (EXE, Installer, Deinstallationsprogramm).
#
# Wird von Tauri als bundle.windows.signCommand aufgerufen (scripts/sign-windows-config.ps1
# schreibt die Konfiguration); Tauri signiert damit die App-EXE in target/release, das
# NSIS-Deinstallationsprogramm und den Installer, und zwar VOR der Updater-Signatur (.sig).
# Darum nie nachtraeglich signieren: die .sig in latest.json passte dann nicht mehr.
#
# Anbieter ueber LVA_SIGN_PROVIDER (Repo-Variable WINDOWS_SIGN_PROVIDER):
#   azure    Azure Artifact Signing (frueher Trusted Signing) ueber trusted-signing-cli.
#            Secrets: AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET
#            Variablen: AZURE_SIGNING_ENDPOINT (z. B. https://weu.codesigning.azure.net),
#                       AZURE_SIGNING_ACCOUNT, AZURE_SIGNING_PROFILE
#   esigner  OV-Zertifikat mit Cloud-HSM bei SSL.com (eSigner CodeSignTool).
#            Secrets: ES_USERNAME, ES_PASSWORD, ES_CREDENTIAL_ID, ES_TOTP_SECRET
#            CodeSignTool liegt unter $env:CODESIGNTOOL_DIR (sign-windows-config.ps1 laedt es).
#
# Das Updater-Schluesselpaar (TAURI_SIGNING_PRIVATE_KEY) ist davon unabhaengig.
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Require-Env([string[]]$Names) {
    foreach ($n in $Names) {
        if (-not [Environment]::GetEnvironmentVariable($n)) { throw "Signieren: Umgebungsvariable $n fehlt" }
    }
}

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Signieren: Datei nicht gefunden: $Path" }
$Path = (Resolve-Path -LiteralPath $Path).Path
$provider = "$env:LVA_SIGN_PROVIDER".Trim().ToLowerInvariant()
Write-Host "Signieren ($provider): $Path"

switch ($provider) {
    'azure' {
        Require-Env @('AZURE_TENANT_ID', 'AZURE_CLIENT_ID', 'AZURE_CLIENT_SECRET',
            'AZURE_SIGNING_ENDPOINT', 'AZURE_SIGNING_ACCOUNT', 'AZURE_SIGNING_PROFILE')
        & trusted-signing-cli -e $env:AZURE_SIGNING_ENDPOINT -a $env:AZURE_SIGNING_ACCOUNT `
            -c $env:AZURE_SIGNING_PROFILE -d 'Local Voice AI' $Path
        if ($LASTEXITCODE -ne 0) { throw "trusted-signing-cli fehlgeschlagen ($LASTEXITCODE)" }
    }
    'esigner' {
        Require-Env @('ES_USERNAME', 'ES_PASSWORD', 'ES_CREDENTIAL_ID', 'ES_TOTP_SECRET', 'CODESIGNTOOL_DIR')
        Push-Location $env:CODESIGNTOOL_DIR
        try {
            # -override: signiert die Datei an Ort und Stelle (Tauri erwartet das).
            # Restrisiko: CodeSignTool nimmt die Zugangsdaten nur als Argumente an; sie stehen
            # waehrend des Aufrufs in der Prozessliste. Auf GitHub-gehosteten Runnern ist die VM
            # ein Wegwerfsystem nur fuer diesen Job, Logs maskieren Secrets. Nicht auf geteilten
            # oder selbst gehosteten Runnern verwenden; dort Azure Artifact Signing nehmen.
            & .\CodeSignTool.bat sign "-username=$env:ES_USERNAME" "-password=$env:ES_PASSWORD" `
                "-credential_id=$env:ES_CREDENTIAL_ID" "-totp_secret=$env:ES_TOTP_SECRET" `
                "-input_file_path=$Path" -override
            if ($LASTEXITCODE -ne 0) { throw "CodeSignTool fehlgeschlagen ($LASTEXITCODE)" }
        } finally { Pop-Location }
    }
    default { throw "Signieren: unbekannter LVA_SIGN_PROVIDER '$provider' (azure|esigner)" }
}

# Gegenprobe: gueltige, zeitgestempelte Signatur. Ohne Zeitstempel liefe sie mit dem
# Zertifikat ab (Artifact Signing: nach ~3 Tagen).
$sig = Get-AuthenticodeSignature -LiteralPath $Path
if ($sig.Status -ne 'Valid') { throw "Signatur ungueltig ($($sig.Status)): $($sig.StatusMessage)" }
if (-not $sig.TimeStamperCertificate) { throw "Signatur ohne Zeitstempel: $Path" }
Write-Host ("Signiert: {0} -> {1}" -f (Split-Path $Path -Leaf), $sig.SignerCertificate.Subject)
