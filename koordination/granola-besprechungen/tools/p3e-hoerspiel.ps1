<#
.SYNOPSIS
    P3e: multi-speaker field test of the speaker separation (Sortformer 4spk) on the TTS
    radio play, against a reference built from the read-aloud script.

.DESCRIPTION
    Steps (all under -Work, default C:\Users\wolff\lva-spikes\p3e):
      1. portable app copy: hard links of the release exe, its DLLs and resources in
         Work\app, marker file "portable" -> the app uses Work\app\Data instead of
         %APPDATA% (settings, logs, models). Models are hard links of the installed
         diarization model and of Parakeet (no download, installed app untouched).
      2. --eval-diarization Work\diar (hoerspiel.wav + hoerspiel.rttm, built by
         p3e_reference.py) --rttm-out Work\out -> DER (collar 0.25 s), hypothesis RTTMs.
      3. --import-meeting <mp3> in a sandbox (LVA_MEETINGS_DIR = Work\meetings, fresh):
         the real import pipeline incl. the speaker step -> stored segments with speaker.
    Every run goes through Invoke-P7bApp (job object: memory cap, below-normal priority,
    CPU cap, kill on close; RAM start gate; timeout). Processes are only ended by PID.
    ASCII only (Windows PowerShell 5.1 parser).

.EXAMPLE
    pwsh -File p3e-hoerspiel.ps1 -Mp3 "...\Emilia...mp3"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Mp3,
    [string]$SourceExe = 'C:\Users\wolff\local-voice-project\apps\local-voice\src-tauri\target\release\local-voice-ai.exe',
    [string]$InstalledModels = (Join-Path $env:APPDATA 'de.wolffappliedai.localvoiceai\models'),
    [string]$Work = 'C:\Users\wolff\lva-spikes\p3e',
    [string]$ImportModel = 'parakeet-tdt-0.6b-v3',
    [int]$CpuPercent = 80,
    [int]$TimeoutSec = 1800,
    # eval corpus dir and result name (default Work\diar -> eval-diarization.json)
    [string]$EvalDir = '',
    [string]$EvalName = 'eval-diarization',
    [switch]$SkipEval,
    [switch]$SkipImport
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'p7b-common.ps1')

function New-HardLinkTree([string]$From, [string]$To) {
    New-Item -ItemType Directory -Force $To | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $From) {
        $dst = Join-Path $To $item.Name
        if ($item.PSIsContainer) { New-HardLinkTree $item.FullName $dst; continue }
        if (-not (Test-Path -LiteralPath $dst)) {
            New-Item -ItemType HardLink -Path $dst -Target $item.FullName | Out-Null
        }
    }
}

# ---- 1. portable app copy ------------------------------------------------------------
$srcDir = Split-Path -Parent (Resolve-Path $SourceExe).Path
$app = Join-Path $Work 'app'
New-Item -ItemType Directory -Force $app | Out-Null
foreach ($f in @(Get-ChildItem -LiteralPath $srcDir -File | Where-Object { $_.Extension -in '.exe', '.dll' -and $_.Name -notlike '*_lib.dll' })) {
    $dst = Join-Path $app $f.Name
    if (Test-Path -LiteralPath $dst) { Remove-Item -LiteralPath $dst -Force }
    New-Item -ItemType HardLink -Path $dst -Target $f.FullName | Out-Null
}
New-HardLinkTree (Join-Path $srcDir 'resources') (Join-Path $app 'resources')
Set-Content -Path (Join-Path $app 'portable') -Value 'Handy Portable Mode' -Encoding ASCII
$data = Join-Path $app 'Data'
New-HardLinkTree (Join-Path $InstalledModels 'diarization') (Join-Path $data 'models\diarization')
New-HardLinkTree (Join-Path $InstalledModels 'parakeet-tdt-0.6b-v3-int8') (Join-Path $data 'models\parakeet-tdt-0.6b-v3-int8')
$exe = Join-Path $app 'local-voice-ai.exe'
$exeInfo = [ordered]@{
    source_exe       = (Resolve-Path $SourceExe).Path
    source_exe_mtime = (Get-Item $SourceExe).LastWriteTime.ToString('s')
    source_exe_bytes = (Get-Item $SourceExe).Length
    app_exe          = $exe
}
Write-Host ("[p3e] portable app: {0} (source {1}, {2})" -f $exe, $exeInfo.source_exe, $exeInfo.source_exe_mtime)

$out = Join-Path $Work 'out'
New-Item -ItemType Directory -Force $out | Out-Null
$runs = [ordered]@{ exe = $exeInfo }

# ---- 2. DER against the script reference ---------------------------------------------
if (-not $SkipEval) {
    if (-not $EvalDir) { $EvalDir = Join-Path $Work 'diar' }
    $evalJson = Join-Path $out ($EvalName + '.json')
    if (Test-Path $evalJson) { Remove-Item $evalJson }
    $r = Invoke-P7bApp -AppExe $exe -Log (Join-Path $out ($EvalName + '.log')) -TimeoutSec $TimeoutSec `
        -CpuPercent $CpuPercent -MemoryLimitMb 12288 -MinFreeMb 10240 `
        -Arguments @('--eval-diarization', $EvalDir, '--rttm-out', $out, '--out', $evalJson) `
        -Env @{ LVA_MEETINGS_DIR = (Join-Path $Work 'meetings-eval') }
    $runs[$EvalName] = $r
    Write-Host ("[p3e] {3}: exit {0}, {1:N1} s, peak {2} MB" -f $r.exit_code, ($r.wall_ms / 1000.0), $r.peak_job_mb, $EvalName)
}

# ---- 3. import through the real pipeline ---------------------------------------------
if (-not $SkipImport) {
    $meet = Join-Path $Work 'meetings'
    if (Test-Path $meet) { Remove-Item -Recurse -Force $meet }
    New-Item -ItemType Directory -Force $meet | Out-Null
    $importJson = Join-Path $out 'import.json'
    if (Test-Path $importJson) { Remove-Item $importJson }
    $r = Invoke-P7bApp -AppExe $exe -Log (Join-Path $out 'import.log') -TimeoutSec $TimeoutSec `
        -CpuPercent $CpuPercent -MemoryLimitMb 12288 -MinFreeMb 10240 `
        -Arguments @('--import-meeting', $Mp3, '--model', $ImportModel, '--out', $importJson) `
        -Env @{ LVA_MEETINGS_DIR = $meet }
    $runs['import'] = $r
    Write-Host ("[p3e] import: exit {0}, {1:N1} s, peak {2} MB" -f $r.exit_code, ($r.wall_ms / 1000.0), $r.peak_job_mb)
}

$runsName = if ($SkipImport) { "runs-$EvalName.json" } elseif ($SkipEval) { 'runs-import.json' } else { 'runs.json' }
$runs | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $out $runsName) -Encoding UTF8
