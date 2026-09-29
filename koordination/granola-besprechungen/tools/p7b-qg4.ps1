<#
.SYNOPSIS
    P7b / QG4: scarce RAM -> clean abort (error or fallback) instead of a frozen machine.

.DESCRIPTION
    The machine is NEVER actually filled. Instead the app's own measurement of free RAM is
    capped with the test switch LVA_TEST_FREE_RAM_MB (process_guard::available_ram_mb; the
    switch can only lower the value, never raise it). Every gate of the meeting path then
    sees "scarce RAM" exactly as it would on a loaded machine:

      A0  control, no cap: scene1 (4.3 min) + final pass Qwen3-ASR 1.7B + speakers + notes
      A   cap 5000 MB: same command  -> final pass kept (low_ram), speakers skipped,
                                         notes fail with the RAM gate, meeting still "ready"
      B   cap 5000 MB: --reindex-meetings on the same sandbox -> vectors skipped (memory_low)
      C   cap 2000 MB: --export-meeting pdf -> pdf_low_memory, no WebView2 started
      D   cap 1500 MB: command A below the watchdog's emergency line (2 GB) -> the watchdog
                        trips and stops the LLM server; the run still ends cleanly

    Per run: exit code, wall time, timeout yes/no, own processes left running (llama-server,
    WebView2), peak job memory and the relevant log lines. All runs in a job object with a
    RAM start gate and timeout (p7b-common.ps1). ASCII only.
#>
[CmdletBinding()]
param(
    [string]$AppExe = '',
    [string]$HfHome = 'C:\Users\wolff\lva-spikes\p7b\hf',
    [string]$BenchDir = '',
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'p7b-common.ps1')

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $AppExe) { $AppExe = Join-Path $repo 'apps\local-voice\src-tauri\target\release\local-voice-ai.exe' }
$AppExe = (Resolve-Path $AppExe).Path
if (-not $BenchDir) { $BenchDir = Join-Path $env:LOCALAPPDATA 'lva-bench' }
$raw = Join-Path $BenchDir 'results\p7b'
New-Item -ItemType Directory -Force $raw | Out-Null
$qwen = 'handy-computer/Qwen3-ASR-1.7B-gguf/Qwen3-ASR-1.7B-Q5_K_M.gguf'
$scene = Join-Path $BenchDir 'synth\scene1_status'
$pattern = 'Zu wenig freier Arbeitsspeicher|RAM gate|low_ram|LowRam|memory_low|memory watchdog|pdf_low_memory|final pass skipped|final pass done|diariz|speaker|KI-Notizen|llama-server gestartet|error'

function Get-Lines([string]$log) {
    if (-not (Test-Path $log)) { return @() }
    @(Select-String -Path $log -Pattern $pattern | Select-Object -First 25 | ForEach-Object {
        $l = $_.Line; if ($l.Length -gt 260) { $l.Substring(0, 260) } else { $l } })
}

function Invoke-Sim([string]$name, $capMb, [string]$sandbox) {
    $json = Join-Path $raw "p7b-qg4-$name.sim.json"
    if (Test-Path $json) { Remove-Item $json }
    $envs = @{ LVA_MEETINGS_DIR = $sandbox; HF_HOME = $HfHome; LVA_TEST_FREE_RAM_MB = $capMb }
    $log = Join-Path $raw "p7b-qg4-$name.log"
    $a = @('--simulate-meeting', '--scene-mic', 'mic_echo.wav', '--scene', $scene, '--final-model', $qwen, '--notes', '--json', '--out', $json)
    $run = Invoke-P7bApp -AppExe $AppExe -Arguments $a -Log $log -TimeoutSec 1200 -MemoryLimitMb 16384 -MinFreeMb 10240 -Env $envs
    $s = if (Test-Path $json) { Get-Content $json -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
    [ordered]@{
        scenario        = $name
        free_ram_cap_mb = $capMb
        run             = $run
        meeting_id      = if ($s) { $s.meeting_id } else { $null }
        status_after    = if ($s) { $s.final.status } else { $null }
        final_plan      = if ($s) { $s.final.plan } else { $null }
        final_kept      = if ($s) { $s.final.report.kept } else { $null }
        final_model     = if ($s) { $s.final.report.model } else { $null }
        final_wall_ms   = if ($s) { $s.final.report.wall_ms } else { $null }
        speakers_state  = if ($s) { $s.final.report.speakers.state } else { $null }
        speakers_report = if ($s) { $s.final.report.speakers } else { $null }
        notes           = if ($s) { $s.notes } else { $null }
        log_lines       = Get-Lines $log
    }
}

$results = [ordered]@{ gate = 'QG4'; started = (Get-Date).ToString('s'); app_exe = $AppExe
    app_exe_mtime = (Get-Item $AppExe).LastWriteTime.ToString('s'); scenarios = @() }
$sbA0 = Join-Path ([System.IO.Path]::GetTempPath()) ('lva-p7b-qg4a0-' + [guid]::NewGuid().ToString('N'))
$sbA = Join-Path ([System.IO.Path]::GetTempPath()) ('lva-p7b-qg4a-' + [guid]::NewGuid().ToString('N'))
$sbD = Join-Path ([System.IO.Path]::GetTempPath()) ('lva-p7b-qg4d-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $sbA0, $sbA, $sbD | Out-Null
try {
    Write-Host '[p7b-qg4] A0 control (no cap)'
    $results.scenarios += Invoke-Sim 'A0-control' $null $sbA0
    Write-Host '[p7b-qg4] A cap 5000 MB'
    $a = Invoke-Sim 'A-cap5000' 5000 $sbA
    $results.scenarios += $a

    Write-Host '[p7b-qg4] B reindex, cap 5000 MB'
    $idx = Join-Path $raw 'p7b-qg4-B.json'
    if (Test-Path $idx) { Remove-Item $idx }
    $logB = Join-Path $raw 'p7b-qg4-B.log'
    $runB = Invoke-P7bApp -AppExe $AppExe -Arguments @('--reindex-meetings', '--json', '--out', $idx) -Log $logB -TimeoutSec 600 `
        -MemoryLimitMb 16384 -MinFreeMb 10240 -Env @{ LVA_MEETINGS_DIR = $sbA; HF_HOME = $HfHome; LVA_TEST_FREE_RAM_MB = 5000 }
    $results.scenarios += [ordered]@{ scenario = 'B-reindex-cap5000'; free_ram_cap_mb = 5000; run = $runB
        result = if (Test-Path $idx) { Get-Content $idx -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }; log_lines = Get-Lines $logB }

    Write-Host '[p7b-qg4] C pdf, cap 2000 MB'
    $pdf = Join-Path $sbA 'p7b-qg4.pdf'
    $logC = Join-Path $raw 'p7b-qg4-C.log'
    $runC = Invoke-P7bApp -AppExe $AppExe -Arguments @('--export-meeting', [string]$a.meeting_id, '--format', 'pdf', '--out', $pdf) -Log $logC `
        -TimeoutSec 300 -MemoryLimitMb 16384 -MinFreeMb 10240 -Env @{ LVA_MEETINGS_DIR = $sbA; LVA_TEST_FREE_RAM_MB = 2000 }
    $results.scenarios += [ordered]@{ scenario = 'C-pdf-cap2000'; free_ram_cap_mb = 2000; run = $runC
        pdf_written = (Test-Path $pdf); log_lines = @(Get-Content $logC -ErrorAction SilentlyContinue | Select-Object -Last 5) }

    Write-Host '[p7b-qg4] D cap 1500 MB (below the watchdog line)'
    $results.scenarios += Invoke-Sim 'D-cap1500' 1500 $sbD
} finally {
    Remove-Item -Recurse -Force $sbA0, $sbA, $sbD -ErrorAction SilentlyContinue
}
$results['finished'] = (Get-Date).ToString('s')
$results | ConvertTo-Json -Depth 8 | Set-Content -Path $Out -Encoding UTF8
foreach ($s in $results.scenarios) {
    Write-Host ("[p7b-qg4] {0}: exit {1}, {2:N1} s, timeout {3}, leftovers {4}" -f $s.scenario, $s.run.exit_code, ($s.run.wall_ms / 1000.0), $s.run.timed_out, @($s.run.leftovers).Count)
}
