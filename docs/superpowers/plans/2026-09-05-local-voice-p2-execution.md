# P2 – laufende Umsetzung nach Benutzerfreigabe

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
- [ ] Neue native Cancellation- und Recovery-UI-Fälle automatisieren; Lifecycle-
  Regressionen mit dem neuen Worker durchführen.
- [x] Warm-/Kaltstart sowie tatsächliche Ende-zu-Ende-Zeiten messen; Wiederholung mit
  Quittungs-/Digest-Audit auf unverändertem finalem Programmstand.
- [ ] Claude-Review, frischer Abschlussbuild/-tests, Ergebnisbericht und Folgeprioritäten.

Erste Commits: `194bc2b` Speicherisolation, `21fbc8d` Jobs/Abbruch,
`ebab6e2` getrennte Lifecycle-Komponenten. P2 ist noch nicht abgeschlossen.

Stand `d414228`: 54 Kerntests, 14 Abbruchgrenzen, drei iPhone-UI-Tests und
ein Watch-UI-Test bestehen. Zwölf Qualitätsfälle und 20 Warm-/Kaltturns beendet;
Antwortmodell macht weiterhin den dokumentierten Rechenfehler. 100er-Abschlusslauf
und abschließender Dateiaudit sind mit 100/100 Antworten und 200 Originaldateien bestanden. Frische Lock-/Wrist-/Wegwischfälle benötigen
den aktuell gesperrten Mac entsperrt. Externer Claude-Review wartet auf die konkret
angefragte neue Übermittlungsfreigabe; bisher keine P2-Übermittlung.
Siehe `docs/apple-evidence/p2/README.md` für Werte und Grenzen.
