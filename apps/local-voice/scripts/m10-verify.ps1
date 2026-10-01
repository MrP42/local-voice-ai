#Requires -Version 7.0
<#
.SYNOPSIS
    Abnahme des Einfuegens gegen Notepad, Chrome-Textfeld, Word und VS Code (Issue #10). ASCII only.

.DESCRIPTION
    Nachfolger von m3-verify.ps1 fuer die Ziele, die dort fehlten. Je Ziel:
      1. eigene Testinstanz starten (Chrome: eigenes --user-data-dir, lokale
         HTML-Seite mit textarea; Word: neues leeres Dokument per COM; VS Code:
         eigenes --user-data-dir/--extensions-dir, leere Datei; Notepad: leere
         Scratch-Datei),
      2. Fokus setzen und pruefen, dass der Vordergrund wirklich zur Testinstanz
         gehoert (sonst FAIL, es wird KEINE Taste gesendet),
      3. den Einfuegepfad ausloesen,
      4. den Text WIRKLICH zuruecklesen (UI Automation Value-/TextPattern am
         fokussierten Element; Word ueber COM Range.Text; VS Code: UIA, sonst
         Zwischenablage-Rundlauf Strg+A/Strg+C mit Sentinel).
    Ergebnis je Ziel: PASS / FAIL / SKIP (nicht installiert oder fremde Instanz
    laeuft schon) mit Grund. Exit 1 bei mindestens einem FAIL, sonst 0.

    Modus -Mode probe (Standard): ahmt clipboard.rs::paste_via_clipboard nach
    (Zwischenablage setzen, paste_delay_ms warten, Strg+V, paste_delay_after_ms
    warten, Zwischenablage zuruecksetzen) und prueft damit die ZIELSEITE des
    Einfuegepfads. Die App wird dafuer nicht gebraucht und nicht angefasst.
    Fehlt: Hotkey, Mikrofon, Modell.

    Modus -Mode app: wie m3-verify.ps1 - der Hotkey aus dem Settings-Store startet
    die Aufnahme der LAUFENDEN App, eine WAV-Fixture wird ueber die Lautsprecher
    gespielt, der zweite Hotkey stoppt; zurueckgelesen wird der erkannte Text
    (Erwartung: das Wort "Termin"). Das spricht durch die echte App und ihr
    Modell; nur mit eigener Testinstanz ausfuehren, nie gegen die produktive App,
    waehrend man arbeitet.

    Sicherheit (-DryRun zuerst ausfuehren): Nur selbst gestartete Prozesse werden per taskkill /PID /T
    beendet. Laeuft Notepad oder Word schon, wird das Ziel uebersprungen (SKIP),
    statt in eine fremde Instanz zu greifen. Die Zwischenablage (Text) wird
    gesichert und wiederhergestellt; Bilder gehen dabei verloren.

.PARAMETER Target
    notepad, chrome, word, vscode oder all (Standard).

.PARAMETER Mode
    probe (Standard) oder app.

.PARAMETER Scratch
    Arbeitsordner fuer Profile/Testdateien (Standard: Temp\lva-m10-<PID>);
    wird am Ende geloescht.

.PARAMETER DryRun
    Zeigt den Plan je Ziel und prueft nur lesend, ob das Ziel installiert und frei
    ist (Dateien, Registrierung, Prozessliste). Oeffnet nichts, sendet keine Taste,
    legt keinen Scratch-Ordner an. Exit immer 0.

.PARAMETER ReportDir
    Wenn gesetzt, wird dort m10-report.md geschrieben.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\m10-verify.ps1 -Target chrome,word,vscode
#>
[CmdletBinding()]
param(
    [ValidateSet('all', 'notepad', 'chrome', 'word', 'vscode')]
    [string[]]$Target = @('all'),

    [ValidateSet('probe', 'app')]
    [string]$Mode = 'probe',

    [string]$Scratch,
    [string]$ReportDir,
    [string]$FixtureDir,
    [switch]$DryRun,
    [int]$PasteDelayMs = 60,
    [int]$PasteDelayAfterMs = 60,
    [int]$TimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
if (-not $Scratch) { $Scratch = Join-Path ([IO.Path]::GetTempPath()) ("lva-m10-{0}" -f $PID) }
if (-not $FixtureDir) { $FixtureDir = Join-Path $PSScriptRoot '..\src-tauri\tests\fixtures' }
if (-not $DryRun) { New-Item -ItemType Directory -Force -Path $Scratch | Out-Null }
$script:Results = @()

# ---------------------------------------------------------------- Win32 interop
if (-not ('M10.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace M10 {
    public static class Native {
        public delegate bool EnumProc(IntPtr h, IntPtr l);
        [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
        [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        public static extern int GetWindowText(IntPtr h, StringBuilder b, int n);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
        [DllImport("user32.dll")]
        public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, int dx, int dy, uint data, UIntPtr extra);

        // One line per visible, titled top-level window: "hwnd|pid|title".
        public static string[] ListWindows() {
            var l = new List<string>();
            EnumWindows(delegate (IntPtr h, IntPtr x) {
                if (!IsWindowVisible(h)) return true;
                var sb = new StringBuilder(512);
                GetWindowText(h, sb, 512);
                if (sb.Length == 0) return true;
                uint pid; GetWindowThreadProcessId(h, out pid);
                l.Add(h.ToInt64() + "|" + pid + "|" + sb.ToString());
                return true;
            }, IntPtr.Zero);
            return l.ToArray();
        }
        public static uint ForegroundPid() {
            uint pid; GetWindowThreadProcessId(GetForegroundWindow(), out pid); return pid;
        }
    }
}
'@
}

$KEYUP = 0x0002
$VK_CONTROL = 0x11; $VK_A = 0x41; $VK_C = 0x43; $VK_V = 0x56

function Send-Key {
    param([byte[]]$VKeys)
    foreach ($k in $VKeys) { [M10.Native]::keybd_event($k, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40 }
    for ($i = $VKeys.Count - 1; $i -ge 0; $i--) {
        [M10.Native]::keybd_event($VKeys[$i], 0, $KEYUP, [UIntPtr]::Zero); Start-Sleep -Milliseconds 40
    }
}

# ---------------------------------------------------------------- Prozess-Helfer
function Get-CmdProcs { Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine }

# All processes of one test instance: command line contains $Needle, plus the
# given roots, plus every descendant of those.
function Get-InstancePids {
    param([string]$Needle, [int[]]$Roots = @())
    $all = @(Get-CmdProcs)
    $set = New-Object 'System.Collections.Generic.HashSet[int]'
    foreach ($r in $Roots) { [void]$set.Add($r) }
    if ($Needle) {
        foreach ($p in $all) { if ($p.CommandLine -and $p.CommandLine.Contains($Needle)) { [void]$set.Add([int]$p.ProcessId) } }
    }
    do {
        $grew = $false
        foreach ($p in $all) {
            if ($set.Contains([int]$p.ParentProcessId) -and -not $set.Contains([int]$p.ProcessId)) {
                [void]$set.Add([int]$p.ProcessId); $grew = $true
            }
        }
    } while ($grew)
    , @($set)
}

# Starts a test instance WITHOUT the environment of an outer VS Code / Chromium host.
# When this script runs from a VS Code terminal or the Claude Code extension,
# ELECTRON_RUN_AS_NODE=1 makes Code.exe behave as plain Node ("bad option") and
# VSCODE_* / CHROME_CRASHPAD_PIPE_NAME point the child at the OUTER instance.
function Start-ProcessClean {
    param([string]$Path, [string[]]$Arguments)
    $stash = @{}
    foreach ($e in @(Get-ChildItem Env: | Where-Object {
                $_.Name -eq 'ELECTRON_RUN_AS_NODE' -or $_.Name -like 'VSCODE_*' -or $_.Name -eq 'CHROME_CRASHPAD_PIPE_NAME' })) {
        $stash[$e.Name] = $e.Value
        Remove-Item -LiteralPath ("Env:" + $e.Name)
    }
    try { Start-Process $Path -PassThru -ArgumentList $Arguments }
    finally { foreach ($k in $stash.Keys) { Set-Item -LiteralPath ("Env:" + $k) -Value $stash[$k] } }
}

function Invoke-Kill {
    param([int]$ProcId)
    & taskkill.exe /PID $ProcId /T /F 2>&1 | Out-Null
}

# Beendet NUR die uebergebenen PIDs (nie per Name, nie das eigene Skript).
function Stop-Instance {
    param([int[]]$Pids)
    foreach ($p in $Pids) {
        if ($p -eq $PID) { continue }
        if (Get-Process -Id $p -ErrorAction SilentlyContinue) { Invoke-Kill -ProcId $p }
    }
}

function Find-Window {
    param([scriptblock]$PidProvider, [string]$TitleRegex, [int]$Seconds = 30)
    $end = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $end) {
        $pids = & $PidProvider
        foreach ($line in [M10.Native]::ListWindows()) {
            $parts = $line.Split('|', 3)
            if ($pids -contains [int]$parts[1] -and $parts[2] -match $TitleRegex) {
                return [pscustomobject]@{ Hwnd = [IntPtr][int64]$parts[0]; Pid = [int]$parts[1]; Title = $parts[2] }
            }
        }
        Start-Sleep -Milliseconds 400
    }
    $null
}

# Brings the window to the foreground and PROVES it got there. Returns $false
# without sending anything if the foreground belongs to someone else.
function Set-Focus {
    param($Window, [scriptblock]$PidProvider)
    for ($i = 0; $i -lt 6; $i++) {
        [M10.Native]::ShowWindow($Window.Hwnd, 9) | Out-Null
        # A zero-distance mouse move counts as input by this process and lifts the
        # foreground lock. NOT an Alt tap: Chrome treats that as "focus the menu".
        if ($i -gt 0) { [M10.Native]::mouse_event(0x0001, 0, 0, 0, [UIntPtr]::Zero) }
        [M10.Native]::SetForegroundWindow($Window.Hwnd) | Out-Null
        Start-Sleep -Milliseconds 500
        if ((& $PidProvider) -contains [int][M10.Native]::ForegroundPid()) { return $true }
    }
    $false
}

# ---------------------------------------------------------------- UI Automation
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
function Get-ElementText {
    param($El)
    foreach ($pat in @([System.Windows.Automation.ValuePattern]::Pattern, [System.Windows.Automation.TextPattern]::Pattern)) {
        $obj = $null
        if ($El.TryGetCurrentPattern($pat, [ref]$obj)) {
            if ($pat -eq [System.Windows.Automation.ValuePattern]::Pattern) {
                $v = $obj.Current.Value
                if ($null -ne $v) { return [string]$v }
            } else {
                $v = $obj.DocumentRange.GetText(-1)
                if ($null -ne $v) { return [string]$v }
            }
        }
    }
    $null
}

# Text of the FOCUSED element (the field the paste went into), restricted to the
# test instance. Falls back to scanning Edit/Document elements of its windows.
function Read-UiaText {
    param([scriptblock]$PidProvider, [string]$Needle)
    $pids = & $PidProvider
    try {
        $f = [System.Windows.Automation.AutomationElement]::FocusedElement
        if ($f -and ($pids -contains [int]$f.Current.ProcessId)) {
            $t = Get-ElementText $f
            if ($null -ne $t -and ($t.Length -gt 0)) { return $t }
        }
    } catch { }
    try {
        $cond = New-Object System.Windows.Automation.OrCondition(
            (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Edit)),
            (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Document)))
        foreach ($line in [M10.Native]::ListWindows()) {
            $parts = $line.Split('|', 3)
            if ($pids -notcontains [int]$parts[1]) { continue }
            $win = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr][int64]$parts[0])
            foreach ($el in $win.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)) {
                $t = Get-ElementText $el
                if ($t -and $Needle -and $t.Contains($Needle)) { return $t }
            }
        }
    } catch { }
    $null
}

function ConvertTo-Norm {
    param([string]$s)
    if ($null -eq $s) { return '' }
    ($s -replace "`r`n|`r|`n", "`n").Trim()
}

# ---------------------------------------------------------------- Testtext
$token = [Guid]::NewGuid().ToString('N').Substring(0, 8)
# Umlauts and sharp s as [char] so this file stays pure ASCII.
$ae = [string][char]0xE4; $oe = [string][char]0xF6; $ue = [string][char]0xFC; $ss = [string][char]0xDF
$script:Expected = "LVA-M10 $token Stra${ss}e K${oe}ln 19 Pr${ue}fung" + "`n" + "Zweite Zeile: sch${oe}ne Gr${ue}${ss}e, B${ae}ren."
$script:Sentinel = "LVA-M10-SENTINEL-$token"

# ---------------------------------------------------------------- Zielbeschreibungen
$script:State = @{}

$Targets = @{}

$Targets['notepad'] = @{
    Plan = @(
        'Scratch-Datei lva-m10-notepad.txt anlegen, Notepad damit starten (eigene PID)',
        'Fokus pruefen, Text einfuegen (Strg+V)',
        'Zuruecklesen: UI Automation Value-/TextPattern des fokussierten Felds',
        'Nur die gestartete Notepad-PID per taskkill /PID /T beenden')
    Check = {
        if (-not (Get-Command notepad.exe -ErrorAction SilentlyContinue)) { return 'Notepad nicht gefunden' }
        if (Get-Process notepad -ErrorAction SilentlyContinue) { return 'Notepad laeuft bereits (fremde Instanz wird nicht angefasst)' }
        $null
    }
    Start = {
        $file = Join-Path $Scratch 'lva-m10-notepad.txt'
        Set-Content -LiteralPath $file -Value '' -Encoding UTF8 -NoNewline
        $p = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
        $script:State.Roots = @($p.Id)
        Start-Sleep -Milliseconds 1500
        # Windows 11 Notepad can be a launcher stub: take every notepad that appeared.
        $script:State.Roots = @(Get-Process notepad -ErrorAction SilentlyContinue | ForEach-Object Id)
    }
    Pids = { Get-InstancePids -Needle 'lva-m10-notepad' -Roots $script:State.Roots }
    Title = 'lva-m10-notepad'
    Read = { Read-UiaText -PidProvider $Targets['notepad'].Pids -Needle $token }
    Stop = { Stop-Instance (& $Targets['notepad'].Pids) }
}

$chromePath = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1

$Targets['chrome'] = @{
    Plan = @(
        'Chrome mit eigenem --user-data-dir (Scratch) und lokaler HTML-Seite mit textarea starten',
        'Fokus pruefen (Vordergrund gehoert zur Testinstanz), Text einfuegen (Strg+V)',
        'Zuruecklesen: UI Automation ValuePattern des textarea (--force-renderer-accessibility)',
        'Nur Prozesse mit dem Test-Profil (samt Kindern) per taskkill /PID /T beenden')
    Check = { if (-not $chromePath) { 'Chrome nicht installiert' } else { $null } }
    Start = {
        $dir = Join-Path $Scratch 'chrome-profile'
        $html = Join-Path $Scratch 'lva-m10-chrome.html'
        $script:State.Needle = $dir
        Set-Content -LiteralPath $html -Encoding UTF8 -Value @'
<!doctype html><meta charset="utf-8"><title>LVA-M10-Chrome</title>
<textarea id="t" autofocus rows="10" cols="60" aria-label="lva-m10-feld"></textarea>
'@
        $url = ([Uri]$html).AbsoluteUri
        $p = Start-ProcessClean $chromePath @(
            "--user-data-dir=`"$dir`"", '--no-first-run', '--no-default-browser-check',
            '--disable-extensions', '--disable-sync', '--force-renderer-accessibility',
            '--new-window', "`"$url`"")
        $script:State.Roots = @($p.Id)
    }
    Pids = { Get-InstancePids -Needle $script:State.Needle -Roots $script:State.Roots }
    Title = 'LVA-M10-Chrome'
    Read = { Read-UiaText -PidProvider $Targets['chrome'].Pids -Needle $token }
    Stop = { Stop-Instance (& $Targets['chrome'].Pids) }
}

$Targets['word'] = @{
    Plan = @(
        'Word per COM starten, neues leeres Dokument (nur wenn Word nicht schon laeuft)',
        'Fokus pruefen, Text einfuegen (Strg+V)',
        'Zuruecklesen: COM Document.Content.Text',
        'Dokument ohne Speichern schliessen, Word beenden; nur die eigene PID notfalls per taskkill /PID /T')
    Check = {
        if (-not [Type]::GetTypeFromProgID('Word.Application')) { return 'Word nicht installiert (Word.Application nicht registriert)' }
        if (Get-Process WINWORD -ErrorAction SilentlyContinue) { return 'Word laeuft bereits (fremde Instanz wird nicht angefasst)' }
        $null
    }
    Start = {
        $w = New-Object -ComObject Word.Application
        $script:State.Word = $w
        $w.Visible = $true
        $script:State.Doc = $w.Documents.Add()
        $w.Activate()
        $script:State.Roots = @(Get-Process WINWORD -ErrorAction SilentlyContinue | ForEach-Object Id)
    }
    Pids = { Get-InstancePids -Needle $null -Roots $script:State.Roots }
    Title = 'Word'
    Read = {
        try { $script:State.Doc.Content.Text } catch { $null }
    }
    Stop = {
        try { if ($script:State.Doc) { $script:State.Doc.Close(0) } } catch { }   # wdDoNotSaveChanges
        try { if ($script:State.Word) { $script:State.Word.Quit(0) } } catch { }
        try { if ($script:State.Word) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($script:State.Word) } } catch { }
        Start-Sleep -Milliseconds 800
        Stop-Instance (& $Targets['word'].Pids)
    }
}

$codePath = @(
    "$env:LOCALAPPDATA\Programs\Microsoft VS Code\Code.exe",
    "$env:ProgramFiles\Microsoft VS Code\Code.exe"
) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1

$Targets['vscode'] = @{
    Plan = @(
        'VS Code mit eigenem --user-data-dir und --extensions-dir (Scratch), leere Datei, ohne Erweiterungen starten',
        'Fokus pruefen, Text einfuegen (Strg+V)',
        'Zuruecklesen: UI Automation, sonst Zwischenablage-Rundlauf (Sentinel, Strg+A, Strg+C)',
        'Nur Prozesse mit dem Test-Profil (samt Kindern) per taskkill /PID /T beenden')
    Check = { if (-not $codePath) { 'VS Code nicht installiert' } else { $null } }
    Start = {
        $data = Join-Path $Scratch 'vscode-data'
        $ext = Join-Path $Scratch 'vscode-ext'
        New-Item -ItemType Directory -Force -Path (Join-Path $data 'User'), $ext | Out-Null
        $script:State.Needle = $data
        Set-Content -LiteralPath (Join-Path $data 'User\settings.json') -Encoding UTF8 -Value @'
{
  "editor.accessibilitySupport": "on",
  "workbench.startupEditor": "none",
  "telemetry.telemetryLevel": "off",
  "update.mode": "none",
  "window.restoreWindows": "none",
  "security.workspace.trust.enabled": false,
  "files.autoSave": "off"
}
'@
        $file = Join-Path $Scratch 'lva-m10-vscode.txt'
        Set-Content -LiteralPath $file -Value '' -Encoding UTF8 -NoNewline
        $p = Start-ProcessClean $codePath @(
            '--new-window', "--user-data-dir=`"$data`"", "--extensions-dir=`"$ext`"",
            '--disable-extensions', '--skip-release-notes', '--skip-welcome', "`"$file`"")
        $script:State.Roots = @($p.Id)
    }
    Pids = { Get-InstancePids -Needle $script:State.Needle -Roots $script:State.Roots }
    Title = 'lva-m10-vscode'
    Read = {
        $t = Read-UiaText -PidProvider $Targets['vscode'].Pids -Needle $token
        if ($t -and (ConvertTo-Norm $t).Contains($token)) { $script:State.Method = 'UI Automation'; return $t }
        # UIA does not expose the Monaco buffer: round-trip through the clipboard
        # with a sentinel, so a stale clipboard cannot pass for editor content.
        Set-Clipboard -Value $script:Sentinel
        Start-Sleep -Milliseconds 200
        Send-Key @([byte]$VK_CONTROL, [byte]$VK_A); Start-Sleep -Milliseconds 200
        Send-Key @([byte]$VK_CONTROL, [byte]$VK_C); Start-Sleep -Milliseconds 600
        $script:State.Method = 'Zwischenablage-Rundlauf (Strg+A, Strg+C, Sentinel)'
        $c = Get-Clipboard -Raw
        if ($c -eq $script:Sentinel) { return $null }   # copy did not reach the editor
        $c
    }
    Stop = { Stop-Instance (& $Targets['vscode'].Pids) }
}

# ---------------------------------------------------------------- Hotkey / App-Modus
$VkByName = @{
    'ctrl' = 0x11; 'ctrl_left' = 0xA2; 'ctrl_right' = 0xA3; 'shift' = 0x10; 'shift_left' = 0xA0; 'shift_right' = 0xA1
    'alt' = 0x12; 'alt_left' = 0xA4; 'alt_right' = 0xA5; 'super' = 0x5B; 'super_left' = 0x5B; 'super_right' = 0x5C
    'space' = 0x20; 'escape' = 0x1B; 'enter' = 0x0D; 'tab' = 0x09
}
foreach ($c in 'a'..'z') { $VkByName[$c] = [byte][char]::ToUpper($c) }
foreach ($d in 0..9) { $VkByName["$d"] = 0x30 + $d }

function Get-HotkeyVKeys {
    $store = Join-Path $env:APPDATA 'de.wolffappliedai.localvoiceai\settings_store.json'
    if (-not (Test-Path $store)) { throw "settings store not found: $store" }
    $binding = (Get-Content $store -Raw | ConvertFrom-Json).settings.bindings.transcribe.current_binding
    $keys = @()
    foreach ($part in ($binding -split '\+')) {
        $name = $part.Trim().ToLower()
        if (-not $VkByName.ContainsKey($name)) { throw "unmapped hotkey component '$name' (binding '$binding')" }
        $keys += [byte]$VkByName[$name]
    }
    , $keys
}

function Invoke-AppDictation {
    param($Window, [scriptblock]$PidProvider)
    $fixture = Join-Path $FixtureDir 'de_short_01.wav'
    if (-not (Test-Path $fixture)) { throw "Fixture fehlt: $fixture" }
    if (-not (Get-Process local-voice-ai -ErrorAction SilentlyContinue)) { throw 'local-voice-ai laeuft nicht' }
    $hk = Get-HotkeyVKeys
    Send-Key $hk; Start-Sleep -Milliseconds 1200            # Aufnahme an
    (New-Object System.Media.SoundPlayer $fixture).PlaySync()
    Start-Sleep -Milliseconds 600
    if (-not (Set-Focus $Window $PidProvider)) { throw 'Fokus vor dem Stopp verloren' }
    Send-Key $hk                                            # Stopp -> Transkription -> Einfuegen
}

function Invoke-ProbePaste {
    # Same order and timing as clipboard.rs::paste_via_clipboard.
    $saved = $null
    try { $saved = Get-Clipboard -Raw -ErrorAction Stop } catch { }
    try {
        Set-Clipboard -Value $script:Expected
        Start-Sleep -Milliseconds $PasteDelayMs
        Send-Key @([byte]$VK_CONTROL, [byte]$VK_V)
        Start-Sleep -Milliseconds $PasteDelayAfterMs
    } finally {
        if ($saved) { Set-Clipboard -Value $saved } else { Set-Clipboard -Value '' }
    }
}

# ---------------------------------------------------------------- Bewertung (rein, testbar)
# Liefert @{ State; Detail } fuer den Probe-Modus: PASS nur bei exakt gleichem Text.
function Get-ProbeVerdict {
    param([string]$Want, [string]$Got, [string]$Method)
    if ($Got -eq $Want) {
        return @{ State = 'PASS'; Detail = ("Text exakt zurueckgelesen ({0} Zeichen, Umlaute, Zeilenumbruch) via {1}" -f $Want.Length, $Method) }
    }
    if ($Got.Length -eq 0) {
        return @{ State = 'FAIL'; Detail = ("nichts zurueckgelesen via {0} (Einfuegen kam nicht an oder Feld nicht lesbar)" -f $Method) }
    }
    @{ State = 'FAIL'; Detail = ("Abweichung via {0}: erwartet {1} Zeichen, gelesen {2} Zeichen; gelesen beginnt mit '{3}'" -f `
                $Method, $Want.Length, $Got.Length, $Got.Substring(0, [Math]::Min(40, $Got.Length))) }
}

# App-Modus: PASS, wenn das diktierte Wort "Termin" im zurueckgelesenen Text steht.
function Get-AppVerdict {
    param([string]$Got, [string]$Method)
    if ($Got -match 'Termin') { return @{ State = 'PASS'; Detail = ("Diktat angekommen ({0} Zeichen) via {1}" -f $Got.Length, $Method) } }
    @{ State = 'FAIL'; Detail = ("kein erkanntes Diktat zurueckgelesen via {0} ({1} Zeichen)" -f $Method, $Got.Length) }
}

# Exit-Code: 1 bei mindestens einem FAIL, sonst 0.
function Get-ExitCode {
    param($Results)
    if (@($Results | Where-Object State -eq 'FAIL').Count -gt 0) { 1 } else { 0 }
}

# ---------------------------------------------------------------- Ablauf
function Add-Result {
    param([string]$Name, [string]$State, [string]$Detail)
    $script:Results += [pscustomobject]@{ Target = $Name; State = $State; Detail = $Detail }
    $color = switch ($State) { 'PASS' { 'Green' } 'FAIL' { 'Red' } 'PLAN' { 'Cyan' } default { 'Yellow' } }
    Write-Host ("{0}  {1,-8} {2}" -f $State, $Name, $Detail) -ForegroundColor $color
}

function Invoke-Target {
    param([string]$Name)
    $t = $Targets[$Name]
    $script:State = @{}
    $skip = & $t.Check
    if ($skip) { Add-Result $Name 'SKIP' $skip; return }

    $savedClip = $null
    try { $savedClip = Get-Clipboard -Raw -ErrorAction Stop } catch { }
    try {
        & $t.Start
        $win = Find-Window -PidProvider $t.Pids -TitleRegex $t.Title -Seconds $TimeoutSeconds
        if (-not $win) { Add-Result $Name 'FAIL' "Fenster erschien nicht innerhalb von ${TimeoutSeconds}s"; return }
        # Chrome/VS Code need a moment after the window exists until the field has focus.
        Start-Sleep -Milliseconds 2500
        if (-not (Set-Focus $win $t.Pids)) {
            $fg = Get-Process -Id ([int][M10.Native]::ForegroundPid()) -ErrorAction SilentlyContinue
            Add-Result $Name 'FAIL' ("Fokus nicht setzbar (Vordergrund: {0}); es wurde keine Taste gesendet" -f $(if ($fg) { $fg.ProcessName } else { '?' }))
            return
        }

        if ($Mode -eq 'probe') { Invoke-ProbePaste } else { Invoke-AppDictation $win $t.Pids }

        $want = ConvertTo-Norm $script:Expected
        $got = ''
        $end = (Get-Date).AddSeconds($(if ($Mode -eq 'probe') { 8 } else { $TimeoutSeconds }))
        $script:State.Method = $(if ($Name -eq 'word') { 'COM Range.Text' } else { 'UI Automation' })
        while ((Get-Date) -lt $end) {
            Start-Sleep -Milliseconds 500
            $got = ConvertTo-Norm (& $t.Read)
            if ($Mode -eq 'probe') { if ($got -eq $want) { break } }
            elseif ($got -match 'Termin') { Start-Sleep -Milliseconds 1200; $got = ConvertTo-Norm (& $t.Read); break }
            if ($Name -eq 'vscode' -and $got.Length -eq 0) { Start-Sleep -Milliseconds 300 }
        }
        $method = $script:State.Method

        $v = if ($Mode -eq 'probe') { Get-ProbeVerdict -Want $want -Got $got -Method $method } else { Get-AppVerdict -Got $got -Method $method }
        Add-Result $Name $v.State $v.Detail
    } catch {
        Add-Result $Name 'FAIL' ("Ausnahme: {0}" -f $_.Exception.Message)
    } finally {
        try { & $t.Stop } catch { }
        try { if ($savedClip) { Set-Clipboard -Value $savedClip } } catch { }
    }
}

$names = if ($Target -contains 'all') { @('notepad', 'chrome', 'word', 'vscode') } else { @($Target) }
Write-Host ("M10 Einfuege-Abnahme, Modus {0}, Ziele: {1}" -f $Mode, ($names -join ', ')) -ForegroundColor Cyan
Write-Host 'Waehrend des Laufs nicht tippen und keine Fenster in den Vordergrund holen.' -ForegroundColor DarkGray

function Show-DryRun {
    Write-Host 'DRY-RUN: es wird nichts geoeffnet, keine Taste gesendet, nichts angelegt.' -ForegroundColor Cyan
    if ($Mode -eq 'app') {
        $fx = Join-Path $FixtureDir 'de_short_01.wav'
        Write-Host ("  App-Modus: Fixture {0}: {1}" -f $fx, $(if (Test-Path -LiteralPath $fx) { 'vorhanden' } else { 'FEHLT' }))
        Write-Host ("  App-Modus: local-voice-ai laeuft: {0}" -f $(if (Get-Process local-voice-ai -ErrorAction SilentlyContinue) { 'ja' } else { 'NEIN (wird zum FAIL fuehren)' }))
        try { $hk = Get-HotkeyVKeys; Write-Host ("  App-Modus: Hotkey aus Settings-Store lesbar ({0} Tasten)" -f @($hk).Count) }
        catch { Write-Host ("  App-Modus: Hotkey NICHT lesbar: {0}" -f $_.Exception.Message) -ForegroundColor Yellow }
    }
    foreach ($n in $names) {
        $skip = & $Targets[$n].Check
        if ($skip) { Add-Result $n 'SKIP' $skip }
        else { Add-Result $n 'PLAN' 'bereit' }
        foreach ($step in $Targets[$n].Plan) { Write-Host ("           - {0}" -f $step) -ForegroundColor DarkGray }
    }
}

if ($DryRun) { Show-DryRun; exit 0 }
foreach ($n in $names) { Invoke-Target $n }

# ---------------------------------------------------------------- Bericht
$fails = @($script:Results | Where-Object State -eq 'FAIL').Count
$pass = @($script:Results | Where-Object State -eq 'PASS').Count
$skips = @($script:Results | Where-Object State -eq 'SKIP').Count
Write-Host ("`n{0} PASS, {1} FAIL, {2} SKIP" -f $pass, $fails, $skips) -ForegroundColor Cyan
if ($ReportDir) {
    New-Item -ItemType Directory -Force -Path $ReportDir | Out-Null
    $lines = @(("# M10 Einfuege-Abnahme {0:yyyy-MM-dd HH:mm:ss}, Modus {1}" -f (Get-Date), $Mode), '', '| Ziel | Ergebnis | Detail |', '|---|---|---|')
    foreach ($r in $script:Results) { $lines += ('| {0} | {1} | {2} |' -f $r.Target, $r.State, $r.Detail) }
    ($lines -join "`n") | Out-File (Join-Path $ReportDir 'm10-report.md') -Encoding UTF8
}
Start-Sleep -Milliseconds 800
Remove-Item -LiteralPath $Scratch -Recurse -Force -ErrorAction SilentlyContinue
exit (Get-ExitCode $script:Results)
