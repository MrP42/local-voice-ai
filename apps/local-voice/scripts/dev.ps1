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

    Zu Fallstrick 2: die Ursache ist eine Umgebungsvariable. Die cmake-Crate
    nimmt den Generator aus CMAKE_GENERATOR; ein Lauf in einer Shell mit
    CMAKE_GENERATOR=Ninja hinterlaesst einen Ninja-Cache, der naechste Lauf ohne
    die Variable scheitert. Das Skript entfernt die Variable fuer seine Laeufe
    (LVA_KEEP_CMAKE_GENERATOR=1 behaelt sie) und raeumt einen vorhandenen
    Nicht-Visual-Studio-Cache vor Test/Build selbst weg.

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
    notices     node scripts/gen-notices.mjs: THIRD-PARTY-NOTICES.md + SBOM
                (CycloneDX) erzeugen; Argumente wie --only sbom/--check gehen durch

.EXAMPLE
    pwsh -File apps\local-voice\scripts\dev.ps1 test
    pwsh -File apps\local-voice\scripts\dev.ps1 check
    pwsh -File apps\local-voice\scripts\dev.ps1 clean-cmake
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('test', 'fmt', 'clippy', 'check', 'build', 'bundle', 'clean-cmake', 'harness', 'notices')]
    [string]$Target = 'check',

    # Weitere Argumente werden an das jeweilige Werkzeug durchgereicht.
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$AppDir     = Split-Path -Parent $PSScriptRoot          # apps\local-voice
$TauriDir   = Join-Path $AppDir 'src-tauri'

# Gemeinsame Waechter (PATH, Assert-Tool, Invoke-Step, CMake-Generator); werden
# auch von scripts\test-dev-guards.ps1 geprueft.
. (Join-Path $PSScriptRoot 'lib\dev-guards.ps1')

# ------------------------------------------------------------------- PATH
$CargoBin = Add-CargoToPath

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
    'notices' {
        Assert-Tool 'node' 'Node.js installieren.'
        $script = Join-Path $PSScriptRoot 'gen-notices.mjs'
        Invoke-Step 'notices + SBOM (gen-notices.mjs)' { node $script @Rest }
        exit 0
    }
}

Assert-Tool 'cargo' 'Rust installieren oder %USERPROFILE%\.cargo\bin pruefen.'

if ($Target -ne 'fmt') {
    # Fallstrick 2 (Issue #8): gleicher CMake-Generator wie in der CI, und ein
    # Cache mit fremdem Generator wird vor dem Lauf geraeumt statt mittendrin
    # mit "Does not match the generator used previously" abzubrechen.
    $null = Remove-CMakeGeneratorEnv
    $foreign = @(Get-ForeignCMakeCaches -TauriDir $TauriDir)
    if ($foreign.Count -gt 0) {
        foreach ($f in $foreign) {
            Write-Host ("CMake-Cache mit Generator '{0}' gefunden: {1}" -f $f.Generator, $f.Path) -ForegroundColor Yellow
        }
        Write-Host 'Raeume ihn weg (naechster Build konfiguriert neu, ~7 Minuten).' -ForegroundColor Yellow
        Clear-CMakeCache
    }
}

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
                # Der Installer rechnet STT wie das Release per Vulkan auf der GPU
                # (Feature gpu-vulkan, P2f). Das braucht das LunarG-SDK; der
                # SDK-Installer setzt VULKAN_SDK maschinenweit, eine vor der
                # Installation geoeffnete Shell sieht es aber noch nicht.
                $featureArgs = @()
                if (-not ($Rest -match '^--features')) {
                    if (-not $env:VULKAN_SDK) {
                        $env:VULKAN_SDK = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine')
                    }
                    if ($env:VULKAN_SDK -and (Test-Path $env:VULKAN_SDK)) {
                        $sdkBin = Join-Path $env:VULKAN_SDK 'Bin'
                        if (-not ($env:PATH -split ';' | Where-Object { $_ -ieq $sdkBin })) {
                            $env:PATH = "$sdkBin;$env:PATH"
                        }
                        $featureArgs = @('--features', 'gpu-vulkan')
                        Write-Host ('Vulkan-SDK ' + $env:VULKAN_SDK + ' -> --features gpu-vulkan') -ForegroundColor DarkGray
                    } else {
                        Write-Warning 'VULKAN_SDK fehlt: Installer wird OHNE GPU-STT gebaut (nur CPU). Siehe docs/BUILD-WINDOWS.md, Abschnitt GPU.'
                    }
                }
                Invoke-Step 'tauri build (Installer)' { npx tauri build @featureArgs @Rest }
            }
            # Hotfix 0.21.1 (#73): ein Haupt-Thread mit nur 1 MiB Stack liess 0.21.0 beim
            # Klick auf Uebersetzen/KI-Notizen abstuerzen. Die gebaute EXE muss die grosse
            # Reserve haben (build.rs); sonst bricht der Lauf hier ab, nicht erst beim Nutzer.
            $builtExe = Join-Path $TauriDir 'target\release\local-voice-ai.exe'
            if (Test-Path -LiteralPath $builtExe) {
                Invoke-Step 'Stack-Pruefung der EXE (check-exe-stack.mjs)' {
                    node (Join-Path $PSScriptRoot 'check-exe-stack.mjs') $builtExe
                }
            }
        }
    }
} finally {
    Pop-Location
}
