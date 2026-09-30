<#
.SYNOPSIS
    P3e comparison: Nemotron-3-Diarization (spike tooling, PyTorch CPU, outside the app) on
    the radio play, to see whether the successor model would separate the similar voices.

.DESCRIPTION
    Runs C:\Users\wolff\lva-spikes\m3\run_nemotron.py (M3 spike, model from the local HF
    cache, HF_HUB_OFFLINE=1 - no download) through Invoke-P7bApp (job object: memory cap,
    below-normal priority, CPU cap, kill on close; RAM start gate; timeout).
    Writes <uri>.rttm per WAV into -OutDir. Scoring afterwards with p3e_analyze.py.
    ASCII only (Windows PowerShell 5.1 parser).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string[]]$Wav,
    [Parameter(Mandatory)][string]$OutDir,
    [string]$Spike = 'C:\Users\wolff\lva-spikes\m3',
    [int]$Threads = 4,
    [int]$CpuPercent = 50,
    [int]$TimeoutSec = 1800
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'p7b-common.ps1')
New-Item -ItemType Directory -Force $OutDir | Out-Null
$py = Join-Path $Spike 'venv\Scripts\python.exe'
$r = Invoke-P7bApp -AppExe $py -Log (Join-Path $OutDir 'nemotron.log') -TimeoutSec $TimeoutSec `
    -CpuPercent $CpuPercent -MemoryLimitMb 8192 -MinFreeMb 10240 `
    -Arguments (@((Join-Path $Spike 'run_nemotron.py'), $OutDir) + $Wav) `
    -Env @{ NTHREADS = $Threads; HF_HUB_OFFLINE = '1'; PYTHONIOENCODING = 'utf-8' }
$r | ConvertTo-Json -Depth 4 | Set-Content -Path (Join-Path $OutDir 'nemotron-run.json') -Encoding UTF8
Write-Host ("[p3e] nemotron: exit {0}, {1:N1} s, peak {2} MB" -f $r.exit_code, ($r.wall_ms / 1000.0), $r.peak_job_mb)
