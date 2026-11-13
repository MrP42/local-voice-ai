# Tests zu scripts/build-installer-modelle.ps1 (lva#574): -OutDir darf nie fremde Dateien loeschen.
# Die Pruefung laeuft vor jedem Download und vor der EXE-Suche; es wird nichts geladen.
#
# Aufruf aus apps/local-voice (PowerShell 7): pwsh scripts/test-build-installer-modelle.ps1
# Exit-Code 0 nur, wenn alle Faelle bestehen.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script = Join-Path $PSScriptRoot 'build-installer-modelle.ps1'
$appDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$pwsh = (Get-Process -Id $PID).Path
$failed = 0

function Invoke-Build([string]$OutDir, [string]$Cwd) {
    $p = Start-Process -FilePath $pwsh -ArgumentList @('-NoProfile', '-File', $script, '-OutDir', $OutDir, '-Exe', 'gibt-es-nicht.exe') `
        -WorkingDirectory $Cwd -PassThru -NoNewWindow -Wait `
        -RedirectStandardOutput (Join-Path $tmp 'out.txt') -RedirectStandardError (Join-Path $tmp 'err.txt')
    return @{ Code = $p.ExitCode; Err = (Get-Content (Join-Path $tmp 'err.txt') -Raw) + (Get-Content (Join-Path $tmp 'out.txt') -Raw) }
}

function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name`n     $Detail"; $script:failed++ }
}

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lva-test-574-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
    # 1. Vorhandener Ordner mit fremder Datei: Abbruch, Datei bleibt.
    $foreign = Join-Path $tmp 'fremd'
    New-Item -ItemType Directory -Force $foreign | Out-Null
    $sentinel = Join-Path $foreign 'wichtig.txt'
    Set-Content $sentinel 'nicht loeschen'
    $r = Invoke-Build $foreign $tmp
    Check 'fremder Ordner: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Marke') $r.Err
    Check 'fremder Ordner: Datei bleibt' (Test-Path $sentinel) 'Sentinel geloescht'

    # 2. Arbeitsverzeichnis als Ziel (-OutDir .): Abbruch, Datei bleibt.
    $cwdSentinel = Join-Path $tmp 'arbeitsordner.txt'
    Set-Content $cwdSentinel 'nicht loeschen'
    $r = Invoke-Build '.' $tmp
    Check 'Arbeitsverzeichnis: Abbruch' ($r.Code -ne 0) $r.Err
    Check 'Arbeitsverzeichnis: Datei bleibt' (Test-Path $cwdSentinel) 'Sentinel geloescht'

    # 3. App-Verzeichnis und ein Vorfahr davon als Ziel: Abbruch vor jeder Aenderung.
    $r = Invoke-Build $appDir $tmp
    Check 'App-Verzeichnis: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'enthaelt') $r.Err
    $r = Invoke-Build (Split-Path $appDir -Parent) $tmp
    Check 'Vorfahr des App-Verzeichnisses: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'enthaelt') $r.Err
    Check 'App-Verzeichnis unversehrt' (Test-Path (Join-Path $appDir 'package.json')) 'package.json fehlt'

    # 4. Laufwerkswurzel: Abbruch.
    $root = [System.IO.Path]::GetPathRoot($tmp)
    $r = Invoke-Build $root $tmp
    Check 'Laufwerkswurzel: Abbruch' ($r.Code -ne 0) $r.Err

    # 5. Fruehere Nutzlast (Marke vorhanden) und leerer Ordner: Pruefung laesst sie zu, das Skript
    #    scheitert danach erst an der fehlenden EXE, ohne etwas geloescht zu haben.
    $own = Join-Path $tmp 'eigene'
    New-Item -ItemType Directory -Force $own | Out-Null
    Set-Content (Join-Path $own '.lva-vorinstalliert-nutzlast') 'x'
    $r = Invoke-Build $own $tmp
    Check 'eigene Nutzlast: Pruefung bestanden, Abbruch erst an der EXE' ($r.Code -ne 0 -and $r.Err -match 'EXE nicht gefunden') $r.Err
    $empty = Join-Path $tmp 'leer'
    New-Item -ItemType Directory -Force $empty | Out-Null
    $r = Invoke-Build $empty $tmp
    Check 'leerer Ordner: Pruefung bestanden' ($r.Code -ne 0 -and $r.Err -match 'EXE nicht gefunden') $r.Err
}
finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed Fall/Faelle fehlgeschlagen."; exit 1 }
Write-Host 'Alle Faelle bestanden.'
