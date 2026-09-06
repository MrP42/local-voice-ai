# P2 – native Apple-Stabilisierung (laufende Abnahme)

Implementiert auf dem isolierten Branch `codex/apple-p0-p1`, ausgehend von
P1 `cfd180a`. Programmstand für die laufende Abschlussmessung: `d414228`.
Virtuelles iPhone 15 Pro Max und Watch Ultra 3 bleiben der beauftragte Umfang.

## Änderungen

- Persistente Aufträge mit drei Versuchen, verzögertem Retry, explizitem Abbruch
  und Wiederaufnahme ab gespeichertem Transkript. Inaktive App beendet den Worker
  kooperativ; kein dauerhaftes Hintergrund-Inferenzversprechen.
- Aufnahme, Transport und Verarbeitung sind getrennt. Die Watch enthält weiterhin
  weder Whisper/Qwen noch Desktop-/ONNX-Engines.
- Beschädigte Einträge bleiben sichtbar und erhalten ihre Originale. Gesunder
  Verlauf bleibt verwendbar. Wiederherstellung vollständiger Staging-Transaktionen,
  additive Identitätsmigration, separate Budgets für temporäre Dateien.
- Modellanzeige, explizite Base-/Small-Auswahl, Import mit exakter Größe/SHA-256
  und atomarem Austausch. Der lokale CPU-Pfad bleibt auf dem iPhone.
- Digitale Stille und stationäres breitbandiges Rauschen werden vor STT abgewiesen.
  Das ist kein allgemeiner Sprachaktivitätsdetektor. Ein begrenzter deutscher
  Aktionsfilter verhindert die geprüften falschen Vollzugsmeldungen.
- Validierter Verlauf wird während der aktiven Sitzung zwischengespeichert;
  Schreibfehler, Wiederherstellung und Aktivierung invalidieren ihn. Einzelzugriffe
  vor Mutation/Transfer prüfen das Audio weiterhin.

## Qualitätsgrenzen

Die zwölf synthetischen Fälle enthalten deutsche Umlaute, Zahlen, Absatzwörter,
Fakten/Rechnen, externe Aktionswünsche, Negation, Stille und Rauschen. Das kleine
Qwen-Modell antwortete auf korrekt transkribiertes „7 + 5“ mit „8“. Dieser Befund
ist nicht behoben und verhindert eine Aussage über allgemeine Antwortzuverlässigkeit.
Die Modellanzeige benennt inhaltliche Fehler ausdrücklich.

Die bestehenden fünf P1-Fälle vergleichen identische Audiohashes: Base erreicht
14/21 wörtliche Schlüsselwörter bei median 7,659 s STT; Small 16/21 bei 27,164 s.
Small verbessert konkrete Wortfehler, braucht aber deutlich länger. Die Wertung
bestraft auch gleichbedeutende Zahlenschreibweisen und ist keine Wortfehlerrate.
Sie ist als Auswertung vorhandener Messungen, nicht als neuer P2-Small-Lauf markiert.

## Messdefinitionen und Grenzen

`reply_e2e_ms` misst Aufnahmeende bzw. gesicherte Testfixture bis zur dauerhaft
angenommenen Antwort auf der Watch. `tts_e2e_ms` endet beim ersten tatsächlichen
AVSpeechSynthesizer-Startcallback. p95 wird aus vollständigen Einzelzeiten gebildet,
nicht aus der Addition von Stufenquantilen. Fixture-Läufe umgehen das Mikrofon;
Aufnahmefeedback stammt deshalb ausschließlich aus den separaten Recorderfällen.

Kalt bedeutet neuer iPhone-Prozess; warm bedeutet der nachfolgende Turn im selben
Prozess. Beide Gruppen starten den Watch-Prozess neu. Betriebssystem-Caches werden
nicht geleert, native Modellkontexte weiterhin pro Aufruf erstellt.

Physische Funkunterbrechungen, verschlüsselte Daten im Gerätesperrzustand,
Anruf-/Bluetooth-Routen, Apples hier nicht verfügbare Modellanbieter und
Akkuprozentwerte sind mit diesem Simulator nicht belegt. Für eine spätere
Hardwareabnahme bleiben drei vergleichbare 8-Stunden-Läufe mit 30 Turns vorgesehen.
Keine automatische Original-Löschung und keine Veröffentlichung.

## Noch laufend

Nur der externe Claude-Review ist offen. Latenzläufe, 100er-Dateiaudit und die am 06.09.2026 nachgeholten Fensterbedienungsfälle sind abgeschlossen.
Die automatische Freigabeprüfung hat die erneute Übermittlung privater Quelldateien
an Claude blockiert. Eine konkrete Rückfrage für genau vier Dateien ist offen;
bis zur Antwort wurde kein P2-Quelltext an Claude übermittelt.

## Frische Laufzeitmessung auf `d414228`

Zehn Paare mit derselben deutschen Sprachdatei, vollständiger Pfad Watch → iPhone
→ Watch, tatsächliche lokale CPU-Inferenz. Alle zwanzig Antworten wurden auf
beiden Seiten mit identischen Texten und bestätigter Antwortidentität geprüft.

| Messung | Kalt Median / p95 | Warm Median / p95 |
|---|---:|---:|
| STT | 8,480 / 8,641 s | 7,536 / 8,015 s |
| Antworterzeugung | 8,733 / 9,262 s | 6,082 / 6,453 s |
| Übertragungsquittung | 4,066 / 4,538 s | 4,134 / 4,893 s |
| E2E bis gespeicherte Watch-Antwort | 22,097 / 22,756 s | 18,412 / 19,432 s |
| E2E bis TTS-Start | 22,397 / 23,083 s | 18,607 / 19,631 s |

Bei n=10 je Gruppe ist der verwendete nearest-rank-p95 zugleich das Maximum,
keine belastbare Aussage über seltene Ausreißer. Die ursprünglichen Latenzziele
werden nicht erreicht. Durchgehender Dialog ist nur bei aktiven Apps und
erreichbarem Gegenüber möglich; „sofortige“ Antworten sind auch dann nicht belegt.

Die drei Recorderproben je Gerät während P2 (verschiedene P2-Zwischenstände)
zeigen median 818 ms Aufnahmefeedback am iPhone und 769 ms an der Watch.
Persistenz: 554 bzw. 537 ms. Diese kleine Stichprobe ersetzt keine Hardwaremessung.

Rohdaten: `results/warm-cold.json`, `results/recording-feedback.json`.

## 100-Aufnahmen-Abschlusslauf

Auf `d414228` wurden 100/100 Aufnahmen in 944,263 Sekunden lokal transkribiert,
beantwortet und auf der Watch quittiert. Hintergrundwechsel, Watch-Neustart,
iPhone-Neustart und iPhone-Shutdown/Wiederverbindung wurden tatsächlich ausgeführt.
Der finale Audit liest alle 200 Originalaudiodateien, prüft SHA-256, identische
Transkripte/Antworten, stabile Antwortidentität und abgeschlossene persistente Jobs.

Stufenzeiten (Median / p95): STT 5,102 / 5,326 s, Antworterzeugung 2,797 / 3,002 s,
Übertragungsquittung 1,191 / 1,892 s. Der Übertragungs-Ausreißer beträgt 53,254 s
und bleibt im Rohprotokoll enthalten. Die kurze Dauerlauf-Datei unterscheidet sich
von der längeren Warm-/Kaltstart-Datei; deren Stufenwerte sind nicht direkt vergleichbar.

Der 100er-Stapel hat einschließlich Warteschlange 540,479 s Median und 882,282 s
p95 bis TTS-Start. Das ist absichtlich eine Rückstands-/Lifecycle-Messung; diese
Wartezeiten werden nicht als normale Einzelturn-Latenz dargestellt.
Rohdaten: `results/local-100-final.json`, `results/local-100-statistics.json`.

## Abschlussstand und verbleibende Benutzeraktionen

Der frische native Abbruchtest auf `d414228` brach STT nach 4,198 s und Generierung
nach 0,414 s ab. Derselbe Auftrag wurde anschließend ohne erneutes STT abgeschlossen.
Diese Zeiten enthalten Modellladen und Reaktion des Workers; Abbruch ist kooperativ.

Am 06.09.2026 war der Mac entsperrt. Die drei zuvor blockierten Fälle wurden
auf unverändertem Programmstand frisch bestanden:

- **Gesperrtes iPhone:** zehn Sekunden ohne Antwort, anschließend derselbe Auftrag
  beantwortet und quittiert; `results/locked.json`.
- **Wrist Down:** tatsächlicher Simulator-Recorder vor dem Senken gestartet,
  Aufnahme gespeichert, danach identisch auf dem iPhone angenommen. Beide
  Originaldateien geprüft; `results/wrist.json`. Die Umgebungsaufnahme ist kein
  kontrollierter Sprachqualitätstest. Aufnahmefeedback 983 ms, Persistenz 218 ms.
- **Wegwischen:** Local-Voice-Karte visuell identifiziert und entfernt, Prozess
  danach nicht gelistet. Neue Watch-Aufnahme blieb zehn Sekunden ohne Antwort
  erhalten und wurde nach Öffnen beantwortet. Zwei Originaldateien mit SHA-256
  geprüft; `results/force-quit.json`, Bilder unter `screenshots/`. Prozessbeobachtung
  gilt nur für die Stichproben, nicht für sämtliche möglichen Hintergrundaufrufe.

Die fehlgeschlagenen Versuche vom Vortag bleiben als Historie erhalten.

Der externe P2-Review wurde zweimal durch die automatische Freigabeprüfung
abgewiesen. Sie akzeptierte den Nachweis der früheren Freigabe aus dem alten
Verlauf nicht als neue Zustimmung zur Übermittlung. Die offene Frage betrifft
ausschließlich `Store.swift`, `Envelope.swift`, `VoiceModel.swift` und
`LocalProviders.swift` an Claude/Anthropic. Kein P2-Quelltext wurde übermittelt.

Die Implementierung und unabhängig ausführbare Abnahme sind abgeschlossen.
**P2 bleibt bis zur Review-Entscheidung und gegebenenfalls Bearbeitung des Reviews offen.**
