# Claude-Review – Befunde und Entscheidungen

Der Benutzer hat die Übermittlung der vier konkreten Swift-Dateien ausdrücklich
freigegeben und Claude Code angemeldet. CLI 2.1.259, toolfreier Safe-Mode-Review;
keine weiteren Quelldateien wurden übermittelt. Der erste Review ist unverändert
in `claude-review-initial.md` abgelegt. Es handelt sich um einen Review der vier
Dateien, nicht um eine vollständige Prüfung aller Abhängigkeiten.

| Nr. | Bewertung und Bearbeitung |
|---|---|
| 1–2 | Aufnahme blieb zuvor auf Platte, war aber ohne normalen Wiederherstellungspfad. Automatische Wiederherstellung ergänzt; stabile ID aus dem Draftnamen verhindert Duplikate nach einem Abbruch zwischen Annahme und Löschen. AVAudioFile muss einen lesbaren Container erkennen, bevor die App bestätigt. Drei neue Kernfälle prüfen Wiederholung, Neustart, vollen Speicher und leere Dateien. |
| 3 | Sender validiert jetzt Antwort und Transkript vor dem Speichern. Leere/Whitespace-Antworten, Antworten über 500 Zeichen und Transkripte über 16000 Zeichen können keinen nicht zustellbaren Abschluss erzeugen. Die CPU-Implementierung war bereits auf 500 Zeichen begrenzt; diese Datei lag Claude nicht vor. |
| 4 | Nicht als bewiesener P0-Defekt übernommen. AVFoundation aktiviert Audiositzungen automatisch; Recorder und Watch-TTS sind im Simulator tatsächlich gelaufen. Asynchrone Watch-Aktivierung für Long-Form-Audio pauschal einzusetzen könnte Routingdialoge einführen. Physische Ausgaberouten sind weiterhin nicht durch Simulator-Callbacks belegt. |
| 5 | Apple-Transkription erhält einen 60-Sekunden-Watchdog, der den Ergebnistask cancelt und cancelAndFinishNow aufruft. Kein TaskGroup-Rennen, das beim Verlassen auf den hängenden Ergebnistask warten würde. Kompiliert; native Apple-Inferenz ist auf diesem Intel-Simulator nicht verfügbar. CPU-Inferenz besitzt bereits eine eigene 90-Sekunden-Rechengrenze. |
| 6 | Nur redundante Transferkopien werden nach Abschluss bzw. nach Aktivierung aufgeräumt; noch ausstehende Transfers werden ausgenommen. Originalaudio bleibt erhalten. Kernprüfung schützt aktive Kopien, Originale und nicht passende Dateinamen. |
| 7 | Eindeutige Reihenfolge: neue, noch nicht angenommene Aufnahmen zuerst, innerhalb der Gruppe älteste zuerst und stabile ID als Tie-Breaker. Der interaktive Slot wird seriell bedient; Dateifallback erfolgt bei tatsächlichem Fehler oder Unerreichbarkeit. Siehe die unten dokumentierte Laufzeitkorrektur. |
| 8 | Der vorgeschlagene compactMap-/Nullgrößen-Fallback wird nicht übernommen: Er würde beschädigte bestätigte Einträge unsichtbar machen und Speicher unterzählen. Fail-closed bleibt explizite P1-Grenze; sichere Quarantäne und Wiederherstellungsoberfläche gehören in P2. Daten werden nicht gelöscht. |
| 9 | Speicherbelegung wird über Dateigrößen ermittelt, ohne jedes Audio komplett einzulesen. Fehlende Größen werden nicht als null gezählt. Der Bytevergleich bei Duplikaten bleibt zum Erkennen beschädigter gespeicherter Audiodaten erhalten. |
| 10 | Unbrauchbare Chunk-Quittungen erhalten denselben dauerhaften Dateitransfer-Fallback wie Transportfehler. |
| 11 | Persistierte Antwort-ID wird direkt beim Envelope-Aufbau übergeben. receiptId und bestätigte messageId bleiben unterschiedliche, absichtlich getrennte Identitäten. |
| 12 | Kein belegter Fehler ohne echte SpeechTranscriber-Segmentdaten. Bestehende originale Segmentzeichen werden nicht blind durch zusätzliche Leerzeichen verändert. Mit unterstütztem Apple-Provider in P2 prüfen. |
| 13 | Indexgrenze vor Chunk-Zugriff defensiv geprüft. |

Zusätzlich unabhängig gefunden und behoben: verspätete Mikrofonfreigabe nach
Szenenwechsel. Sechs Kernfälle plus frische iPhone-/Watch-UI-Prüfung.
Aktueller Kernstand: 31 Tests bestanden. Der zweite Review liegt unverändert in
`claude-review-followup.md`: keine weiteren bewiesenen P0/P1-Blocker in den vier
Dateien. Das ist keine Abnahme der nicht übermittelten Abhängigkeiten.
Zusätzliche Hinweise daraus: sofortiges Wiederholen nach fehlgeschlagener
Quittungsspeicherung verhindert; Apple-Generierung auf 128 Tokens begrenzt und
Cancellation nach 60 Sekunden angefordert; unlesbare Drafts erhalten ein sichtbares
Signal. Die Apple-API-Cancellation bleibt kooperativ und ist im Intel-Simulator
nicht mit nativer Modellinferenz prüfbar. Verwaiste `.partial-*`-Verzeichnisse und
sichere Quarantäne bleiben P2-Wartungsaufgaben.

Der anschließende Dauerlauf zeigte eine reale Leistungsregression durch den
zusätzlichen Versandpfad. In `3498c44` werden beim belegten interaktiven Slot keine
zusätzlichen Dateitransfers mehr gestartet. Bei tatsächlichem Fehler bzw.
Unerreichbarkeit bleibt der Dateifallback für die wartenden Aufnahmen erhalten.
Der Testcontroller wurde gezielt unterbrochen und mit denselben 100 IDs fortgesetzt;
der Bericht kennzeichnet den Versionswechsel. Ein statischer Review ersetzt
solche Laufzeitprüfungen nicht.

Quellen zur Audioaktivierung:
- [Apple: Activating an Audio Session](https://developer.apple.com/library/archive/documentation/Audio/Conceptual/AudioSessionProgrammingGuide/ConfiguringanAudioSession/ConfiguringanAudioSession.html)
- [Apple: Watch async activation](https://developer.apple.com/documentation/avfaudio/avaudiosession/activate(options:completionhandler:))

Die abschließende UI-Prüfung mit dem gewachsenen Verlauf fand zusätzlich eine
Darstellungsregression: Eager-Rendering machte Accessibility-Aktionen so langsam,
dass das 30-Sekunden-Aufnahmelimit vor dem Stop-Tap ablief. Beide ScrollViews
verwenden nun LazyVStack; der unveränderte iPhone-Test besteht damit. Kein
Verlauf wurde zur Beschleunigung entfernt. Diese SwiftUI-Datei war nicht Teil
des externen Reviews.
