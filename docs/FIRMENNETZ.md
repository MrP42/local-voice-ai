# Local Voice im Firmennetz

## Haltung

Local Voice verhaelt sich wie eine normale Unternehmensanwendung. Es gibt keine Funktion,
die Sicherheitskontrollen verbirgt oder umgeht. Alles Folgende ist so gebaut, dass die
Kontrollen der Firma greifen.

## Proxy und Anmeldung

Die Proxy-Wahl folgt Windows (PAC/WPAD oder fest eingetragener Proxy) oder einer eigenen
Einstellung. Die Anmeldung am Proxy erfolgt mit dem Windows-Konto (Negotiate/NTLM, SSPI)
ueber einen lokalen Weiterleiter im Programm. Er bindet nur an 127.0.0.1 auf einem
Zufallsport, verlangt ein Zufalls-Token je Programmstart und leitet nur CONNECT zu den
Zielen weiter. Zugangsdaten werden weder gespeichert noch protokolliert.

## Zertifikate und TLS-Inspektion

Alle Verbindungen nutzen den Windows-Zertifikatsspeicher (SChannel bzw. die Prueffunktion
des Betriebssystems). Eine Inspektions-CA der Firma wird akzeptiert, wenn sie im
Windows-Speicher steht. Eingebaute Mozilla-Wurzelzertifikate (`webpki-roots`) sind im
Netzweg ausgeschlossen. Tests in `netz::firmentauglich_tests` sichern dies ab.
Die Abo-Programme der Anbieter (Claude Code, Codex) verwalten Zertifikate selbst. Bei
TLS-Inspektion richtet sich deren Einrichtung nach der Anleitung des Anbieters; die App
setzt dafuer keine Variablen, die nicht belegt sind (Handprobe am Firmen-PC offen).

## Prozesse

Die App installiert Claude Code und Codex nicht, laedt keine Skripte und fuehrt keine aus.
Sie startet kein PowerShell mit `-EncodedCommand` oder `-ExecutionPolicy Bypass`.
Vorhandene Abo-Programme werden ueber PATH, Benutzerprofil und npm global erkannt.
Fehlen sie, verweist die App auf die IT bzw. die offizielle Anleitung. Der Link wird nur
auf ausdruecklichen Klick im Browser geoeffnet.

Die Abo-Programme laufen als Kindprozess ohne Konsolenfenster. Der Firmenproxy wird nur
in deren Prozessumgebung gesetzt (`HTTPS_PROXY`, `HTTP_PROXY`,
`NO_PROXY=localhost,127.0.0.1`), nie fuer den ganzen Rechner. Bei der Anmeldung der
Anbieter-CLI oeffnet die App ein Konsolenfenster mit deren eigenem Anmeldebefehl
(`claude auth login`, `codex login`) und der Browser-Anmeldung des Anbieters.

## Netzprofil

Die App nutzt nach aussen nur ausgehendes HTTPS auf Port 443. Sie lauscht nur auf
127.0.0.1 (Weiterleiter fuer die Proxy-Anmeldung, Anmeldung bei Microsoft 365). Die
Loopback-Bindung ist durch Quelltext-Tests abgesichert.

## Zieladressen

Diese Tabelle wird aus `apps/local-voice/src-tauri/src/netz/ziele.rs` erzeugt; ein Test prueft, dass sie nicht abweicht. In der App: Einstellungen, Netz, Adressliste kopieren.

<!-- ZIELE:BEGIN -->
| Funktion | Adresse | Port | Zweck |
|---|---|---|---|
| Programm-Updates | github.com | 443 | Update-Manifest und Installer (Release-Dateien von MrP42/local-voice-ai) |
| Programm-Updates | objects.githubusercontent.com | 443 | Auslieferung der GitHub-Release-Dateien (Weiterleitung) |
| Programm-Updates | release-assets.githubusercontent.com | 443 | Auslieferung der GitHub-Release-Dateien (Weiterleitung) |
| Sprach- und Sprachmodelle | huggingface.co | 443 | Modell-Downloads (Hugging Face) |
| Sprach- und Sprachmodelle | cas-bridge.xethub.hf.co | 443 | Weiterleitung der Hugging-Face-Dateien |
| Sprach- und Sprachmodelle | cdn-lfs.hf.co | 443 | Weiterleitung der Hugging-Face-Dateien |
| Sprach- und Sprachmodelle | blob.handy.computer | 443 | Spracherkennungsmodelle (Direktdownload) |
| KI-Anbieter (API, Schluessel des Nutzers) | api.anthropic.com | 443 | Claude API |
| KI-Anbieter (API, Schluessel des Nutzers) | api.openai.com | 443 | OpenAI API |
| KI-Anbieter (API, Schluessel des Nutzers) | eu.api.openai.com | 443 | OpenAI API mit EU-Datenresidenz |
| KI-Anbieter (API, Schluessel des Nutzers) | api.mistral.ai | 443 | Mistral API |
| KI-Anbieter (API, Schluessel des Nutzers) | generativelanguage.googleapis.com | 443 | Google Gemini API |
| KI-Anbieter (API, Schluessel des Nutzers) | bedrock-mantle.eu-central-1.api.aws | 443 | Amazon Bedrock, Region Frankfurt |
| KI-Anbieter (API, Schluessel des Nutzers) | bedrock-mantle.eu-west-1.api.aws | 443 | Amazon Bedrock, Region Irland |
| KI-Anbieter (API, Schluessel des Nutzers) | bedrock-mantle.eu-south-1.api.aws | 443 | Amazon Bedrock, Region Mailand |
| KI-Anbieter (API, Schluessel des Nutzers) | bedrock-mantle.eu-north-1.api.aws | 443 | Amazon Bedrock, Region Stockholm |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | api.anthropic.com | 443 | Claude Code (Programm von Anthropic, nicht von der App installiert) |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | claude.ai | 443 | Anmeldung von Claude Code |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | chatgpt.com | 443 | Codex mit ChatGPT-Konto (Programm von OpenAI, nicht von der App installiert) |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | api.openai.com | 443 | Codex (Programm von OpenAI) |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | registry.npmjs.org | 443 | Codex-Update per npm (nur bei eingeschalteter automatischer Aktualisierung) |
| Optionale Dienste (nur wenn der Nutzer sie einrichtet) | graph.microsoft.com | 443 | Microsoft 365: Kalender und Dateien |
| Optionale Dienste (nur wenn der Nutzer sie einrichtet) | login.microsoftonline.com | 443 | Microsoft 365: Anmeldung |
| Optionale Dienste (nur wenn der Nutzer sie einrichtet) | api.gradium.ai | 443 | Stimmerzeugung (nur mit Einwilligung) |
| Optionale Dienste (nur wenn der Nutzer sie einrichtet) | openlibrary.org | 443 | Buchdaten (Metadaten) |
<!-- ZIELE:END -->

Die Hosts der Abo-Programme richten sich nach deren Anbieter-Dokumentation;
die Liste ist der Stand beim Erstellen.

## Signatur

Windows-Installer und Programmdatei werden mit Authenticode signiert, sobald ein
Signaturdienst konfiguriert ist. Die Repo-Variable `WINDOWS_SIGN_PROVIDER` waehlt
`azure` (Azure Trusted Signing, empfohlen) oder `esigner` (SSL.com eSigner).
Ist `REQUIRE_SIGNING` auf `1` oder `true` gesetzt und kein Signaturdienst konfiguriert,
bricht der Release-Lauf im Schritt "Prepare Authenticode signing" ab.
Ohne Signatur zeigt Windows SmartScreen "Unbekannter Herausgeber" an.

## Handproben am Firmen-PC

Offen, durch den Eigentuemer durchzufuehren:

- Abo-CLI ueber den Proxy mit Windows-Anmeldung verwenden.
- Ein Modell herunterladen.
- Die Update-Pruefung ausfuehren.
- Pruefung durch die IT-Sicherheit veranlassen.
