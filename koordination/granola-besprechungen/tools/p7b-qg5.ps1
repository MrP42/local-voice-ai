<#
.SYNOPSIS
    P7b / QG5 (F27): the whole meeting path without network traffic, proven by an
    observed offline run.

.DESCRIPTION
    Chain (one sandbox, the same meeting where possible):
      1 --simulate-meeting (s1 + s2, 10.7 min) --final-model auto --notes
          live STT, final pass, speaker step (Sortformer), AI notes (llama-server)
      2 --reindex-meetings        search index + vectors (second llama-server, BGE-M3)
      3 --eval-chat <fixtures>    chat questions over meetings (both llama-servers)
      4 --export-meeting <id> --format pdf   (hidden WebView2 window)
    Observation, both read-only and without admin rights:
      - p7b-netwatch.ps1: every TCP connection / UDP endpoint of the exe and all its
        descendants, polled every 400 ms
      - p7b_proxy_log.py: HTTP(S)_PROXY / ALL_PROXY point to a local dead-end proxy that
        logs every proxied request and answers 403 (NO_PROXY = loopback), so short-lived
        HTTP requests are caught too and nothing leaves the machine through them.
    A positive control first sends one request through the proxy (must be logged).
    ASCII only.
#>
[CmdletBinding()]
param(
    [string]$AppExe = '',
    [string]$HfHome = 'C:\Users\wolff\lva-spikes\p7b\hf',
    [string]$BenchDir = '',
    [Parameter(Mandatory)][string]$OutDir,
    [int]$ProxyPort = 18089
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'p7b-common.ps1')

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $AppExe) { $AppExe = Join-Path $repo 'apps\local-voice\src-tauri\target\release\local-voice-ai.exe' }
$AppExe = (Resolve-Path $AppExe).Path
if (-not $BenchDir) { $BenchDir = Join-Path $env:LOCALAPPDATA 'lva-bench' }
$raw = Join-Path $BenchDir 'results\p7b'
New-Item -ItemType Directory -Force $raw, $OutDir | Out-Null

$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ('lva-p7b-qg5-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $sandbox | Out-Null
$stop = Join-Path $sandbox 'stop.flag'
$proxyLog = Join-Path $OutDir 'p7b-qg5-proxy.jsonl'
$netOut = Join-Path $OutDir 'p7b-qg5-net'
if (Test-Path $proxyLog) { Remove-Item $proxyLog }

# ---- observers
$proxy = Start-Process -FilePath 'python' -ArgumentList @((Join-Path $PSScriptRoot 'p7b_proxy_log.py'), '--port', $ProxyPort, '--log', $proxyLog, '--stop-file', $stop) -PassThru -WindowStyle Hidden
$pwshExe = (Get-Process -Id $PID).Path
$watch = Start-Process -FilePath $pwshExe -ArgumentList @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'p7b-netwatch.ps1'), '-AppExe', $AppExe, '-Out', $netOut, '-StopFile', $stop) -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

# positive control: one request through the proxy must appear in the log
$control = 'failed'
try {
    & python -c "import urllib.request as u; o=u.build_opener(u.ProxyHandler({'http':'http://127.0.0.1:$ProxyPort'})); o.open('http://p7b-control.invalid/', timeout=5)" 2>$null
} catch {}
Start-Sleep -Milliseconds 500
if ((Test-Path $proxyLog) -and ((Get-Content $proxyLog -Raw) -match 'p7b-control.invalid')) { $control = 'logged' }

$proxyUrl = "http://127.0.0.1:$ProxyPort"
$envs = @{
    LVA_MEETINGS_DIR = $sandbox; HF_HOME = $HfHome
    HTTP_PROXY = $proxyUrl; HTTPS_PROXY = $proxyUrl; ALL_PROXY = $proxyUrl
    NO_PROXY = 'localhost,127.0.0.1,::1'   # Windows env names are case-insensitive
}
$free = Get-P7bFreeMb
$cap = [Math]::Min(20480, [Math]::Max(8192, $free - 6144))
$steps = [ordered]@{}
try {
    # 1 simulate + final pass + speakers + notes
    $sim = Join-Path $OutDir 'p7b-qg5-sim.json'
    $simRaw = Join-Path $raw 'p7b-qg5-sim.full.json'
    $a = @('--simulate-meeting', '--scene-mic', 'mic_echo.wav', '--final-model', 'auto', '--notes', '--json', '--out', $simRaw,
        '--scene', (Join-Path $BenchDir 'synth\scene1_status'), '--scene', (Join-Path $BenchDir 'synth\scene2_angebot'))
    $steps['1_simulate_final_speakers_notes'] = Invoke-P7bApp -AppExe $AppExe -Arguments $a -Log (Join-Path $raw 'p7b-qg5-1.log') `
        -TimeoutSec 1800 -MemoryLimitMb $cap -MinFreeMb 10240 -Env $envs
    $s = Get-Content $simRaw -Raw -Encoding UTF8 | ConvertFrom-Json
    $meetingId = [string]$s.meeting_id
    [ordered]@{
        meeting_id = $meetingId; audio_ms = $s.audio_ms; live_model = $s.model
        final_model = $s.final.report.model; final_kept = $s.final.report.kept
        speakers_state = $s.final.report.speakers.state; notes = $s.notes; status = $s.final.status
    } | ConvertTo-Json -Depth 5 | Set-Content -Path $sim -Encoding UTF8

    # 2 index + vectors
    $idx = Join-Path $OutDir 'p7b-qg5-reindex.json'
    $steps['2_reindex_embeddings'] = Invoke-P7bApp -AppExe $AppExe -Arguments @('--reindex-meetings', '--json', '--out', $idx) `
        -Log (Join-Path $raw 'p7b-qg5-2.log') -TimeoutSec 900 -MemoryLimitMb $cap -MinFreeMb 10240 -Env $envs

    # 3 chat
    $chat = Join-Path $raw 'p7b-qg5-chat.json'
    $fixtures = Join-Path $repo 'apps\local-voice\src-tauri\tests\fixtures\chat'
    $steps['3_chat'] = Invoke-P7bApp -AppExe $AppExe -Arguments @('--eval-chat', $fixtures, '--json', '--out', $chat) `
        -Log (Join-Path $raw 'p7b-qg5-3.log') -TimeoutSec 1200 -MemoryLimitMb $cap -MinFreeMb 10240 -Env $envs

    # 4 PDF export
    $pdf = Join-Path $sandbox 'p7b-export.pdf'
    $steps['4_export_pdf'] = Invoke-P7bApp -AppExe $AppExe -Arguments @('--export-meeting', $meetingId, '--format', 'pdf', '--out', $pdf) `
        -Log (Join-Path $raw 'p7b-qg5-4.log') -TimeoutSec 300 -MemoryLimitMb $cap -MinFreeMb 10240 -Env $envs
    $pdfHead = if (Test-Path $pdf) { [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($pdf)[0..4]) } else { '' }
    $pdfBytes = if (Test-Path $pdf) { (Get-Item $pdf).Length } else { 0 }
} finally {
    New-Item -ItemType File -Force $stop | Out-Null
    $watch.WaitForExit(30000) | Out-Null
    $proxy.WaitForExit(10000) | Out-Null
    if (-not $watch.HasExited) { Stop-Process -Id $watch.Id -Force }
    if (-not $proxy.HasExited) { Stop-Process -Id $proxy.Id -Force }
}

$net = Get-Content "$netOut.json" -Raw -Encoding UTF8 | ConvertFrom-Json
$proxyEntries = @(Get-Content $proxyLog -Encoding UTF8 | Where-Object { $_ -and $_ -notmatch 'p7b-control.invalid' } | ForEach-Object { $_ | ConvertFrom-Json })
$chatJson = if (Test-Path $chat) { Get-Content $chat -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$idxJson = if (Test-Path $idx) { Get-Content $idx -Raw -Encoding UTF8 | ConvertFrom-Json } else { $null }
$summary = [ordered]@{
    gate                   = 'QG5'
    finished               = (Get-Date).ToString('s')
    app_exe                = $AppExe
    app_exe_mtime          = (Get-Item $AppExe).LastWriteTime.ToString('s')
    proxy_control          = $control
    steps                  = $steps
    meeting_id             = $meetingId
    reindex                = $idxJson
    chat_accuracy          = if ($chatJson) { $chatJson.aggregate.accuracy } else { $null }
    chat_questions         = if ($chatJson) { $chatJson.aggregate.questions } else { $null }
    pdf_magic              = $pdfHead
    pdf_bytes              = $pdfBytes
    processes_seen         = $net.processes_seen
    sockets_seen           = $net.sockets
    netwatch_ticks         = $net.ticks
    netwatch_ticks_with_app = $net.ticks_with_tree
    non_loopback_sockets   = $net.non_loopback
    non_loopback_count     = $net.non_loopback_count
    proxy_requests         = $proxyEntries
    proxy_request_count    = $proxyEntries.Count
    qg5_pass               = ($net.non_loopback_count -eq 0) -and ($proxyEntries.Count -eq 0) -and ($control -eq 'logged')
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -Path (Join-Path $OutDir 'p7b-qg5.json') -Encoding UTF8
Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
Write-Host ("[p7b-qg5] non-loopback sockets {0}, proxy requests {1}, control {2}, pass {3}" -f $net.non_loopback_count, $proxyEntries.Count, $control, $summary.qg5_pass)
