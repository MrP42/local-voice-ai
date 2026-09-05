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
  weiterreichen. Tatsächlicher Simulator-Abbruchnachweis folgt noch.
- [x] Aufnahme, WatchConnectivity und Jobverarbeitung in eigene Komponenten trennen;
  beide nativen Start-/Stop-/Home-Bedienungstests nach Aufteilung bestanden.
- [ ] Prozessabbrüche an allen neuen Commit-/Quittungsgrenzen sowie simuliertes ENOSPC
  abschließend ausführen und Rohdaten ablegen.
- [ ] Speicherbudgets einschließlich Staging/Transfer/Teiltransfer und unbestätigter
  Aufnahmeentwürfe abschließend prüfen; keine automatische Original-Löschung.
- [ ] Sprachqualität: deutsche Antwortfälle, unzulässige Aktionsbehauptungen, digitale
  Stille/Geräusche, Base-/Small-Auswertung und nachvollziehbare Grenzen.
- [ ] Modellverfügbarkeit, Installation und Speicherbedarf in der App sichtbar machen;
  Modellintegrität vor Verwendung prüfen.
- [ ] Neue native Cancellation- und Recovery-UI-Fälle automatisieren; Lifecycle-
  Regressionen mit dem neuen Worker durchführen.
- [ ] Warm-/Kaltstart sowie tatsächliche Ende-zu-Ende-Zeiten messen; Wiederholung mit
  Quittungs-/Digest-Audit auf unverändertem finalem Programmstand.
- [ ] Claude-Review, frischer Abschlussbuild/-tests, Ergebnisbericht und Folgeprioritäten.

Bereits committed: `194bc2b` Speicherisolation, `21fbc8d` Jobs/Abbruch,
`ebab6e2` getrennte Lifecycle-Komponenten. P2 ist noch nicht abgeschlossen.
