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
| iPhone gesperrt | OFFEN | macOS-XCTest-Fensterautomation nicht freigegeben; kein Lock-Nachweis |
| iPhone Hintergrund | PASS: Jobs bleiben gespeichert, Verarbeitung nach Rückkehr | kein unbegrenzter Hintergrundbetrieb |
| iPhone-App beendet | TEILWEISE: Prozessende/Neustart im 100er-Lauf ohne Verlust | kein Nachweis der Swipe-Force-Quit-Wake-up-Semantik |
| Handgelenk gesenkt | OFFEN | Simulator-Wrist-Down-Steuerung noch nicht geprüft |
| Verbindung unterbrochen/wiederhergestellt | PASS im Simulator: iPhone heruntergefahren, neu gestartet, alle 100 abgeschlossen | kein physischer Funknachweis |
| Neustart während Übertragung | PASS: Watch nach erstem dauerhaftem 32-KiB-Teil beendet; 142099-Byte-Aufnahme nach Neustart vollständig angenommen | kein Stromausfalltest |
| Doppelte Nachrichten | Kern-PASS: stabile IDs/Quittungen; Live-Replay ausgeführt | Kern-Aussagen nicht mit Netzwerkliefergarantie gleichsetzen |
| Mikrofon verweigert | PASS: tatsächlicher Berechtigungspfad, Ereignis microphone_denied | simulatorgesteuerte Berechtigungsänderung |
| Aufnahme starten/stoppen | PASS: nativer Watch-UI-Test und 30-Sekunden-Recorderprobe, dauerhaft gesichert | Mikrofonhardware der Watch nicht simuliert |
| Audio unterbrochen | TEILWEISE: gezielter Synthesizer-Stop auf iPhone, Antworten bleiben erhalten | Watch-Anruf-/Routenunterbrechung offen |
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

macOS meldet `DevToolsSecurity: disabled`; der Simulator-Steuerungs-UI-Test scheiterte
beim Initialisieren der Automation. Lock/Wrist-Down bleiben bis zur Freigabe offen.
Der angefragte externe Claude-Code-Review wurde von der automatischen Freigabeprüfung
wegen fehlender ausdrücklicher Zustimmung zur Übermittlung der vier konkreten privaten
Quelldateien abgelehnt. Kein Claude-Review wurde durchgeführt.
P2 wird in `P2-follow-up.md` geplant; spätere Produktstufen werden nicht umgesetzt.
