# P2 – abgeschlossen im vereinbarten Simulatorumfang

Der Benutzer hat nach Abschluss von P1 ausdrücklich P2 beauftragt. Der bestehende
isolierte Branch `codex/apple-p0-p1` bleibt erhalten. Ausgang für P2 ist `cfd180a`.
Virtuelle iPhone-/Watch-Geräte bleiben der vereinbarte Testumfang. Physische
Akku-, Funk- und Data-Protection-Prüfungen werden weiterhin nicht fingiert.

- [x] Beschädigte Einträge sichtbar isolieren, gesunden Verlauf weiter verarbeiten,
  Identitäten additiv sichern, vollständige verwaiste Transaktionen wiederherstellen.
- [x] Persistente Auftragsphasen, maximal drei automatische Versuche, expliziter
  Benutzer-Retry, erhaltener STT-Zwischenstand und abbrechbare Workersteuerung.
- [x] Native atomare Abbruchsignale für Whisper/llama ergänzen; Apple-Cancellation
  weiterreichen. Tatsächlicher CPU-Simulator-Abbruch nachgewiesen; Apple-Provider hier nicht verfügbar.
- [x] Aufnahme, WatchConnectivity und Jobverarbeitung in eigene Komponenten trennen;
  beide nativen Start-/Stop-/Home-Bedienungstests nach Aufteilung bestanden.
- [x] Prozessabbrüche an allen neuen Commit-/Quittungsgrenzen sowie simuliertes ENOSPC
  abschließend ausführen und Rohdaten ablegen.
- [x] Speicherbudgets einschließlich Staging/Transfer/Teiltransfer und unbestätigter
  Aufnahmeentwürfe abschließend prüfen; keine automatische Original-Löschung.
- [x] Sprachqualität: deutsche Antwortfälle, unzulässige Aktionsbehauptungen, digitale
  Stille/Geräusche, Base-/Small-Auswertung und nachvollziehbare Grenzen.
- [x] Modellverfügbarkeit, Installation und Speicherbedarf in der App sichtbar machen;
  Modellintegrität vor Verwendung prüfen.
- [x] Neue native Cancellation- und Recovery-UI-Fälle automatisieren; Lifecycle-
  Regressionen mit dem neuen Worker durchführen.
- [x] Warm-/Kaltstart sowie tatsächliche Ende-zu-Ende-Zeiten messen; Wiederholung mit
  Quittungs-/Digest-Audit auf unverändertem finalem Programmstand.
- [x] Frischer Abschlussbuild/-tests, Ergebnisbericht und Folgeprioritäten.
- [x] Tatsächlicher Claude-Review nach ausdrücklicher Freigabe, Befunde geprüft und korrigiert.
- [x] Abschließende Mess-/Lifecycle-Wiederholung nach den Review-Korrekturen.

Abschluss 06.09.2026: finaler Programmstand `a9fbf65`. 59 Kerntests, 14 tatsächliche
Prozessabbrüche, vier iPhone-UI-Tests, ein Watch-UI-Test und zusätzlicher finaler
Setup-Retry bestanden. 20 Warm-/Kaltturns, 100/100 Antworten, alle 200 Originaldateien
geprüft; sämtliche Simulator-Lifecyclefälle nach Review erneut bestanden.
Tatsächlicher Claude-Review abgeschlossen und alle Befunde bewertet; bestätigte
Fehler in `4c8ac04`, `71a1d5a`, `a9fbf65` behoben. Keine offene Benutzeraktion.

P2 ist abgeschlossen als Simulator-Stabilisierung, nicht als Nachweis erreichter
Echtzeit, verlässlicher Modellantworten oder physischen Energieverbrauchs.
Siehe `docs/apple-evidence/p2/README.md`, `lifecycle-matrix.md` und `next-priorities.md`.
Ältere Rohdaten/fehlgeschlagene Versuche bleiben zur Nachvollziehbarkeit erhalten.
