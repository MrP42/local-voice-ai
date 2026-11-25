# Nur AST und isolierte Funktionen mit synthetischen Daten; kein Paketbau, kein Prozess/Netz.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'build-portable-zip.ps1'), [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Parserfehler im Paketskript' }
function Assert([bool]$Ok, [string]$Name) {
    if (-not $Ok) { throw "Fehlgeschlagen: $Name" }
}
function Function-Text([string]$Name) {
    $node = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -eq $Name }, $true)
    if (-not $node) { throw "Funktion fehlt: $Name" }
    $node.Extent.Text
}
# Ausschliesslich diese reinen Helfer laden, niemals den Skriptkoerper.
foreach ($name in @('Read-PeHeader', 'Get-ProgrammHashes', 'Test-ProgrammHashes', 'Get-FirmenprofilName', 'Get-FirmenprofilIds', 'Test-FirmenprofilAusgabe', 'Read-FirmenprofilAusgabe', 'Get-FirmenprofilPruefung')) {
    . ([scriptblock]::Create((Function-Text $name)))
}
foreach ($case in @(
    @{ Bytes = [byte[]]@(77,90,0); Pe = $true },
    @{ Bytes = [byte[]]@(77,90); Pe = $true },
    @{ Bytes = [byte[]]@(77); Pe = $false },
    @{ Bytes = [byte[]]@(); Pe = $false },
    @{ Bytes = [byte[]]@(90,77); Pe = $false }
)) {
    $stream = New-Object System.IO.MemoryStream (,$case.Bytes)
    try { Assert ((Read-PeHeader $stream) -eq $case.Pe) 'MZ unabhaengig von Endung' }
    finally { $stream.Dispose() }
}
Assert ((Get-FirmenprofilName '1.2.3' 'Valid') -eq 'LocalVoiceAI_1.2.3_x64-portable-firmenprofil') 'gueltige Signatur'
foreach ($complete in @($false, $true)) {
    foreach ($status in @('Valid', 'NotSigned')) {
        $expectedName = 'LocalVoiceAI_1.2.3_x64-portable-firmenprofil'
        if (-not $complete) { $expectedName += '-unvollstaendig' }
        if ($status -ne 'Valid') { $expectedName += '-unsigniert' }
        Assert ((Get-FirmenprofilName '1.2.3' $status $complete) -ceq $expectedName) 'kombinierte Namenszusaetze in fester Reihenfolge'
    }
}
foreach ($status in @('NotSigned', 'UnknownError', 'HashMismatch', 'NotTrusted')) {
    Assert ((Get-FirmenprofilName '1.2.3' $status) -eq 'LocalVoiceAI_1.2.3_x64-portable-firmenprofil-unsigniert') 'ungueltige Signatur'
}
$ids = @(Get-FirmenprofilIds @('basis'))
foreach ($id in @('basis', 'phonon-runtime', 'phonon-40m-de', 'phonon-31m-en', 'llm-runtime-windows-x64-cpu', 'llm-qwen3-0.6b-q4')) {
    Assert ($ids -contains $id) "Pflichtmodell $id"
}
Assert (-not ($ids -match 'vulkan')) 'keine Vulkan-Laufzeit'
$failed = $false
try { Get-FirmenprofilIds @('llm-runtime-windows-x64-vulkan') | Out-Null } catch { $failed = $true }
Assert $failed 'Gegenprobe: Vulkan auch bei eigener Basis abgelehnt'
$policyJson = '{"keine_programm_downloads":true,"kein_selbstupdate":true,"kein_autostart":true,"keine_tastatur_hooks":true,"keine_besprechungserkennung":true,"kein_systemton":true,"herkunft":"datei","durchgesetzt":[]}'
$allNames = '["keine_programm_downloads","kein_selbstupdate","kein_autostart","keine_tastatur_hooks","keine_besprechungserkennung","kein_systemton"]'
$completeJson = $policyJson.Replace('[]', $allNames)
Assert (Test-FirmenprofilAusgabe ($completeJson | ConvertFrom-Json)) 'alle sechs Schalter wirken'
Assert (-not (Test-FirmenprofilAusgabe ($policyJson | ConvertFrom-Json))) 'leeres Array ist unvollstaendig'
Assert (-not (Test-FirmenprofilAusgabe ($policyJson.Replace('[]', '["kein_autostart"]') | ConvertFrom-Json))) 'genau ein Element bleibt Array und unvollstaendig'
foreach ($bad in @($policyJson.Replace('true', 'false'), $policyJson.Replace('true', '"true"'),
    $policyJson.Replace('"datei"', '"datei_fehlerhaft"'), $policyJson.Replace('"kein_systemton":true,', ''), 'null',
    $policyJson.Replace(',"durchgesetzt":[]', ''), $policyJson.Replace('[]', 'null'),
    $policyJson.Replace('[]', '"kein_autostart"'), $policyJson.Replace('[]', '[42]'),
    $policyJson.Replace('[]', '["fremd"]'), $policyJson.Replace('[]', '["KEIN_AUTOSTART"]'),
    $policyJson.Replace('[]', '["kein_autostart","kein_autostart"]'), $policyJson.Replace('[]}', '[],"extra":true}'))) {
    $failed = $false
    try { Test-FirmenprofilAusgabe ($bad | ConvertFrom-Json) } catch { $failed = $true }
    Assert $failed 'Gegenprobe: Schalter, Typ, Herkunft, fehlendes Feld'
}
# Nur die Prozess-/Dateigrenze ersetzen; Abfrage, Validierung und Release-Sperre bleiben echt.
$stage = $PSScriptRoot
$processCase = 'complete'
function Start-Process {
    param($FilePath, $ArgumentList, $WorkingDirectory, [switch]$PassThru, $WindowStyle, $RedirectStandardOutput, $RedirectStandardError)
    Assert ($ArgumentList -ceq '--richtlinie-zeigen') 'richtiger CLI-Parameter'
    $p = [pscustomobject]@{ Handle = 1; ExitCode = $(if ($processCase -eq 'exit') { 2 } else { 0 }); Killed = $false }
    $p | Add-Member ScriptMethod WaitForExit { param($ms) Assert ($ms -eq 60000) 'Zeitlimit 60 Sekunden'; return $processCase -ne 'timeout' }
    $p | Add-Member ScriptMethod Kill { $this.Killed = $true }
    $script:lastProcess = $p
    return $p
}
function Get-Content {
    param($LiteralPath, [switch]$Raw)
    switch ($processCase) {
        'complete' { $completeJson }
        'empty' { $policyJson }
        'single' { $policyJson.Replace('[]', '["kein_autostart"]') }
        'no-json' { 'synthetisch kein JSON' }
        'null' { 'null' }
        default { $completeJson }
    }
}
foreach ($processCase in @('complete', 'empty', 'single', 'exit', 'timeout', 'no-json', 'null')) {
    foreach ($releaseMode in @($false, $true)) {
        $failed = $false
        $result = $null
        $warnings = @()
        try { $result = Get-FirmenprofilPruefung $PSScriptRoot 'synthetisch' $releaseMode -WarningVariable warnings -WarningAction SilentlyContinue }
        catch { $failed = $true }
        $complete = $processCase -eq 'complete'
        Assert ($failed -eq ($releaseMode -and -not $complete)) "Release-Sperre: $processCase"
        if (-not $failed) {
            Assert ($result.Vollstaendig -eq $complete) "Vollstaendigkeit: $processCase"
            Assert ($warnings.Count -eq $(if ($complete) { 0 } else { 1 })) 'deutliche Warnung nur bei unvollstaendigem Abnahmestand'
        }
        if ($processCase -eq 'timeout') { Assert $lastProcess.Killed 'Zeitlimit beendet nur den gestarteten Kindprozess' }
    }
}
Remove-Item Function:\Start-Process, Function:\Get-Content
# Gegenprobe: ohne die Release-Sperre wird ein unvollstaendiger Stand akzeptiert.
$originalCheck = Function-Text 'Get-FirmenprofilPruefung'
function Read-FirmenprofilAusgabe { param($RootPath, $LogName) $policyJson | ConvertFrom-Json }
. ([scriptblock]::Create($originalCheck.Replace('if ($Release) { throw $message }', 'if ($false) { throw $message }')))
$failed = $false
try { Get-FirmenprofilPruefung $PSScriptRoot 'synthetisch' $true -WarningAction SilentlyContinue | Out-Null }
catch { $failed = $true }
Assert (-not $failed) 'Gegenprobe: entfernte Release-Sperre laesst Paket A durch'
. ([scriptblock]::Create($originalCheck))
. ([scriptblock]::Create((Function-Text 'Read-FirmenprofilAusgabe')))
# Gegenprobe: ohne Vollstaendigkeitspruefung muss die leere Liste faelschlich gruen werden.
$originalPolicyTest = Function-Text 'Test-FirmenprofilAusgabe'
. ([scriptblock]::Create($originalPolicyTest.Replace('return @($keys | Where-Object { $seen -cnotcontains $_ }).Count -eq 0', 'return $true')))
Assert (Test-FirmenprofilAusgabe ($policyJson | ConvertFrom-Json)) 'Gegenprobe: leere Liste entlarvt fehlende Durchsetzungspruefung'
. ([scriptblock]::Create($originalPolicyTest))
# Gegenprobe zur alten Sieben-Felder-Struktur: die neue gueltige Ausgabe wuerde abgelehnt.
. ([scriptblock]::Create($originalPolicyTest.Replace('-ne 8', '-ne 7')))
$failed = $false
try { Test-FirmenprofilAusgabe ($completeJson | ConvertFrom-Json) | Out-Null } catch { $failed = $true }
Assert $failed 'Gegenprobe: alte Eigenschaftenzahl lehnt acht Felder ab'
. ([scriptblock]::Create($originalPolicyTest))
# Dateisystemzugriffe der Hashhelfer durch synthetische Dateien ersetzen.
$fixtureRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixtureFiles = @(
    [pscustomobject]@{ FullName = (Join-Path $fixtureRoot 'z.exe') },
    [pscustomobject]@{ FullName = (Join-Path $fixtureRoot 'Data/runtime/ohne-endung') },
    [pscustomobject]@{ FullName = (Join-Path $fixtureRoot 'daten.exe') }
)
function Get-ChildItem { param($LiteralPath, [switch]$Recurse, [switch]$File, [switch]$Force) $fixtureFiles }
function Test-PeFile([string]$Path) { -not $Path.EndsWith('daten.exe') }
function Get-FileHash {
    param($LiteralPath, $Algorithm)
    [pscustomobject]@{ Hash = $(if ($LiteralPath.EndsWith('z.exe')) { 'AB' * 32 } else { 'CD' * 32 }) }
}
$expected = @((('cd' * 32) + '  Data/runtime/ohne-endung'), (('ab' * 32) + '  z.exe'))
$lines = @(Get-ProgrammHashes ($fixtureRoot + [System.IO.Path]::DirectorySeparatorChar))
Assert (($lines -join "`n") -ceq ($expected -join "`n")) 'vollstaendig, sortiert, relative Pfade mit /'
Test-ProgrammHashes $fixtureRoot $lines
foreach ($bad in @(
    @{ Lines = @($lines[0]) },
    @{ Lines = @($lines[0], $lines[0], $lines[1]) },
    @{ Lines = @($lines[0].Replace('cd', 'ab'), $lines[1]) },
    @{ Lines = @($lines[0].Replace('Data/', '../'), $lines[1]) }
)) {
    $failed = $false
    try { Test-ProgrammHashes $fixtureRoot $bad.Lines } catch { $failed = $true }
    Assert $failed 'Gegenprobe: fehlend, doppelt, falscher Hash, fremder Pfad'
}
# Gegenprobe: Mutation der MZ-Pruefung muss denselben Test verletzen.
$original = Function-Text 'Read-PeHeader'
. ([scriptblock]::Create($original.Replace('-eq 90', '-eq 89')))
$stream = New-Object System.IO.MemoryStream (,[byte[]]@(77,90))
try { Assert (-not (Read-PeHeader $stream)) 'Gegenprobe MZ-Mutation' } finally { $stream.Dispose() }
$source = $ast.Extent.Text
Assert ($source.Contains('if ($Firmenprofil -and $Offline)')) 'Modi gegenseitig ausschliessen'
$guard = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.IfStatementAst] -and
    $n.Extent.Text.StartsWith('if ($Firmenprofil -and $Offline)') }, $true)
foreach ($Firmenprofil in @($false, $true)) {
    foreach ($Offline in @($false, $true)) {
        $failed = $false
        try { & ([scriptblock]::Create($guard.Extent.Text)) } catch { $failed = $true }
        Assert ($failed -eq ($Firmenprofil -and $Offline)) 'Modus-Sperre mit allen Kombinationen'
    }
}
Assert ($source.Contains('if ($Firmenprofil -or ($Release -and $RequireHfToken))')) 'HF_TOKEN im Firmenprofil Pflicht'
Assert ($source.Contains("'--richtlinie-zeigen'")) 'Richtlinien-Rauchtest'
$stageCheckIndex = $source.IndexOf("Get-FirmenprofilPruefung `$root 'richtlinie-stage'")
Assert ($stageCheckIndex -ge 0 -and $stageCheckIndex -lt $source.IndexOf('[System.IO.Compression.ZipFile]::Open(')) 'Release-Sperre vor dem ersten ZIP'
Assert ($source.Contains("Get-FirmenprofilPruefung `$probeRoot 'richtlinie-probe'")) 'erneute Pruefung nach Entpacken'
$comparison = $ast.Find({ param($n) $n -is [System.Management.Automation.Language.IfStatementAst] -and
    $n.Extent.Text.StartsWith('if (($probeCheck.Ausgabe') }, $true)
Assert ($null -ne $comparison) 'Vergleich mit Staging-Ausgabe vorhanden'
foreach ($probeJson in @($completeJson, $policyJson, $policyJson.Replace('[]', '["kein_autostart"]'))) {
    $policyCheck = [pscustomobject]@{ Ausgabe = ($completeJson | ConvertFrom-Json) }
    $probeCheck = [pscustomobject]@{ Ausgabe = ($probeJson | ConvertFrom-Json) }
    $failed = $false
    try { & ([scriptblock]::Create($comparison.Extent.Text)) } catch { $failed = $true }
    Assert ($failed -eq ($probeJson -cne $completeJson)) 'Gegenprobe: abweichende Ausgabe nach Entpacken abgelehnt'
}
$workflow = [System.IO.File]::ReadAllText((Join-Path $PSScriptRoot '../../../.github/workflows/release-windows.yml'))
Assert ($workflow.Contains('firmenprofil=true')) 'Release erkennt Modul im gebauten Quellstand'
Assert ($workflow.Contains("steps.version.outputs.firmenprofil == 'true'")) 'alte Version ohne Modul ueberspringen'
Assert ($workflow.Contains('./scripts/build-portable-zip.ps1 -Release -Firmenprofil -SmokeTest -OutDir $out')) 'Firmenprofil im Release streng pruefen'
Write-Host 'Firmenprofil: Parser und synthetische Helfertests bestanden.'
