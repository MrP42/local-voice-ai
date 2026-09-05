# P0/P1 – Simulator-Nachweis, 05.09.2026

Der native SwiftUI-Prototyp läuft auf dem gekoppelten iPhone-15-Pro-Max- und
Watch-Ultra-3-Simulator. **P1 ist noch nicht vollständig abgenommen.** Der Benutzer
hat ausdrücklich virtuelle Xcode-Geräte statt physischer Geräte beauftragt.
Dieser Bericht ersetzt die frühere Zwischenabnahme mit ausschließlich fester Antwort.

## Umgebung und Schutz des Bestands

Intel MacBookPro16,1, i9-9980HK, 8 Kerne, 32 GB; macOS 15.7.9; Xcode 26.3,
iOS/watchOS SDK 26.2. iPhone-Simulator iOS 26.3.1, Watch-Simulator watchOS 26.2.
Worktree: `/Users/patrick/Documents/Codex/local-voice-ai-apple-p0-p1`, Branch
`codex/apple-p0-p1`, Ausgangscommit `ccdee8f75c2a21aa8cffdc58c0618542d908cb5c`.
Desktop-App und lokale Änderungen im ursprünglichen Checkout bleiben erhalten.
Kein Push und keine Veröffentlichung. Simulator-Builds benötigen kein Developer-Team.

## Ausgeführte Prüfungen

- Frischer VoicePhone-Simulator-Build inklusive Watch: erfolgreich.
- Persistenz-/Protokolltests: 17 Tests, 0 Fehler, 2,620 Sekunden am 05.09.2026.
- Native Watch-UI-Prüfung für Aufnahme, Stop, Home und Wiederöffnung: 1 Test bestanden,
  47,8 Sekunden. Derselbe iPhone-UI-Test frisch bestanden: 1 Test, 61,568 Sekunden.
  Kein unabhängiger akustischer Hörnachweis.
- Native CPU-Inferenz für Gerätearchitektur zuvor erfolgreich ohne Signing kompiliert;
  kein physischer Gerätetest.
- 100 echte lokale STT-/Modellantwort-Turns auf Stand `a907d58`: alle dauerhaft
  angenommen, transkribiert, beantwortet und auf der Watch quittiert; 821 Sekunden.
  Frischer Audit prüft alle 100 IDs, identische Audio-Digests, Transkripte und Antworten,
  stabile Antwort-IDs, dauerhafte Antwortquittungen und beantwortete Zustände.
  Rohdaten: `results/native-local-100.json`; Prüfskript: `audit_local_report.py`.
- Deutsche Qualitätsfälle: fünf aus der Desktop-Testmenge regenerierte synthetische
  Aufnahmen, tatsächlich durch WatchConnectivity, Whisper und Qwen verarbeitet.
  `results/german-quality.json` dokumentiert Wortfehler von Base. Schlagworttests sind
  keine vollständige Qualitätsabnahme; Zahlenschreibweisen müssen semantisch beurteilt werden.

Der feste Antwortpfad wurde zuerst getrennt getestet. Danach lief der vollständige
Pfad mit Whisper Base multilingual und Qwen2.5-0.5B Q4_K_M direkt im iPhone-Prozess.
Apple SpeechTranscriber und Foundation Models melden im Intel-Simulator weiterhin
unavailable/modelNotReady. Es gibt keinen Cloud- oder Mac-Server-Fallback.
Watch verwendet ausschließlich native Aufnahme, Transport und System-TTS.

## Lifecycle-Matrix

| Fall | Ergebnis und Nachweis | Verbleibende Grenze |
|---|---|---|
| iPhone Vordergrund | PASS: lokaler STT-/Antwortpfad, 100 geprüfte Turns | Simulator, synthetische Audiodateien |
| iPhone gesperrt | PASS im Simulator: Aufnahme dauerhaft angenommen, 15,5 s ohne Antwort, nach Entsperren dieselbe Aufnahme lokal verarbeitet und quittiert | kein physischer Data-Protection-Nachweis |
| iPhone Hintergrund | PASS: Jobs bleiben gespeichert, Verarbeitung nach Rückkehr | kein unbegrenzter Hintergrundbetrieb |
| iPhone-App beendet | PASS im Simulator: Local-Voice-Karte per UI weggewischt, Prozessabfrage und Screenshot geprüft; neue Aufnahme gespeichert, Antwort erst nach Wiederöffnung | keine Übertragung der Simulator-Wake-up-Beobachtung auf reale Geräte |
| Handgelenk gesenkt | PASS im Simulator: Always On während Aufnahme, nach 16,572 s automatisch gesichert; nach Anheben lokal verarbeitet und quittiert | bei abgesenktem Zustand zunächst nur auf Watch gespeichert |
| Verbindung unterbrochen/wiederhergestellt | PASS im Simulator: iPhone heruntergefahren, neu gestartet, alle 100 abgeschlossen | kein physischer Funknachweis |
| Neustart während Übertragung | PASS: Watch nach erstem dauerhaftem 32-KiB-Teil beendet; 142099-Byte-Aufnahme nach Neustart vollständig angenommen | kein Stromausfalltest |
| Doppelte Nachrichten | PASS: frischer WCSession-Replay, nichtleere Antwort; alle 220 iPhone-IDs, Quittungen, Audio-Digests und Antwort-IDs unverändert | ein gezielter Live-Replay plus Kerntests |
| Mikrofon verweigert | PASS: tatsächlicher Berechtigungspfad, Ereignis microphone_denied | simulatorgesteuerte Berechtigungsänderung |
| Aufnahme starten/stoppen | PASS: nativer Watch-UI-Test und 30-Sekunden-Recorderprobe, dauerhaft gesichert | Mikrofonhardware der Watch nicht simuliert |
| Audio unterbrochen | PASS für gezielten Stop auf iPhone und Watch: frischer Watch-Startcallback, Stop, alle 217 bestehenden Antworten unverändert | Anruf-/Routenunterbrechung weiterhin offen |
| Speicherlimit/Versionsfehler/korruptes Audio | PASS im Kern: keine Annahmequittung und keine Überschreibung | kein voller physischer Datenträger |
| Modelle fehlen | PASS: Aufnahme bleibt deferred | explizite lokale Modellinstallation erforderlich |

## Messwerte

Gemischter Simulator-Dauerlauf mit Hintergrund, Neustarts und Verbindungsunterbrechung.
Keine kontrollierte Warm-/Kaltstart- oder physische Referenzmessung. Die folgenden
Werte gehören genau zu den 100 IDs im lokalen Modelllauf, nicht zum älteren Festantwortlauf.

| Stufe | n | Median ms | p95 ms | Maximum ms |
|---|---:|---:|---:|---:|
| Transfer und dauerhafte Empfangsquittung | 100 | 1301,30 | 2602,31 | 58413,33 |
| STT (Base) | 100 | 5242,69 | 6008,97 | 13225,43 |
| lokale Antworterzeugung | 100 | 1834,54 | 2213,54 | 9829,37 |
| Watch-TTS-Aufruf bis Startcallback | 96 | 92,50 | 124,55 | 290,03 |

Separate tatsächliche Recorderprobe: Aufnahmefeedback 280,566 ms, dauerhafte
Sicherung 93,101 ms (jeweils n=1, kein p95). Der lange Turn nach Teiltransfer-Neustart
benötigte rund 10070,95 ms für Transfer/Quittung. Ein unterbrochener Sprachstart liefert
keine abgeschlossene Timingprobe; Antworten bleiben unabhängig von Wiedergabe gespeichert.

Die Performanceziele sind **nicht erreicht/nicht nachgewiesen**. Insbesondere ist
STT-p95 bereits größer als das Drei-Sekunden-Ziel für die erste gesprochene Antwort.
Der abgeschlossene Small-Vergleich (`results/german-quality-small.json`) verbessert
„morgen“ und „Zeile“, verliert aber im Umlautfall „Köln“ und erkennt „Glühwein“
weiter falsch. STT-Median über fünf unterschiedliche Qualitätsaufnahmen: Base
7658,52 ms, Small 27164,28 ms. Base bleibt auf dem Testsimulator ausgewählt; Small
ist eine explizite Vergleichsoption. Diese fünf Fälle ersetzen nicht den 100er-Lauf.

## Echtzeit und Aufschub

Bei aktivem iPhone, erreichbarem Paar und vorhandenen lokalen Modellen funktioniert
der vollständige lokale Antwortpfad. Die gemessene Intel-Simulator-Latenz erlaubt
keine Zusage einer Echtzeitantwort im ursprünglichen Zielbereich.
Bei Hintergrund, Unerreichbarkeit oder fehlendem Modell bleibt die Aufnahme dauerhaft
mit „gespeichert – Verarbeitung folgt“ erhalten. Verarbeitung startet bei erneuter
Aktivierung; Quittungen erfolgen erst nach dauerhafter Sicherung. Bestätigte Originale
werden in P1 nicht automatisch gelöscht. Partielle Transferquittungen sind keine
Bestätigung einer vollständig gesicherten Aufnahme auf dem iPhone.

## Akku und offene Freigaben

**Akkuauswirkung unbekannt und im Simulator nicht belastbar messbar.** Der geplante
8-Stunden-Vergleich wurde nicht ausgeführt. Keine Leerlauf-Mikrofon-/ML-Schleife,
kein Heartbeat-Polling; daraus folgt keine bezifferte Akkuersparnis.

Der Benutzer hat die macOS-Entwicklerfreigabe aktiviert. Der frische macOS-UI-Test
kann den Simulator steuern; Lock/Wrist-Down sind nun als Deferred-Fälle nachgewiesen.
Frühere fehlgeschlagene Automationstests bleiben historische Fehlversuche.
Der angefragte externe Claude-Code-Review wurde von der automatischen Freigabeprüfung
wegen fehlender ausdrücklicher Zustimmung zur Übermittlung der vier konkreten privaten
Quelldateien abgelehnt. Kein Claude-Review wurde durchgeführt.
P2 wird in `P2-follow-up.md` geplant; spätere Produktstufen werden nicht umgesetzt.

### Zusätzliche gezielte Simulatorprüfungen

`results/watch-playback-stop.json`: Watch mit `--interrupt-playback` gestartet;
frischer TTS-Startcallback und frisches `playback_stopped`-Ereignis, alle vorhandenen
Eintrags-IDs, Audio-Digests und Antworttexte erhalten. Kein simulierter Telefonanruf.
`results/live-duplicate.json`: aktive Phone-App und Watch mit `--replay-capture`;
nichtleere Transportantwort sowie unveränderte iPhone-IDs, persistente Quittungen,
Audio-Digests und Antwort-IDs. Die 217/220 Gesamteinträge enthalten zusätzliche
Einzelprüfungen und sind nicht die abgegrenzte 100-Turn-Stichprobe.

### Sperren und Handgelenk – nach aktivierter Entwicklerfreigabe

`results/locked-phone.json`: sichtbare Sleep/Wake-Taste per macOS XCTest betätigt,
Sperrbildschirm per Simulator-Screenshot visuell überprüft. Neue Watch-Fixture wurde
auf dem gesperrten iPhone dauerhaft angenommen, aber während 15,5 Sekunden nicht
beantwortet. Nach Home/Aufwecken und Öffnen der iPhone-App wurde genau dieselbe ID
mit unverändertem Audio-Digest transkribiert, beantwortet und auf der Watch quittiert.

`results/wrist-down.json`: tatsächliche Recorderprobe, danach Always-On-Schalter
im Watch-Simulator. Dauer 16,572 Sekunden per `afinfo`, also vor dem automatischen
30-Sekunden-Limit gesichert. Im abgesenkten Zustand blieb die Aufnahme auf der Watch;
nach Anheben erfolgten Übertragung, lokale Antwort und dauerhafte Quittung. Ein
früherer fehlgeschlagener UI-Test produzierte separat 29,884 Sekunden Audio und wird
nicht als Nachweis für automatisches Sichern bei Wrist Down gezählt.

Die numerische Checkbox-Auswertung wurde im Test korrigiert. Der Recorderstart
muss außerhalb der XCTest-App-Sandbox erfolgen; `scripts/test_wrist_simulator.py`
koordiniert ihn mit dem sichtbaren UI-Schalter. Diese Testwartezeit existiert nur
im Prüfablauf und ist keine Verzögerung der App.
Apple beschreibt den [Always-On-Simulatorschalter](https://developer.apple.com/documentation/watchos-apps/designing-your-app-for-the-always-on-state)
und die [Simulation des Wrist-Down-Ereignisses](https://developer.apple.com/videos/play/wwdc2021/10002/).

### Benutzerseitiges Beenden im App-Umschalter

`results/force-quit.json`: Der echte Doppelklick auf Simulator-Home öffnete den
App-Umschalter. Die sichtbare Local-Voice-Karte wurde per UI nach oben weggewischt;
der folgende Screenshot zeigte nur noch Safari. `launchctl list` enthielt danach
keinen Local-Voice-Prozess. Eine neue Watch-Fixture wurde dauerhaft angenommen,
blieb aber 15,5 Sekunden ohne Antwort. Nach expliziter Wiederöffnung wurden dieselbe
ID und derselbe Audio-Digest transkribiert, beantwortet und auf der Watch quittiert.
Die Prozessabfrage wurde durch eine positive Kontrolle gegen die anschließend
laufende App validiert. Abfragen sind Stichproben: kurzzeitige Hintergrundausführung
zwischen ihnen ist nicht ausgeschlossen. Keine Aussage über identisches Verhalten
auf einem physischen iPhone.

Vorherige UI-Versuche mit getrennten Home-Aktionen bzw. einer Geste innerhalb des
iPhone-Test-Runners öffneten den App-Umschalter nicht zuverlässig und zählen nicht
als Erfolg. Die geprüfte Sequenz ist `testOpenPhoneSwitcher`, visuelle Kontrolle der
Local-Voice-Karte, anschließend `testDismissVisiblePhoneCard` im macOS-Testziel.
Die Koordinaten des zweiten Hilfstests gelten ausschließlich für den dokumentierten
Simulatorfensterzustand; nicht blind auf einen anderen App-Umschalter anwenden.
