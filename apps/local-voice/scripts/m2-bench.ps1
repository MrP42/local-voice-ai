<#
.SYNOPSIS
    M2 live benchmark (Goal granola-besprechungen, P2b2): AK5 latency and live WER.

.DESCRIPTION
    Pushes the synthetic multi-speaker scenes from %LOCALAPPDATA%\lva-bench\synth through
    the app's headless --simulate-meeting --realtime path: the SAME DSP thread (VAD, echo
    cancellation) and transcription worker as a live meeting, paced by the wall clock.
    Per segment the app reports vad_end_ms (end of the utterance, audio time) and
    emitted_at_ms (wall clock since start); latency = emitted_at_ms - vad_end_ms.
    Live WER is computed in Rust against reference.json (selftest::SelfTestResult).
    Then the FLEURS sentence benchmark from P2b1 (scripts/bench/sentence_bench.py).

      -Quick   scene1_status (4.3 min) in real time + 20 FLEURS sentences (smoke results)
      -Full    scene3_sprint + scene1_status (12.8 min >= 10 min) in real time
               + the full FLEURS sentence benchmark (resumes; -FreshFleurs re-measures)

    Output: one JSON object (stdout and -Out, default results\live\m2-live-<mode>.json) with latency_p50_ms, latency_p95_ms,
    latency_max_ms, wer_live, wer_live_ich, wer_live_gegen, fleurs_wer, ak5_pass.

    System protection: one process at a time, RAM start gate, the app runs in a job
    object (below-normal priority, memory cap, CPU hard cap, kill on close), a timeout
    per run, and the meetings sandbox LVA_MEETINGS_DIR (a fresh temp dir, removed after).

.EXAMPLE
    pwsh apps/local-voice/scripts/m2-bench.ps1 -Quick
    pwsh apps/local-voice/scripts/m2-bench.ps1 -Full -Out C:\temp\m2-full.json
#>
[CmdletBinding()]
param(
    [switch]$Quick,
    [switch]$Full,
    # Default: <script dir>\..\src-tauri\target\release\local-voice-ai.exe (set below:
    # $PSScriptRoot is empty inside param() defaults on Windows PowerShell 5.1).
    [string]$AppExe = '',
    # Live model id; empty = the meeting model configured in the app.
    [string]$Model = '',
    [string]$BenchDir = '',
    [string]$Out = '',
    # Microphone track of each scene: mic_echo.wav (speaker echo, AEC on) or mic.wav.
    [string]$SceneMic = 'mic_echo.wav',
    [switch]$NoAec,
    [switch]$SkipFleurs,
    [switch]$FreshFleurs,
    [int]$MemoryLimitMb = 8192,
    [int]$CpuPercent = 50,
    [int]$MinFreeMb = 4096
)

$ErrorActionPreference = 'Stop'
if ($Quick -and $Full) { throw 'use either -Quick or -Full' }
if (-not $Full) { $Quick = $true }
$mode = if ($Full) { 'full' } else { 'quick' }

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here '..\..\..')).Path
if (-not $AppExe) { $AppExe = Join-Path $here '..\src-tauri\target\release\local-voice-ai.exe' }
$AppExe = (Resolve-Path $AppExe).Path
if (-not $BenchDir) {
    $BenchDir = if ($env:LVA_BENCH_DIR) { $env:LVA_BENCH_DIR } else { Join-Path $env:LOCALAPPDATA 'lva-bench' }
}
$results = Join-Path $BenchDir 'results'
# Own files go to results\live: sentence_bench.py reads every results\*.json as a model result.
$liveDir = Join-Path $results 'live'
New-Item -ItemType Directory -Force $liveDir | Out-Null
if (-not $Out) { $Out = Join-Path $liveDir "m2-live-$mode.json" }

$sceneNames = if ($Full) { @('scene3_sprint', 'scene1_status') } else { @('scene1_status') }
$scenes = foreach ($n in $sceneNames) {
    $d = Join-Path $BenchDir "synth\$n"
    foreach ($f in @($SceneMic, 'system.wav', 'reference.json')) {
        if (-not (Test-Path (Join-Path $d $f))) {
            throw "$d\$f missing - first: python scripts/bench/make_corpus.py synth"
        }
    }
    $d
}

# ------------------------------------------------------------------ job object
if (-not ('LvaBench.Job' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace LvaBench {
    public static class Job {
        [StructLayout(LayoutKind.Sequential)]
        struct Basic {
            public long PerProcessUserTimeLimit; public long PerJobUserTimeLimit;
            public uint LimitFlags; public UIntPtr MinimumWorkingSetSize; public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit; public UIntPtr Affinity; public uint PriorityClass; public uint SchedulingClass;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct Io { public ulong R, W, O, RB, WB, OB; }
        [StructLayout(LayoutKind.Sequential)]
        struct Ext {
            public Basic BasicLimit; public Io IoInfo; public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit; public UIntPtr PeakProcessMemoryUsed; public UIntPtr PeakJobMemoryUsed;
        }
        [StructLayout(LayoutKind.Sequential)]
        struct CpuRate { public uint ControlFlags; public uint Rate; }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref Ext info, uint length);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetInformationJobObject(IntPtr job, int infoClass, ref CpuRate info, uint length);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
        [DllImport("kernel32.dll")]
        public static extern bool CloseHandle(IntPtr handle);

        // Job with memory cap, below-normal priority, kill on close and a CPU hard cap.
        public static IntPtr Create(ulong memoryBytes, uint cpuPercent) {
            IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
            if (job == IntPtr.Zero) throw new Exception("CreateJobObject " + Marshal.GetLastWin32Error());
            Ext ext = new Ext();
            ext.BasicLimit.LimitFlags = 0x200 | 0x20 | 0x2000; // JOB_MEMORY | PRIORITY_CLASS | KILL_ON_JOB_CLOSE
            ext.BasicLimit.PriorityClass = 0x4000;              // BELOW_NORMAL_PRIORITY_CLASS
            ext.JobMemoryLimit = new UIntPtr(memoryBytes);
            if (!SetInformationJobObject(job, 9, ref ext, (uint)Marshal.SizeOf(typeof(Ext))))
                throw new Exception("SetInformationJobObject(limits) " + Marshal.GetLastWin32Error());
            CpuRate cpu = new CpuRate();
            cpu.ControlFlags = 0x1 | 0x4;                       // ENABLE | HARD_CAP
            cpu.Rate = Math.Max(10u, Math.Min(100u, cpuPercent)) * 100u;
            SetInformationJobObject(job, 15, ref cpu, (uint)Marshal.SizeOf(typeof(CpuRate)));
            return job;
        }
        public static void Assign(IntPtr job, IntPtr process) {
            if (!AssignProcessToJobObject(job, process))
                throw new Exception("AssignProcessToJobObject " + Marshal.GetLastWin32Error());
        }
    }
}
'@
}

function Get-FreeMb {
    [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024)
}

function Assert-RamGate {
    $free = Get-FreeMb
    if ($free -lt $MinFreeMb) { throw "RAM gate: only $free MB free (< $MinFreeMb MB) - nothing started" }
}

function Format-Arg([string]$a) {
    if ($a -match '[\s"]') { return '"' + ($a -replace '"', '\"') + '"' }
    return $a
}

# Runs the app once in a job object; returns the exit code. Kills the tree on timeout.
function Invoke-Guarded([string[]]$Arguments, [int]$TimeoutSec, [string]$ErrLog) {
    Assert-RamGate
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $AppExe
    $psi.Arguments = ($Arguments | ForEach-Object { Format-Arg $_ }) -join ' '
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    $job = [LvaBench.Job]::Create([uint64]$MemoryLimitMb * 1MB, [uint32]$CpuPercent)
    try {
        $p = [System.Diagnostics.Process]::Start($psi)
        [LvaBench.Job]::Assign($job, $p.Handle)
        $errTask = $p.StandardError.ReadToEndAsync()
        $outTask = $p.StandardOutput.ReadToEndAsync()
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            & taskkill.exe /PID $p.Id /T /F | Out-Null
            throw "timeout after $TimeoutSec s (pid $($p.Id) killed)"
        }
        $p.WaitForExit()
        Set-Content -Path $ErrLog -Value ($errTask.Result + $outTask.Result) -Encoding UTF8
        return $p.ExitCode
    } finally {
        [LvaBench.Job]::CloseHandle($job) | Out-Null
    }
}

# ------------------------------------------------------------------ live run
$audioMs = 0
foreach ($d in $scenes) {
    $audioMs += [int64](Get-Content (Join-Path $d 'reference.json') -Raw -Encoding UTF8 | ConvertFrom-Json).duration_ms
}
$sandbox = Join-Path ([System.IO.Path]::GetTempPath()) ("lva-m2-bench-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force $sandbox | Out-Null
$simJson = Join-Path $liveDir "m2-live-$mode.sim.json"
$simLog = Join-Path $liveDir "m2-live-$mode.sim.log"
$simArgs = @('--simulate-meeting', '--realtime', '--scene-mic', $SceneMic, '--json', '--out', $simJson)
foreach ($d in $scenes) { $simArgs += @('--scene', $d) }
if ($Model) { $simArgs += @('--model', $Model) }
if ($NoAec) { $simArgs += '--no-aec' }

Write-Host ("[m2-bench] {0}: {1} in real time ({2:N1} min), sandbox {3}" -f $mode, ($sceneNames -join ' + '), ($audioMs / 60000.0), $sandbox)
$prevSandbox = $env:LVA_MEETINGS_DIR
$env:LVA_MEETINGS_DIR = $sandbox
try {
    if (Test-Path $simJson) { Remove-Item $simJson }
    $rc = Invoke-Guarded $simArgs ([int]($audioMs / 1000 * 1.5) + 300) $simLog
} finally {
    $env:LVA_MEETINGS_DIR = $prevSandbox
    Remove-Item -Recurse -Force $sandbox -ErrorAction SilentlyContinue
}
if ($rc -ne 0 -or -not (Test-Path $simJson)) { throw "simulation failed (exit $rc), see $simLog" }
$sim = Get-Content $simJson -Raw -Encoding UTF8 | ConvertFrom-Json

# ------------------------------------------------------------------ FLEURS sentences (P2b1)
$fleurs = $null
if (-not $SkipFleurs) {
    $liveModel = [string]$sim.model
    if ($liveModel -eq 'parakeet-tdt-0.6b-v3') { $spec = 'parakeet-onnx'; $alias = 'parakeet-onnx' }
    else { $spec = "cli:$liveModel"; $alias = $liveModel -replace '/', '_' }
    $pyArgs = @((Join-Path $repo 'scripts\bench\sentence_bench.py'), '--models', $spec)
    if ($Quick) { $pyArgs += @('--limit', '20', '--fresh') }
    elseif ($FreshFleurs) { $pyArgs += '--fresh' }
    $prevCli = $env:LVA_BENCH_CLI
    $prevDir = $env:LVA_BENCH_DIR
    $env:LVA_BENCH_CLI = $AppExe
    $env:LVA_BENCH_DIR = $BenchDir
    try {
        Assert-RamGate
        & python @pyArgs | Write-Host
        if ($LASTEXITCODE -ne 0) { throw "sentence_bench.py failed (exit $LASTEXITCODE)" }
    } finally {
        $env:LVA_BENCH_CLI = $prevCli
        $env:LVA_BENCH_DIR = $prevDir
    }
    $fdir = if ($Quick) { Join-Path $results 'smoke' } else { $results }
    $agg = (Get-Content (Join-Path $fdir "$alias.json") -Raw -Encoding UTF8 | ConvertFrom-Json).aggregate
    $fleurs = [ordered]@{ sentences = $agg.sentences; wer = $agg.wer; errors = $agg.errors; ref_words = $agg.ref_words; rtf = $agg.rtf; failed = $agg.failed }
}

# ------------------------------------------------------------------ summary
$p95 = $sim.latency_p95_ms
$summary = [ordered]@{
    mode              = $mode
    finished          = (Get-Date).ToString('s')
    app_exe           = $AppExe
    app_exe_mtime     = (Get-Item $AppExe).LastWriteTime.ToString('s')
    model             = $sim.model
    scenes            = $sceneNames
    scene_mic         = $SceneMic
    aec               = $sim.aec
    audio_ms          = $sim.audio_ms
    wall_ms           = $sim.wall_ms
    realtime          = $sim.realtime
    segments          = $sim.latency_ms.count
    latency_p50_ms    = $sim.latency_ms.p50
    latency_p95_ms    = $p95
    latency_max_ms    = $sim.latency_ms.max
    ak5_pass          = ($null -ne $p95 -and $p95 -le 5000 -and $sim.audio_ms -ge 600000)
    wer_live          = $sim.wer_live
    wer_live_ich      = $sim.wer_live_detail.ich.wer
    wer_live_gegen    = $sim.wer_live_detail.gegen.wer
    ich_far_word_leak = $sim.ich_far_word_leak
    echo_dropped      = $sim.echo_dropped
    fleurs            = $fleurs
    fleurs_wer        = if ($fleurs) { $fleurs.wer } else { $null }
    sim_json          = $simJson
}
$json = $summary | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($Out, $json, (New-Object System.Text.UTF8Encoding($false)))
Write-Output $json
