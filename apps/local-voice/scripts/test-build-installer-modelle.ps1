# Tests zu scripts/build-installer-modelle.ps1 (lva#574): -OutDir darf nie fremde Dateien loeschen.
#
# Faelle mit Zielen unter einem eigenen Temp-Ordner laufen mit einer VORHANDENEN Platzhalter-EXE.
# Fehlte der Schutz, kaeme das Skript in einem gebauten Baum (src-tauri/transcribe-libs vorhanden)
# bis zur Loeschung; in jedem Baum werden die Faelle ueber die erwarteten Meldungen rot. Faelle mit dem
# App-Verzeichnis oder der Laufwerkswurzel laufen absichtlich mit fehlender EXE, damit ein
# fehlerhafter Stand dort nie etwas loeschen kann. Es wird nichts heruntergeladen: mit der
# Platzhalter-EXE scheitert das Skript spaetestens beim ersten Programmstart.
#
# Aufruf aus apps/local-voice (PowerShell 7): pwsh scripts/test-build-installer-modelle.ps1
# Exit-Code 0 nur, wenn alle Faelle bestehen.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script = Join-Path $PSScriptRoot 'build-installer-modelle.ps1'
$appDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$pwsh = (Get-Process -Id $PID).Path
$failed = 0

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lva test 574 " + [guid]::NewGuid().ToString('N'))
$null = [System.IO.Directory]::CreateDirectory($tmp)
$dummyExe = Join-Path $tmp 'platzhalter.exe'
[System.IO.File]::WriteAllText($dummyExe, 'kein Programm')
$missingExe = Join-Path $tmp 'gibt-es-nicht.exe'

# Eigener Prozess, Arbeitsverzeichnis $Cwd; der Aufrufoperator quotiert Argumente mit Leerzeichen.
function Invoke-Build([string]$OutDir, [string]$Cwd, [string]$ExePath) {
    Push-Location -LiteralPath $Cwd
    try {
        $out = & $pwsh -NoProfile -File $script -OutDir $OutDir -Exe $ExePath 2>&1 | Out-String
        return @{ Code = $LASTEXITCODE; Err = $out }
    } finally { Pop-Location }
}

function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name`n     $Detail"; $script:failed++ }
}

function New-Dir([string]$Path, [string]$Sentinel, [switch]$Own) {
    $null = [System.IO.Directory]::CreateDirectory($Path)
    if ($Sentinel) { [System.IO.File]::WriteAllText((Join-Path $Path $Sentinel), 'nicht loeschen') }
    if ($Own) { [System.IO.File]::WriteAllText((Join-Path $Path '.lva-vorinstalliert-nutzlast'), 'x') }
}


try {
    # 1. Vorhandener Ordner mit fremder Datei: Abbruch, Datei bleibt.
    New-Dir (Join-Path $tmp 'fremd') 'wichtig.txt'
    $r = Invoke-Build (Join-Path $tmp 'fremd') $tmp $dummyExe
    Check 'fremder Ordner: Abbruch mit Hinweis auf Marke' ($r.Code -ne 0 -and $r.Err -match 'Marke') $r.Err
    Check 'fremder Ordner: Datei bleibt' (Test-Path -LiteralPath (Join-Path $tmp 'fremd/wichtig.txt')) 'Sentinel geloescht'

    # 2. Platzhalter: Geschwister mit und ohne Marke; ein Muster darf keinen von beiden loeschen.
    New-Dir (Join-Path $tmp 'ausgabe-eigen') 'eigen.txt' -Own
    New-Dir (Join-Path $tmp 'ausgabe-fremd') 'fremd.txt'
    $r = Invoke-Build (Join-Path $tmp 'ausgabe*') $tmp $dummyExe
    Check 'Platzhalter: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Platzhalter') $r.Err
    Check 'Platzhalter: eigener Geschwisterordner unversehrt' (Test-Path -LiteralPath (Join-Path $tmp 'ausgabe-eigen/eigen.txt')) 'geloescht'
    Check 'Platzhalter: fremder Geschwisterordner unversehrt' (Test-Path -LiteralPath (Join-Path $tmp 'ausgabe-fremd/fremd.txt')) 'geloescht'
    $r = Invoke-Build (Join-Path $tmp 'ausgabe-[ef]*') $tmp $dummyExe
    Check 'Platzhalter [ ]: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Platzhalter') $r.Err

    # 3. Arbeitsverzeichnis als Ziel (-OutDir .): Abbruch, Datei bleibt.
    New-Dir (Join-Path $tmp 'arbeit') 'arbeitsordner.txt'
    $r = Invoke-Build '.' (Join-Path $tmp 'arbeit') $dummyExe
    Check 'Arbeitsverzeichnis: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'enthaelt') $r.Err
    Check 'Arbeitsverzeichnis: Datei bleibt' (Test-Path -LiteralPath (Join-Path $tmp 'arbeit/arbeitsordner.txt')) 'Sentinel geloescht'

    # 4. Relativer Pfad: aufgeloest gegen das PowerShell-Arbeitsverzeichnis, nicht gegen das
    #    Prozessverzeichnis. Unter dem PS-Verzeichnis liegt ein fremder Ordner `rel`, unter dem
    #    Prozessverzeichnis ein eigener mit Marke. Richtig: Abbruch an der Marke mit dem PS-Pfad.
    #    Falsch aufgeloest (GetFullPath) ginge die Pruefung am eigenen Ordner durch und scheiterte
    #    erst an der fehlenden EXE: rot. Fehlende EXE, damit in keinem Stand etwas geloescht wird.
    $psDir = Join-Path $tmp 'ps-ort'
    $procDir = Join-Path $tmp 'prozess-ort'
    New-Dir (Join-Path $psDir 'rel') 'fremd-rel.txt'
    New-Dir (Join-Path $procDir 'rel') -Own
    $savedCwd = [Environment]::CurrentDirectory
    Push-Location -LiteralPath $psDir
    [Environment]::CurrentDirectory = $procDir
    $msg = ''
    try {
        try { & $script -OutDir 'rel' -Exe $missingExe *> $null } catch { $msg = "$_" }
    } finally {
        [Environment]::CurrentDirectory = $savedCwd
        Pop-Location
    }
    Check 'relativer Pfad: Pruefung am Ordner unter dem PS-Arbeitsverzeichnis' ($msg -match 'Marke' -and $msg -match [regex]::Escape((Join-Path $psDir 'rel'))) $msg
    Check 'relativer Pfad: fremder Ordner unversehrt' (Test-Path -LiteralPath (Join-Path $psDir 'rel/fremd-rel.txt')) 'geloescht'

    # 5. Verknuepfung als Ziel: Abbruch, Ziel der Verknuepfung unversehrt.
    $linkTarget = Join-Path $tmp 'verknuepft-ziel'
    New-Dir $linkTarget 'hinter-link.txt' -Own
    $link = Join-Path $tmp 'verknuepfung'
    $linkType = if ($IsWindows) { 'Junction' } else { 'SymbolicLink' }
    $linkOk = $true
    try { $null = New-Item -ItemType $linkType -Path $link -Target $linkTarget } catch {
        $linkOk = $false
        # Unter Windows braucht eine Junction keine Sonderrechte: Scheitern ist ein Fehler, kein Skip.
        if ($IsWindows) { Check 'Verknuepfung anlegen' $false "$_" } else { Write-Host "skip Verknuepfung: $_" }
    }
    if ($linkOk) {
        $r = Invoke-Build $link $tmp $dummyExe
        Check 'Verknuepfung: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Verknuepfung') $r.Err
        $r = Invoke-Build (Join-Path $link 'unter') $tmp $dummyExe
        Check 'Verknuepfung im Elternpfad: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Verknuepfung') $r.Err
        Check 'Verknuepfung: Ziel unversehrt' (Test-Path -LiteralPath (Join-Path $linkTarget 'hinter-link.txt')) 'geloescht'
    }

    # 5b. Arbeitsverzeichnis unter einer Junction auf das Ziel (Codex R3): Das echte Ziel enthaelt
    #     die Marke und einen Sentinel; PowerShell steht unter dem Alias, das Prozessverzeichnis
    #     liegt ausserhalb. Ein isolierter App-Baum (Kopie des Skripts mit transcribe-libs,
    #     resources, Lizenzdatei) und die Platzhalter-EXE erfuellen alle Voraussetzungen, damit ein
    #     Stand ohne physischen Vergleich deterministisch bis zur Loeschung kaeme: dann rot.
    $fixApp = Join-Path $tmp 'fixture-app'
    $null = [System.IO.Directory]::CreateDirectory((Join-Path $fixApp 'scripts'))
    foreach ($d in @('src-tauri/transcribe-libs', 'src-tauri/resources', 'src-tauri/nsis')) {
        $null = [System.IO.Directory]::CreateDirectory((Join-Path $fixApp $d))
    }
    [System.IO.File]::WriteAllText((Join-Path $fixApp 'src-tauri/nsis/LIZENZEN-MODELLE.txt'), 'x')
    Copy-Item -LiteralPath $script -Destination (Join-Path $fixApp 'scripts/build-installer-modelle.ps1')
    $fixScript = Join-Path $fixApp 'scripts/build-installer-modelle.ps1'
    $aliasTarget = Join-Path $tmp 'alias-ziel'
    New-Dir $aliasTarget 'alias-sentinel.txt' -Own
    $alias = Join-Path $tmp 'alias'
    $aliasOk = $true
    try { $null = New-Item -ItemType $linkType -Path $alias -Target $aliasTarget } catch {
        $aliasOk = $false
        if ($IsWindows) { Check 'Alias-Junction anlegen' $false "$_" } else { Write-Host "skip Alias: $_" }
    }
    if ($aliasOk) {
        $savedCwd = [Environment]::CurrentDirectory
        $savedHf = $env:HF_TOKEN
        $env:HF_TOKEN = $null
        Push-Location -LiteralPath $alias
        [Environment]::CurrentDirectory = $tmp
        $msg = ''
        try {
            try { & $fixScript -OutDir $aliasTarget -Exe $dummyExe *> $null } catch { $msg = "$_" }
        } finally {
            [Environment]::CurrentDirectory = $savedCwd
            Pop-Location
            $env:HF_TOKEN = $savedHf
        }
        Check 'Junction-Arbeitsverzeichnis: Abbruch vor Loeschung' ($msg -match 'enthaelt') $msg
        Check 'Junction-Arbeitsverzeichnis: Sentinel unversehrt' (Test-Path -LiteralPath (Join-Path $aliasTarget 'alias-sentinel.txt')) 'Sentinel geloescht'
    }

    # 6. App-Verzeichnis, ein Vorfahr davon und die Laufwerkswurzel: Abbruch (fehlende EXE als Netz).
    $r = Invoke-Build $appDir $tmp $missingExe
    Check 'App-Verzeichnis: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'enthaelt') $r.Err
    $r = Invoke-Build (Split-Path $appDir -Parent) $tmp $missingExe
    Check 'Vorfahr des App-Verzeichnisses: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'enthaelt') $r.Err
    Check 'App-Verzeichnis unversehrt' (Test-Path -LiteralPath (Join-Path $appDir 'package.json')) 'package.json fehlt'
    $r = Invoke-Build ([System.IO.Path]::GetPathRoot($tmp)) $tmp $missingExe
    Check 'Laufwerkswurzel: Abbruch' ($r.Code -ne 0 -and $r.Err -match 'Laufwerkswurzel|enthaelt') $r.Err

    # 7. Eigene fruehere Nutzlast (Marke) und leerer Ordner: Pruefung laesst sie zu; das Skript
    #    scheitert erst danach (fehlende EXE).
    New-Dir (Join-Path $tmp 'eigene') -Own
    $r = Invoke-Build (Join-Path $tmp 'eigene') $tmp $missingExe
    Check 'eigene Nutzlast: Pruefung bestanden' ($r.Code -ne 0 -and $r.Err -match 'EXE nicht gefunden') $r.Err
    New-Dir (Join-Path $tmp 'leer')
    $r = Invoke-Build (Join-Path $tmp 'leer') $tmp $missingExe
    Check 'leerer Ordner: Pruefung bestanden' ($r.Code -ne 0 -and $r.Err -match 'EXE nicht gefunden') $r.Err
}
finally {
    foreach ($l in @('verknuepfung', 'alias')) {
        if (Test-Path -LiteralPath (Join-Path $tmp $l)) { (Get-Item -LiteralPath (Join-Path $tmp $l) -Force).Delete() }
    }
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed Fall/Faelle fehlgeschlagen."; exit 1 }
Write-Host 'Alle Faelle bestanden.'
