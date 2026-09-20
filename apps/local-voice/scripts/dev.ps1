#Requires -Version 7.0
<#
.SYNOPSIS
    Entwickler-Wrapper: setzt den PATH und kapselt die ueblichen Ziele.

.DESCRIPTION
    ASCII only (Projektregel: Geviertstriche brechen den PS-5.1-Parser).

    Loest die beiden Toolchain-Fallstricke aus Issue #8 strukturell:

    1. cargo liegt unter %USERPROFILE%\.cargo\bin und ist weder in Git Bash
       noch in PowerShell im PATH. Das Skript setzt ihn selbst und bricht mit
       einer klaren Meldung ab, wenn cargo trotzdem fehlt - statt einer
       "command not found"-Zeile, die hinter einer Pipe als Exit 0 endet.
    2. Der CMake-Generator-Konflikt von transcribe-cpp-sys sitzt hinter einer
       NTFS-Junction; nur %LOCALAPPDATA%\tcs zu loeschen genuegt nicht.
       'clean-cmake' raeumt beide Seiten.

    Jedes Ziel prueft seinen eigenen Exit-Code. Das Skript endet nur dann mit
    0, wenn das Ziel wirklich erfolgreich war.

.PARAMETER Target
    test        cargo test --lib (Rust-Tests, kein Frontend noetig)
    fmt         cargo fmt --check
    clippy      cargo clippy --all-targets -- -D warnings
                (Achtung: der aus Handy uebernommene Bestand hat noch rund
                25 offene Clippy-Befunde - das Ziel ist heute noch kein Gate)
    check       fmt + test in dieser Reihenfolge
    build       npx tauri build --no-bundle (der EINZIGE gueltige Build-Weg,
                siehe docs/BUILD-WINDOWS.md, Stolperstein 3)
    bundle      npx tauri build (mit Installer)
    clean-cmake loescht den CMake-Cache auf beiden Seiten der Junction
    harness     pwsh scripts/m8-verify.ps1 (Abnahme-Harness)

.EXAMPLE
    pwsh -File apps\local-voice\scripts\dev.ps1 test
    pwsh -File apps\local-voice\scripts\dev.ps1 check
    pwsh -File apps\local-voice\scripts\dev.ps1 clean-cmake
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('test', 'fmt', 'clippy', 'check', 'build', 'bundle', 'clean-cmake', 'harness')]
    [string]$Target = 'check',

    # Weitere Argumente werden an das jeweilige Werkzeug durchgereicht.
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$AppDir     = Split-Path -Parent $PSScriptRoot          # apps\local-voice
$TauriDir   = Join-Path $AppDir 'src-tauri'
$CargoBin   = Join-Path $env:USERPROFILE '.cargo\bin'

# ------------------------------------------------------------------- PATH
if (Test-Path $CargoBin) {
    if (-not ($env:PATH -split ';' | Where-Object { $_ -ieq $CargoBin })) {
        $env:PATH = "$CargoBin;$env:PATH"
    }
}

function Assert-Tool {
    param([string]$Name, [string]$Hint)
    $found = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $found) {
        Write-Host "FEHLT: $Name nicht gefunden. $Hint" -ForegroundColor Red
        exit 2
    }
    Write-Host ("{0,-6} {1}" -f $Name, $found.Source) -ForegroundColor DarkGray
}

function Invoke-Step {
    param([string]$Label, [scriptblock]$Body)
    Write-Host "`n=== $Label ===" -ForegroundColor Cyan
    & $Body
    # $LASTEXITCODE gilt nur fuer native Programme; alle Schritte hier sind
    # native Aufrufe, deshalb ist die Pruefung aussagekraeftig.
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FEHLGESCHLAGEN: $Label (Exit $LASTEXITCODE)" -ForegroundColor Red
        exit $LASTEXITCODE
    }
    Write-Host "OK: $Label" -ForegroundColor Green
}

function Clear-CMakeCache {
    # Beide Seiten der Junction: der Cache liegt im echten target-Verzeichnis,
    # %LOCALAPPDATA%\tcs ist nur der kurze Pfad darauf. Wird nur eine Seite
    # geloescht, ist der alte Generator sofort wieder da.
    $buildDirs = @(Get-ChildItem -Path (Join-Path $TauriDir 'target') -Directory `
                                 -Filter 'transcribe-cpp-sys-*' -Recurse -ErrorAction SilentlyContinue |
                   Where-Object { $_.Parent.Name -eq 'build' })
    foreach ($dir in $buildDirs) {
        Write-Host ("entferne " + $dir.FullName) -ForegroundColor DarkGray
        Remove-Item -LiteralPath $dir.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
    $tcs = Join-Path $env:LOCALAPPDATA 'tcs'
    if (Test-Path $tcs) {
        Write-Host ("entferne " + $tcs) -ForegroundColor DarkGray
        Remove-Item -LiteralPath $tcs -Recurse -Force -ErrorAction SilentlyContinue
    }
    Write-Host ("CMake-Cache geraeumt ({0} Build-Verzeichnis(se) + tcs)." -f $buildDirs.Count) -ForegroundColor Green
}

# ------------------------------------------------------------------ Ziele
switch ($Target) {
    'clean-cmake' {
        Clear-CMakeCache
        exit 0
    }
    'harness' {
        $script = Join-Path $PSScriptRoot 'm8-verify.ps1'
        Invoke-Step 'harness (m8-verify)' { pwsh -File $script @Rest }
        exit 0
    }
}

Assert-Tool 'cargo' 'Rust installieren oder %USERPROFILE%\.cargo\bin pruefen.'

Push-Location $TauriDir
try {
    switch ($Target) {
        'test'   { Invoke-Step 'cargo test --lib' { cargo test --lib @Rest } }
        'fmt'    { Invoke-Step 'cargo fmt --check' { cargo fmt --check @Rest } }
        'clippy' { Invoke-Step 'cargo clippy' { cargo clippy --all-targets @Rest -- -D warnings } }
        'check'  {
            # Bewusst ohne clippy: der uebernommene Bestand ist dort noch nicht
            # sauber, ein rotes 'check' fuer fremde Altlasten waere wertlos.
            Invoke-Step 'cargo fmt --check' { cargo fmt --check }
            Invoke-Step 'cargo test --lib'  { cargo test --lib }
        }
        default {
            # build/bundle laufen ueber die Tauri-CLI aus dem App-Verzeichnis:
            # 'cargo build --release' erzeugt kein lauffaehiges Produkt
            # (docs/BUILD-WINDOWS.md, Stolperstein 3).
            Pop-Location
            Push-Location $AppDir
            Assert-Tool 'npx' 'Node.js installieren.'
            if ($Target -eq 'build') {
                Invoke-Step 'tauri build --no-bundle' { npx tauri build --no-bundle @Rest }
            } else {
                # createUpdaterArtifacts=true verlangt den Minisign-Schluessel,
                # sonst bricht der Lauf NACH dem fertigen Installer beim
                # Signieren ab (20.09.2026). Lokal liegt der Schluessel unter
                # ~/.tauri; wenn die Variable fehlt, aus der Datei lesen.
                $keyFile = Join-Path $HOME '.tauri\local-voice-ai.key'
                if (-not $env:TAURI_SIGNING_PRIVATE_KEY -and (Test-Path $keyFile)) {
                    $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -LiteralPath $keyFile -Raw).Trim()
                    if ($null -eq $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) {
                        $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
                    }
                    Write-Host ('Signaturschluessel aus ' + $keyFile) -ForegroundColor DarkGray
                }
                Invoke-Step 'tauri build (Installer)' { npx tauri build @Rest }
            }
        }
    }
} finally {
    Pop-Location
}
