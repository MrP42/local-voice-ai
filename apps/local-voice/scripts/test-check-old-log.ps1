<#
.SYNOPSIS
    Prueft scripts\check-old-log.ps1 gegen eine Fixture im Temp-Ordner (Issue #6). ASCII only.

.DESCRIPTION
    Kein Zugriff auf echte Logs. Exit 0 = alles gruen, sonst 1.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\test-check-old-log.ps1
#>
$ErrorActionPreference = 'Continue'   # 5.1: stderr nativer Prozesse sonst terminierend
Set-StrictMode -Version Latest

$script = Join-Path $PSScriptRoot 'check-old-log.ps1'
$engines = @()
foreach ($n in 'pwsh.exe', 'powershell.exe') {
    $c = Get-Command $n -ErrorAction SilentlyContinue
    if ($c) { $engines += $c.Source }
}
$failures = 0

function Check {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    if ($Ok) { Write-Host "PASS  $Name" -ForegroundColor Green }
    else { Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red; $script:failures++ }
}

foreach ($pwsh in $engines) {
Write-Host ("--- Engine: {0}" -f $pwsh) -ForegroundColor Cyan
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("lva-oldlog-" + [Guid]::NewGuid().ToString('N'))
$logs = Join-Path $tmp 'logs'
New-Item -ItemType Directory -Force -Path $logs | Out-Null
try {
    $secret = 'GEHEIMWORT' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    $dirty = Join-Path $logs 'handy.log.alt'
    $clean = Join-Path $logs 'handy.log'

    # 3 STREAMDIAG-Klartextzeilen (2 committed, 1 delta), 2 Zaehler-Zeilen (zaehlen nicht),
    # 1 Diktatzeile, 2 Nicht-Diktat-Zeilen (zaehlen nicht).
    @(
        '[INFO] Start',
        "[INFO] STREAMDIAG committed(len=11)=`"$secret eins`" | already=0 | append_only=true",
        "[INFO] STREAMDIAG committed(len=11)=`"$secret zwei`" | already=11 | append_only=true",
        "[INFO] STREAMDIAG delta(len=6)=`"$secret`"",
        '[DEBUG] STREAMDIAG committed_len=11 already=0 append_only=true',
        '[DEBUG] STREAMDIAG delta_len=6',
        "[INFO] Transcription result: $secret drei",
        '[INFO] Transcription result: 42 chars',
        '[INFO] Transcription result is empty'
    ) | Set-Content -LiteralPath $dirty -Encoding UTF8
    @('[INFO] Start', '[DEBUG] STREAMDIAG delta_len=6', '[INFO] Transcription result: 42 chars') |
        Set-Content -LiteralPath $clean -Encoding UTF8

    # ---- 1. lesender Lauf -----------------------------------------------
    $out = & $pwsh -NoProfile -File $script -LogDir $logs 2>&1 | Out-String
    $code = $LASTEXITCODE
    Check 'Lesen: Exit 1 bei Klartext' ($code -eq 1) "Exit $code"
    Check 'Lesen: STREAMDIAG-Klartext = 3' ($out -match 'STREAMDIAG-Klartext: 3;') $out
    Check 'Lesen: Diktat-Zeilen = 1' ($out -match 'Diktat-Zeilen \(Transcription result\): 1') $out
    Check 'Lesen: saubere Datei ohne Treffer' ($out -match 'STREAMDIAG-Klartext: 0; Diktat-Zeilen \(Transcription result\): 0') $out
    Check 'Lesen: Zusammenfassung 2 geprueft, 1 Treffer' ($out -match '2 Logdatei\(en\) gepr\S+, 1 mit Klartext') $out
    Check 'Lesen: Inhalte werden NICHT ausgegeben' ($out -notmatch [regex]::Escape($secret)) 'Geheimwort im Output'
    Check 'Lesen: Loeschbefehl ausgegeben' ($out -match [regex]::Escape("Remove-Item -LiteralPath '$dirty'")) $out
    Check 'Lesen: nichts geloescht' ((Test-Path $dirty) -and (Test-Path $clean)) 'Datei fehlt'

    # ---- 2. -Delete -WhatIf loescht nichts -------------------------------
    $out = & $pwsh -NoProfile -File $script -LogDir $logs -Delete -WhatIf 2>&1 | Out-String
    Check '-Delete -WhatIf: Datei bleibt' (Test-Path $dirty) $out

    # ---- 3. -Delete ohne Bestaetigung moeglich (nicht-interaktiv) scheitert nicht still
    # In einer nicht-interaktiven Sitzung kann die Rueckfrage nicht beantwortet werden:
    # die Datei darf dann nicht verschwinden.
    $out = & $pwsh -NoProfile -NonInteractive -File $script -LogDir $logs -Delete 2>&1 | Out-String
    Check '-Delete ohne -Force nicht-interaktiv: Datei bleibt' (Test-Path $dirty) $out

    # ---- 4. -Delete -Force loescht NUR die Klartext-Datei --------
    $out = & $pwsh -NoProfile -File $script -LogDir $logs -Delete -Force 2>&1 | Out-String
    $code = $LASTEXITCODE
    Check 'Delete: Exit 0' ($code -eq 0) "Exit $code"
    Check 'Delete: Klartext-Datei weg' (-not (Test-Path $dirty)) $out
    Check 'Delete: saubere Datei bleibt' (Test-Path $clean) $out

    # ---- 5. danach: sauber ----------------------------------------------
    $out = & $pwsh -NoProfile -File $script -LogDir $logs 2>&1 | Out-String
    Check 'Danach: Exit 0 und "Nichts zu tun"' (($LASTEXITCODE -eq 0) -and ($out -match 'Nichts zu tun')) $out

    # ---- 6. kein Logordner ------------------------------------------------
    $out = & $pwsh -NoProfile -File $script -LogDir (Join-Path $tmp 'gibt-es-nicht') 2>&1 | Out-String
    Check 'Fehlender Ordner: Exit 0' ($LASTEXITCODE -eq 0) $out
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
}

if ($failures -gt 0) { Write-Host "$failures Pruefung(en) fehlgeschlagen" -ForegroundColor Red; exit 1 }
Write-Host 'Alles gruen' -ForegroundColor Green
exit 0
