# Trockenlauf fuer release-entwurf-pruefen.ps1 (lva#889) mit nachgestellter gh-Antwort.
# Aufruf: powershell -NoProfile -File scripts/release-entwurf-pruefen.test.ps1  (5.1 und 7)
$ErrorActionPreference = 'Stop'
$skript = Join-Path $PSScriptRoot 'release-entwurf-pruefen.ps1'
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lva889-" + [guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
$fehler = 0

function Fall($name, $ausgabe, $exit, $nachpacken, $erwartetOk) {
  $fake = Join-Path $tmp "gh-$name.ps1"
  # Gibt die Antwort aus (bei Exit != 0 auf stderr wie gh) und endet mit dem Exit-Code.
  $zeile = if ($exit -ne 0) { "[Console]::Error.WriteLine('$ausgabe')" } else { "Write-Output '$ausgabe'" }
  Set-Content -LiteralPath $fake -Value "$zeile`nexit $exit" -Encoding ASCII
  $env:LVA_NACHPACKEN = $nachpacken
  $ok = $true
  try { & $skript -Tag 'app-v9.9.9' -Gh $fake | Out-Null } catch { $ok = $false; $meldung = $_.Exception.Message }
  $env:LVA_NACHPACKEN = $null
  if ($ok -eq $erwartetOk) { Write-Host "OK   $name" }
  else { Write-Host "FAIL $name (erwartet ok=$erwartetOk, war ok=$ok $meldung)"; $script:fehler++ }
}

Fall 'entwurf'          'true'                 0 ''           $true
Fall 'veroeffentlicht'  'false'                0 ''           $false
Fall 'unerwartet'       'null'                 0 ''           $false
Fall 'fehlt'            'release not found'    1 ''           $false
Fall 'nachpacken'       'false'                0 'app-v0.29.0' $true

Remove-Item -Recurse -Force $tmp
if ($fehler) { Write-Host "$fehler Fall/Faelle rot"; exit 1 }
Write-Host 'Alle Faelle gruen'
