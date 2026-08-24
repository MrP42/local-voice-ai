<#
.SYNOPSIS
    Einmalige Probe: startet den kleinen fishaudio/s1-mini-TTS-Server aus der
    lokal installierten fish-speech-Codebase, wartet auf "healthy", schickt
    einen deutschen Testsatz durch den vorhandenen Client und berichtet
    Ladezeit, VRAM-Delta und TTS-Dauer. Kein App-Code wird beruehrt, keine
    Modelldateien werden heruntergeladen (das HF-gated Modell muss vorher
    manuell besorgt werden, siehe docs/probe-s1-mini.md).

.DESCRIPTION
    NUR ASCII in dieser Datei (Projektregel: Umlaute und Gedankenstriche
    brechen den Windows-PowerShell-5.1-Parser). Deutsche Kommentare daher mit
    ae/oe/ue/ss statt Umlauten.

    Ablauf: Vorbedingungen pruefen -> VRAM-Snapshot -> Server als
    Hintergrundprozess starten -> Health-Poll -> TTS-Testsatz -> VRAM-Snapshot
    -> Report -> Aufraeumen (Serverprozess samt Kindprozessen beenden).

    Kein Netzzugriff ausser lokalen Aufrufen an den selbst gestarteten Server
    auf 127.0.0.1. Der einmalige Modell-Download ist manuell (siehe Doku).

.PARAMETER FishDir
    Pfad zum lokalen fish-speech-Checkout (S2-Branch, uv-venv bereits gebaut).

.PARAMETER ModelDir
    Pfad zum bereits heruntergeladenen s1-mini-Checkpoint.

.PARAMETER Port
    Port fuer den Probe-Server. Bewusst anders als der App-eigene S2-Pro-Port
    (Standard 8080), damit beide nicht kollidieren.

.PARAMETER CheckOnly
    Trockenlauf: prueft nur die Vorbedingungen (FishDir, venv-Python,
    tools/api_server.py, tools/api_client.py, ModelDir, Codec-Datei) und endet
    VOR dem Serverstart. Kein Prozess wird gestartet, kein Netzzugriff.

.EXAMPLE
    .\probe-s1-mini.ps1 -CheckOnly

.EXAMPLE
    .\probe-s1-mini.ps1

.NOTES
    Exit-Codes: 0 = Erfolg (CheckOnly: Vorbedingungen ok; sonst: Server healthy
    und WAV erzeugt), 1 = Probe gelaufen, aber Server nicht healthy oder
    TTS-Test fehlgeschlagen, 3 = Vorbedingung fehlgeschlagen (Abbruch vor
    jedem Serverstart).
#>
[CmdletBinding()]
param(
    [string]$FishDir = 'C:\AI\fish-speech',
    [string]$ModelDir = 'C:\AI\models\fish-audio\s1-mini',
    [int]$Port = 8081,
    [switch]$CheckOnly
)

# Continue statt Stop: einzelne riskante Aufrufe sind gezielt mit try/catch
# bzw. -ErrorAction SilentlyContinue abgesichert (gleiches Muster wie
# m8-verify.ps1) - ein einzelner Fehlschlag soll eine klare Meldung geben,
# nicht das ganze Skript mit Stacktrace beenden.
$ErrorActionPreference = 'Continue'

function Write-Step {
    param([string]$Text)
    Write-Host $Text -ForegroundColor Cyan
}

function Fail {
    param([string]$Text, [int]$Code = 3)
    Write-Host ""
    Write-Host "[FEHLER] $Text" -ForegroundColor Red
    exit $Code
}

function Get-VramSnapshot {
    # Gibt $null zurueck, wenn nvidia-smi fehlt (z. B. kein NVIDIA-Treiber) -
    # der Aufrufer muss das behandeln, nicht abbrechen (Brief: "falls
    # nvidia-smi fehlt: Hinweis, weiter").
    $cmd = Get-Command 'nvidia-smi' -ErrorAction SilentlyContinue
    if (-not $cmd) { return $null }
    try {
        $raw = & nvidia-smi --query-gpu=index,name,memory.used,memory.total --format=csv,noheader,nounits 2>$null
    } catch {
        return $null
    }
    if (-not $raw) { return $null }
    $rows = @()
    foreach ($line in @($raw)) {
        $parts = $line -split ',\s*'
        if ($parts.Count -ge 4) {
            $rows += [pscustomobject]@{
                Index    = $parts[0]
                Name     = $parts[1]
                UsedMiB  = [int]$parts[2]
                TotalMiB = [int]$parts[3]
            }
        }
    }
    if ($rows.Count -eq 0) { return $null }
    return $rows
}

function Format-VramSnapshot {
    param($Snapshot)
    if (-not $Snapshot) { return 'nicht gemessen (nvidia-smi fehlt oder lieferte keine Daten)' }
    $parts = @()
    foreach ($row in $Snapshot) {
        $parts += ("GPU{0} {1}: {2}/{3} MiB" -f $row.Index, $row.Name, $row.UsedMiB, $row.TotalMiB)
    }
    return ($parts -join '; ')
}

function Get-VramUsedSum {
    param($Snapshot)
    if (-not $Snapshot) { return $null }
    return ($Snapshot | Measure-Object -Property UsedMiB -Sum).Sum
}

function Get-LogTail {
    param([string]$Path, [int]$Lines = 40)
    if (-not (Test-Path -LiteralPath $Path)) { return @("(Logdatei nicht gefunden: $Path)") }
    $content = @(Get-Content -LiteralPath $Path -Tail $Lines -ErrorAction SilentlyContinue)
    if ($content.Count -eq 0) { return @('(Logdatei ist leer)') }
    return $content
}

# ============================================================ a) Vorbedingungen
Write-Step "=== s1-mini-Probe: Vorbedingungen ==="

$pythonExe    = Join-Path $FishDir '.venv\Scripts\python.exe'
$serverScript = Join-Path $FishDir 'tools\api_server.py'
$clientScript = Join-Path $FishDir 'tools\api_client.py'

if (-not (Test-Path -LiteralPath $FishDir)) {
    Fail "FishDir nicht gefunden: $FishDir (Parameter -FishDir pruefen)"
}
Write-Host "[OK] FishDir        : $FishDir"

if (-not (Test-Path -LiteralPath $pythonExe)) {
    Fail ("venv-Python nicht gefunden: $pythonExe`n" +
          "         fish-speech-Installation im FishDir unvollstaendig? (uv sync noetig)")
}
Write-Host "[OK] venv-Python    : $pythonExe"

if (-not (Test-Path -LiteralPath $serverScript)) {
    Fail "tools\api_server.py nicht gefunden unter: $FishDir (falscher FishDir oder falscher Checkout?)"
}
Write-Host "[OK] tools/api_server.py : $serverScript"

if (-not (Test-Path -LiteralPath $clientScript)) {
    Fail "tools\api_client.py nicht gefunden unter: $FishDir (wird fuer den TTS-Testsatz gebraucht)"
}
Write-Host "[OK] tools/api_client.py : $clientScript"

if (-not (Test-Path -LiteralPath $ModelDir)) {
    Fail ("ModelDir nicht gefunden: $ModelDir`n" +
          "         Download-Anleitung: siehe docs/probe-s1-mini.md (einmalig, manuell, HF-Login noetig).")
}
$modelFiles = @(Get-ChildItem -LiteralPath $ModelDir -File -Recurse -ErrorAction SilentlyContinue)
if ($modelFiles.Count -eq 0) {
    Fail ("ModelDir ist vorhanden, aber leer: $ModelDir`n" +
          "         Download-Anleitung: siehe docs/probe-s1-mini.md (einmalig, manuell, HF-Login noetig).")
}
Write-Host ("[OK] ModelDir       : $ModelDir ({0} Datei(en))" -f $modelFiles.Count)

# Codec-Datei per Muster suchen (Dateiname unterscheidet sich je nach
# fish-speech-Version): zuerst codec.pth, dann aeltere firefly-gan-vq-*.pth.
$codecCandidates = @()
$codecCandidates += @(Get-ChildItem -LiteralPath $ModelDir -Filter 'codec.pth' -File -ErrorAction SilentlyContinue)
$codecCandidates += @(Get-ChildItem -LiteralPath $ModelDir -Filter 'firefly-gan-vq-*.pth' -File -ErrorAction SilentlyContinue)
$codecCandidates = @($codecCandidates | Sort-Object FullName -Unique)

if ($codecCandidates.Count -eq 0) {
    $allPth = @(Get-ChildItem -LiteralPath $ModelDir -Filter '*.pth' -File -ErrorAction SilentlyContinue)
    $list = if ($allPth.Count -gt 0) { ($allPth.Name -join ', ') } else { '(keine .pth-Dateien im ModelDir gefunden)' }
    Fail ("Kein Codec-Checkpoint gefunden (erwartet: codec.pth oder firefly-gan-vq-*.pth).`n" +
          "         Gefundene .pth-Kandidaten im ModelDir: $list")
}
$codecFile = $codecCandidates[0]
Write-Host "[OK] Codec-Datei    : $($codecFile.Name)"

if ($CheckOnly) {
    Write-Host ""
    Write-Host "CHECKONLY: alle Vorbedingungen erfuellt, Server wird NICHT gestartet." -ForegroundColor Green
    exit 0
}

# ============================================================ b) VRAM vorher
Write-Step "`n=== VRAM-Snapshot vor dem Start ==="
$vramBefore = Get-VramSnapshot
if (-not $vramBefore) {
    Write-Host "[HINWEIS] nvidia-smi nicht gefunden oder ohne Ausgabe - VRAM-Messung wird ausgelassen." -ForegroundColor Yellow
} else {
    Write-Host (Format-VramSnapshot $vramBefore)
}

# ============================================================ c) Server starten
Write-Step "`n=== Server starten (Port $Port) ==="

$stamp   = Get-Date -Format 'yyyyMMdd-HHmmss'
$logDir  = Join-Path $env:TEMP ("s1-mini-probe-" + $stamp)
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$stderrLog  = Join-Path $logDir 'server-stderr.log'
$stdoutLog  = Join-Path $logDir 'server-stdout.log'
$clientLog  = Join-Path $logDir 'client-output.log'
$reportFile = Join-Path $logDir 'report.txt'
$wavOutBase = Join-Path $logDir 'probe-tts'   # api_client.py haengt ".<format>" an

# HF_HUB_DISABLE_TELEMETRY: gleiche Konvention wie start-fish-speech.ps1 im
# FishDir - kein Netzzugriff ausser dem, was der lokale Server/Client selbst
# gegen 127.0.0.1 macht.
$env:HF_HUB_DISABLE_TELEMETRY = '1'

$serverArgsRaw = @(
    $serverScript,
    '--llama-checkpoint-path', $ModelDir,
    '--decoder-checkpoint-path', $codecFile.FullName,
    '--listen', "127.0.0.1:$Port"
)
# Nicht-Flag-Werte quoten (Pfade koennten Leerzeichen enthalten) - gleiches
# Muster wie in m8-verify.ps1's Invoke-App.
$q = '"'
$serverArgs = @()
foreach ($a in $serverArgsRaw) {
    if ($a -like '--*') { $serverArgs += $a } else { $serverArgs += ($q + $a + $q) }
}

Write-Host "Kommando: $pythonExe $($serverArgs -join ' ')"
Write-Host "Log (stderr): $stderrLog"
Write-Host "Log (stdout): $stdoutLog"

$proc = $null
try {
    $proc = Start-Process -FilePath $pythonExe -ArgumentList $serverArgs -WorkingDirectory $FishDir `
        -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdoutLog -RedirectStandardError $stderrLog
} catch {
    Fail "Server liess sich nicht starten: $($_.Exception.Message)" 1
}
Write-Host "Server-Prozess gestartet: PID $($proc.Id)"

# ============================================================ d) Health-Poll
Write-Step "`n=== Health-Poll (http://127.0.0.1:$Port/v1/health) ==="

$healthUrl    = "http://127.0.0.1:$Port/v1/health"
$timeoutSec   = 240
$pollInterval = 2
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$healthy = $false
$lastPollError = $null

while ($sw.Elapsed.TotalSeconds -lt $timeoutSec) {
    if ($proc.HasExited) {
        Write-Host ("[FEHLER] Serverprozess ist bereits beendet (Exit-Code {0})." -f $proc.ExitCode) -ForegroundColor Red
        break
    }
    try {
        $resp = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 5 -ErrorAction Stop
        if ($resp.status -eq 'ok') {
            $healthy = $true
            break
        }
    } catch {
        $lastPollError = $_.Exception.Message
    }
    Start-Sleep -Seconds $pollInterval
}
$sw.Stop()
$loadSeconds = [math]::Round($sw.Elapsed.TotalSeconds, 1)

if ($healthy) {
    Write-Host "[OK] Server healthy nach $loadSeconds s" -ForegroundColor Green
} else {
    Write-Host ("[FEHLER] Server wurde in {0} s nicht healthy (letzter Poll-Fehler: {1})." -f $timeoutSec, $lastPollError) -ForegroundColor Red
    Write-Host "`nLetzte 40 Zeilen stderr ($stderrLog):" -ForegroundColor Yellow
    Get-LogTail $stderrLog 40 | ForEach-Object { Write-Host "  $_" }
    Write-Host "`nLetzte 40 Zeilen stdout ($stdoutLog):" -ForegroundColor Yellow
    Get-LogTail $stdoutLog 40 | ForEach-Object { Write-Host "  $_" }
}

# ============================================================ e) TTS-Test
$ttsSeconds = $null
$wavPath    = $null
$wavSize    = $null
$ttsError   = $null

if ($healthy) {
    Write-Step "`n=== TTS-Testsatz ==="
    # ASCII-Regel gilt fuer diese Datei, daher deutscher Testsatz ohne
    # echte Umlaut-Zeichen (ae/oe/ue statt Umlaut). Das (excited)-Tag testet
    # die inline-Emotionsmarkierung von s1-mini.
    $testText = 'Hallo Patrick, dies ist ein kurzer deutscher Testsatz (excited) fuer die Sprachausgabe von s1-mini.'

    $noPlaySupported = $false
    try {
        $helpText = & $pythonExe $clientScript --help 2>&1
        if (($helpText | Out-String) -match '--no-play') { $noPlaySupported = $true }
    } catch {
        Write-Host "[HINWEIS] '--help' des Clients fehlgeschlagen, versuche ohne --no-play." -ForegroundColor Yellow
    }
    Write-Host ("Client unterstuetzt --no-play: {0}" -f $noPlaySupported)

    $clientArgs = @(
        $clientScript,
        '-u', "http://127.0.0.1:$Port/v1/tts",
        '-t', $testText,
        '-o', $wavOutBase
    )
    if ($noPlaySupported) { $clientArgs += '--no-play' }

    $ttsSw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        & $pythonExe @clientArgs *> $clientLog
        $ttsExit = $LASTEXITCODE
    } catch {
        $ttsExit = -1
        $ttsError = $_.Exception.Message
    }
    $ttsSw.Stop()
    $ttsSeconds = [math]::Round($ttsSw.Elapsed.TotalSeconds, 1)

    $wavPath = "$wavOutBase.wav"
    if (Test-Path -LiteralPath $wavPath) {
        $wavSize = (Get-Item -LiteralPath $wavPath).Length
        Write-Host "[OK] WAV erzeugt: $wavPath ($wavSize Bytes) in $ttsSeconds s" -ForegroundColor Green
    } else {
        if (-not $ttsError) {
            $ttsError = "Client-Exit-Code $ttsExit, keine WAV-Datei unter $wavPath gefunden. Siehe $clientLog."
        }
        Write-Host "[FEHLER] $ttsError" -ForegroundColor Red
    }
} else {
    Write-Host "`n=== TTS-Testsatz uebersprungen (Server nicht healthy) ===" -ForegroundColor Yellow
}

# ============================================================ f) VRAM nachher
Write-Step "`n=== VRAM-Snapshot nach dem Laden ==="
$vramAfter = Get-VramSnapshot
if (-not $vramAfter) {
    Write-Host "[HINWEIS] nvidia-smi nicht gefunden oder ohne Ausgabe - VRAM-Messung wird ausgelassen." -ForegroundColor Yellow
} else {
    Write-Host (Format-VramSnapshot $vramAfter)
}

$vramDeltaText = 'nicht gemessen (nvidia-smi fehlt)'
$beforeSum = Get-VramUsedSum $vramBefore
$afterSum  = Get-VramUsedSum $vramAfter
if (($null -ne $beforeSum) -and ($null -ne $afterSum)) {
    $delta = $afterSum - $beforeSum
    $vramDeltaText = "$delta MiB (vorher $beforeSum MiB, nachher $afterSum MiB, ueber alle GPUs summiert)"
}

# ============================================================ h) Aufraeumen
Write-Step "`n=== Aufraeumen ==="
if ($proc -and (-not $proc.HasExited)) {
    Write-Host ("Beende Serverprozess PID {0} samt Kindprozessen (taskkill /T /F) ..." -f $proc.Id)
    try {
        & taskkill /PID $proc.Id /T /F | Out-Null
    } catch {
        Write-Host "[WARNUNG] taskkill fehlgeschlagen: $($_.Exception.Message)" -ForegroundColor Yellow
    }
} elseif ($proc) {
    Write-Host ("Serverprozess war bereits beendet (Exit-Code {0})." -f $proc.ExitCode)
}
# Kurze Pause, damit der Treiber den VRAM tatsaechlich freigegeben hat, bevor
# die Gegenprobe laeuft.
Start-Sleep -Seconds 2
$vramAfterCleanup = Get-VramSnapshot
$cleanupSum = Get-VramUsedSum $vramAfterCleanup

$zombieHint = 'nicht geprueft (nvidia-smi fehlt)'
if (($null -ne $beforeSum) -and ($null -ne $cleanupSum)) {
    $residual = $cleanupSum - $beforeSum
    if ($residual -gt 200) {
        $zombieHint = ("MOEGLICHER VRAM-ZOMBIE: nach dem Beenden sind noch {0} MiB mehr belegt als vor dem Start. " -f $residual) +
            "Pruefen mit 'nvidia-smi'; im Zweifel verbliebene python.exe-Prozesse im Task-Manager beenden " +
            "(Vorsicht: nicht pauschal alle python.exe killen, falls andere Python-Prozesse laufen)."
    } else {
        $zombieHint = "kein Hinweis auf VRAM-Zombie (Restdifferenz $residual MiB)."
    }
    Write-Host $zombieHint
} else {
    Write-Host "[HINWEIS] $zombieHint" -ForegroundColor Yellow
}

# ============================================================ g) Report
$loadedText = if ($healthy) { 'ja' } else { 'nein' }
$wavSizeText = if ($wavSize) { "$wavSize Bytes" } else { 'FEHLT' }

$report = @()
$report += "s1-mini-Probe-Report"
$report += "===================="
$report += "Zeitpunkt        : $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
$report += "FishDir          : $FishDir"
$report += "ModelDir         : $ModelDir"
$report += "Codec-Datei      : $($codecFile.Name)"
$report += "Port             : $Port"
$report += "Server geladen   : $loadedText"
$report += "Ladezeit         : $loadSeconds s (Timeout $timeoutSec s)"
$report += "VRAM-Delta       : $vramDeltaText"
if ($healthy) {
    $report += "TTS-Dauer        : $ttsSeconds s"
    $report += "TTS-WAV          : $wavPath ($wavSizeText)"
    if ($ttsError) { $report += "TTS-Fehler       : $ttsError" }
} else {
    $report += "TTS-Test         : uebersprungen (Server nicht healthy)"
}
$report += "Log (stderr)     : $stderrLog"
$report += "Log (stdout)     : $stdoutLog"
$report += "VRAM-Zombie-Check: $zombieHint"
if (-not $healthy) {
    $report += ""
    $report += "Letzte 40 Zeilen stderr (dort steht der Ladefehler):"
    $report += (Get-LogTail $stderrLog 40)
}

($report -join "`n") | Out-File -LiteralPath $reportFile -Encoding UTF8

Write-Host ""
Write-Host "=== Ergebnis ===" -ForegroundColor Cyan
$report | ForEach-Object { Write-Host $_ }
Write-Host ""
Write-Host "Report-Datei: $reportFile" -ForegroundColor Cyan

if ($healthy -and $wavSize -gt 0) { exit 0 }
exit 1
