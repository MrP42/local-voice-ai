# Nutzlast fuer den Installer mit Modellen (lva#574): Nemotron 3.5 ASR, Parakeet V3 und Phonon
# (Programm, Deutsch und Englisch mit allen Stimmen), ohne Download nach der Installation.
#
# Baut KEINE Rust-Quellen. Nimmt die EXE, die `tauri build` hinterlassen hat, und laedt jede
# Komponente einzeln ueber `--provision` in einen eigenen portablen Probeordner (derselbe Ladeweg
# wie die Modellseite: Pruefsummen, Installationsmarken). Das Ergebnis ist genau die Ablage, die
# die App als installiert erkennt; der Installer kopiert sie unveraendert in den Datenordner.
#
# Ergebnis unter -OutDir (Vorgabe target/release/bundle/vorinstalliert):
#   nemotron\models\...      Spracherkennung, Ziel im Installer: %APPDATA%\<Kennung>\models
#   parakeet\models\...      Spracherkennung, Ziel im Installer: %APPDATA%\<Kennung>\models
#   phonon\tts\...           Vorlesen,        Ziel im Installer: %LOCALAPPDATA%\<Kennung>\tts
#   LIZENZEN-MODELLE.txt     landet neben der EXE
#   vorinstalliert.nsh       Kennungen und Texte fuer nsis/installer.nsi
#   INHALT.json              Komponenten, Kennungen, Groessen
#
# Danach den Installer bauen (der Release-Workflow macht das so):
#   $env:LVA_VORINSTALLIERT = '<OutDir>'
#   pnpm tauri build --features gpu-vulkan --bundles nsis --config <Datei mit createUpdaterArtifacts=false>
# Ohne LVA_VORINSTALLIERT baut dieselbe Vorlage den gewohnten schlanken Installer.
#
# Aufruf aus apps/local-voice (PowerShell 7):
#   pwsh scripts/build-installer-modelle.ps1 -SmokeTest
#   $env:HF_TOKEN = '…'; pwsh scripts/build-installer-modelle.ps1 -RequireHfToken -SmokeTest
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$OutDir,
    [switch]$SmokeTest,
    # Ohne HF_TOKEN abbrechen statt das deutsche Phonon-Modell (gated) auszulassen.
    [switch]$RequireHfToken,
    # Deutsches Phonon-Modell (CC BY-NC 4.0) gar nicht erst einpacken, z. B. fuer eine Weitergabe
    # mit geschaeftlichem Zweck. Englisch und alle englischen Stimmen bleiben.
    [switch]$OhnePhononDe
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tauriDir = Join-Path $appDir 'src-tauri'

if (-not $OutDir) { $OutDir = Join-Path $tauriDir 'target/release/bundle/vorinstalliert' }

# -OutDir wird nie geloescht oder ersetzt (Codex R1-R4 zu lva#574): Ein vorhandener Ordner muss
# leer sein, sonst bricht das Skript ab, bevor es irgendetwas schreibt. Damit gibt es keinen
# Loeschpfad, und Aliasnamen (Junction, subst, 8.3, UNC) koennen keine fremden Dateien treffen.
# Eine fruehere Nutzlast entfernt, wer sie nicht mehr braucht, selbst.
if ($OutDir.IndexOfAny([char[]]'*?[]') -ge 0) { throw "-OutDir enthaelt Platzhalter (* ? [ ]): $OutDir; bitte einen konkreten Ordner angeben" }
$outFull = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)
if (Test-Path -LiteralPath $outFull) {
    if (-not (Test-Path -LiteralPath $outFull -PathType Container)) { throw "-OutDir ist eine Datei: $outFull" }
    if (@(Get-ChildItem -LiteralPath $outFull -Force).Count -gt 0) {
        throw "-OutDir $outFull ist nicht leer; das Skript loescht nichts. Bitte einen neuen oder leeren Ordner angeben (eine fruehere Nutzlast vorher selbst entfernen)"
    }
}

if (-not $Exe) {
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $tauriDir 'target' }
    $Exe = Join-Path $targetDir 'release/local-voice-ai.exe'
}
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "EXE nicht gefunden: $Exe (erst 'npx tauri build' ausführen)" }

$libsDir = Join-Path $tauriDir 'transcribe-libs'
$resDir = Join-Path $tauriDir 'resources'
$licenses = Join-Path $tauriDir 'nsis/LIZENZEN-MODELLE.txt'
foreach ($p in @($libsDir, $resDir)) {
    if (-not (Test-Path $p -PathType Container)) { throw "Ordner fehlt: $p" }
}
if (-not (Test-Path $licenses -PathType Leaf)) { throw "Datei fehlt: $licenses" }

# Kennungen wie in build-portable-zip.ps1 (dort im Release erprobt). Data-Ordner je Komponente:
# Spracherkennung liegt unter models\, Sprachausgabe unter tts\ (siehe installer.nsi).
$components = [ordered]@{
    nemotron = @{ Ids = @('nemotron-3.5-asr-streaming-0.6b-Q8_0'); Folder = 'models'; Asr = $true }
    parakeet = @{ Ids = @('parakeet-tdt-0.6b-v3'); Folder = 'models'; Asr = $true }
    phonon   = @{ Ids = @('phonon-runtime', 'phonon-40m-de', 'phonon-31m-en'); Folder = 'tts'; Asr = $false }
}

$phononDe = $true
if ($OhnePhononDe) {
    Write-Host 'Ohne deutsches Phonon-Modell (-OhnePhononDe).'
    $components.phonon.Ids = @($components.phonon.Ids | Where-Object { $_ -ne 'phonon-40m-de' })
    $phononDe = $false
} elseif (-not $env:HF_TOKEN) {
    if ($RequireHfToken) { throw 'HF_TOKEN fehlt: phonon-40m-de ist auf Hugging Face zugangsbeschraenkt.' }
    Write-Warning 'HF_TOKEN fehlt: das deutsche Phonon-Modell (gated) wird ausgelassen, Englisch bleibt.'
    if ($env:GITHUB_ACTIONS) { Write-Host '::warning::HF_TOKEN fehlt: Installer mit Modellen ohne deutsches Phonon-Modell' }
    $components.phonon.Ids = @($components.phonon.Ids | Where-Object { $_ -ne 'phonon-40m-de' })
    $phononDe = $false
}

$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("lva-vorinst-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $stage | Out-Null

# Hugging-Face-Cache fuer Kindprozesse umlenken (HF_HUB_CACHE/HUGGINGFACE_HUB_CACHE haetten
# Vorrang vor HF_HOME); gibt die alten Werte zum Wiederherstellen zurueck.
$HfVars = @('HF_HOME', 'HF_HUB_CACHE', 'HUGGINGFACE_HUB_CACHE')
function Set-HfCache([string]$HfHome) {
    $saved = @{}
    foreach ($v in $HfVars) { $saved[$v] = [Environment]::GetEnvironmentVariable($v) }
    [Environment]::SetEnvironmentVariable('HF_HUB_CACHE', $null)
    [Environment]::SetEnvironmentVariable('HUGGINGFACE_HUB_CACHE', $null)
    [Environment]::SetEnvironmentVariable('HF_HOME', $HfHome)
    return $saved
}
function Restore-HfCache([hashtable]$Saved) {
    foreach ($v in $HfVars) { [Environment]::SetEnvironmentVariable($v, $Saved[$v]) }
}

# Portabler Probeordner: EXE, Laufzeit-DLLs, resources\ und die Marke `portable`
# (portable.rs: alle Daten nach Data\ neben der EXE).
function New-AppRoot([string]$Dir) {
    New-Item -ItemType Directory -Force $Dir | Out-Null
    Copy-Item -LiteralPath $Exe (Join-Path $Dir 'local-voice-ai.exe')
    Get-ChildItem $libsDir -File | Copy-Item -Destination $Dir
    Copy-Item $resDir (Join-Path $Dir 'resources') -Recurse
    [System.IO.File]::WriteAllText((Join-Path $Dir 'portable'), 'Handy Portable Mode', [System.Text.Encoding]::ASCII)
    New-Item -ItemType Directory -Force (Join-Path $Dir 'Data') | Out-Null
}

function Invoke-App([string]$Root, [string[]]$AppArgs, [string]$LogName, [int]$TimeoutMin) {
    $out = Join-Path $stage "$LogName.out.txt"
    $err = Join-Path $stage "$LogName.err.txt"
    $p = Start-Process -FilePath (Join-Path $Root 'local-voice-ai.exe') -ArgumentList $AppArgs `
        -WorkingDirectory $Root -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $p.Handle
    if (-not $p.WaitForExit($TimeoutMin * 60 * 1000)) { $p.Kill(); throw "$LogName haengt ($TimeoutMin min)" }
    foreach ($f in @($out, $err)) { if (Test-Path $f) { Get-Content $f | ForEach-Object { Write-Host "  $_" } } }
    return @{ Code = $p.ExitCode; Out = $out }
}

function Get-DirBytes([string]$Dir) {
    $sum = (Get-ChildItem $Dir -Recurse -File -Force | Measure-Object Length -Sum).Sum
    if ($sum) { [int64]$sum } else { [int64]0 }
}

try {
    # Oben geprueft: fehlt oder ist leer. Es wird nichts geloescht.
    $null = [System.IO.Directory]::CreateDirectory($outFull)
    $selected = @{}
    $summary = [ordered]@{}

    foreach ($name in $components.Keys) {
        $c = $components[$name]
        Write-Host "== ${name}: $($c.Ids -join ', ')"
        $root = Join-Path $stage $name
        New-AppRoot $root
        $hfHome = Join-Path $stage "hf-$name"
        $hfSaved = Set-HfCache $hfHome
        try {
            $r = Invoke-App $root @('--provision', ($c.Ids -join ',')) "provision-$name" 180
        } finally { Restore-HfCache $hfSaved }
        if ($r.Code -ne 0) { throw "--provision $name endete mit Exit-Code $($r.Code)" }

        $data = Join-Path $root 'Data'
        if ($c.Asr) {
            # Modelle von Hugging Face landen im HF-Cache, nicht unter Data\. Wie im portablen
            # Paket nach Data\models kopieren; dort erkennt model.rs sie als installiert.
            $modelsDir = Join-Path $data 'models'
            New-Item -ItemType Directory -Force $modelsDir | Out-Null
            $snapshots = @(Get-ChildItem (Join-Path $hfHome 'hub') -Directory -Filter 'models--*' -ErrorAction SilentlyContinue |
                ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'snapshots') -Directory -ErrorAction SilentlyContinue })
            foreach ($snap in $snapshots) {
                Get-ChildItem $snap.FullName -File -Force | ForEach-Object {
                    [System.IO.File]::Copy($_.FullName, (Join-Path $modelsDir $_.Name), $true)
                    Write-Host ("  aus HF-Cache: {0}" -f $_.Name)
                }
            }
            # Die Kennung, die die App fuer das Modell fuehrt: provision.rs stellt das erste
            # Spracherkennungsmodell ein. Genau diese Kennung schreibt der Installer in erststart.ini.
            $store = Join-Path $data 'settings_store.json'
            if (-not (Test-Path $store)) { throw "${name}: --provision hat keine Voreinstellung geschrieben" }
            $settings = (Get-Content $store -Raw | ConvertFrom-Json).PSObject.Properties['settings']
            $selProp = if ($settings) { $settings.Value.PSObject.Properties['selected_model'] } else { $null }
            $sel = if ($selProp) { $selProp.Value } else { $null }
            if (-not $sel) { throw "${name}: keine Spracherkennung voreingestellt" }
            if ($sel -match '["$`]') { throw "${name}: unerwartete Kennung '$sel'" }
            $selected[$name] = [string]$sel
            Write-Host "  Kennung: $sel"
        }
        Remove-Item $hfHome -Recurse -Force -ErrorAction SilentlyContinue

        $src = Join-Path $data $c.Folder
        if (-not (Test-Path $src) -or (Get-DirBytes $src) -eq 0) { throw "${name}: nichts unter Data\$($c.Folder)" }
        Get-ChildItem $src -Recurse -File -Force |
            Where-Object { $_.Extension -in @('.partial', '.download', '.tmp') -or $_.Name -like '*.tar.gz' } |
            Remove-Item -Force
        $dst = Join-Path $outFull $name
        New-Item -ItemType Directory -Force $dst | Out-Null
        Move-Item $src (Join-Path $dst $c.Folder)
        $bytes = Get-DirBytes $dst
        $summary[$name] = [ordered]@{ ids = $c.Ids; bytes = $bytes; kennung = $selected[$name] }
        Write-Host ("  Nutzlast {0}: {1:N0} MB" -f $name, ($bytes / 1MB))
        Get-ChildItem $dst -Recurse -File | ForEach-Object {
            Write-Host ("  {0,12:N0}  {1}" -f $_.Length, $_.FullName.Substring($dst.Length + 1))
        }
        Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
    }

    # Ohne diese Pruefungen entstuende still ein Installer, der doch wieder laedt.
    if (-not (Get-ChildItem -LiteralPath (Join-Path $outFull 'nemotron/models') -File -Filter 'nemotron-3.5-asr-*.gguf' -ErrorAction SilentlyContinue)) {
        throw 'Nemotron 3.5 ASR fehlt in der Nutzlast'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $outFull 'phonon/tts/phonon'))) { throw 'Phonon fehlt in der Nutzlast (tts\phonon)' }
    # Je installiertem Phonon-Teil (Programm, je Sprachmodell) schreibt phonon_models.rs zuletzt
    # die Marke .lva-install.json; ohne sie gilt der Teil in der App als nicht installiert.
    $markers = @(Get-ChildItem -LiteralPath (Join-Path $outFull 'phonon/tts/phonon') -Recurse -File -Force -Filter '.lva-install.json')
    foreach ($id in @('phonon-31m-en') + @(if ($phononDe) { 'phonon-40m-de' })) {
        if (-not ($markers | Where-Object { (Get-Content $_.FullName -Raw) -match [regex]::Escape("`"$id`"") })) {
            throw "Phonon: keine Installationsmarke fuer $id"
        }
    }

    Copy-Item $licenses (Join-Path $outFull 'LIZENZEN-MODELLE.txt')

    $phononText = if ($phononDe) {
        'Programm, Deutsch und Englisch mit allen Stimmen. Lizenzen: Programm MIT/Apache-2.0, Englisch und Stimmen Apache-2.0, Deutsch CC BY-NC 4.0 (nur nicht-kommerziell). Einschalten unter Einstellungen > Ausgabe.'
    } else {
        'Programm und Englisch mit allen Stimmen (Apache-2.0). Das deutsche Modell ist in diesem Installer nicht enthalten. Einschalten unter Einstellungen > Ausgabe.'
    }
    $phononName = if ($phononDe) { 'Vorlesen Phonon (Deutsch und Englisch, alle Stimmen)' } else { 'Vorlesen Phonon (Englisch, alle Stimmen)' }
    $nsh = @(
        '; Erzeugt von scripts/build-installer-modelle.ps1, nicht von Hand aendern.'
        ('!define VORINST_PHONON_NAME "{0}"' -f $phononName)
        ('!define VORINST_NEMOTRON_ID "{0}"' -f $selected['nemotron'])
        ('!define VORINST_PARAKEET_ID "{0}"' -f $selected['parakeet'])
        ('!define VORINST_PHONON_TEXT "{0}"' -f $phononText)
    )
    # UTF-8 mit BOM: makensis liest Umlaute dann unabhaengig von -INPUTCHARSET richtig.
    [System.IO.File]::WriteAllLines((Join-Path $outFull 'vorinstalliert.nsh'), $nsh, (New-Object System.Text.UTF8Encoding $true))
    $summary['phonon_de'] = $phononDe
    [System.IO.File]::WriteAllText((Join-Path $outFull 'INHALT.json'),
        ($summary | ConvertTo-Json -Depth 5), (New-Object System.Text.UTF8Encoding $false))

    $total = Get-DirBytes $outFull
    Write-Host ("Nutzlast gesamt: {0:N0} MB unkomprimiert" -f ($total / 1MB))
    # NSIS-Installer duerfen hoechstens 2 GB gross werden, GitHub nimmt je Datei 2 GiB. Modell-
    # gewichte komprimieren kaum; mit App (~150 MB) muss die Nutzlast deutlich darunter bleiben.
    if ($total -gt [int64](1.75 * 1GB)) { throw ("Nutzlast {0:N0} MB zu gross fuer einen Installer" -f ($total / 1MB)) }

    if ($SmokeTest) {
        # Alle Komponenten so zusammenlegen, wie der Installer sie ablegt (portabel: beides in
        # Data\), und ohne HF-Cache pruefen, ob die App die Spracherkennung als installiert fuehrt.
        $probe = Join-Path $stage 'probe'
        New-AppRoot $probe
        foreach ($name in $components.Keys) {
            Copy-Item (Join-Path $outFull "$name/*") (Join-Path $probe 'Data') -Recurse -Force
        }
        $hfSaved = Set-HfCache (Join-Path $stage 'hf-leer')
        try {
            $r = Invoke-App $probe @('--list-models') 'list' 15
        } finally { Restore-HfCache $hfSaved }
        if ($r.Code -ne 0) { throw "Rauchtest: --list-models Exit-Code $($r.Code)" }
        $list = Get-Content $r.Out -Raw
        if ($list -notmatch '(?m)^\s*✓.*nemotron-3\.5-asr') { throw 'Rauchtest: Nemotron 3.5 ASR nicht als installiert erkannt' }
        if ($list -notmatch '(?m)^\s*✓.*parakeet-tdt-0\.6b-v3') { throw 'Rauchtest: Parakeet V3 nicht als installiert erkannt' }
        Write-Host 'Rauchtest: bestanden.'
    }
    Write-Host "Nutzlast: $outFull"
}
finally {
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
