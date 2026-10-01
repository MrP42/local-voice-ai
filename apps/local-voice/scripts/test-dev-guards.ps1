#Requires -Version 7.0
<#
.SYNOPSIS
    Prueft die Waechter aus scripts\lib\dev-guards.ps1 und dev.ps1 (Issue #8).

.DESCRIPTION
    ASCII only. Kein cargo-Lauf, kein Build; dauert wenige Sekunden.
      1. cargo fehlt (leeres USERPROFILE, PATH ohne cargo) -> dev.ps1 meldet
         "FEHLT: cargo" und endet mit Exit 2.
      2. Ein nativer Schritt mit Exit 7 beendet das Skript mit Exit 7
         (kein gruener Exit-Code aus einer Pipeline).
      3. Ein CMake-Cache mit Ninja-Generator wird erkannt, ein Visual-Studio-
         Cache nicht.
      4. CMAKE_GENERATOR wird aus der Umgebung entfernt, CMAKE_GENERATOR_PLATFORM
         bleibt.
    Exit 0 = alles gruen, sonst 1.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\test-dev-guards.ps1
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$here = $PSScriptRoot
$dev = Join-Path $here 'dev.ps1'
$lib = Join-Path $here 'lib\dev-guards.ps1'
$pwsh = (Get-Process -Id $PID).Path
$failures = 0

function Check {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    if ($Ok) { Write-Host "PASS  $Name" -ForegroundColor Green }
    else { Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red; $script:failures++ }
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("lva-devguards-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    # ---- 1. cargo fehlt ------------------------------------------------
    $oldProfile = $env:USERPROFILE
    $oldPath = $env:PATH
    try {
        $env:USERPROFILE = Join-Path $tmp 'leer'
        New-Item -ItemType Directory -Force -Path $env:USERPROFILE | Out-Null
        $env:PATH = ($oldPath -split ';' | Where-Object {
                $_ -and -not (Test-Path (Join-Path $_ 'cargo.exe')) -and -not (Test-Path (Join-Path $_ 'cargo.cmd'))
            }) -join ';'
        $out = & $pwsh -NoProfile -File $dev fmt 2>&1 | Out-String
        $code = $LASTEXITCODE
    } finally {
        $env:USERPROFILE = $oldProfile
        $env:PATH = $oldPath
    }
    Check 'cargo fehlt: Exit 2' ($code -eq 2) "Exit $code"
    Check 'cargo fehlt: klare Meldung' ($out -match 'FEHLT: cargo nicht gefunden') $out

    # ---- 2. Exit-Code eines Schritts wird durchgereicht ----------------
    $cmd = ". '$lib'; Invoke-Step 'Test' { cmd /c exit 7 }; Write-Host 'NICHT ERREICHT'"
    $out = & $pwsh -NoProfile -Command $cmd 2>&1 | Out-String
    Check 'Invoke-Step: Exit 7 wird durchgereicht' ($LASTEXITCODE -eq 7) "Exit $LASTEXITCODE"
    Check 'Invoke-Step: Skript laeuft nicht weiter' ($out -notmatch 'NICHT ERREICHT')
    $cmd = ". '$lib'; Invoke-Step 'Test' { cmd /c exit 0 }; Write-Host 'WEITER'"
    $out = & $pwsh -NoProfile -Command $cmd 2>&1 | Out-String
    Check 'Invoke-Step: Exit 0 laeuft weiter' ($LASTEXITCODE -eq 0 -and $out -match 'WEITER') "Exit $LASTEXITCODE"

    # ---- 3. Fremder CMake-Cache ----------------------------------------
    . $lib
    $tauri = Join-Path $tmp 'src-tauri'
    $cacheDir = Join-Path $tauri 'target\debug\build\transcribe-cpp-sys-abc123\out\build'
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $cache = Join-Path $cacheDir 'CMakeCache.txt'
    Set-Content -LiteralPath $cache -Value "CMAKE_BUILD_TYPE:STRING=Release`nCMAKE_GENERATOR:INTERNAL=Ninja`n"
    $hits = @(Get-ForeignCMakeCaches -TauriDir $tauri)
    Check 'Ninja-Cache wird erkannt' ($hits.Count -eq 1 -and $hits[0].Generator -eq 'Ninja') ($hits | Out-String)
    Set-Content -LiteralPath $cache -Value "CMAKE_GENERATOR:INTERNAL=Visual Studio 17 2022`n"
    $hits = @(Get-ForeignCMakeCaches -TauriDir $tauri)
    Check 'Visual-Studio-Cache bleibt unberuehrt' ($hits.Count -eq 0) ($hits | Out-String)
    Check 'ohne target-Verzeichnis: kein Treffer' (@(Get-ForeignCMakeCaches -TauriDir (Join-Path $tmp 'nirgends')).Count -eq 0)

    # ---- 4. Generator-Umgebung -----------------------------------------
    $cmd = ". '$lib'; `$n = @(Remove-CMakeGeneratorEnv); " +
           "Write-Host ('REMOVED=' + (`$n -join ',')); " +
           "Write-Host ('GEN=[' + `$env:CMAKE_GENERATOR + ']'); Write-Host ('PLAT=[' + `$env:CMAKE_GENERATOR_PLATFORM + ']')"
    $env:CMAKE_GENERATOR = 'Ninja'
    $env:CMAKE_GENERATOR_PLATFORM = 'x64'
    try { $out = & $pwsh -NoProfile -Command $cmd 2>&1 | Out-String }
    finally { Remove-Item Env:CMAKE_GENERATOR; Remove-Item Env:CMAKE_GENERATOR_PLATFORM }
    Check 'CMAKE_GENERATOR wird entfernt' ($out -match 'REMOVED=CMAKE_GENERATOR\b' -and $out -match 'GEN=\[\]') $out
    Check 'CMAKE_GENERATOR_PLATFORM bleibt' ($out -match 'PLAT=\[x64\]') $out
} finally {
    Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

if ($failures -gt 0) { Write-Host "`n$failures Pruefung(en) fehlgeschlagen." -ForegroundColor Red; exit 1 }
Write-Host "`nAlle Pruefungen gruen." -ForegroundColor Green
exit 0
