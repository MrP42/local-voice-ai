<#
.SYNOPSIS
    Compares local LLMs for German meeting minutes (Protokoll) on the synthetic fixtures.

.DESCRIPTION
    Runs the app's headless `--eval-minutes` once per model, serially (one LLM server at a
    time, RAM start gate and memory watchdog of the app apply). The fixtures in
    src-tauri\tests\fixtures\notes carry `fakten` (required statements) and `falsch`
    (misattributions); the report per model holds fact coverage, false hits, dropped blocks,
    duration and tokens. Only synthetic data is used; the minutes text lands in the JSON
    files under -Out, never in the app log.

    Models: pass GGUF paths (first shard of split files, no mmproj) or ids from the app
    (`ext-...`). For a path the id is derived exactly like the app does
    (managers::llm::external::id_for_path: SHA-256 of the lower-cased full path, first six
    bytes hex, prefix `ext-`), so the GGUF's folder MUST be listed under
    Einstellungen > KI-Modelle > Modellordner (llm_model_dirs) and the path must be spelled
    as the app builds it (<folder as configured>\<file name>).

    Output: one JSON per model plus ergebnis.md (table) in -Out.

.EXAMPLE
    pwsh apps/local-voice/scripts/protokoll-modellvergleich.ps1 `
        -Models 'Bolt-74B=D:\llm\modelle\NVIDIA-Nemotron-3.5-Bolt-74B-A9B-24GB.gguf', `
                'Gemma4-26B=D:\llm\modelle\gemma-4-26b-a4b-it-Q4_K_M.gguf'
#>
[CmdletBinding()]
param(
    # 'Name=<path to .gguf or ext-id>' entries, run in this order.
    [Parameter(Mandatory = $true)][string[]]$Models,
    # Default: <script dir>\..\src-tauri\target\release\local-voice-ai.exe (set below).
    [string]$AppExe = '',
    [string]$Fixtures = '',
    [string]$Out = '',
    # auto = the app chooses the template (default), otherwise a template id.
    [string]$Template = 'auto',
    # Per model; long fixtures on CPU-only machines need a lot of time.
    [int]$TimeoutMinutes = 90
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
if (-not $AppExe) { $AppExe = Join-Path $root 'src-tauri\target\release\local-voice-ai.exe' }
if (-not $Fixtures) { $Fixtures = Join-Path $root 'src-tauri\tests\fixtures\notes' }
if (-not $Out) { $Out = Join-Path $env:LOCALAPPDATA ('lva-bench\protokoll-' + (Get-Date -Format 'yyyyMMdd-HHmm')) }
if (-not (Test-Path $AppExe)) { throw "App not found: $AppExe" }
if (-not (Test-Path $Fixtures)) { throw "Fixtures not found: $Fixtures" }
New-Item -ItemType Directory -Force -Path $Out | Out-Null

function Get-ModelId([string]$spec) {
    if ($spec -like 'ext-*' -or -not ($spec -match '\.gguf$')) { return $spec }
    $sha = [System.Security.Cryptography.SHA256]::Create()
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($spec.ToLowerInvariant())
    $hash = $sha.ComputeHash($bytes)
    return 'ext-' + (($hash[0..5] | ForEach-Object { $_.ToString('x2') }) -join '')
}

$rows = @()
foreach ($entry in $Models) {
    $name, $spec = $entry -split '=', 2
    if (-not $spec) { throw "Expected 'Name=path-or-id', got '$entry'" }
    if ($spec -match '\.gguf$' -and -not (Test-Path $spec)) { throw "GGUF not found: $spec" }
    $id = Get-ModelId $spec
    $json = Join-Path $Out ($name + '.json')
    $log = Join-Path $Out ($name + '.stderr.txt')
    Write-Host "== $name ($id)"
    $started = Get-Date
    $proc = Start-Process -FilePath $AppExe -PassThru -NoNewWindow `
        -ArgumentList @('--eval-minutes', "`"$Fixtures`"", '--model', $id, '--minutes-template', $Template, '--json', '--out', "`"$json`"") `
        -RedirectStandardError $log -RedirectStandardOutput (Join-Path $Out ($name + '.stdout.txt'))
    if (-not $proc.WaitForExit($TimeoutMinutes * 60 * 1000)) {
        $proc.Kill()
        Write-Warning "${name}: timeout after $TimeoutMinutes min"
    }
    $secs = [math]::Round(((Get-Date) - $started).TotalSeconds)
    $row = [ordered]@{ Modell = $name; Id = $id; Exit = $proc.ExitCode; Sekunden = $secs
        Fakten = '-'; Abdeckung = '-'; Falsch = '-'; Luecken = '-'; TokS = '-'; Fehler = '' }
    if (Test-Path $json) {
        $r = Get-Content $json -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($r.error) { $row.Fehler = $r.error }
        $a = $r.aggregate
        if ($a) {
            $row.Fakten = "$($a.facts_found)/$($a.facts_total)"
            if ($null -ne $a.fact_coverage) { $row.Abdeckung = '{0:P0}' -f [double]$a.fact_coverage }
            $row.Falsch = $a.false_hits
            $row.Luecken = $a.with_gaps
        }
        $tok = 0; $ms = 0
        foreach ($f in $r.fixtures) {
            if ($f.usage) { $tok += [double]$f.usage.completion_tokens; $ms += [double]$f.usage.llm_ms }
            if ($f.error) { $row.Fehler += " $($f.name): $($f.error)" }
        }
        if ($ms -gt 0) { $row.TokS = [math]::Round($tok / ($ms / 1000), 1) }
    } else {
        $row.Fehler = 'no report (see ' + (Split-Path -Leaf $log) + ')'
    }
    $rows += [pscustomobject]$row
    $rows[-1] | Format-List | Out-String | Write-Host
}

$md = @('# Protokoll-Modellvergleich', '', "Fixtures: $Fixtures, Vorlage: $Template, $(Get-Date -Format 'yyyy-MM-dd HH:mm')", '',
    '| Modell | Fakten | Abdeckung | Falsch | Luecken | Tok/s (Ausgabe) | Sekunden | Exit | Fehler |',
    '|---|---|---|---|---|---|---|---|---|')
foreach ($r in $rows) {
    $md += "| $($r.Modell) | $($r.Fakten) | $($r.Abdeckung) | $($r.Falsch) | $($r.Luecken) | $($r.TokS) | $($r.Sekunden) | $($r.Exit) | $($r.Fehler.Trim()) |"
}
$md += ''
$md += 'Fakten = gefundene Pflichtaussagen; Falsch = getroffene Falschzuordnungen; Luecken = Fixtures mit verworfenem Block. Tok/s enthaelt den Serverstart des ersten Aufrufs.'
$mdPath = Join-Path $Out 'ergebnis.md'
[System.IO.File]::WriteAllLines($mdPath, $md, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "wrote $mdPath"
$rows | Format-Table Modell, Fakten, Abdeckung, Falsch, Luecken, TokS, Sekunden, Exit -AutoSize
