# Gemeinsame Waechter fuer dev.ps1 (Issue #8). ASCII only (PS-5.1-Parser).
# Wird per Dot-Sourcing geladen und von scripts/test-dev-guards.ps1 geprueft.

function Add-CargoToPath {
    # cargo liegt unter %USERPROFILE%\.cargo\bin und ist weder in Git Bash noch
    # in PowerShell im PATH. Setzt ihn selbst, wenn das Verzeichnis existiert.
    param([string]$UserProfile = $env:USERPROFILE)
    $bin = Join-Path $UserProfile '.cargo\bin'
    if (Test-Path $bin) {
        if (-not ($env:PATH -split ';' | Where-Object { $_ -ieq $bin })) {
            $env:PATH = "$bin;$env:PATH"
        }
    }
    return $bin
}

function Assert-Tool {
    # Bricht mit klarer Meldung und Exit 2 ab, wenn das Werkzeug fehlt - statt
    # einer "command not found"-Zeile, die hinter einer Pipe als Exit 0 endet.
    param([string]$Name, [string]$Hint)
    $found = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $found) {
        Write-Host "FEHLT: $Name nicht gefunden. $Hint" -ForegroundColor Red
        exit 2
    }
    Write-Host ("{0,-6} {1}" -f $Name, $found.Source) -ForegroundColor DarkGray
}

function Invoke-Step {
    # Fuehrt einen NATIVEN Aufruf aus und beendet das Skript mit dessen
    # Exit-Code, wenn er != 0 ist. Kein gruenes Ergebnis aus einer Pipeline.
    param([string]$Label, [scriptblock]$Body)
    Write-Host "`n=== $Label ===" -ForegroundColor Cyan
    $global:LASTEXITCODE = 0
    & $Body
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FEHLGESCHLAGEN: $Label (Exit $LASTEXITCODE)" -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "OK: $Label" -ForegroundColor Green
}

# --------------------------------------------------------- CMake-Generator
# Ursache des Generator-Konflikts (Issue #8): transcribe-cpp-sys baut ueber die
# cmake-Crate, und die nimmt den Generator aus der UMGEBUNG (CMAKE_GENERATOR,
# auch als CMAKE_GENERATOR_<target>/TARGET_CMAKE_GENERATOR). Ohne Variable
# waehlt CMake Visual Studio. Hat ein frueherer Lauf in einer Shell mit
# CMAKE_GENERATOR=Ninja gebaut, liegt ein Ninja-Cache im Build-Verzeichnis, und
# der naechste Lauf ohne die Variable scheitert mit "Does not match the
# generator used previously". Gegenmittel: die Variablen fuer unsere Laeufe
# entfernen (immer derselbe Generator wie in der CI) und einen vorhandenen
# fremden Cache vorher wegraeumen.

function Get-CMakeGeneratorEnvNames {
    # Namen der Umgebungsvariablen, die den cmake-Crate-Generator festlegen.
    # CMAKE_GENERATOR_PLATFORM/_TOOLSET sind etwas anderes und bleiben.
    Get-ChildItem Env: |
        Where-Object { $_.Name -match '^(TARGET_|HOST_)?CMAKE_GENERATOR(_.+)?$' -and
                       $_.Name -notmatch 'CMAKE_GENERATOR_(PLATFORM|TOOLSET|INSTANCE)$' } |
        ForEach-Object { $_.Name }
}

function Remove-CMakeGeneratorEnv {
    # Entfernt die Variablen aus der Umgebung dieses Skripts (und seiner
    # Kindprozesse). LVA_KEEP_CMAKE_GENERATOR=1 schaltet das ab.
    if ($env:LVA_KEEP_CMAKE_GENERATOR -eq '1') { return @() }
    $names = @(Get-CMakeGeneratorEnvNames)
    foreach ($n in $names) {
        Write-Host ("Umgebung: {0}={1} ignoriert (Cache-Konflikt-Ursache, Issue #8; LVA_KEEP_CMAKE_GENERATOR=1 behaelt sie)" -f $n, (Get-Item "Env:$n").Value) -ForegroundColor Yellow
        Remove-Item "Env:$n"
    }
    return $names
}

function Get-ForeignCMakeCaches {
    # CMake-Caches von transcribe-cpp-sys unter <TauriDir>\target, deren
    # Generator NICHT Visual Studio ist. Liefert Objekte mit Path und Generator.
    param([string]$TauriDir)
    $target = Join-Path $TauriDir 'target'
    if (-not (Test-Path $target)) { return @() }
    $dirs = Get-ChildItem -Path $target -Directory -Filter 'transcribe-cpp-sys-*' -Recurse -ErrorAction SilentlyContinue |
            Where-Object { $_.Parent.Name -eq 'build' }
    $hits = @()
    foreach ($d in $dirs) {
        $caches = Get-ChildItem -Path $d.FullName -Filter 'CMakeCache.txt' -Recurse -File -ErrorAction SilentlyContinue
        foreach ($c in $caches) {
            $line = Select-String -LiteralPath $c.FullName -Pattern '^CMAKE_GENERATOR:INTERNAL=(.*)$' -List
            if ($line) {
                $gen = $line.Matches[0].Groups[1].Value.Trim()
                if ($gen -notlike 'Visual Studio*') {
                    $hits += [pscustomobject]@{ Path = $c.FullName; Generator = $gen }
                }
            }
        }
    }
    return $hits
}
