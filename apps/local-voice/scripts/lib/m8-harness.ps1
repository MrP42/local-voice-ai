# Hilfsfunktionen des M8-Abnahmeharness (m8-verify.ps1), als Punktquelle, damit
# test-m8-verify.ps1 sie ohne Harness-Lauf pruefen kann. ASCII only (Geviertstriche
# brechen den Parser in Windows PowerShell 5.1).

# Full segment arrays would make the report unreadable (the 10 min fixture alone is
# 9 segments of transcript) and would put the whole spoken text into a committed
# document. The counts and boundary times are what the assertions use; drop the bodies.
function Remove-Segments {
    param($Obj)
    if ($null -eq $Obj) { return $null }
    if ($Obj -is [System.Collections.IDictionary]) {
        $c = @{}
        foreach ($k in $Obj.Keys) { $c[$k] = Remove-Segments $Obj[$k] }
        return $c
    }
    # Leading comma: without it PowerShell unwraps a one-element array,
    # which would print "channels": 2 where the store holds [2].
    # `foreach` statt `ForEach-Object`: ein Wert aus der Pipeline ist PSObject-
    # umhuellt, und `-is [pscustomobject]` ist fuer jeden PSObject wahr -- eine 2
    # wurde so zu {} ("channels": [{}]), ein Text zu {"Length":1}.
    if ($Obj -is [System.Collections.IEnumerable] -and $Obj -isnot [string]) {
        $items = @(foreach ($x in $Obj) { Remove-Segments $x })
        return , $items
    }
    # Nur ein echtes Objekt aus ConvertFrom-Json, nie ein umhuellter Grundwert.
    if ($Obj -is [System.Management.Automation.PSCustomObject]) {
        $c = @{}
        foreach ($p in $Obj.PSObject.Properties) {
            if ($p.Name -eq 'segments') { continue }
            $c[$p.Name] = Remove-Segments $p.Value
        }
        return $c
    }
    return $Obj
}

# Prueft, ob eine EXE ein eingebettetes Frontend hat (GUI-tauglich) oder nur
# headless laeuft. `cargo build --release` ueberschreibt target\release\*.exe mit
# einer EXE, deren Webview localhost:1420 (den Entwicklungsserver) laedt: das
# faellt headless nie auf, der GUI-Start ist danach tot (docs/BUILD-WINDOWS.md,
# Stolperstein 3). Entscheidend ist das eingebettete Asset "index-<hash>.js": eine
# EXE mit dem `dev`-Flag aus build.rs bettet kein Frontend ein.
#
# "localhost:1420" allein sagt NICHTS: auch die GUI-taugliche, installierte EXE
# enthaelt es (die Icon-URL "http://localhost:1420/icons/128x128.png" aus der
# eingebetteten Konfiguration, gemessen am 01.10.2026 an der installierten 0.20.x).
# Der Marker bleibt als Auskunft im Ergebnis (`HasDevUrl`), entscheidet aber nicht.
#
# Liest die Datei in 4-MB-Stuecken mit Ueberlappung (die EXE hat mehr als 100 MB;
# ein Marker kann auf einer Stuckgrenze liegen) und bricht ab, sobald beide
# Marker bekannt sind.
function Test-EmbeddedFrontend {
    param([Parameter(Mandatory)][string]$Exe)
    $result = [pscustomobject]@{
        Exe        = $Exe
        Exists     = $false
        HasDevUrl  = $false
        HasAsset   = $false
        GuiCapable = $false
        Reason     = ''
    }
    if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) {
        $result.Reason = 'Datei nicht gefunden'
        return $result
    }
    $result.Exists = $true

    $devMarker = 'localhost:1420'
    $assetRx = [regex]'index-[A-Za-z0-9_-]{6,}\.js'
    $chunk = 4MB
    $overlap = 64   # laenger als jeder Marker (14 bzw. ~30 Zeichen)
    $buffer = New-Object byte[] ($chunk + $overlap)
    $carry = 0
    $stream = [System.IO.File]::Open($Exe, 'Open', 'Read', 'ReadWrite')
    try {
        while ($true) {
            $read = $stream.Read($buffer, $carry, $chunk)
            if ($read -le 0) { break }
            $length = $carry + $read
            $text = [System.Text.Encoding]::ASCII.GetString($buffer, 0, $length)
            if (-not $result.HasDevUrl -and $text.Contains($devMarker)) { $result.HasDevUrl = $true }
            if (-not $result.HasAsset -and $assetRx.IsMatch($text)) { $result.HasAsset = $true }
            if ($result.HasDevUrl -and $result.HasAsset) { break }
            $carry = [Math]::Min($overlap, $length)
            [Array]::Copy($buffer, $length - $carry, $buffer, 0, $carry)
        }
    } finally {
        $stream.Dispose()
    }

    $result.GuiCapable = $result.HasAsset
    if (-not $result.GuiCapable) {
        $result.Reason = if ($result.HasDevUrl) {
            'kein eingebettetes Frontend-Asset (index-<hash>.js), dafuer localhost:1420: die Webview laedt den Entwicklungsserver'
        } else {
            'kein eingebettetes Frontend-Asset (index-<hash>.js) gefunden'
        }
    }
    return $result
}
