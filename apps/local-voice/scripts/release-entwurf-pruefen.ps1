<#
.SYNOPSIS
  Bricht ab, wenn das Ziel-Release eines Release-Laufs nicht (mehr) ein Entwurf ist (lva#889).

.DESCRIPTION
  Der Release-Lauf prueft am Anfang, dass das Ziel fehlt oder ein Entwurf ist. Zwischen dieser
  Pruefung und den spaeteren Uploads vergeht der ganze Bau; wird der Entwurf in dieser Zeit von
  Hand veroeffentlicht, landeten die Uploads in einem oeffentlichen Release. Darum ruft jeder
  Schreibschritt dieses Skript unmittelbar vor dem Schreiben auf.

  Beim Nachpacken (LVA_NACHPACKEN gesetzt) ist das Ziel absichtlich ein veroeffentlichtes
  Release; dann prueft das Skript nichts. Ganz schliessen laesst sich das Fenster nur
  organisatorisch: waehrend eines Laufs kein Release von Hand veroeffentlichen.

  Die Abfrage laeuft ueber `gh release view`, das Entwuerfe auch dann findet, wenn
  releases/tags/<tag> sie als 404 meldet (lva#887). Jede andere Antwort als "true" bricht ab.

.PARAMETER Tag
  Tag des Releases, z. B. app-v0.30.0.

.PARAMETER Gh
  Nur fuer den Trockenlauf: Befehl, der statt `gh` aufgerufen wird.
#>
param(
  [Parameter(Mandatory = $true)][string]$Tag,
  [string]$Gh = 'gh'
)

# Nicht 'Stop': unter Windows PowerShell 5.1 wuerde jede stderr-Zeile von gh (2>&1) sonst als
# Fehler abbrechen, bevor Exit-Code und Text ausgewertet sind. Abgebrochen wird per throw.
$ErrorActionPreference = 'Continue'

if ($env:LVA_NACHPACKEN) {
  Write-Host "Nachpacken von ${Tag}: Ziel darf veroeffentlicht sein, keine Entwurfspruefung."
  return
}

$antwort = & $Gh release view $Tag --json isDraft --jq .isDraft --repo $env:GITHUB_REPOSITORY 2>&1
$code = $LASTEXITCODE
$text = ($antwort | Out-String).Trim()
if ($code -ne 0) {
  throw "Release-Abfrage fuer $Tag fehlgeschlagen (Exit $code): $text"
}
if ($text -eq 'false') {
  throw "$Tag ist inzwischen veroeffentlicht. Kein Upload in ein oeffentliches Release; Korrektur als neue Patch-Version."
}
if ($text -ne 'true') {
  throw "Unerwartete Antwort fuer ${Tag}: $text"
}
Write-Host "Ziel ${Tag}: weiterhin Entwurf"
