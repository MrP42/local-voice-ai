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

Unter Windows kann der Nutzer per Klick
das offizielle Claude- oder Codex-Programm sichtbar herunterladen und sich danach anmelden.
Es wird kein Installer gestartet, kein npm und kein Installationsskript aufgerufen.
Sie startet kein PowerShell mit `-EncodedCommand` oder `-ExecutionPolicy Bypass`.
Ein ausgewaehltes lokales `claude.exe` bzw. `codex.exe` hat Vorrang vor PATH,
Benutzerprofil und npm global. Die Auswahl ist entfernbar. Wird sie ungueltig, zeigt die
App einen Hinweis und faellt auf die Suche zurueck. UNC- und Netzlaufwerke werden abgelehnt.
Die offizielle Anleitung bleibt als Alternative auf Klick erreichbar, ebenso die
Programm-Auswahl und eine API-Verbindung.

Der Direktdownload nutzt denselben Proxyweg wie die App einschliesslich NTLM-Weiterleiter.
Nur HTTPS/443 und die unten aufgefuehrten Download-Hosts sind erlaubt, auch bei jeder
Weiterleitung. Metadaten: maximal 1 MiB und 30 Sekunden je Abruf; Programm/Archiv und
entpackte EXE: jeweils maximal 512 MiB; gesamte Bereitstellung: maximal 300 Sekunden.
SHA-256 kommt bei Claude aus dem Plattform-Manifest, bei Codex aus dem Release-`digest`
des ZIP-Assets. Erst nach erfolgreicher Archivpruefung wird genau die erwartete EXE
entnommen, kein Archivpfad wird ins Dateisystem uebernommen.
Temporaere Dateien liegen im Zielordner und werden bei Abbruch oder Fehler entfernt.
Die gepruefte Datei wird atomar umbenannt; ein gesperrtes bestehendes Programm wird
nicht vorher geloescht. Ablage: `%USERPROFILE%\.local\bin\claude.exe` bzw.
`%LOCALAPPDATA%\Programs\codex\codex.exe`. Je Anbieter ist nur ein Auftrag gleichzeitig
moeglich; Fortschritt und Abbrechen sind sichtbar. Ein bestehendes Programm wird nur
nach erneutem Download-Klick ersetzt. Die App protokolliert weder Download-Antworten
noch Zugangsdaten; Fehler nennen nur die Art und gegebenenfalls den Zielhost.

### Abo-Programme nicht herunterladen

Die Richtlinie `settings.cli_download_disabled: true` in `settings_store.json` sperrt
beide Direktdownloads auch im Backend. Sie kann fuer die IT-Verteilung vorbelegt werden.
In der App steht sie bei KI-Modelle & Anbieter. Sie nutzt denselben gesperrten
Richtlinien-Schreibpfad wie das Kundenprofil; veraltete Gesamtkopien von Einstellungen
koennen die Sperre nicht aufheben. Wie das bestehende Kundenprofil ist sie eine
verteilbare Einstellung, keine gegen den lokalen Benutzer administrativ verriegelte
Gruppenrichtlinie. Programm-Auswahl, Anleitung und Anmeldung bleiben verfuegbar.

Die Abo-Programme laufen als Kindprozess ohne Konsolenfenster. Der Firmenproxy wird nur
in deren Prozessumgebung gesetzt (`HTTPS_PROXY`, `HTTP_PROXY`,
`NO_PROXY=localhost,127.0.0.1`), nie fuer den ganzen Rechner. Bei der Anmeldung der
Anbieter-CLI oeffnet die App ein Konsolenfenster mit deren eigenem Anmeldebefehl
(`claude auth login`, `codex login`) und der Browser-Anmeldung des Anbieters.

Unter Windows werden Piper, Fish Speech, llama-server und verwaltete CLI-Läufe
angehalten gestartet, vor ihrem ersten eigenen Befehl einem Job-Objekt zugeordnet
und erst danach fortgesetzt. Der Job beendet beim Schließen auch Unterprozesse;
scheitert die Zuordnung (etwa durch eine übergeordnete Job-Richtlinie), wird der
Start abgebrochen. Die vom Nutzer geöffneten Anmeldefenster bleiben unabhängig
und dürfen die App überleben. Nur für einen Batch-Starter bei „Anmelden“ wird
ausdrücklich der Kommandozeileninterpreter gestartet.
Für reguläre CLI-Läufe muss eine native Programmdatei oder der vorhandene direkte
Node-Starter auflösbar sein. Ein übrig gebliebener Batch-Starter wird dort abgelehnt,
da auch die Standardbibliothek sonst implizit einen Interpreter starten würde.

Prozess-Stopp und Port-Abfrage verwenden unter Windows Win32-APIs; die App startet
dafür weder `taskkill` noch `netstat`. Private Sync-Dateien erhalten bereits beim
Anlegen eine geschützte DACL ausschließlich für die eigene Benutzer-SID, ohne
`icacls`. Einstellungen und Seitendateien öffnet die App über `ShellExecuteW`.
Der bisherige TTS-Sonderweg zum Beenden eines fremd gestarteten Servers bleibt
auf den konfigurierten IPv4-Loopback-Port begrenzt; beim normalen Stopp eines
fremden Servers wird zusätzlich dessen Gesundheitsantwort geprüft. Hartes Beenden
und das Aufräumen beim App-Ende hatten diese Gesundheitsprüfung bereits bisher
nicht. Vor dem Beenden wird der Port-Besitzer mit gehaltenem Prozess-Handle erneut
abgeglichen.

**Verbleibende Ausnahme (lva#877):** Der optionale Hub-Sync verwendet für POSTs
weiterhin `curl.exe` aus dem Windows-Systemverzeichnis. Der vorhandene Code
dokumentiert HTTP 500 des IONOS-Frontends für dieselbe Anfrage über reqwest/hyper,
während System-curl funktioniert. Das Netzmodul stellt derzeit ebenfalls
reqwest-Clients bereit; ein gleichwertiger Ersatz einschließlich Proxy,
Systemzertifikaten und Anmeldung ist nicht nachgewiesen. Deshalb bleibt dieser
Transport unverändert: sensible Konfiguration nur über stdin, private temporäre
Body-Datei, Zeit- und Antwortgrößenlimit sowie Abbruch-Watchdog. Eine allgemeine
Aussage „keine Windows-Systemwerkzeuge im Normalbetrieb“ gilt damit noch nicht.

## Update-Verhalten

Die App aktualisiert nichts still. Insbesondere gilt:

- **Kein stilles npm.** Die App ruft `npm` nicht auf. Meldet der Server, dass die Codex-CLI zu alt ist, zeigt die App nur einen Hinweis. Ein neuer Direktdownload erfordert einen ausdruecklichen Klick und ist durch die Firmenrichtlinie sperrbar. Der fruehere Schalter "CLI automatisch aktualisieren" bleibt ohne Wirkung; ein gespeichertes "an" wird einmalig auf "aus" gesetzt.
- **Kein Update-Skript in %TEMP%.** Der Update-Weg schreibt keine Hilfsskripte und startet weder einen Kommandozeileninterpreter noch Systemwerkzeuge. Die oben beschriebenen Ausnahmen für CLI-Anmeldung und Hub-Sync gehören zu anderen Funktionen.
- **Lokale Installer nur auf Wunsch.** Ein Update aus einem Ordner gibt es nur, wenn die IT oder der Nutzer den Ordner ausdruecklich in den Einstellungen eingetragen hat; ohne Eintrag sucht die App nicht. Ein Klick startet keinen Installer: die App zeigt den Pfad und oeffnet den Ordner im Explorer. Der Nutzer beendet die App und startet den Installer selbst sichtbar.
- **Spaeter:** ein signierter, mitgelieferter Update-Helfer (lva#746) soll den Start nach App-Ende uebernehmen. Bis dahin ist der IT-Verteilweg (Softwareverteilung) der vorgesehene Weg fuer Firmenrechner.

## Netzprofil

Die App nutzt nach aussen nur ausgehendes HTTPS auf Port 443. Sie lauscht nur auf
127.0.0.1 (Weiterleiter fuer die Proxy-Anmeldung, Anmeldung bei Microsoft 365). Die
Loopback-Bindung ist durch Quelltext-Tests abgesichert.

## Zieladressen

Diese Tabelle wird aus `apps/local-voice/src-tauri/src/netz/ziele.rs` erzeugt; ein Test prueft, dass sie nicht abweicht. In der App: Einstellungen, Netz, Adressliste kopieren.

<!-- ZIELE:BEGIN -->
| Funktion | Adresse | Port | Zweck |
|---|---|---|---|
| Abo-Programme (Direktdownload nur auf Klick) | downloads.claude.ai | 443 | Claude: Version, Manifest und Programmdatei |
| Abo-Programme (Direktdownload nur auf Klick) | api.github.com | 443 | Codex: Release-Metadaten mit SHA-256 des Archivs |
| Abo-Programme (Direktdownload nur auf Klick) | github.com | 443 | Codex: offizielles Windows-Archiv |
| Abo-Programme (Direktdownload nur auf Klick) | objects.githubusercontent.com | 443 | GitHub-Release-Dateien (Weiterleitung) |
| Abo-Programme (Direktdownload nur auf Klick) | release-assets.githubusercontent.com | 443 | GitHub-Release-Dateien (Weiterleitung) |
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
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | api.anthropic.com | 443 | Claude Code (Programm von Anthropic) |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | claude.ai | 443 | Anmeldung von Claude Code |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | chatgpt.com | 443 | Codex mit ChatGPT-Konto (Programm von OpenAI) |
| KI-Abo (Programme der Anbieter, von der IT oder dem Nutzer installiert) | api.openai.com | 443 | Codex (Programm von OpenAI) |
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
