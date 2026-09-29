<#
.SYNOPSIS
    P7b / QG3: time from "stop" to TranscriptFinal (final pass incl. speaker step) and to
    finished AI notes, for a ~60 min meeting (GPU) and a ~13 min meeting (CPU-only path).

.DESCRIPTION
    Runs local-voice-ai.exe --simulate-meeting over the synthetic scenes (mic_echo.wav +
    system.wav, AEC on) followed by --final-model and --notes. In the simulation the live
    pipeline is drained first (= the moment the user presses stop); the final pass job and
    the notes then run exactly as after a real stop (final_pass::run_job with the app
    environment, then notes::enhance::enhance_meeting like the automatic run).

      gpu-qwen  scenes (s1,s2,s3) x3 + s1 = 61.9 min, final model Qwen3-ASR 1.7B, notes
      gpu-auto  same audio, final model "auto" (Whisper large-v3 Q5 when installed), notes
      cpu-auto  s3 + s1 = 12.8 min on the CPU-only copy of the exe, "auto" (-> CpuOnly), notes
      cpu-qwen  s3 + s1, explicit Qwen3-ASR 1.7B on the CPU (what the final pass costs there)

    Sandboxes: LVA_MEETINGS_DIR (fresh temp dir, removed afterwards), HF_HOME (hard links of the
    STT models, no download). CPU variants set CUDA_VISIBLE_DEVICES=-1 so the llama-server
    (CUDA runtime) computes on the CPU as well. Output: summary JSON (-Out) and the raw
    simulation JSON next to it in %LOCALAPPDATA%\lva-bench\results\p7b.

.EXAMPLE
    pwsh -File p7b-qg3.ps1 -Variant gpu-qwen -Out ..\abnahme\p7b-qg3-gpu-qwen.json
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('gpu-qwen', 'gpu-auto', 'cpu-auto', 'cpu-qwen')][string]$Variant,
    [string]$AppExe = '',
    [string]$CpuExe = '',
    [string]$HfHome = 'C:\Users\wolff\lva-spikes\p7b\hf',
    [string]$BenchDir = '',
    [Parameter(Mandatory)][string]$Out,
    [int]$CpuPercent = 80,
    [int]$TimeoutSec = 3600,
    # suffix for the raw files (repeat runs)
    [string]$Tag = '',
    # local LLM for the notes in this run only (e.g. llm-qwen3.5-9b-q4); default: configured
    [string]$NotesModel = ''
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'p7b-common.ps1')

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
if (-not $AppExe) { $AppExe = Join-Path $repo 'apps\local-voice\src-tauri\target\release\local-voice-ai.exe' }
if (-not $CpuExe) { $CpuExe = Join-Path $env:LOCALAPPDATA 'lva-bench\p7b-cpu\local-voice-ai.exe' }
if (-not $BenchDir) { $BenchDir = Join-Path $env:LOCALAPPDATA 'lva-bench' }
$qwen = 'handy-computer/Qwen3-ASR-1.7B-gguf/Qwen3-ASR-1.7B-Q5_K_M.gguf'
$raw = Join-Path $BenchDir 'results\p7b'
New-Item -ItemType Directory -Force $raw | Out-Null

$gpu = $Variant -like 'gpu-*'
$exe = if ($gpu) { $AppExe } else { $CpuExe }
$exe = (Resolve-Path $exe).Path
$names = if ($gpu) {
    @('scene1_status', 'scene2_angebot', 'scene3_sprint') * 3 + @('scene1_status')
} else { @('scene3_sprint', 'scene1_status') }
$final = if ($Variant -like '*-qwen') { $qwen } else { 'auto' }
$withNotes = $Variant -ne 'cpu-qwen'

$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ('lva-p7b-qg3-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $sandbox | Out-Null
$sfx = if ($Tag) { "-$Tag" } else { '' }
$simJson = Join-Path $raw "p7b-qg3-$Variant$sfx.sim.json"
$simLog = Join-Path $raw "p7b-qg3-$Variant$sfx.log"
$a = @('--simulate-meeting', '--scene-mic', 'mic_echo.wav', '--final-model', $final, '--json', '--out', $simJson)
foreach ($n in $names) { $a += @('--scene', (Join-Path $BenchDir "synth\$n")) }
if ($withNotes) { $a += '--notes' }
if ($withNotes -and $NotesModel) { $a += @('--notes-model', $NotesModel) }
$envs = @{ LVA_MEETINGS_DIR = $sandbox; HF_HOME = $HfHome }
if (-not $gpu) { $envs['CUDA_VISIBLE_DEVICES'] = '-1' }

$free = Get-P7bFreeMb
$cap = [Math]::Min(20480, [Math]::Max(8192, $free - 6144))
Write-Host ("[p7b-qg3] {0}: {1} scenes, final {2}, notes {3}, job cap {4} MB, free {5} MB" -f $Variant, $names.Count, $final, $withNotes, $cap, $free)
if (Test-Path $simJson) { Remove-Item $simJson }
try {
    $run = Invoke-P7bApp -AppExe $exe -Arguments $a -Log $simLog -TimeoutSec $TimeoutSec `
        -MemoryLimitMb $cap -CpuPercent $CpuPercent -MinFreeMb 10240 -Env $envs
} finally {
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
}
if (-not (Test-Path $simJson)) { throw "no simulation JSON (exit $($run.exit_code)), see $simLog" }
$sim = Get-Content $simJson -Raw -Encoding UTF8 | ConvertFrom-Json
$rep = $sim.final.report
$notesMs = if ($sim.notes -and $sim.notes.ran) { [int64]$sim.notes.ms } else { 0 }
$stopToFinal = [int64]$rep.wall_ms
$stopToNotes = $stopToFinal + $notesMs
$summary = [ordered]@{
    gate              = 'QG3'
    variant           = $Variant
    tag               = $Tag
    cpu_cap_percent   = $CpuPercent
    finished          = (Get-Date).ToString('s')
    app_exe           = $exe
    app_exe_mtime     = (Get-Item $exe).LastWriteTime.ToString('s')
    gpu_backend       = $gpu
    scenes            = $names
    audio_ms          = [int64]$sim.audio_ms
    audio_min         = [Math]::Round($sim.audio_ms / 60000.0, 1)
    live_model        = $sim.model
    live_load_ms      = $sim.load_ms
    live_wall_ms      = $sim.wall_ms
    live_segments     = $sim.final.live_segments
    wer_live          = $sim.wer_live
    final_requested   = $final
    final_plan        = $sim.final.plan
    final_model       = $rep.model
    final_kept        = $rep.kept
    final_load_ms     = $rep.load_ms
    final_transcribe_ms = $rep.transcribe_ms
    final_rtf         = $rep.rtf
    final_segments    = $rep.segments
    speakers_state    = $rep.speakers.state
    speakers_wall_ms  = $rep.speakers.wall_ms
    speakers_channels = $rep.speakers.channels
    stop_to_transcript_final_ms = $stopToFinal
    notes             = $sim.notes
    notes_invalid_json_answers = @(Select-String -Path $simLog -Pattern 'Antwort war kein g.+ltiges JSON \(Versuch').Count
    notes_blocks_dropped = @(Select-String -Path $simLog -Pattern 'nicht ausgewertet').Count
    stop_to_notes_ms  = if ($withNotes) { $stopToNotes } else { $null }
    status_after      = $sim.final.status
    qg3_limit_ms      = 180000
    qg3_pass          = if ($gpu -and $withNotes) { ($sim.notes.ok -eq $true) -and ($stopToNotes -le 180000) } else { $null }
    run               = $run
    sim_json          = $simJson
    system_note       = 'job object: below-normal priority, memory cap, CPU hard cap; a parallel cargo build (P5f) was running'
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -Path $Out -Encoding UTF8
Write-Host ("[p7b-qg3] {0}: stop->TranscriptFinal {1:N1} s, notes {2:N1} s, stop->notes {3:N1} s, exit {4}" -f `
    $Variant, ($stopToFinal / 1000.0), ($notesMs / 1000.0), ($stopToNotes / 1000.0), $run.exit_code)
