# P0/P1 – Simulator-Zwischenabnahme, 05.09.2026

**Ergebnis: nativer Watch-/iPhone-Prototyp baut und lief im gekoppelten Simulator.
P0/P1 sind noch nicht vollständig abgenommen.** Der Benutzer hat physische Geräte
für diese Runde durch virtuelle Xcode-Geräte ersetzt. Daher keine Behauptungen
über reale Akkuauswirkung, Funk, gesperrte Geräte oder gesenktes Handgelenk.

## Arbeitsstand und Schutz des Desktop-Projekts

Worktree `/Users/patrick/Documents/Codex/local-voice-ai-apple-p0-p1`, Branch
`codex/apple-p0-p1`, Ausgangs-HEAD `ccdee8f75c2a21aa8cffdc58c0618542d908cb5c`.
Die vorhandenen lokalen Desktop-Änderungen liegen weiterhin im ursprünglichen
Checkout. Keine Desktop-Quelldateien geändert, kein Push, keine Veröffentlichung.

Commits: `f4bc08a` Preflight/Plan, `e7ae325` Persistenzkern,
`bb475cc` Protokoll/Quittungsschutz, `70549ec` nativer Prototyp.

## Umgebung und frische Prüfungen

- MacBookPro16,1, Intel i9-9980HK, 2,40 GHz, 8 Kerne, 32 GB; macOS 15.7.9.
- Xcode 26.3 (17C529); iOS/watchOS SDK 26.2.
- iPhone 15 Pro Max: iOS 26.3.1 (23D8133), Universal-Simulator.
- Watch Ultra 3 (49 mm): watchOS 26.2 (23S303), Universal-Simulator.
- Virtuelles Paar über simctl angelegt, aktiviert und verbunden.
- **10 XCTest-Tests, 0 Fehler**, frischer Lauf, 2,274 s Testsuite.
- **Simulator-Build erfolgreich**, VoicePhone einschließlich eingebetteter Watch-App.
- **Gerätearchitekturen erfolgreich kompiliert**, ohne Signing; kein Gerätetest.
- `git diff --check` erfolgreich. Desktop-Builds/-Tests in dieser Runde nicht ausgeführt.

Die ersten sechs Persistenztests wurden vor der Implementierung ausgeführt:
Rot wegen fehlendem Zielcode, danach Grün. Weitere Tests decken beschädigtes
Audio, verspätete Quittung, Payload und Envelope-Roundtrip ab.

## Nachgewiesener vertikaler Teilpfad

Synthetische deutsche Audio-Testdatei → dauerhafter Watch-Speicher → tatsächlicher
WCSession-Adapter im Simulator → dauerhafter iPhone-Speicher → feste Testantwort →
Antworttext zur Watch → AVSpeechSynthesizer-Startcallback → gespeicherter Verlauf.
Die Testdatei wurde mit macOS-System-TTS erzeugt; sie ersetzt keine Mikrofonabnahme.
Watch-TTS-Callbacks sind kein unabhängiger Hörnachweis.

Ein erster Einzelturn plus ein 100-Aufnahmen-Lauf endeten mit **101 Einträgen auf
beiden Geräten, alle beantwortet, keine fehlende bestätigte Aufnahme**. Während
dieses Laufs wurden Watch und iPhone beendet/neugestartet und das iPhone durch
Safari in den Hintergrund versetzt. Der Hintergrund-Snapshot zeigt sechs auf dem
iPhone gespeicherte, noch nicht beantwortete Aufnahmen. Nach Wiederöffnung wurden
auch diese verarbeitet. Alle Watch-IDs blieben nach Neustart erhalten.

Der Dauerlauf entstand während der Implementierung vor der abschließenden
Envelope-Struktur-/Mikrofon-API-Änderung. Der abschließende Quellstand hat frische
Builds und Protokolltests; ein kompletter erneuter Simulator-Dauerlauf auf diesem
Stand ist **noch offen**. Ein Live-Duplikatversuch endete im Transportfehler, während
der Watch-Berechtigungsdialog die weitere Prüfung blockierte. Das ist kein PASS.

## Lifecycle-Matrix

| Angeforderter Fall | Tatsächliches Ergebnis | Grenze |
|---|---|---|
| iPhone Vordergrund | PASS im Simulator: fester Antwortpfad, persistenter Verlauf | synthetische Audiodatei; keine lokale KI |
| iPhone gesperrt | NICHT VERIFIZIERT | keine belastbare Data-Protection-/Lock-Abnahme |
| iPhone-App Hintergrund | PASS im Simulator: angenommen, Verarbeitung aufgeschoben, bei Rückkehr beendet | keine Zusage für unbegrenzten Hintergrundbetrieb |
| iPhone-App vom Benutzer beendet | TEILWEISE: Prozessende/Neustart mit ausstehenden Jobs geprüft | simctl terminate ist kein Beleg für iOS-Swipe-Force-Quit-/Wake-up-Semantik |
| Gesenktes Handgelenk | NICHT VERIFIZIERT | kein physischer Wrist-down-/Audio-Routing-Test |
| Verbindungsabbruch/Wiederherstellung | OFFEN: spätere Testaufnahme bleibt während Unerreichbarkeit auf Watch gesichert | kein abgeschlossener gezielter Funk-/Reconnect-Test |
| App-Neustart während Übertragung | PASS im Simulator: alle bestätigten IDs erhalten; spätere Verarbeitung | kein Stromausfalltest |
| Doppelte Nachrichten | PASS im Kern: 100 erneute Zustellungen, unveränderte Quittungen/Eintragszahl; Live-Test OFFEN | letzter Live-Versuch meldete Transportfehler |
| Mikrofonberechtigung verweigert | OFFEN: nativer Watch-Dialog sichtbar, Benutzeraktion angefordert | simctl revoke unterdrückte Dialog nicht zuverlässig |
| Audio unterbrochen | TEILWEISE: gezieltes AVSpeechSynthesizer-Stop auf iPhone, 101 Antworten erhalten | Watch-Audio-Unterbrechung/Anruf/Route noch offen |
| Speicher voll/falsche Version/beschädigte Daten | PASS im Kern: keine Quittung, keine Überschreibung | kein echter voller Simulator-Datenträger |
| Lokale Modelle fehlen | PASS als Deferred-Fall direkt auf iPhone: Aufnahme bleibt, Zustand deferred | kein erfolgreicher STT-/KI-Inferenzlauf |

## Messwerte – ausschließlich Simulator

Gemischter Entwicklungs-Dauerlauf inklusive Hintergrund/Neustart, Debug-Build,
synthetische kurze komplette Turns. Keine physische Referenzlatenz und keine
sauber getrennte Warm-/Kaltstartstatistik. Rohdaten unter `results/100-turns.json`.

| Stufe | n | Median | p95 | Maximum |
|---|---:|---:|---:|---:|
| Transfer + persistente Empfangsquittung (Roundtrip) | 100 | 2.953 ms | 12.967 ms | 40.531 ms |
| System-TTS-Aufruf bis didStart-Callback auf Watch | 95 | 55,4 ms | 122,0 ms | 136,5 ms |
| Auswahl der festen Testantwort (ohne Persistenz/TTS) | 101 | 0,002 ms | 0,004 ms | 0,028 ms |

Erster Einzelturn: Transfer/Quittung 4.812,5 ms, TTS-Start 109,5 ms. Die Werte
sind Teil des späteren gemischten Datensatzes. Ein bei Neustart unterbrochener
Versuch hat keine abgeschlossene Timing-Probe; nicht jeder beantwortete Turn
wurde automatisch gesprochen. Es gibt keine Messwerte für Mikrofonfeedback,
STT oder lokale Antwortgenerierung. Der Code instrumentiert Aufnahmefeedback,
Persistenz, STT und Modellantwort für spätere ausführbare Läufe.

Die angegebenen Performanceziele sind damit **nicht nachgewiesen**; Transfer-p95
allein überschreitet das ursprüngliche Ziel für eine erste Antwort deutlich.
Die Auswahl einer Konstanten ist keine Messung von KI-Antwortgenerierung.

## Lokale Modelle und Echtzeitentscheidung

Die echte API-Abfrage im iPhone-Simulator liefert:

- SpeechTranscriber.isAvailable = false
- deutsche SpeechTranscriber-Assets: unsupported
- Foundation Models: unavailable, modelNotReady

Daher: Der feste Testantwortpfad funktioniert bei aktiven, erreichbaren Apps.
Die Laufzeit ist in dieser Umgebung noch kein belastbarer Echtzeitnachweis.
Bei fehlenden Modellen oder nicht aktiver iPhone-Verarbeitung bleibt die Notiz
mit „gespeichert – Verarbeitung folgt“ erhalten. Es gibt keinen Cloud-Fallback.

Nach dem Dauerlauf kamen zwei getrennte Probeaufnahmen hinzu: eine weiterhin auf
der Watch wartende Aufnahme und eine direkt auf dem iPhone importierte Deferred-
Aufnahme. Deshalb sind die neuesten 102 IDs je Gerät nicht identisch; das ist in
`results/latest.json` sichtbar und kein Widerspruch zur abgeschlossenen 101er-
Dauerlauf-Stichprobe. Die wartende Watch-Aufnahme darf nicht als übertragen gelten.

## Akku und offene Abnahme

**Akkuauswirkung unbekannt.** Simulatorwerte oder Host-CPU-Last sind keine Watch-
Akkumessung. Der geplante kontrollierte 8-Stunden-Vergleich wurde nicht ausgeführt.

Nächste konkrete Benutzeraktion: im geöffneten Watch-Simulator für den aktuellen
Verweigerungstest „Nicht erlauben“ wählen. Anschließend erneute Zustellung,
Watch-Audioabbruch, erfolgreicher PTT-Start/Stop und finale Regression prüfen.
Für erfolgreiche lokale STT/KI ist zusätzlich eine tatsächlich unterstützte
Ausführungsumgebung erforderlich. P2 wird nur geplant, nicht implementiert;
siehe `P2-follow-up.md`.
