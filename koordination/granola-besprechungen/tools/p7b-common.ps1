<#
.SYNOPSIS
    P7b shared helpers (dot-source): guarded app runs for the QG3/QG4/QG5 measurements.

.DESCRIPTION
    Invoke-P7bApp starts local-voice-ai.exe once
      - in a job object (memory cap, below-normal priority, CPU hard cap, kill on close),
      - behind a RAM start gate (free RAM >= MinFreeMb, else nothing starts),
      - with a timeout (the whole process tree is killed by PID on expiry),
      - with per-run environment overrides (sandbox dirs, test switches),
    and returns exit code, wall time, peak job memory and the log path.
    ASCII only (Windows PowerShell 5.1 parser).
#>

if (-not ('P7b.Job' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace P7b {
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
        static extern bool QueryInformationJobObject(IntPtr job, int infoClass, ref Ext info, uint length, IntPtr ret);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
        [DllImport("kernel32.dll")]
        public static extern bool CloseHandle(IntPtr handle);

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
        public static ulong PeakJobBytes(IntPtr job) {
            Ext ext = new Ext();
            if (!QueryInformationJobObject(job, 9, ref ext, (uint)Marshal.SizeOf(typeof(Ext)), IntPtr.Zero)) return 0;
            return (ulong)ext.PeakJobMemoryUsed;
        }
    }
}
'@
}

function Get-P7bFreeMb {
    [int]((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory / 1024)
}

function Format-P7bArg([string]$a) {
    if ($a -match '[\s"]') { return '"' + ($a -replace '"', '\"') + '"' }
    return $a
}

# Own child processes still alive after a run (llama-server, WebView2) - by parent chain
# is impossible once the parent died, so by name + start time + path.
function Get-P7bLeftovers([datetime]$Since, [string]$AppExe) {
    Get-CimInstance Win32_Process |
        Where-Object {
            $_.CreationDate -ge $Since -and (
                ($_.Name -eq 'local-voice-ai.exe' -and $_.ExecutablePath -eq $AppExe) -or
                ($_.Name -match '^(llama-server|msedgewebview2)\.exe$' -and "$($_.CommandLine) $($_.ExecutablePath)" -match 'localvoiceai|lva-')
            )
        } |
        Select-Object ProcessId, Name, ExecutablePath
}

<#
 Runs the app once. Returns [ordered] with exit_code, wall_ms, peak_job_mb, timed_out,
 leftovers (own processes still running after exit), log.
 -Env: hashtable of environment overrides for this run only (value $null removes).
#>
function Invoke-P7bApp {
    param(
        [Parameter(Mandatory)][string]$AppExe,
        [Parameter(Mandatory)][string[]]$Arguments,
        [Parameter(Mandatory)][string]$Log,
        [int]$TimeoutSec = 1800,
        [int]$MemoryLimitMb = 16384,
        [int]$CpuPercent = 80,
        [int]$MinFreeMb = 8192,
        [hashtable]$Env = @{}
    )
    $free = Get-P7bFreeMb
    if ($free -lt $MinFreeMb) { throw "RAM gate: only $free MB free (< $MinFreeMb MB) - nothing started" }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $AppExe
    $psi.Arguments = ($Arguments | ForEach-Object { Format-P7bArg $_ }) -join ' '
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardError = $true
    $psi.RedirectStandardOutput = $true
    foreach ($k in $Env.Keys) {
        if ($null -eq $Env[$k]) { $psi.EnvironmentVariables.Remove($k) }
        else { $psi.EnvironmentVariables[$k] = [string]$Env[$k] }
    }
    $job = [P7b.Job]::Create([uint64]$MemoryLimitMb * 1MB, [uint32]$CpuPercent)
    $started = Get-Date
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    $peak = 0
    try {
        $p = [System.Diagnostics.Process]::Start($psi)
        [P7b.Job]::Assign($job, $p.Handle)
        $errTask = $p.StandardError.ReadToEndAsync()
        $outTask = $p.StandardOutput.ReadToEndAsync()
        if (-not $p.WaitForExit($TimeoutSec * 1000)) {
            $timedOut = $true
            & taskkill.exe /PID $p.Id /T /F | Out-Null
            $p.WaitForExit(10000) | Out-Null
        }
        $p.WaitForExit()
        $sw.Stop()
        $peak = [P7b.Job]::PeakJobBytes($job)
        $text = ''
        try { $text = $errTask.Result + $outTask.Result } catch { $text = "(output unavailable: $_)" }
        Set-Content -Path $Log -Value $text -Encoding UTF8
        $code = if ($timedOut) { -1 } else { $p.ExitCode }
        # Before closing the job (KILL_ON_JOB_CLOSE would hide what the app left running).
        Start-Sleep -Milliseconds 1500
        $left = @(Get-P7bLeftovers $started $AppExe)
    } finally {
        [P7b.Job]::CloseHandle($job) | Out-Null
    }
    [ordered]@{
        args          = ($Arguments -join ' ')
        env           = $Env
        exit_code     = $code
        timed_out     = $timedOut
        wall_ms       = [int64]$sw.ElapsedMilliseconds
        peak_job_mb   = [int]($peak / 1MB)
        free_mb_start = $free
        leftovers     = $left
        log           = $Log
        started       = $started.ToString('s')
    }
}
