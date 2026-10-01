#Requires -Version 7.0
<#
.SYNOPSIS
    M8 meetings acceptance harness: runs every scenario that is automatable
    without a microphone, a loopback device or a human, and writes
    docs/m8-evidence/harness-report.md.

.DESCRIPTION
    ASCII only (project rule: em dashes break the PS 5.1 parser).

    Design notes, in the order they cost time to learn:

    * The release binary targets the Windows GUI subsystem. Its stdout is
      visible in a console but a calling script cannot reliably capture it
      (same lesson as selftest-matrix.ps1), so every result travels through
      --out <json> and Start-Process -Wait.
    * The script never dies on its own error path (lesson from M3): each
      scenario runs inside a try/catch, a thrown scenario becomes a FAIL row
      with the exception text, and the report is written even if everything
      before it failed.
    * The scenarios run against the REAL app data directory - the same
      meetings.db the app uses - because that is where the installed model
      lives. They therefore leave test meetings behind ("m8_short_de",
      "m8_import", "m8_sub", "m8_silence_gap", "m8_stereo_44k",
      "Crash-Test"); delete them in the app when you are done. The retention
      scenario temporarily rewrites meeting_audio_retention in
      settings_store.json and restores the whole file afterwards.

    What this harness deliberately does NOT do: anything needing a live
    microphone, WASAPI loopback, a real LLM or the UI. Those scenarios are
    listed as OPEN in the report with step-by-step instructions.

.PARAMETER Scenario
    all (default) or one of: import-wav, import-matrix, silence-timeline,
    log-privacy, retention, orphan-recovery.
#>
[CmdletBinding()]
param(
    [ValidateSet('all', 'import-wav', 'import-matrix', 'silence-timeline',
                 'log-privacy', 'retention', 'orphan-recovery')]
    [string]$Scenario = 'all',
    [string]$AppExe      = "$PSScriptRoot\..\src-tauri\target\release\local-voice-ai.exe",
    [string]$FixtureDir  = "$PSScriptRoot\..\src-tauri\tests\fixtures",
    [string]$ArtifactDir = "$PSScriptRoot\..\..\..\docs\m8-evidence",
    [int]$TimeoutSeconds = 1800,
    # Nur den Preflight (EXE vorhanden, Frontend eingebettet) und Ende: fuer
    # test-m8-verify.ps1 und als schneller Check vor einem langen Lauf.
    [switch]$PreflightOnly
)

# Hilfsfunktionen (Remove-Segments, Test-EmbeddedFrontend), getrennt testbar.
. (Join-Path $PSScriptRoot 'lib\m8-harness.ps1')

# Continue, not Stop: a failing scenario must become a FAIL row, not a dead
# script. Individual risky calls are guarded explicitly.
$ErrorActionPreference = 'Continue'
$script:Results = @()
$script:Notes   = @()

# ------------------------------------------- meetings-DB sandbox (mandatory)
# The harness must NEVER touch the productive meetings.db (M8 acceptance
# ruling). It therefore always runs against a fresh sandbox directory via
# LVA_MEETINGS_DIR, and a guard hard-aborts the whole script if the binary
# ever reports a DB outside that sandbox. LVA_HARNESS_DESTRUCTIVE stays an
# ADDITIONAL safeguard for --make-orphan, not the only one.
$script:ProductiveMeetingsDb = Join-Path $env:APPDATA 'de.wolffappliedai.localvoiceai\meetings\meetings.db'
$script:SandboxDir = Join-Path $env:TEMP ("lva-m8-sandbox-" + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $script:SandboxDir | Out-Null
$env:LVA_MEETINGS_DIR = $script:SandboxDir
Write-Host ("Meetings sandbox (LVA_MEETINGS_DIR): " + $script:SandboxDir) -ForegroundColor DarkGray
$script:Notes += ("sandbox: Harness lief gegen LVA_MEETINGS_DIR=" + $script:SandboxDir + " - die produktive meetings.db wird nie beruehrt; ein Guard bricht sonst hart ab (Exit 10).")

function Assert-SandboxDb {
    param([string]$DbPath)
    if (-not $DbPath) { return }
    $normalized = [System.IO.Path]::GetFullPath($DbPath)
    $sandbox    = [System.IO.Path]::GetFullPath($script:SandboxDir)
    $productive = [System.IO.Path]::GetFullPath($script:ProductiveMeetingsDb)
    $inSandbox  = $normalized.StartsWith($sandbox, [System.StringComparison]::OrdinalIgnoreCase)
    if (($normalized -ieq $productive) -or (-not $inSandbox)) {
        Write-Host ("SAFEGUARD TRIPPED: binary reports meetings DB outside the sandbox: " + $normalized) -ForegroundColor Red
        Write-Host ("Expected under: " + $sandbox + " - aborting the whole harness, productive data at risk.") -ForegroundColor Red
        [Environment]::Exit(10)
    }
}

# ---------------------------------------------------------------- plumbing
function New-Result {
    param($Name, $Pass, $Detail, $Data = $null)
    $script:Results += [pscustomobject]@{
        Scenario = $Name; Pass = [bool]$Pass; Detail = "$Detail"; Data = $Data
    }
    $tag = if ($Pass) { 'PASS' } else { 'FAIL' }
    $color = if ($Pass) { 'Green' } else { 'Red' }
    Write-Host ("[{0}] {1} - {2}" -f $tag, $Name, $Detail) -ForegroundColor $color
}

function Invoke-Scenario {
    param([string]$Name, [scriptblock]$Body)
    if ($Scenario -ne 'all' -and $Scenario -ne $Name) { return }
    Write-Host "`n=== $Name ===" -ForegroundColor Cyan
    try {
        $r = & $Body
        New-Result $Name $r.Pass $r.Detail $r.Data
    } catch {
        New-Result $Name $false ("EXCEPTION: " + $_.Exception.Message)
    }
}

# Runs the headless binary and returns the parsed --out payload.
# Start-Process because `& $exe` returns immediately for a GUI-subsystem
# binary; a result FILE because its stdout cannot be piped back here.
function Invoke-App {
    param([string[]]$CliArgs, [int]$Timeout = $TimeoutSeconds)
    $outFile = Join-Path $env:TEMP ("m8-{0}.json" -f [guid]::NewGuid().ToString('N').Substring(0, 8))
    $errFile = "$outFile.err"
    $soFile  = "$outFile.stdout"
    $q = '"'
    $quoted = @()
    foreach ($a in $CliArgs) {
        if ($a -like '--*') { $quoted += $a } else { $quoted += ($q + $a + $q) }
    }
    $quoted += @('--out', ($q + $outFile + $q))

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $AppExe -ArgumentList $quoted -PassThru `
                          -WindowStyle Hidden -RedirectStandardError $errFile `
                          -RedirectStandardOutput $soFile
    if (-not $proc.WaitForExit($Timeout * 1000)) {
        try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch { }
        throw "timed out after $Timeout s: $($CliArgs -join ' ')"
    }
    $sw.Stop()

    $payload = $null
    if (Test-Path -LiteralPath $outFile) {
        try { $payload = Get-Content -LiteralPath $outFile -Raw | ConvertFrom-Json } catch { }
    }
    $stdout = if (Test-Path -LiteralPath $soFile) { (Get-Content -LiteralPath $soFile -Raw) } else { '' }
    $stderr = if (Test-Path -LiteralPath $errFile) { (Get-Content -LiteralPath $errFile -Raw) } else { '' }

    # Fallback channel. The --out file is the primary one (a GUI-subsystem
    # binary's stdout is not dependable), but when stdout DID come through we
    # can still recover the key/value lines from it rather than fail the run.
    if ($null -eq $payload -and $stdout -match 'MEETING_ID=([0-9A-Za-z]+)') {
        $payload = [pscustomobject]@{ meeting_id = $Matches[1] }
        if ($stdout -match 'ORPHAN_WAV=(.+)')  { $payload | Add-Member orphan_wav $Matches[1].Trim() }
        if ($stdout -match 'IMPORT_MS=(\d+)')  { $payload | Add-Member import_ms ([int]$Matches[1]) }
        if ($stdout -match 'DB=(.+)')          { $payload | Add-Member db $Matches[1].Trim() }
    }
    Remove-Item -LiteralPath $outFile, $errFile, $soFile -Force -ErrorAction SilentlyContinue

    if ($null -eq $payload) {
        $tail = ($stderr -split "`n" | Select-Object -Last 6) -join ' | '
        throw ("no result from '{0}' (exit {1}): {2}" -f ($CliArgs -join ' '), $proc.ExitCode, $tail)
    }
    # Sandbox guard: every run that reports its DB must report the sandbox DB.
    if ($payload.PSObject.Properties['db']) { Assert-SandboxDb $payload.db }
    return [pscustomobject]@{
        Payload  = $payload
        ExitCode = $proc.ExitCode
        Stdout   = $stdout
        Stderr   = $stderr
        WallMs   = [int]$sw.ElapsedMilliseconds
    }
}

function Import-Fixture {
    param([string]$File)
    $path = Join-Path $FixtureDir $File
    if (-not (Test-Path $path)) { throw "fixture missing: $path (run make-m8-fixtures.ps1)" }
    $run = Invoke-App @('--import-meeting', $path)
    if (-not $run.Payload.meeting_id) { throw "import produced no meeting_id for $File" }
    $script:CreatedMeetings += $run.Payload.meeting_id
    return [pscustomobject]@{
        Id       = $run.Payload.meeting_id
        Db       = $run.Payload.db
        ImportMs = [int]$run.Payload.import_ms
        WallMs   = $run.WallMs
        # The full post-import state, observed inside the importing process.
        # A separate --dump-meeting run cannot see it under a short retention
        # policy: every meetings run purges due audio at startup, so by the
        # time a second process looks, the audio it should have seen is gone.
        State    = $run.Payload
    }
}

function Get-MeetingDump {
    param([string]$Id)
    return (Invoke-App @('--dump-meeting', $Id)).Payload
}

# ---------------------------------------------------------------- preflight
if (-not (Test-Path $AppExe)) {
    Write-Host "binary not found: $AppExe - build with 'dev.ps1 build' (cargo build --release yields a non-working binary, see docs/BUILD-WINDOWS.md)" -ForegroundColor Red
    exit 2
}
# Der Harness laeuft headless und braucht kein Frontend. Aber `cargo build --release`
# (fuer den Harness-Umbau noetig) ueberschreibt die GUI-taugliche EXE in
# target\release mit einer ohne eingebettetes Frontend: headless faellt das nie auf,
# der GUI-Start ist danach tot (Vorfall 20.08.). Darum VOR dem Lauf warnen (#15).
$frontend = Test-EmbeddedFrontend -Exe $AppExe
if (-not $frontend.GuiCapable) {
    $warning = "diese EXE ist headless-tauglich, aber nicht GUI-tauglich ($($frontend.Reason))"
    Write-Host ("WARNUNG: $warning. Nach dem Harness ggf. neu bauen: npx tauri build --no-bundle (docs/BUILD-WINDOWS.md, Stolperstein 3).") -ForegroundColor Yellow
    $script:Notes += "preflight: $warning - nach dem Harness ggf. 'npx tauri build --no-bundle'."
}
if ($PreflightOnly) {
    Write-Host 'Preflight beendet (-PreflightOnly).'
    exit 0
}
New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$ArtifactDir = (Resolve-Path $ArtifactDir).Path
$script:CreatedMeetings = @()

$manifestPath = Join-Path $FixtureDir 'm8_fixtures.json'
$manifest = $null
if (Test-Path $manifestPath) {
    try { $manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json } catch { }
}
$privacyWords = if ($manifest) { $manifest.privacy_words } else { @('Quartalsbericht', 'Nordlicht') }
$vttCues = if ($manifest) { $manifest.vtt_cues_ms } else { $null }

$LogDir  = Join-Path $env:LOCALAPPDATA 'de.wolffappliedai.localvoiceai\logs'
$LogFile = Join-Path $LogDir 'handy.log'
$SettingsFile = Join-Path $env:APPDATA 'de.wolffappliedai.localvoiceai\settings_store.json'

Write-Host "app:      $AppExe"
Write-Host "fixtures: $FixtureDir"
Write-Host "log:      $LogFile"

# ---------------------------------------------------------------- 1. import-wav
Invoke-Scenario 'import-wav' {
    $imp = Import-Fixture 'm8_short_de.wav'
    $d = Get-MeetingDump $imp.Id
    $script:LastWavImport = $d

    $checks = @(
        @{ n = 'status ready';        ok = ($d.status -eq 'ready') },
        @{ n = 'segments >= 1';       ok = ($d.segment_count -ge 1) },
        @{ n = 'channel = 2 (mixed)'; ok = (@($d.channels) -join ',') -eq '2' },
        @{ n = 'transcript non-empty'; ok = ($d.total_text_chars -gt 0) },
        @{ n = 'duration ~60 s';      ok = ($d.duration_ms -ge 58000 -and $d.duration_ms -le 62000) }
    )
    $failed = @($checks | Where-Object { -not $_.ok } | ForEach-Object { $_.n })
    $detail = "status={0}, segments={1}, channels=[{2}], chars={3}, duration={4} ms, import={5} ms" -f `
        $d.status, $d.segment_count, (@($d.channels) -join ','), $d.total_text_chars,
        $d.duration_ms, $imp.ImportMs
    if ($failed.Count) { $detail += "; FAILED: " + ($failed -join ', ') }
    return @{ Pass = ($failed.Count -eq 0); Detail = $detail; Data = $d }
}

# ---------------------------------------------------------------- 2. import-matrix
Invoke-Scenario 'import-matrix' {
    $rows = @()
    $allOk = $true

    # a) same content as MP4 (ffmpeg mux) - exercises media::ensure_wav.
    $mp4 = Import-Fixture 'm8_import.mp4'
    $dm = Get-MeetingDump $mp4.Id
    $ok = ($dm.status -eq 'ready') -and ($dm.segment_count -ge 1) -and ($dm.total_text_chars -gt 0)
    $allOk = $allOk -and $ok
    $rows += "mp4: status={0}, segments={1}, chars={2}, {3} ms" -f `
        $dm.status, $dm.segment_count, $dm.total_text_chars, $mp4.ImportMs

    # b) VTT: exactly three cues, at the times the fixture manifest declares.
    $vtt = Import-Fixture 'm8_sub.vtt'
    $dv = Get-MeetingDump $vtt.Id
    $cueOk = ($dv.status -eq 'ready') -and ($dv.segment_count -eq 3)
    if ($cueOk -and $vttCues) {
        for ($i = 0; $i -lt 3; $i++) {
            if ([int]$dv.segments[$i].start_ms -ne [int]$vttCues[$i].start_ms) { $cueOk = $false }
            if ([int]$dv.segments[$i].end_ms   -ne [int]$vttCues[$i].end_ms)   { $cueOk = $false }
        }
    }
    $allOk = $allOk -and $cueOk
    $rows += "vtt: status={0}, segments={1}, times=[{2}]" -f $dv.status, $dv.segment_count,
        ((@($dv.segments) | ForEach-Object { "$($_.start_ms)-$($_.end_ms)" }) -join ' ')

    # c) 44.1 kHz STEREO - the resample/downmix branch of
    #    read_wav_i16_mono_16k (deferred Task 9 review item). A broken
    #    resample shows up as a duration that is off by the 44100/16000
    #    ratio, so the duration assertion IS the resample assertion.
    $st = Import-Fixture 'm8_stereo_44k.wav'
    $ds = Get-MeetingDump $st.Id
    $stOk = ($ds.status -eq 'ready') -and ($ds.segment_count -ge 1) -and
            ($ds.duration_ms -ge 58000 -and $ds.duration_ms -le 62000) -and
            ($ds.total_text_chars -gt 0)
    $allOk = $allOk -and $stOk
    $rows += "stereo-44k1: status={0}, segments={1}, duration={2} ms, chars={3}, {4} ms" -f `
        $ds.status, $ds.segment_count, $ds.duration_ms, $ds.total_text_chars, $st.ImportMs

    return @{ Pass = $allOk; Detail = ($rows -join ' || '); Data = @{ mp4 = $dm; vtt = $dv; stereo = $ds } }
}

# ---------------------------------------------------------------- 3. C1 silence
Invoke-Scenario 'silence-timeline' {
    # 3 min speech, 3 min silence, 4 min speech. If the batch path dropped
    # silence from the timeline (instead of merely producing no segments for
    # it), everything after the gap would slide 180 s earlier and the last
    # segment would land around 240 s instead of past 350 s.
    $imp = Import-Fixture 'm8_silence_gap.wav'
    $d = Get-MeetingDump $imp.Id

    $lastStart = [int64]$d.last_start_ms

    # The sharpest single number: the largest hole between two consecutive
    # segments. The fixture's silence window is 180 s, so anything near 180 s
    # means the gap survived on the timeline; a value near 0 would mean the
    # silence was squeezed out and everything after it slid forward.
    $segs = @($d.segments)
    $maxGap = 0; $gapAt = 0
    for ($i = 1; $i -lt $segs.Count; $i++) {
        $g = [int64]$segs[$i].start_ms - [int64]$segs[$i - 1].end_ms
        if ($g -gt $maxGap) { $maxGap = $g; $gapAt = [int64]$segs[$i - 1].end_ms }
    }

    $checks = @(
        @{ n = 'status ready';              ok = ($d.status -eq 'ready') },
        @{ n = 'last segment past 350 s';   ok = ($lastStart -gt 350000) },
        @{ n = 'last segment inside file';  ok = ($lastStart -le 600000) },
        @{ n = 'silence gap ~180 s kept';   ok = ($maxGap -ge 150000 -and $maxGap -le 200000) },
        @{ n = 'gap starts near 180 s';     ok = ($gapAt -ge 150000 -and $gapAt -le 200000) },
        @{ n = 'duration ~600 s';           ok = ($d.duration_ms -ge 595000 -and $d.duration_ms -le 605000) }
    )
    $failed = @($checks | Where-Object { -not $_.ok } | ForEach-Object { $_.n })
    $detail = "status={0}, segments={1}, first_start={2} ms, last_start={3} ms, last_end={4} ms, duration={5} ms, silence gap {6} ms starting at {7} ms, import={8} ms" -f `
        $d.status, $d.segment_count, $d.first_start_ms, $d.last_start_ms, $d.last_end_ms,
        $d.duration_ms, $maxGap, $gapAt, $imp.ImportMs
    if ($failed.Count) { $detail += "; FAILED: " + ($failed -join ', ') }
    return @{ Pass = ($failed.Count -eq 0); Detail = $detail; Data = $d }
}

# ---------------------------------------------------------------- 4. log-privacy
Invoke-Scenario 'log-privacy' {
    # Reads the real log after a real import instead of auditing the source.
    # An audit missed a leak once already (M3, 2026-08-17: "Transcription
    # result: {}"), so only measurement counts here.
    if (-not (Test-Path $LogDir)) { throw "log directory not found: $LogDir" }

    $imp = Import-Fixture 'm8_short_de.wav'
    $d = Get-MeetingDump $imp.Id
    if ([int]$d.total_text_chars -le 0) {
        return @{ Pass = $false
                  Detail = 'INCONCLUSIVE - the import produced no text, so nothing could leak'
                  Data = $d }
    }

    # handy.log rotates at 500 KB (KeepOne), so probing only the current file
    # would be blind to a leak that has already rotated away. The sweep takes
    # every handy.log* in the log directory - the rotated copies are just as
    # readable, and just as much a data leak.
    $logFiles = @(Get-ChildItem -Path $LogDir -Filter 'handy.log*' -File -ErrorAction SilentlyContinue |
                  Sort-Object Name)
    if ($logFiles.Count -eq 0) { throw "no log files found in: $LogDir" }

    $leaks = @()
    foreach ($w in $privacyWords) {
        $hits = @(Select-String -Path $logFiles.FullName -Pattern ([regex]::Escape($w)) -ErrorAction SilentlyContinue)
        if ($hits.Count -gt 0) {
            $where = ($hits | ForEach-Object { Split-Path -Leaf $_.Path } | Sort-Object -Unique) -join '/'
            $leaks += ("{0} x{1} in {2}" -f $w, $hits.Count, $where)
        }
    }
    $totalKb = [math]::Round((($logFiles | Measure-Object Length -Sum).Sum) / 1KB, 1)
    $fileList = ($logFiles | ForEach-Object { "{0} ({1} KB)" -f $_.Name, [math]::Round($_.Length / 1KB, 1) }) -join ', '
    $script:Notes += ("log-privacy: geprueft wurden alle {0} Logdatei(en) im Log-Verzeichnis (Rotation bei 500 KB, KeepOne): {1}." -f $logFiles.Count, $fileList)
    $detail = if ($leaks.Count -eq 0) {
        "no spoken word appears in any log file ({0} words probed, transcript {1} chars, {2} file(s), {3} KB total: {4})" -f `
            @($privacyWords).Count, $d.total_text_chars, $logFiles.Count, $totalKb, $fileList
    } else {
        "LEAK: " + ($leaks -join ', ')
    }
    return @{ Pass = ($leaks.Count -eq 0); Detail = $detail; Data = $d }
}

# ---------------------------------------------------------------- 5. retention
Invoke-Scenario 'retention' {
    if (-not (Test-Path $SettingsFile)) { throw "settings store not found: $SettingsFile" }
    $backup = "$SettingsFile.m8bak"
    Copy-Item -LiteralPath $SettingsFile -Destination $backup -Force

    try {
        # Days(0): the audio expires the moment the meeting ends, so the very
        # next startup purge must remove it. The AfterMinutes default needs a
        # real minutes run (Ollama) and stays a manual scenario.
        $json = Get-Content -LiteralPath $SettingsFile -Raw | ConvertFrom-Json
        $json.settings | Add-Member -NotePropertyName 'meeting_audio_retention' `
                                    -NotePropertyValue @{ days = 0 } -Force
        $json | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $SettingsFile -Encoding UTF8

        $imp = Import-Fixture 'm8_short_de.wav'
        $before = $imp.State
        $wav = $before.mic_audio_path
        $hadFile = [bool]$before.audio_file_exists
        $hadMarker = ($null -ne $before.audio_retention_until)

        # Second headless run: every meetings run does the same startup
        # housekeeping the app does (recover_orphans + purge_due_audio), so
        # this IS the "restart the app" step.
        Start-Sleep -Seconds 2
        $after = Get-MeetingDump $imp.Id

        $checks = @(
            @{ n = 'audio existed before purge';   ok = $hadFile },
            @{ n = 'retention marker was set';     ok = $hadMarker },
            @{ n = 'audio file deleted';           ok = ($wav -and -not (Test-Path -LiteralPath $wav)) },
            @{ n = 'mic path nulled';              ok = ($null -eq $after.mic_audio_path) },
            @{ n = 'retention marker cleared';     ok = ($null -eq $after.audio_retention_until) },
            @{ n = 'transcript intact';            ok = ([int]$after.segment_count -eq [int]$before.segment_count -and [int]$after.segment_count -gt 0) },
            @{ n = 'transcript text intact';       ok = ([int]$after.total_text_chars -eq [int]$before.total_text_chars) }
        )
        $failed = @($checks | Where-Object { -not $_.ok } | ForEach-Object { $_.n })
        $detail = "policy=Days(0); before: file={0}, until={1}, segments={2}; after: mic_path={3}, until={4}, segments={5}, chars={6}" -f `
            $hadFile, $before.audio_retention_until, $before.segment_count,
            ($after.mic_audio_path ?? 'null'), ($after.audio_retention_until ?? 'null'),
            $after.segment_count, $after.total_text_chars
        if ($failed.Count) { $detail += "; FAILED: " + ($failed -join ', ') }
        return @{ Pass = ($failed.Count -eq 0); Detail = $detail; Data = @{ before = $before; after = $after } }
    } finally {
        # Restore the whole file, not just the one key: the app may have
        # rewritten other fields while running.
        Copy-Item -LiteralPath $backup -Destination $SettingsFile -Force
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
        $script:Notes += 'retention: settings_store.json wurde nach dem Lauf aus dem Backup zurueckgeschrieben.'
    }
}

# ---------------------------------------------------------------- 6. orphan recovery
Invoke-Scenario 'orphan-recovery' {
    # Covers the DB+WAV half of crash recovery deterministically: a meeting
    # left on 'recording' with a WAV whose RIFF/data sizes were never patched
    # (finalize() never ran), exactly what a hard kill leaves behind. The
    # recovery is NOT simulated - the next run goes through the real
    # recover_orphans() the app runs at startup.
    #
    # The other half (a LIVE recording actually being killed mid-capture)
    # needs a microphone and stays manual; see the report.
    $src = Join-Path $FixtureDir 'm8_short_de.wav'
    if (-not (Test-Path $src)) { throw "fixture missing: $src" }

    # --make-orphan fabricates rows in the real meetings.db, so the release
    # binary refuses it without this explicit opt-in. Set for exactly one
    # invocation and removed again, whatever happens.
    $env:LVA_HARNESS_DESTRUCTIVE = '1'
    try {
        $mk = Invoke-App @('--make-orphan', $src)
    } finally {
        Remove-Item Env:\LVA_HARNESS_DESTRUCTIVE -ErrorAction SilentlyContinue
    }
    $id = $mk.Payload.meeting_id
    $wav = $mk.Payload.orphan_wav
    $script:CreatedMeetings += $id

    # The declared data-chunk length at byte 40 is exactly what finalize()
    # would have patched and what repair_orphan_wav reconstructs, so read it
    # directly. ffprobe is NOT a usable probe here: it happily decodes to EOF
    # past a zero-length data chunk and reports the full duration either way,
    # which would make a broken file look healthy.
    function Get-WavDataLen {
        param([string]$Path)
        $fs = [System.IO.File]::OpenRead($Path)
        try {
            $buf = New-Object byte[] 44
            $null = $fs.Read($buf, 0, 44)
            return [System.BitConverter]::ToUInt32($buf, 40)
        } finally { $fs.Dispose() }
    }

    $fileSize = (Get-Item -LiteralPath $wav).Length
    $brokenLen = Get-WavDataLen $wav

    $d = Get-MeetingDump $id   # this run's startup housekeeping repairs it
    $fixedLen = Get-WavDataLen $wav
    $fixedDur = & ffprobe -v error -show_entries format=duration -of csv=p=0 -- "$wav" 2>$null
    $fixedDur = if ($fixedDur) { [double]$fixedDur } else { 0 }

    $checks = @(
        @{ n = 'orphan really had an unpatched header'; ok = ($brokenLen -eq 0) },
        @{ n = 'status recovered to ready';             ok = ($d.status -eq 'ready') },
        @{ n = 'data length reconstructed';             ok = ($fixedLen -eq ($fileSize - 44)) },
        @{ n = 'wav plays for its full length';         ok = ($fixedDur -gt 55.0) },
        @{ n = 'segments up to the kill kept';          ok = ([int]$d.segment_count -ge 1) }
    )
    $failed = @($checks | Where-Object { -not $_.ok } | ForEach-Object { $_.n })
    $detail = "orphan wav {0} bytes; declared data length {1} -> {2} (expected {3}); ffprobe after repair {4:N2} s; status={5}, segments={6}" -f `
        $fileSize, $brokenLen, $fixedLen, ($fileSize - 44), $fixedDur, $d.status, $d.segment_count
    $script:Notes += ('orphan-recovery: ``--make-orphan`` schreibt erfundene Zeilen in die echte ' +
        'meetings.db und wird im Release-Binary verweigert, solange nicht ' +
        '``LVA_HARNESS_DESTRUCTIVE=1`` gesetzt ist (Produktivdaten-Integritaet; der Harness ' +
        'laeuft gegen Release). Der Harness setzt die Variable fuer genau diesen einen Aufruf ' +
        'und entfernt sie danach wieder.')
    if ($failed.Count) { $detail += "; FAILED: " + ($failed -join ', ') }
    Remove-Item -LiteralPath (Split-Path $wav) -Recurse -Force -ErrorAction SilentlyContinue
    return @{ Pass = ($failed.Count -eq 0); Detail = $detail; Data = $d }
}

# ---------------------------------------------------------------- report
# Everything below runs no matter what happened above.
try {
    $stamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
    $passed = @($script:Results | Where-Object { $_.Pass }).Count
    $total  = @($script:Results).Count

    $r = @()
    $r += "# M8 meetings - Abnahme-Harness"
    $r += ""
    $r += "Lauf: $stamp | Szenario-Satz: ``$Scenario`` | Binary: ``$AppExe``"
    $r += ""
    $r += "**Automatisiert: $passed/$total PASS.** Alles darunter unter ""Manuell offen"" wurde"
    $r += "NICHT gemessen und ist als offen zu lesen - keine dieser Zeilen ist ein Ergebnis."
    $r += ""
    $r += "Nachstellen:"
    $r += ""
    $r += '```powershell'
    $r += 'cd apps\local-voice'
    $r += '.\scripts\make-m8-fixtures.ps1        # idempotent, ueberspringt vorhandene Fixtures'
    $r += '.\scripts\m8-verify.ps1               # oder -Scenario <name> fuer einen einzelnen Fall'
    $r += '```'
    $r += ""
    $r += "Die Szenarien laufen headless ueber die CLI-Flags ``--import-meeting``,"
    $r += "``--dump-meeting`` und ``--make-orphan``. Jeder Meetings-Lauf macht vorher"
    $r += "dieselbe Startroutine wie die App (``recover_orphans`` + ``purge_due_audio``);"
    $r += "ein zweiter Lauf ist damit exakt ein App-Neustart."
    $r += ""
    $r += "| Szenario | Ergebnis | Messung |"
    $r += "|---|---|---|"
    foreach ($x in $script:Results) {
        $r += "| {0} | {1} | {2} |" -f $x.Scenario, $(if ($x.Pass) { 'PASS' } else { 'FAIL' }),
              ($x.Detail -replace '\|', '/')
    }
    $r += ""

    # Full segment arrays would make this file unreadable; `Remove-Segments`
    # (lib\m8-harness.ps1) drops the bodies and keeps counts and boundary times.

    foreach ($x in $script:Results) {
        if ($null -ne $x.Data) {
            $r += "### $($x.Scenario) - Rohdaten (ohne Segmenttexte)"
            $r += '```json'
            try {
                $r += ((Remove-Segments $x.Data) | ConvertTo-Json -Depth 6)
            } catch { $r += '(nicht serialisierbar)' }
            $r += '```'
            $r += ""
        }
    }

    if ($script:CreatedMeetings.Count) {
        $r += "### Zurueckgelassene Test-Meetings"
        $r += ""
        $r += "Der Harness laeuft gegen die echte ``meetings.db`` (dort liegt das installierte"
        $r += "Modell). Diese Meetings bleiben stehen und koennen in der App geloescht werden:"
        $r += ""
        foreach ($id in $script:CreatedMeetings) { $r += "- ``$id``" }
        $r += ""
    }
    foreach ($n in $script:Notes) { $r += "- $n" }
    $r += ""

    $r += @'
## Manuell offen (braucht Patrick am Geraet)

Diese Szenarien brauchen Mikrofon, WASAPI-Loopback, echte Wanduhr-Zeit, die UI
oder einen laufenden Ollama-Server. Sie wurden NICHT gemessen. Fuer jedes steht
unten der Ablauf und das erwartete Ergebnis; die Ist-Spalte bleibt leer, bis
jemand sie ausfuellt.

### M1 Clock-Drift ueber >= 60 min (Spec C2)

1. Referenzdatei bereitlegen: eine Mediendatei mit bekannter Laenge >= 60 min
   (z. B. `ffmpeg -f lavfi -i sine=f=440:d=3600 -ar 48000 ref60.wav`).
2. App starten, Meetings-Bereich, Consent bestaetigen, Systemton-Mitschnitt AN.
3. Aufnahme starten, gleichzeitig `ref60.wav` ueber die Standard-Wiedergabe
   abspielen. Startzeit per Uhr notieren.
4. Nach >= 60 min Aufnahme stoppen, Stoppzeit notieren.
5. Beide WAVs im Meeting-Ordner messen:
   `ffprobe -v error -show_entries format=duration -of csv=p=0 mic.wav`
   dasselbe fuer `system.wav`.

| Groesse | Soll | Ist |
|---|---|---|
| Wanduhr-Dauer | >= 3600 s | |
| `mic.wav` Dauer | Wanduhr +/- 0,5 s je Stunde | |
| `system.wav` Dauer | Wanduhr +/- 0,5 s je Stunde | |
| Differenz mic/system | < 500 ms pro Stunde | |
| Letztes Segment `start_ms` | innerhalb 2 s vor Aufnahmeende | |

Messhilfe fuer das letzte Segment:
`local-voice-ai.exe --dump-meeting <ID> --out drift.json`

### M2 Loopback-Stille (Live-Variante von C1)

1. Aufnahme starten, 3 min sprechen.
2. 3 min NICHTS - kein Mikro, keine Wiedergabe (echte Stille, nicht Mute).
3. 4 min sprechen, stoppen.

| Groesse | Soll | Ist |
|---|---|---|
| Meeting-Status | `ready` | |
| Letztes Segment `start_ms` | > 350000 | |
| `mic.wav` Dauer | ~600 s | |
| Segmente im Stillefenster | keine (oder leerer Text) | |

Der Batch-Zwilling dieses Falls (`silence-timeline`) ist oben automatisiert und
gemessen - was hier fehlt, ist ausschliesslich der Live-Capture-Pfad.

### M3 Crash-Recovery einer LIVEN Aufnahme

Der DB-/WAV-Teil ist oben als `orphan-recovery` automatisiert gemessen. Offen
bleibt der echte Kill mitten im Capture:

1. Aufnahme starten, ca. 2 min sprechen.
2. `Stop-Process -Name local-voice-ai -Force` (kein sauberes Beenden).
3. App neu starten, Meetings oeffnen.

| Groesse | Soll | Ist |
|---|---|---|
| Meeting-Status nach Neustart | `ready` (war `recording`) | |
| `mic.wav` per ffprobe lesbar | ja, ~2 min | |
| Segmente bis zum Kill | vorhanden | |
| Log-Zeile | `meetings: recovered N orphan(s)` | |

### M4 Consent-Gate in der UI (Spec A1)

1. App frisch starten, Meetings oeffnen.
2. Aufnahme starten OHNE die Einwilligung zu bestaetigen.

| Groesse | Soll | Ist |
|---|---|---|
| Fehler | `consent_required` (uebersetzt angezeigt) | |
| Neue Meeting-Zeile | keine | |
| Aufnahmeindikator | bleibt aus | |

Gegenprobe: Einwilligung bestaetigen, starten - Meeting entsteht,
`consent_confirmed_at` ist gesetzt (`--dump-meeting <ID>`).

### M5 Loopback-Hoertest (Qualitaet)

Rein subjektiv, deshalb nicht automatisierbar: eine Videokonferenz mitschneiden
und `system.wav` anhoeren.

| Groesse | Soll | Ist |
|---|---|---|
| Gegenstelle verstaendlich | ja | |
| Aussetzer / Knacken | keine | |
| Lautstaerke | ohne Nachverstaerkung hoerbar | |

### M6 Protokoll mit echtem Ollama

1. Ollama starten, Modell laden.
2. Ein importiertes Meeting oeffnen, Protokoll erzeugen.

| Groesse | Soll | Ist |
|---|---|---|
| Protokoll-Dokument | entsteht, Schema-valide | |
| Redeanteile | aus den Segmenten gerechnet, nicht vom LLM erfunden | |
| Retention `AfterMinutes` | Audio direkt danach geloescht, Pfade genullt | |
| Transkript | unveraendert vorhanden | |

Der Retention-Teil ist oben unter `retention` mit `Days(0)` automatisiert
gemessen; offen ist nur der `AfterMinutes`-Ausloeser ueber ein echtes Protokoll.
'@

    $reportPath = Join-Path $ArtifactDir 'harness-report.md'
    ($r -join "`n") | Out-File -LiteralPath $reportPath -Encoding UTF8
    Write-Host "`n$passed/$total automatisierte Szenarien bestanden" -ForegroundColor Cyan
    Write-Host "Report: $reportPath"
} catch {
    Write-Host "report writing failed: $($_.Exception.Message)" -ForegroundColor Red
}

if (@($script:Results | Where-Object { -not $_.Pass }).Count -gt 0) { exit 1 }
exit 0
