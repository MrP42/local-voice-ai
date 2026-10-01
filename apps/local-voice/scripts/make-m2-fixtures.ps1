#Requires -Version 5.1
<#
.SYNOPSIS
    Generates the M2 echo fixtures (AEC + aligner tests of meetings::echo).

.DESCRIPTION
    ASCII only on purpose - PowerShell 5.1 chokes on em dashes and umlauts in
    .ps1 files (project rule). The spoken German sentences therefore live in
    make-m2-fixtures.py (UTF-8), which renders them with the built-in SAPI
    voices (System.Speech; the far-end voice "Microsoft Stefan" needs
    PowerShell 7 = pwsh on PATH), mixes the echo scene like the M2 spike did
    and writes:

      m2_echo_mic.wav     20 s, 16 kHz mono PCM16: near talker + room echo of
                          the far end (-6 dB, 50 ms, RT60 0.3 s) + noise
      m2_echo_render.wav  20 s, same time axis: far end = AEC reference

    The near talker is silent during the first 5 s and 5.6-9.9 s is far-end
    only, so ERLE is measurable; 15.8-18.6 s is double talk. The timeline is
    documented in the Python script and mirrored by the REGION_* constants in
    src-tauri/src/managers/meetings/echo.rs.

    Idempotent: existing fixtures are left alone unless -Force is passed.
    The output directory is git-ignored (see apps/local-voice/.gitignore); the
    two M2 files are meant to be force-added (git add -f) when they change.

.PARAMETER Force
    Regenerate even if both files exist.

.PARAMETER OutDir
    Default: src-tauri/tests/fixtures next to this script's parent folder.

.PARAMETER Python
    Python 3 executable with numpy, scipy and soundfile. Default: python.
#>
[CmdletBinding()]
param(
    [switch]$Force,
    [string]$OutDir = '',
    [string]$Python = 'python'
)

$ErrorActionPreference = 'Stop'

# $PSScriptRoot is empty inside param() defaults under Windows PowerShell 5.1
# (the default then resolved to C:\src-tauri), so the default is set here.
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $OutDir) { $OutDir = Join-Path $scriptDir '..\src-tauri\tests\fixtures' }

if (-not (Get-Command $Python -ErrorAction SilentlyContinue)) {
    throw "Python not found on PATH ($Python). Install Python 3 and: pip install numpy scipy soundfile"
}
& $Python -c "import numpy, scipy, soundfile" 2>$null
if ($LASTEXITCODE -ne 0) {
    throw "Missing Python packages. Run: $Python -m pip install numpy scipy soundfile"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
Write-Host "OutDir: $OutDir"

$py = Join-Path $scriptDir 'make-m2-fixtures.py'
$pyArgs = @($py, '--out-dir', $OutDir)
if ($Force) { $pyArgs += '--force' }
& $Python @pyArgs
if ($LASTEXITCODE -ne 0) { throw "make-m2-fixtures.py failed (exit $LASTEXITCODE)" }

Get-ChildItem $OutDir -Filter 'm2_echo_*' | ForEach-Object {
    "  {0,-22} {1,8:N0} KB" -f $_.Name, ($_.Length / 1KB)
}
