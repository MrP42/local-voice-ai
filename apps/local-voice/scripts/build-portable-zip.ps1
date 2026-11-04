# Portable Windows-Version von Local Voice AI als ZIP: entpacken und starten, ohne Installation.
#
# Baut KEINE Rust-Quellen, sondern packt den Stand, den `tauri build` hinterlassen hat:
#   - die Haupt-EXE (eingebettetes Frontend, wie im Installer),
#   - die Laufzeit-DLLs aus src-tauri/transcribe-libs/ (Whisper/ggml, ggml-vulkan,
#     onnxruntime, VC++-Laufzeit; der Installer legt sie ebenfalls neben die EXE),
#   - src-tauri/resources/ als resources\ (wie tauri.windows.conf.json),
#   - die Markierungsdatei `portable` (portable.rs: alle Daten nach Data\ neben der EXE),
#   - LIESMICH-PORTABLE.txt.
# Damit ist der Inhalt derselbe wie bei einer portablen Installation über den NSIS-Installer,
# nur ohne Installer.
#
# Aufruf aus apps/local-voice (PowerShell 7, nach `npx tauri build ...`):
#   pwsh scripts/build-portable-zip.ps1                     # Version aus tauri.conf.json
#   pwsh scripts/build-portable-zip.ps1 -Exe D:/b/abnahme/local-voice-ai.exe -OutDir D:/lv-build/portable
#   pwsh scripts/build-portable-zip.ps1 -SmokeTest          # zusätzlich entpacken und `--help` starten
#   pwsh scripts/build-portable-zip.ps1 -Release -SmokeTest # CI: alle Release-DLLs Pflicht
#   pwsh scripts/build-portable-zip.ps1 -Release -Offline -SmokeTest
#        # Offline-Paket: Modelle vorinstallieren (HF_TOKEN fuer Phonon DE), auf ZIPs <2 GiB verteilen
#
# Ergebnis: <OutDir>/LocalVoiceAI_<Version>_x64-portable.zip und .zip.sha256; mit -Offline
# LocalVoiceAI_<Version>_x64-portable-offline.teil<N>von<M>.zip (je < 2 GiB, alle in denselben
# Ordner entpacken; geteilte Modelle setzt die App beim ersten Start zusammen, portable_parts.rs).
# Exit-Code 0 nur, wenn alle Pflichtteile vorhanden sind (und der Rauchtest besteht).
[CmdletBinding()]
param(
    [string]$Exe,
    [string]$OutDir,
    [string]$Version,
    [switch]$SmokeTest,
    [switch]$Release,
    # Offline-Paket: Modelle vorinstallieren (--provision) und auf mehrere ZIPs verteilen.
    [switch]$Offline,
    # Katalogkennungen fuer -Offline. Vorgabe: Laptop ohne Grafikkarte, 16 GB RAM
    # (Core Ultra 5 225U): Nemotron 3.5 ASR, Phonon DE+EN mit Stimmen, LLM-Laufzeit
    # Vulkan (iGPU) + CPU, Qwen 3.5 4B Q4, Stimmabdruck CAM++ (Sprecher-Vorschlaege).
    [string[]]$Provision = @(
        'nemotron-3.5-asr-streaming-0.6b-Q8_0',
        'phonon-runtime', 'phonon-40m-de', 'phonon-31m-en',
        'llm-runtime-windows-x64-vulkan', 'llm-runtime-windows-x64-cpu',
        'llm-qwen3.5-4b-q4',
        'spk-campplus-voxceleb-16k'
    ),
    # Mit -Release: ohne HF_TOKEN abbrechen statt das deutsche Phonon-Modell auszulassen.
    [switch]$RequireHfToken
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$appDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tauriDir = Join-Path $appDir 'src-tauri'

if (-not $Version) {
    $conf = Get-Content (Join-Path $tauriDir 'tauri.conf.json') -Raw | ConvertFrom-Json
    $Version = $conf.version
}
if ($Version -notmatch '^\d+\.\d+\.\d+([-+].+)?$') { throw "Ungültige Version: '$Version'" }

if (-not $Exe) {
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $tauriDir 'target' }
    $Exe = Join-Path $targetDir 'release/local-voice-ai.exe'
}
if (-not (Test-Path $Exe -PathType Leaf)) { throw "EXE nicht gefunden: $Exe (erst 'npx tauri build' ausführen)" }
if (-not $OutDir) { $OutDir = Join-Path $tauriDir 'target/release/bundle/portable' }

$libsDir = Join-Path $tauriDir 'transcribe-libs'
$resDir = Join-Path $tauriDir 'resources'
$readme = Join-Path $tauriDir 'portable/LIESMICH-PORTABLE.txt'
foreach ($p in @($libsDir, $resDir)) {
    if (-not (Test-Path $p -PathType Container)) { throw "Ordner fehlt: $p" }
}
if (-not (Test-Path $readme -PathType Leaf)) { throw "Datei fehlt: $readme" }

# Release-Stand (CI): ORT dynamisch gebunden, VC++-Laufzeit app-lokal, Vulkan an (build.rs
# legt die DLLs nur mit ORT_PREFER_DYNAMIC_LINK / HANDY_VC_REDIST_DIRS / gpu-vulkan ab).
# Mit -Release fehlt keine davon, sonst startet die ZIP auf fremden PCs nicht oder nur mit CPU.
# Ohne -Release (lokaler Abnahmestand) nur Warnungen.
$releaseDlls = @('onnxruntime.dll', 'msvcp140.dll', 'vcruntime140.dll', 'ggml-vulkan.dll')
$missing = @($releaseDlls | Where-Object { -not (Test-Path (Join-Path $libsDir $_)) })
if ($missing.Count -gt 0) {
    $msg = "Laufzeit-DLLs fehlen in transcribe-libs/: $($missing -join ', ')"
    if ($Release) { throw $msg } else { Write-Warning "$msg (lokaler Stand, kein Release)" }
}

$folderName = 'Local Voice AI'
$baseName = if ($Offline) { "LocalVoiceAI_${Version}_x64-portable-offline" } else { "LocalVoiceAI_${Version}_x64-portable" }
$stage = Join-Path ([System.IO.Path]::GetTempPath()) ("lva-portable-" + [guid]::NewGuid().ToString('N'))
$root = Join-Path $stage $folderName
New-Item -ItemType Directory -Force $root | Out-Null

# GitHub nimmt je Release-Datei hoechstens 2 GiB. Budget je ZIP mit Reserve fuer
# ZIP-Verwaltungsdaten; groessere Dateien werden in Stuecke von $PartBytes geteilt.
$ZipBudget = [int64](1.90 * 1GB)
$PartBytes = [int64](1.50 * 1GB)

function Invoke-App([string[]]$AppArgs, [string]$LogName) {
    $out = Join-Path $stage "$LogName.out.txt"
    $err = Join-Path $stage "$LogName.err.txt"
    $p = Start-Process -FilePath (Join-Path $root 'local-voice-ai.exe') -ArgumentList $AppArgs `
        -WorkingDirectory $root -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
    $null = $p.Handle
    if (-not $p.WaitForExit(3 * 60 * 60 * 1000)) { $p.Kill(); throw "$LogName haengt (3 h)" }
    foreach ($f in @($out, $err)) { if (Test-Path $f) { Get-Content $f | ForEach-Object { Write-Host "  $_" } } }
    return $p.ExitCode
}

function Split-LargeFile([System.IO.FileInfo]$File) {
    $hash = (Get-FileHash $File.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    $names = @()
    $buf = New-Object byte[] (4MB)
    $in = [System.IO.File]::OpenRead($File.FullName)
    try {
        $n = 0
        while ($in.Position -lt $in.Length) {
            $n++
            $name = '{0}.teil{1:D3}' -f $File.Name, $n
            $names += $name
            $out = [System.IO.File]::Create((Join-Path $File.DirectoryName $name))
            try {
                $left = $PartBytes
                while ($left -gt 0) {
                    $read = $in.Read($buf, 0, [int][math]::Min($buf.Length, $left))
                    if ($read -le 0) { break }
                    $out.Write($buf, 0, $read)
                    $left -= $read
                }
            } finally { $out.Dispose() }
        }
    } finally { $in.Dispose() }
    # Beschreibung fuer portable_parts.rs (Feldnamen dort).
    $manifest = [ordered]@{ file = $File.Name; size = $File.Length; sha256 = $hash; parts = $names }
    [System.IO.File]::WriteAllText((Join-Path $File.DirectoryName "$($File.Name).teile.json"),
        ($manifest | ConvertTo-Json -Compress), (New-Object System.Text.UTF8Encoding $false))
    Remove-Item $File.FullName -Force
    Write-Host ("  geteilt: {0} ({1:N0} MB) in {2} Teile" -f $File.Name, ($File.Length / 1MB), $names.Count)
}

try {
    Copy-Item $Exe (Join-Path $root 'local-voice-ai.exe')
    Get-ChildItem $libsDir -File | Copy-Item -Destination $root
    Copy-Item $resDir (Join-Path $root 'resources') -Recurse
    Copy-Item $readme (Join-Path $root 'LIESMICH-PORTABLE.txt')

    # Inhalt muss exakt dem Magic-String in portable.rs entsprechen (ASCII, ohne BOM).
    # Den Text teilt sich die App mit dem NSIS-Installer (Option "Portable Installation").
    [System.IO.File]::WriteAllText((Join-Path $root 'portable'), 'Handy Portable Mode', [System.Text.Encoding]::ASCII)

    if ($Offline) {
        Copy-Item (Join-Path $tauriDir 'portable/LIZENZEN-MODELLE.txt') (Join-Path $root 'LIZENZEN-MODELLE.txt')
        $ids = @($Provision)
        if (-not $env:HF_TOKEN -and ($ids -contains 'phonon-40m-de')) {
            if ($Release -and $RequireHfToken) { throw 'HF_TOKEN fehlt: phonon-40m-de ist auf Hugging Face zugangsbeschraenkt.' }
            Write-Warning 'HF_TOKEN fehlt: das deutsche Phonon-Modell (gated) wird ausgelassen, Englisch bleibt.'
            $ids = @($ids | Where-Object { $_ -ne 'phonon-40m-de' })
        }
        Write-Host "Vorinstallieren: $($ids -join ', ')"
        $code = Invoke-App @('--provision', ($ids -join ',')) 'provision'
        if ($code -ne 0) { throw ("--provision endete mit Exit-Code {0}" -f $code) }

        # Nur die Modellablagen und die Voreinstellung mitnehmen; alles Rechnerspezifische
        # (Protokolle, WebView-Zwischenspeicher, Geraete-Probe, Geheimnisse) bleibt draussen.
        $data = Join-Path $root 'Data'
        $keep = @('models', 'llm', 'tts', 'settings_store.json')
        Get-ChildItem $data -Force | Where-Object { $keep -notcontains $_.Name } | Remove-Item -Recurse -Force
        Get-ChildItem $data -Recurse -File -Force |
            Where-Object { $_.Name -in @('external-probe.json', 'backend-probe.json') -or $_.Extension -in @('.partial', '.download') } |
            Remove-Item -Force
        # Voreinstellung unter eigenem Namen: portable_parts.rs uebernimmt sie nur, wenn es noch
        # keine Einstellungen gibt; Entpacken ueber einen bestehenden Ordner aendert nichts.
        $store = Join-Path $data 'settings_store.json'
        if (-not (Test-Path $store)) { throw 'Voreinstellung fehlt: --provision hat Data\settings_store.json nicht geschrieben' }
        Move-Item $store (Join-Path $data 'settings_store.seed.json') -Force
        Get-ChildItem $data -Recurse -File | Where-Object { $_.Length -gt $ZipBudget } | ForEach-Object { Split-LargeFile $_ }
        Write-Host 'Inhalt Data\ im Paket:'
        Get-ChildItem $data -Recurse -File | ForEach-Object {
            Write-Host ("  {0,12:N0}  {1}" -f $_.Length, $_.FullName.Substring($data.Length + 1))
        }
    }

    # Dateien auf ZIPs verteilen: App-Teil (alles ausser Data\) immer in Teil 1, dann
    # Data\ absteigend nach Groesse in die erste ZIP mit Platz (first fit decreasing).
    $all = Get-ChildItem $root -Recurse -File -Force
    $dataPrefix = (Join-Path $root 'Data') + [System.IO.Path]::DirectorySeparatorChar
    $appFiles = @($all | Where-Object { -not $_.FullName.StartsWith($dataPrefix) })
    $dataFiles = @($all | Where-Object { $_.FullName.StartsWith($dataPrefix) } | Sort-Object Length -Descending)
    $bins = New-Object System.Collections.ArrayList
    $null = $bins.Add([pscustomobject]@{ Size = [int64](($appFiles | Measure-Object Length -Sum).Sum); Files = [System.Collections.ArrayList]@($appFiles) })
    foreach ($f in $dataFiles) {
        $bin = $bins | Where-Object { $_.Size + $f.Length -le $ZipBudget } | Select-Object -First 1
        if (-not $bin) {
            $bin = [pscustomobject]@{ Size = [int64]0; Files = New-Object System.Collections.ArrayList }
            $null = $bins.Add($bin)
        }
        $null = $bin.Files.Add($f)
        $bin.Size += $f.Length
    }
    if ($bins[0].Size -gt $ZipBudget) { throw 'App-Teil allein groesser als das ZIP-Budget' }

    New-Item -ItemType Directory -Force $OutDir | Out-Null
    $outFull = (Resolve-Path $OutDir).Path
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    # Modellgewichte komprimieren kaum: speichern statt packen spart beim Bau und Entpacken Minuten.
    $stored = @('.gguf', '.safetensors', '.onnx', '.bin')
    $zips = @()
    for ($i = 0; $i -lt $bins.Count; $i++) {
        $zipName = if ($bins.Count -eq 1) { "$baseName.zip" } else { '{0}.teil{1}von{2}.zip' -f $baseName, ($i + 1), $bins.Count }
        $zipPath = Join-Path $outFull $zipName
        if (Test-Path $zipPath) { Remove-Item $zipPath -Force }
        $zip = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
        try {
            foreach ($f in $bins[$i].Files) {
                # Beim Entpacken entsteht der Ordner "Local Voice AI" statt loser Dateien.
                $rel = $folderName + '/' + $f.FullName.Substring($root.Length + 1).Replace('\', '/')
                $level = if ($stored -contains $f.Extension.ToLowerInvariant() -or $f.Name -match '\.teil\d{3}$') {
                    [System.IO.Compression.CompressionLevel]::NoCompression
                } else { [System.IO.Compression.CompressionLevel]::Optimal }
                $null = [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $f.FullName, $rel, $level)
            }
        } finally { $zip.Dispose() }
        $len = (Get-Item $zipPath).Length
        if ($len -ge 2GB) { throw "$zipName ist $len Bytes gross (GitHub-Grenze 2 GiB)" }
        $hash = (Get-FileHash $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
        [System.IO.File]::WriteAllText("$zipPath.sha256", "$hash  $zipName`n", [System.Text.Encoding]::ASCII)
        Write-Host ("ZIP: {0} ({1:N1} MB)  SHA-256 {2}" -f $zipPath, ($len / 1MB), $hash)
        $zips += $zipPath
    }

    if ($SmokeTest) {
        # Alle Teile in denselben Ordner entpacken, wie es die Anleitung verlangt.
        $probe = Join-Path $stage 'probe'
        foreach ($z in $zips) { Expand-Archive -Path $z -DestinationPath $probe -Force }
        $probeRoot = Join-Path $probe $folderName
        $must = @('local-voice-ai.exe', 'portable', 'LIESMICH-PORTABLE.txt', 'resources/default_settings.json')
        if ($Offline) { $must += @('LIZENZEN-MODELLE.txt', 'Data/settings_store.seed.json') }
        foreach ($f in $must) {
            if (-not (Test-Path (Join-Path $probeRoot $f))) { throw "Rauchtest: $f fehlt im ZIP" }
        }
        # `--help` beendet sich vor jeder Tauri-Initialisierung (clap in main.rs), laedt aber
        # alle statisch gebundenen DLLs: fehlt eine, endet der Prozess mit 0xC0000135.
        $p = Start-Process -FilePath (Join-Path $probeRoot 'local-voice-ai.exe') -ArgumentList '--help' `
            -WorkingDirectory $probeRoot -PassThru -WindowStyle Hidden
        $null = $p.Handle
        if (-not $p.WaitForExit(60000)) { $p.Kill(); throw 'Rauchtest: --help haengt' }
        if ($p.ExitCode -ne 0) { throw ("Rauchtest: --help endete mit Exit-Code 0x{0:X8}" -f $p.ExitCode) }
        if ($Offline) {
            # Ein headless Lauf setzt geteilte Dateien zusammen (portable_parts.rs) und listet
            # die Spracherkennung; die vorinstallierten Modelle muessen als installiert gelten.
            $listOut = Join-Path $stage 'list.out.txt'
            $p = Start-Process -FilePath (Join-Path $probeRoot 'local-voice-ai.exe') -ArgumentList @('--list-models') `
                -WorkingDirectory $probeRoot -PassThru -NoNewWindow -RedirectStandardOutput $listOut -RedirectStandardError (Join-Path $stage 'list.err.txt')
            $null = $p.Handle
            if (-not $p.WaitForExit(15 * 60 * 1000)) { $p.Kill(); throw 'Rauchtest: --list-models haengt' }
            $list = Get-Content $listOut -Raw
            Write-Host $list
            if ($p.ExitCode -ne 0) { throw "Rauchtest: --list-models Exit-Code $($p.ExitCode)" }
            if ($list -notmatch '(?m)^\s*✓') { throw 'Rauchtest: kein Spracherkennungsmodell als installiert erkannt' }
            $left = @(Get-ChildItem (Join-Path $probeRoot 'Data') -Recurse -File -Filter '*.teile.json')
            if ($left.Count -gt 0) { throw "Rauchtest: nicht zusammengesetzt: $($left.FullName -join ', ')" }
        }
        Write-Host 'Rauchtest: bestanden.'
    }
}
finally {
    Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
}
