# Tests zu scripts/build-installer-modelle.ps1 (lva#574): -OutDir darf nie fremde Dateien loeschen.
#
# Seit Codex R4 loescht oder ersetzt das Skript -OutDir nie: ein vorhandener Ordner muss leer sein.
# Jeder Fall mit nicht leerem Ziel laeuft gegen einen ISOLIERTEN App-Baum (Kopie des Skripts mit
# transcribe-libs, resources, Lizenzdatei) und eine vorhandene Platzhalter-EXE. Damit sind alle
# Voraussetzungen erfuellt: Ein Stand, der doch loeschte, kaeme deterministisch bis zur Loeschung,
# und der Sentinel-Check wuerde rot. Erst der Programmstart der Platzhalter-EXE scheitert; es wird
# nichts heruntergeladen.
#
# Aufruf aus apps/local-voice (PowerShell 7): pwsh scripts/test-build-installer-modelle.ps1
# Exit-Code 0 nur, wenn alle Faelle bestehen.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$realScript = Join-Path $PSScriptRoot 'build-installer-modelle.ps1'
$appDir = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$pwsh = (Get-Process -Id $PID).Path
$failed = 0

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lva test 574 " + [guid]::NewGuid().ToString('N'))
$null = [System.IO.Directory]::CreateDirectory($tmp)
$dummyExe = Join-Path $tmp 'platzhalter.exe'
[System.IO.File]::WriteAllText($dummyExe, 'kein Programm')
$missingExe = Join-Path $tmp 'gibt-es-nicht.exe'

# Isolierter App-Baum: Das Skript nimmt sein App-Verzeichnis aus $PSScriptRoot\..
$fixApp = Join-Path $tmp 'fixture-app'
foreach ($d in @('scripts', 'src-tauri/transcribe-libs', 'src-tauri/resources', 'src-tauri/nsis')) {
    $null = [System.IO.Directory]::CreateDirectory((Join-Path $fixApp $d))
}
[System.IO.File]::WriteAllText((Join-Path $fixApp 'src-tauri/nsis/LIZENZEN-MODELLE.txt'), 'x')
$fixScript = Join-Path $fixApp 'scripts/build-installer-modelle.ps1'
Copy-Item -LiteralPath $realScript -Destination $fixScript

$linkType = if ($IsWindows) { 'Junction' } else { 'SymbolicLink' }
$links = [System.Collections.Generic.List[string]]::new()
$substDrive = $null

# Eigener Prozess, Arbeitsverzeichnis $Cwd; der Aufrufoperator quotiert Argumente mit Leerzeichen.
function Invoke-Build([string]$OutDir, [string]$Cwd, [string]$ExePath, [string]$ScriptPath = $fixScript) {
    Push-Location -LiteralPath $Cwd
    try {
        $out = & $pwsh -NoProfile -File $ScriptPath -OutDir $OutDir -Exe $ExePath 2>&1 | Out-String
        return @{ Code = $LASTEXITCODE; Err = $out }
    } finally { Pop-Location }
}

function Check([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { Write-Host "ok   $Name" } else { Write-Host "FAIL $Name`n     $Detail"; $script:failed++ }
}

function New-Dir([string]$Path, [string]$Sentinel) {
    $null = [System.IO.Directory]::CreateDirectory($Path)
    if ($Sentinel) { [System.IO.File]::WriteAllText((Join-Path $Path $Sentinel), 'nicht loeschen') }
}

function New-Link([string]$Path, [string]$Target) {
    try { $null = New-Item -ItemType $linkType -Path $Path -Target $Target; $links.Add($Path); return $true }
    catch {
        # Unter Windows braucht eine Junction keine Sonderrechte: Scheitern ist ein Fehler, kein Skip.
        if ($IsWindows) { Check "Verknuepfung anlegen ($Path)" $false "$_" } else { Write-Host "skip Verknuepfung: $_" }
        return $false
    }
}

function Test-Kept([string]$Name, [hashtable]$R, [string]$Sentinel) {
    Check "${Name}: Abbruch 'nicht leer'" ($R.Code -ne 0 -and $R.Err -match 'nicht leer') $R.Err
    Check "${Name}: Sentinel unversehrt" (Test-Path -LiteralPath $Sentinel) 'Sentinel geloescht'
}

try {
    # 1. Vorhandener Ordner mit fremder Datei.
    New-Dir (Join-Path $tmp 'fremd') 'wichtig.txt'
    $r = Invoke-Build (Join-Path $tmp 'fremd') $tmp $dummyExe
    Test-Kept 'fremder Ordner' $r (Join-Path $tmp 'fremd/wichtig.txt')

    # 2. Fruehere Nutzlast mit alter Marke wird ebenfalls nicht mehr ersetzt.
    New-Dir (Join-Path $tmp 'alt') '.lva-vorinstalliert-nutzlast'
    $r = Invoke-Build (Join-Path $tmp 'alt') $tmp $dummyExe
    Test-Kept 'fruehere Nutzlast' $r (Join-Path $tmp 'alt/.lva-vorinstalliert-nutzlast')

    # 3. Platzhalter: Abbruch vor jeder Pruefung, Geschwister unversehrt.
    New-Dir (Join-Path $tmp 'ausgabe-a') 'a.txt'
    New-Dir (Join-Path $tmp 'ausgabe-b') 'b.txt'
    foreach ($pat in @('ausgabe*', 'ausgabe-[ab]')) {
        $r = Invoke-Build (Join-Path $tmp $pat) $tmp $dummyExe
        Check "Platzhalter ${pat}: Abbruch" ($r.Code -ne 0 -and $r.Err -match 'Platzhalter') $r.Err
    }
    Check 'Platzhalter: Geschwister unversehrt' ((Test-Path -LiteralPath (Join-Path $tmp 'ausgabe-a/a.txt')) -and (Test-Path -LiteralPath (Join-Path $tmp 'ausgabe-b/b.txt'))) 'geloescht'

    # 4. Arbeitsverzeichnis als Ziel (-OutDir .).
    New-Dir (Join-Path $tmp 'arbeit') 'arbeitsordner.txt'
    $r = Invoke-Build '.' (Join-Path $tmp 'arbeit') $dummyExe
    Test-Kept 'Arbeitsverzeichnis' $r (Join-Path $tmp 'arbeit/arbeitsordner.txt')

    # 5. Relativer Pfad wird gegen das PowerShell-Arbeitsverzeichnis aufgeloest, nicht gegen das
    #    Prozessverzeichnis: unter dem PS-Verzeichnis liegt ein nicht leerer, unter dem
    #    Prozessverzeichnis ein leerer Ordner `rel`. Richtig: Abbruch 'nicht leer' mit dem PS-Pfad.
    $psDir = Join-Path $tmp 'ps-ort'
    $procDir = Join-Path $tmp 'prozess-ort'
    New-Dir (Join-Path $psDir 'rel') 'fremd-rel.txt'
    New-Dir (Join-Path $procDir 'rel')
    $savedCwd = [Environment]::CurrentDirectory
    Push-Location -LiteralPath $psDir
    [Environment]::CurrentDirectory = $procDir
    $msg = ''
    try {
        try { & $fixScript -OutDir 'rel' -Exe $missingExe *> $null } catch { $msg = "$_" }
    } finally {
        [Environment]::CurrentDirectory = $savedCwd
        Pop-Location
    }
    Check 'relativer Pfad: Pruefung am Ordner unter dem PS-Arbeitsverzeichnis' ($msg -match 'nicht leer' -and $msg -match [regex]::Escape((Join-Path $psDir 'rel'))) $msg

    # 6. Verknuepfungen: Ziel ist eine Junction; Arbeitsverzeichnis ist eine Junction auf das
    #    echte Ziel (Codex R3), im selben Prozess, Prozessverzeichnis ausserhalb.
    $real = Join-Path $tmp 'echtes-ziel'
    New-Dir $real 'hinter-link.txt'
    $link = Join-Path $tmp 'verknuepfung'
    if (New-Link $link $real) {
        $r = Invoke-Build $link $tmp $dummyExe
        Test-Kept 'Verknuepfung als Ziel' $r (Join-Path $real 'hinter-link.txt')
        $savedCwd = [Environment]::CurrentDirectory
        Push-Location -LiteralPath $link
        [Environment]::CurrentDirectory = $tmp
        $msg = ''
        try {
            try { & $fixScript -OutDir $real -Exe $dummyExe *> $null } catch { $msg = "$_" }
        } finally {
            [Environment]::CurrentDirectory = $savedCwd
            Pop-Location
        }
        Check 'Junction-Arbeitsverzeichnis: Abbruch' ($msg -match 'nicht leer') $msg
        Check 'Junction-Arbeitsverzeichnis: Sentinel unversehrt' (Test-Path -LiteralPath (Join-Path $real 'hinter-link.txt')) 'geloescht'
    }

    # 7. subst-Laufwerk als Arbeitsverzeichnis, echter Pfad als -OutDir (Codex R4), nur Windows.
    if ($IsWindows) {
        $used = @(Get-PSDrive -PSProvider FileSystem | ForEach-Object Name)
        $substDrive = @([char[]]'QRSTUVWXYZ' | Where-Object { "$_" -notin $used -and -not (Test-Path "${_}:\") }) | Select-Object -First 1
        if ($substDrive) {
            & subst "${substDrive}:" $real | Out-Null
            $r = Invoke-Build $real "${substDrive}:\" $dummyExe
            Test-Kept 'subst-Arbeitsverzeichnis' $r (Join-Path $real 'hinter-link.txt')
            $r = Invoke-Build "${substDrive}:\" $tmp $dummyExe
            Test-Kept 'subst-Laufwerk als Ziel' $r (Join-Path $real 'hinter-link.txt')
        } else { Check 'subst: freier Laufwerksbuchstabe' $false 'keiner frei' }
    }

    # 8. Echtes App-Verzeichnis, ein Vorfahr davon und die Laufwerkswurzel (gegen das echte Skript,
    #    fehlende EXE als zusaetzliches Netz).
    foreach ($t in @($appDir, (Split-Path $appDir -Parent), [System.IO.Path]::GetPathRoot($tmp))) {
        $r = Invoke-Build $t $tmp $missingExe $realScript
        Check "geschuetzter Ordner ${t}: Abbruch" ($r.Code -ne 0 -and $r.Err -match 'nicht leer') $r.Err
    }
    Check 'App-Verzeichnis unversehrt' (Test-Path -LiteralPath (Join-Path $appDir 'package.json')) 'package.json fehlt'

    # 9. Leerer und fehlender Ordner: Pruefung bestanden, Abbruch erst an der fehlenden EXE.
    New-Dir (Join-Path $tmp 'leer')
    foreach ($t in @((Join-Path $tmp 'leer'), (Join-Path $tmp 'neu'))) {
        $r = Invoke-Build $t $tmp $missingExe
        Check "frei ${t}: Pruefung bestanden" ($r.Code -ne 0 -and $r.Err -match 'EXE nicht gefunden') $r.Err
    }
}
finally {
    if ($substDrive) { & subst "${substDrive}:" /D | Out-Null }
    foreach ($l in $links) { if (Test-Path -LiteralPath $l) { (Get-Item -LiteralPath $l -Force).Delete() } }
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failed -gt 0) { Write-Host "$failed Fall/Faelle fehlgeschlagen."; exit 1 }
Write-Host 'Alle Faelle bestanden.'
