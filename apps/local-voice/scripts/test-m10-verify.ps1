<#
.SYNOPSIS
    Prueft scripts\m10-verify.ps1 ohne jedes Fenster (Issue #10). ASCII only.

.DESCRIPTION
    Es wird weder Notepad noch Chrome, Word oder VS Code gestartet. Geprueft wird:
      1. alle neuen Skripte sind reines ASCII und parsen fehlerfrei,
      2. -DryRun: Exit 0, Plan je Ziel, kein Scratch-Ordner, kein Prozess,
      3. Prozess-Zuordnung (nur eigene PIDs) mit gefaelschter Prozesstabelle,
      4. Stop-Instance beendet nur uebergebene PIDs, nie das eigene Skript,
      5. PASS/FAIL-Bewertung und Exit-Code,
      6. statisch: kein taskkill /IM, kein Stop-Process nach Name.
    Die Funktionen werden per AST aus m10-verify.ps1 gezogen (das Skript selbst
    laeuft dabei nicht). Laeuft unter pwsh 7 und Windows PowerShell 5.1; der
    -DryRun-Lauf geht immer ueber pwsh 7 (das Skript verlangt 7).
    Exit 0 = alles gruen, sonst 1.

.EXAMPLE
    pwsh -File apps\local-voice\scripts\test-m10-verify.ps1
    powershell -File apps\local-voice\scripts\test-m10-verify.ps1
#>
$ErrorActionPreference = 'Continue'
Set-StrictMode -Version Latest

$here = $PSScriptRoot
$m10 = Join-Path $here 'm10-verify.ps1'
$failures = 0

function Check {
    param([string]$Name, [bool]$Ok, [string]$Detail = '')
    if ($Ok) { Write-Host "PASS  $Name" -ForegroundColor Green }
    else { Write-Host "FAIL  $Name  $Detail" -ForegroundColor Red; $script:failures++ }
}

# ---- 1. ASCII und Syntax ------------------------------------------------
foreach ($n in 'm10-verify.ps1', 'check-old-log.ps1', 'test-m10-verify.ps1', 'test-check-old-log.ps1') {
    $path = Join-Path $here $n
    $bytes = [IO.File]::ReadAllBytes($path)
    $bad = @($bytes | Where-Object { $_ -gt 127 }).Count
    Check "ASCII: $n" ($bad -eq 0) "$bad Nicht-ASCII-Bytes"
    $errs = $null; $tokens = $null
    [void][System.Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errs)
    Check "Syntax: $n" (@($errs).Count -eq 0) ($errs | Out-String)
}

# ---- Funktionen per AST laden --------------------------------------------
$errs = $null; $tokens = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($m10, [ref]$tokens, [ref]$errs)
$wanted = 'ConvertTo-Norm', 'Get-InstancePids', 'Invoke-Kill', 'Stop-Instance', 'Get-ProbeVerdict', 'Get-AppVerdict', 'Get-ExitCode'
$defs = $ast.FindAll({ param($a) $a -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)
foreach ($w in $wanted) {
    $d = $defs | Where-Object Name -eq $w | Select-Object -First 1
    Check "Funktion vorhanden: $w" ($null -ne $d)
    if ($d) { Invoke-Expression $d.Extent.Text }
}

# ---- 2. -DryRun -----------------------------------------------------------
$engine = Get-Command pwsh.exe -ErrorAction SilentlyContinue
if (-not $engine) {
    Write-Host 'SKIP  DryRun-Pruefungen (pwsh 7 nicht gefunden)' -ForegroundColor Yellow
} else {
    $pwsh = $engine.Source
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("lva-m10test-" + [Guid]::NewGuid().ToString('N'))
    $scratch = Join-Path $tmp 'scratch'
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        $out = & $pwsh -NoProfile -File $m10 -DryRun -Scratch $scratch 2>&1 | Out-String
        $code = $LASTEXITCODE
        Check 'DryRun: Exit 0' ($code -eq 0) "Exit $code"
        Check 'DryRun: Hinweis "nichts geoeffnet"' ($out -match 'nichts geoeffnet') $out
        foreach ($t in 'notepad', 'chrome', 'word', 'vscode') {
            Check "DryRun: Ziel $t mit PLAN oder SKIP" ($out -match "(PLAN|SKIP)\s+$t\b") $out
        }
        Check 'DryRun: Plan nennt UI Automation / COM' (($out -match 'UI Automation') -and ($out -match 'COM Document.Content.Text')) $out
        Check 'DryRun: kein Scratch-Ordner angelegt' (-not (Test-Path -LiteralPath $scratch)) $scratch
        $leak = @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.Contains($tmp) -and $_.ProcessId -ne $PID })
        Check 'DryRun: kein Prozess mit dem Scratch-Pfad' ($leak.Count -eq 0) ($leak | Out-String)

        $out = & $pwsh -NoProfile -File $m10 -DryRun -Target notepad -Scratch $scratch 2>&1 | Out-String
        Check 'DryRun -Target notepad: nur Notepad' (($out -match '(PLAN|SKIP)\s+notepad') -and ($out -notmatch '(PLAN|SKIP)\s+chrome')) $out
        $out = & $pwsh -NoProfile -File $m10 -DryRun -Mode app -Target word -Scratch $scratch 2>&1 | Out-String
        Check 'DryRun -Mode app: Vorbedingungen gemeldet, Exit 0' (($LASTEXITCODE -eq 0) -and ($out -match 'App-Modus: Fixture')) $out
        $null = & $pwsh -NoProfile -File $m10 -Target gibtsnicht -DryRun 2>&1
        Check 'Ungueltiges Ziel: Exit <> 0' ($LASTEXITCODE -ne 0) "Exit $LASTEXITCODE"
    } finally {
        Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ---- 3. Prozess-Zuordnung: nur eigene PIDs -------------------------------
function Get-CmdProcs {
    @(
        [pscustomobject]@{ ProcessId = 100; ParentProcessId = 1; Name = 'chrome.exe'; CommandLine = 'chrome.exe --new-window' }
        [pscustomobject]@{ ProcessId = 101; ParentProcessId = 100; Name = 'chrome.exe'; CommandLine = 'chrome.exe --type=renderer' }
        [pscustomobject]@{ ProcessId = 102; ParentProcessId = 101; Name = 'chrome.exe'; CommandLine = 'chrome.exe --type=gpu' }
        [pscustomobject]@{ ProcessId = 200; ParentProcessId = 1; Name = 'chrome.exe'; CommandLine = 'chrome.exe --user-data-dir=C:\Users\x\fremd' }
        [pscustomobject]@{ ProcessId = 201; ParentProcessId = 200; Name = 'chrome.exe'; CommandLine = 'chrome.exe --type=renderer' }
        [pscustomobject]@{ ProcessId = 300; ParentProcessId = 1; Name = 'Code.exe'; CommandLine = 'Code.exe --user-data-dir=C:\scratch\LVAPROFILE\data' }
        [pscustomobject]@{ ProcessId = 301; ParentProcessId = 300; Name = 'Code.exe'; CommandLine = 'Code.exe --type=utility' }
        [pscustomobject]@{ ProcessId = 400; ParentProcessId = 1; Name = 'WINWORD.EXE'; CommandLine = $null }
    )
}
[int[]]$set = Get-InstancePids -Needle 'LVAPROFILE' -Roots @(100)
$set = $set | Sort-Object
Check 'Get-InstancePids: Root + Kinder + Needle + Kinder' (($set -join ',') -eq '100,101,102,300,301') ($set -join ',')
[int[]]$set = Get-InstancePids -Needle $null -Roots @(400)
Check 'Get-InstancePids: ohne Needle nur Root' (($set -join ',') -eq '400') ($set -join ',')
[int[]]$set = Get-InstancePids -Needle 'LVAPROFILE' -Roots @()
Check 'Get-InstancePids: fremde Instanz (200/201) nie enthalten' (($set -notcontains 200) -and ($set -notcontains 201) -and ($set -notcontains 400)) ($set -join ',')

# ---- 4. Stop-Instance -------------------------------------------------------
$script:killed = New-Object System.Collections.Generic.List[int]
function Invoke-Kill { param([int]$ProcId) $script:killed.Add($ProcId) }
function Get-Process { param($Id, $ErrorAction) if ($Id -in 100, 101, $PID) { [pscustomobject]@{ Id = $Id } } }
Stop-Instance @(100, 999, $PID, 101)
Check 'Stop-Instance: nur vorhandene fremdlose PIDs, nie das eigene Skript' (($script:killed -join ',') -eq '100,101') ($script:killed -join ',')
Remove-Item Function:\Get-Process, Function:\Invoke-Kill -ErrorAction SilentlyContinue

# ---- 5. Bewertung und Exit-Code ----------------------------------------------
$want = "Zeile eins`nZweite"
$v = Get-ProbeVerdict -Want $want -Got $want -Method 'UIA'
Check 'Verdict: gleicher Text -> PASS' ($v.State -eq 'PASS') $v.Detail
$v = Get-ProbeVerdict -Want $want -Got '' -Method 'UIA'
Check 'Verdict: nichts gelesen -> FAIL' (($v.State -eq 'FAIL') -and ($v.Detail -match 'nichts zurueckgelesen')) $v.Detail
$v = Get-ProbeVerdict -Want $want -Got 'Zeile eins' -Method 'UIA'
Check 'Verdict: Abweichung -> FAIL' (($v.State -eq 'FAIL') -and ($v.Detail -match 'Abweichung')) $v.Detail
$v = Get-ProbeVerdict -Want $want -Got ($want + ' ') -Method 'UIA'
Check 'Verdict: exakter Vergleich, Anhang -> FAIL' ($v.State -eq 'FAIL') $v.Detail
$v = Get-AppVerdict -Got 'Hallo Termin morgen' -Method 'UIA'
Check 'AppVerdict: Termin gefunden -> PASS' ($v.State -eq 'PASS') $v.Detail
$v = Get-AppVerdict -Got 'Hallo' -Method 'UIA'
Check 'AppVerdict: kein Termin -> FAIL' ($v.State -eq 'FAIL') $v.Detail
Check 'Exit-Code: PASS+SKIP -> 0' ((Get-ExitCode @([pscustomobject]@{ State = 'PASS' }, [pscustomobject]@{ State = 'SKIP' })) -eq 0)
Check 'Exit-Code: ein FAIL -> 1' ((Get-ExitCode @([pscustomobject]@{ State = 'PASS' }, [pscustomobject]@{ State = 'FAIL' })) -eq 1)
Check 'Exit-Code: leer -> 0' ((Get-ExitCode @()) -eq 0)
Check 'ConvertTo-Norm: CRLF/CR -> LF, getrimmt' ((ConvertTo-Norm "  a`r`nb`rc  ") -eq "a`nb`nc")
Check 'ConvertTo-Norm: null -> leer' ((ConvertTo-Norm $null) -eq '')

# ---- 6. statische Sicherheitspruefung ----------------------------------------
$src = Get-Content -LiteralPath $m10 -Raw
Check 'Statisch: kein taskkill /IM' ($src -notmatch '(?i)taskkill(\.exe)?\s+/IM')
Check 'Statisch: kein Stop-Process' ($src -notmatch '(?i)Stop-Process')
Check 'Statisch: kein Kill ueber Prozessnamen (.Kill())' ($src -notmatch '\.Kill\(')
$tk = @([regex]::Matches($src, '(?i)taskkill\.exe\s+(\S+)') | ForEach-Object { $_.Groups[1].Value })
Check 'Statisch: taskkill nur mit /PID' (($tk.Count -ge 1) -and (@($tk | Where-Object { $_ -ne '/PID' }).Count -eq 0)) ($tk -join ',')

if ($failures -gt 0) { Write-Host "$failures Pruefung(en) fehlgeschlagen" -ForegroundColor Red; exit 1 }
Write-Host 'Alles gruen' -ForegroundColor Green
exit 0
