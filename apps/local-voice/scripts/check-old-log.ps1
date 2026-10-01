<#
.SYNOPSIS
    Sucht alte Logdateien mit Klartext-Diktaten (STREAMDIAG, Issue #6). ASCII only.

.DESCRIPTION
    Bis 2026-08-17 schrieb die App bei debug_mode=true vollstaendige Diktate im
    Klartext in die Logdatei (DECISIONS.md D9). Dieses Skript findet solche
    Dateien, ohne je einen Inhalt auszugeben:

      * STREAMDIAG-Klartext: Zeilen "STREAMDIAG committed(len=N)=..." und
        "STREAMDIAG delta(len=N)=..." (die Zaehler-Zeilen "committed_len=" und
        "delta_len=" enthalten keinen Text und zaehlen nicht).
      * "Transcription result: <Text>" (ganze Diktate; "... N chars" und
        "... is empty" zaehlen nicht).

    Gesucht wird in %LOCALAPPDATA%\<identifier>\logs\ fuer den aktuellen
    Identifier aus src-tauri\tauri.conf.json UND den frueheren Identifier
    de.wolffappliedai.sprechstift, jeweils in handy.log* und *.log*.

    Ausgegeben werden nur Pfad, Groesse, Datum und Trefferzahlen sowie der
    ausfuellfertige Loeschbefehl. Geloescht wird NUR mit -Delete, und dann nur
    Dateien mit Klartext-Treffern, jede nach Rueckfrage (Standard-Bestaetigung
    von PowerShell; -Force unterdrueckt sie, -WhatIf zeigt nur an).

    Exit 0 = kein Klartext gefunden (oder erfolgreich geloescht),
    Exit 1 = Klartext gefunden und nicht geloescht.

.PARAMETER LogDir
    Verzeichnisse mit Logdateien statt der Standardorte (fuer Tests).

.PARAMETER Delete
    Loescht die gefundenen Dateien mit Klartext-Treffern (nach Rueckfrage).

.PARAMETER Force
    Mit -Delete: ohne Rueckfrage loeschen.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\check-old-log.ps1

.EXAMPLE
    pwsh -File apps\local-voice\scripts\check-old-log.ps1 -Delete
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'High')]
param(
    [string[]]$LogDir,
    [switch]$Delete,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
# -Force = ohne Rueckfrage loeschen (in Windows PowerShell 5.1 ist -Confirm:$false per -File nicht nutzbar).
if ($Force) { $ConfirmPreference = 'None' }

# Zwei Muster, getrennt gezaehlt. Beide verlangen den Text-Teil, nicht den Zaehler.
$StreamDiagRegex = 'STREAMDIAG (committed|delta)\(len=\d+\)='
$ResultRegex = 'Transcription result: (?!is empty)(?!\d+ chars)\S'

function Get-LogDirs {
    $ids = New-Object System.Collections.Generic.List[string]
    $conf = Join-Path $PSScriptRoot '..\src-tauri\tauri.conf.json'
    try {
        $id = (Get-Content -LiteralPath $conf -Raw | ConvertFrom-Json).identifier
        if ($id) { $ids.Add([string]$id) }
    } catch { }
    foreach ($fallback in 'de.wolffappliedai.localvoiceai', 'de.wolffappliedai.sprechstift') {
        if (-not $ids.Contains($fallback)) { $ids.Add($fallback) }
    }
    foreach ($i in $ids) { Join-Path $env:LOCALAPPDATA (Join-Path $i 'logs') }
}

function Measure-LogFile {
    param([string]$Path)
    $stream = 0; $result = 0; $lines = 0
    # FileShare.ReadWrite: die laufende App haelt ihre aktuelle Logdatei offen.
    $fs = New-Object System.IO.FileStream($Path, [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
    $reader = New-Object System.IO.StreamReader($fs, [System.Text.Encoding]::UTF8, $true)
    try {
        while ($null -ne ($line = $reader.ReadLine())) {
            $lines++
            if ($line -match $StreamDiagRegex) { $stream++ }
            if ($line -match $ResultRegex) { $result++ }
        }
    } finally { $reader.Dispose() }
    [pscustomobject]@{ Lines = $lines; StreamDiag = $stream; Result = $result }
}

$dirs = if ($LogDir) { $LogDir } else { @(Get-LogDirs) }
$found = @()
$scanned = 0

foreach ($dir in $dirs) {
    if (-not (Test-Path -LiteralPath $dir)) {
        Write-Host ("Kein Logordner: {0}" -f $dir) -ForegroundColor DarkGray
        continue
    }
    $files = Get-ChildItem -LiteralPath $dir -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like 'handy.log*' -or $_.Name -like '*.log*' } |
        Sort-Object FullName -Unique
    foreach ($f in $files) {
        $scanned++
        try { $m = Measure-LogFile -Path $f.FullName }
        catch {
            Write-Host ("Nicht lesbar: {0} ({1})" -f $f.FullName, $_.Exception.Message) -ForegroundColor Yellow
            continue
        }
        $hits = $m.StreamDiag + $m.Result
        $tag = if ($hits -gt 0) { 'KLARTEXT' } else { 'sauber  ' }
        Write-Host ("[{0}] {1}" -f $tag, $f.FullName) -ForegroundColor $(if ($hits -gt 0) { 'Red' } else { 'Green' })
        Write-Host ("           {0:N0} KB, {1:yyyy-MM-dd HH:mm}, {2} Zeilen; STREAMDIAG-Klartext: {3}; Diktat-Zeilen (Transcription result): {4}" -f `
                ($f.Length / 1KB), $f.LastWriteTime, $m.Lines, $m.StreamDiag, $m.Result)
        if ($hits -gt 0) {
            $found += [pscustomobject]@{ Path = $f.FullName; Hits = $hits }
        }
    }
}

Write-Host ""
Write-Host ("{0} Logdatei(en) geprueft, {1} mit Klartext-Treffern. Inhalte werden nie ausgegeben." -f $scanned, @($found).Count)

if (@($found).Count -eq 0) {
    Write-Host "Nichts zu tun." -ForegroundColor Green
    exit 0
}

Write-Host ""
Write-Host "Loeschbefehl (zum Kopieren):" -ForegroundColor Cyan
foreach ($x in $found) {
    Write-Host ("Remove-Item -LiteralPath '{0}'" -f $x.Path.Replace("'", "''"))
}
Write-Host ("Oder mit Rueckfrage durch dieses Skript:  pwsh -File `"{0}`" -Delete" -f $PSCommandPath)

if (-not $Delete) {
    Write-Host "Nichts geloescht (ohne -Delete wird nur gelesen)." -ForegroundColor Yellow
    exit 1
}

$failed = 0
foreach ($x in $found) {
    if ($PSCmdlet.ShouldProcess($x.Path, ("Logdatei mit {0} Klartext-Treffern loeschen" -f $x.Hits))) {
        try {
            Remove-Item -LiteralPath $x.Path -Force
            Write-Host ("Geloescht: {0}" -f $x.Path) -ForegroundColor Green
        } catch {
            Write-Host ("Nicht geloescht: {0} ({1})" -f $x.Path, $_.Exception.Message) -ForegroundColor Red
            $failed++
        }
    } else {
        Write-Host ("Uebersprungen: {0}" -f $x.Path) -ForegroundColor Yellow
        $failed++
    }
}
if ($failed -gt 0) { exit 1 }
exit 0
