#Requires -Version 7.0
<#
.SYNOPSIS
    Prueft die Hilfsfunktionen und den Preflight des M8-Harness (Issue #15).

.DESCRIPTION
    ASCII only. Kein cargo-Lauf, kein Harness-Lauf mit der echten App; dauert
    wenige Sekunden.
      1. Remove-Segments serialisiert ein Feld mit einem Element als [2], nicht
         als [{}] (Ganzzahlen und Texte aus der Pipeline sind PSObject-umhuellt
         und galten als pscustomobject).
      2. Test-EmbeddedFrontend erkennt eine GUI-taugliche EXE (eingebettetes Asset
         index-<hash>.js; ein localhost:1420 daneben, etwa die Icon-URL der
         Konfiguration, stoert nicht), eine Dev-EXE (kein Asset) und eine leere,
         auch wenn der Marker ueber eine Lesegrenze hinweg liegt.
      3. Der Preflight von m8-verify.ps1 warnt VOR dem Lauf bei einer EXE ohne
         eingebettetes Frontend, bricht aber nicht ab (headless bleibt moeglich),
         schweigt bei einer GUI-tauglichen und meldet eine fehlende EXE mit Exit 2.
    Exit 0 = alles gruen, sonst 1.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\test-m8-verify.ps1
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$here = $PSScriptRoot
$lib = Join-Path $here 'lib\m8-harness.ps1'
$harness = Join-Path $here 'm8-verify.ps1'
$pwsh = (Get-Process -Id $PID).Path
$failures = 0

function Check {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    if ($Ok) { Write-Host "PASS  $Name" -ForegroundColor Green }
    else { Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red; $script:failures++ }
}

. $lib

# ---- 1. Remove-Segments ----------------------------------------------------
$data = '{"status":"ready","channels":[2],"list":[1,2],"words":["a"],"segments":[{"text":"geheim"}],"nested":{"one":[7]}}' |
    ConvertFrom-Json
$json = (Remove-Segments $data) | ConvertTo-Json -Depth 6 -Compress
Check 'Remove-Segments: ein Element bleibt [2]' ($json -match '"channels":\[2\]') $json
Check 'Remove-Segments: Ganzzahlen bleiben Ganzzahlen' ($json -match '"list":\[1,2\]') $json
Check 'Remove-Segments: Texte bleiben Texte' ($json -match '"words":\["a"\]') $json
Check 'Remove-Segments: verschachtelt' ($json -match '"one":\[7\]') $json
Check 'Remove-Segments: keine leeren Objekte' ($json -notmatch '\{\}') $json
Check 'Remove-Segments: Segmenttexte fallen weg' ($json -notmatch 'geheim' -and $json -notmatch '"segments"') $json
Check 'Remove-Segments: $null bleibt $null' ($null -eq (Remove-Segments $null))

# ---- 2. Test-EmbeddedFrontend -------------------------------------------
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("lva-m8verify-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $rng = [Random]::new(42)
    function New-FakeExe {
        param([string]$Name, [string]$Marker = '', [int]$Pad = 2048)
        $path = Join-Path $tmp $Name
        $bytes = New-Object byte[] $Pad
        $rng.NextBytes($bytes)
        # Zufallsbytes duerfen den Marker nicht zufaellig bilden; in ASCII-Bereich gedrueckt waere das moeglich, so nicht.
        $tail = [Text.Encoding]::ASCII.GetBytes($Marker)
        [IO.File]::WriteAllBytes($path, [byte[]]($bytes + $tail))
        return $path
    }

    $gui = New-FakeExe 'gui.exe' '/assets/index-Ab12Cd34Ef.js'
    $r = Test-EmbeddedFrontend -Exe $gui
    Check 'GUI-taugliche EXE wird erkannt' ($null -ne $r -and $r.GuiCapable -and $r.HasAsset -and -not $r.HasDevUrl) ($r | Out-String)

    $dev = New-FakeExe 'dev.exe' 'devUrl=http://localhost:1420/'
    $r = Test-EmbeddedFrontend -Exe $dev
    Check 'Dev-EXE (localhost:1420, kein Asset) ist nicht GUI-tauglich' ($null -ne $r -and -not $r.GuiCapable -and $r.HasDevUrl -and -not $r.HasAsset) ($r | Out-String)

    # Wie die installierte, funktionierende EXE: Asset UND die Icon-URL aus der Konfiguration.
    $both = New-FakeExe 'both.exe' 'http://localhost:1420/icons/128x128.png /assets/index-Ab12Cd34Ef.js'
    $r = Test-EmbeddedFrontend -Exe $both
    Check 'Asset UND localhost:1420 (Icon-URL): GUI-tauglich, der Dev-Marker entscheidet nicht' ($null -ne $r -and $r.GuiCapable -and $r.HasDevUrl -and $r.HasAsset) ($r | Out-String)

    $none = New-FakeExe 'none.exe' ''
    $r = Test-EmbeddedFrontend -Exe $none
    Check 'weder Marker noch Asset: nicht GUI-tauglich, mit Grund' ($null -ne $r -and -not $r.GuiCapable -and $r.Reason -ne '') ($r | Out-String)

    # Marker genau ueber eine 4-MB-Lesegrenze: die Ueberlappung muss ihn finden.
    $edge = New-FakeExe 'edge.exe' '/assets/index-Zz99Yy88Xx.js' (4MB - 10)
    $r = Test-EmbeddedFrontend -Exe $edge
    Check 'Asset-Name ueber der Lesegrenze wird gefunden' ($null -ne $r -and $r.HasAsset) ($r | Out-String)

    $r = Test-EmbeddedFrontend -Exe (Join-Path $tmp 'gibt-es-nicht.exe')
    Check 'fehlende EXE: nicht vorhanden, nicht GUI-tauglich' ($null -ne $r -and -not $r.Exists -and -not $r.GuiCapable) ($r | Out-String)

    # ---- 3. Preflight im Harness ---------------------------------------
    $sandboxTemp = Join-Path $tmp 'temp'
    New-Item -ItemType Directory -Force -Path $sandboxTemp | Out-Null
    function Invoke-Preflight {
        param([string]$Exe)
        $oldTemp = $env:TEMP
        try {
            $env:TEMP = $sandboxTemp
            $out = & $pwsh -NoProfile -File $harness -AppExe $Exe -ArtifactDir (Join-Path $tmp 'evidence') -PreflightOnly 2>&1 | Out-String
            return [pscustomobject]@{ Code = $LASTEXITCODE; Out = $out }
        } finally { $env:TEMP = $oldTemp }
    }

    $p = Invoke-Preflight $dev
    Check 'Preflight: Dev-EXE wird VOR dem Lauf gewarnt' ($p.Out -match 'nicht GUI-tauglich' -and $p.Out -match 'tauri build --no-bundle') $p.Out
    Check 'Preflight: die Warnung bricht nicht ab (Exit 0, headless bleibt moeglich)' ($p.Code -eq 0) "Exit $($p.Code)"

    $p = Invoke-Preflight $gui
    Check 'Preflight: GUI-taugliche EXE ohne Warnung' ($p.Out -notmatch 'nicht GUI-tauglich' -and $p.Code -eq 0) "Exit $($p.Code): $($p.Out)"

    $p = Invoke-Preflight (Join-Path $tmp 'gibt-es-nicht.exe')
    Check 'Preflight: fehlende EXE endet mit Exit 2' ($p.Code -eq 2) "Exit $($p.Code)"
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failures -gt 0) { Write-Host "$failures Pruefung(en) fehlgeschlagen" -ForegroundColor Red; exit 1 }
Write-Host 'alles gruen' -ForegroundColor Green
exit 0
