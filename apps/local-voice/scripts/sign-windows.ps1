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
        $jar = Get-ChildItem (Join-Path $env:CODESIGNTOOL_DIR 'jar') -Filter 'code_sign_tool-*.jar' |
            Select-Object -First 1
        if (-not $jar) { throw "CodeSignTool-JAR nicht gefunden unter $env:CODESIGNTOOL_DIR/jar" }
        # Zugangsdaten nicht als Prozessargumente (Prozessliste), sondern ueber eine
        # Java-Argumentdatei (Java >= 9, `java @datei`): Kommandozeile zeigt nur den Pfad.
        # Datei nur fuer den aktuellen Benutzer lesbar, im finally geloescht. Quoting nach
        # Java-Argumentdatei-Regeln: Backslash und Anfuehrungszeichen escapen ('\' ersetzt
        # im .NET-Ersetzungstext literal, ergibt also zwei Backslashes).
        function Quote-Arg([string]$v) { '"' + ($v -replace '\\', '\\' -replace '"', '\"') + '"' }
        $argFile = Join-Path ([System.IO.Path]::GetTempPath()) ("sign-" + [guid]::NewGuid().ToString('N') + '.args')
        [System.IO.File]::WriteAllText($argFile, '')
        & icacls $argFile /inheritance:r /grant:r "$($env:USERNAME):(R,W,D)" | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'icacls fuer Argumentdatei fehlgeschlagen' }
        $lines = @(
            '-jar', (Quote-Arg $jar.FullName), 'sign',
            (Quote-Arg "-username=$env:ES_USERNAME"),
            (Quote-Arg "-password=$env:ES_PASSWORD"),
            (Quote-Arg "-credential_id=$env:ES_CREDENTIAL_ID"),
            (Quote-Arg "-totp_secret=$env:ES_TOTP_SECRET"),
            (Quote-Arg "-input_file_path=$Path"),
            # -override: signiert die Datei an Ort und Stelle (Tauri erwartet das).
            '-override'
        )
        Push-Location $env:CODESIGNTOOL_DIR
        try {
            [System.IO.File]::WriteAllLines($argFile, $lines, (New-Object System.Text.UTF8Encoding $false))
            & java "@$argFile"
            if ($LASTEXITCODE -ne 0) { throw "CodeSignTool fehlgeschlagen ($LASTEXITCODE)" }
        } finally {
            Pop-Location
            Remove-Item -LiteralPath $argFile -Force -ErrorAction SilentlyContinue
        }
    }
    default { throw "Signieren: unbekannter LVA_SIGN_PROVIDER '$provider' (azure|esigner)" }
}

# Gegenprobe: gueltige, zeitgestempelte Signatur. Ohne Zeitstempel liefe sie mit dem
# Zertifikat ab (Artifact Signing: nach ~3 Tagen).
$sig = Get-AuthenticodeSignature -LiteralPath $Path
if ($sig.Status -ne 'Valid') { throw "Signatur ungueltig ($($sig.Status)): $($sig.StatusMessage)" }
if (-not $sig.TimeStamperCertificate) { throw "Signatur ohne Zeitstempel: $Path" }
Write-Host ("Signiert: {0} -> {1}" -f (Split-Path $Path -Leaf), $sig.SignerCertificate.Subject)
