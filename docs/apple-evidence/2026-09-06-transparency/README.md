# App-Update vom 06.09.2026

- iPhone: aktualisierte App installiert und Start durch Gerätewerkzeug bestätigt.
- Menü heißt jetzt „Transkripte“.
- Modellkarten bieten echte HTTPS-Downloads; Installation erst nach Größen- und SHA-256-Prüfung. Bei Fehlern bleibt der bisherige Modellbestand erhalten. Die App muss während des Downloads geöffnet bleiben; nach Abbruch erneut starten. Kein Prozentfortschritt oder dauerhafter Hintergrunddownload zugesagt.
- Antworten formatieren Markdown-Überschriften, Hervorhebungen und Listen. Modell und Uhrzeit neben Wiedergabe/Stopp; Details per Tipp. Verlaufsvorschau zeigt das Antwortmodell.
- Tatsächlich benutzte STT-/Antwortanbieter und Auswertungsmodelle werden mit Abschlusszeit und Dauer gespeichert. Feste Regeln werden als solche unterschieden. Sprachausgabe protokolliert Systemstimme und Startlatenz. Metadaten werden mit Antworten zur Watch übertragen. Alte Einträge bleiben ausdrücklich „nicht dokumentiert“.
- Apple veröffentlicht keine genaue interne Version seiner Systemmodelle.

## Prüfung

73 Core-Tests erfolgreich, einschließlich Neustart, Watch-Übertragung und tatsächlich benutztem Provider versus fester Regel. Zwei gezielte iPhone-Bedienungstests im Dark Mode erfolgreich: Modell-Download startet, Markdown wird formatiert und Modelldetails öffnen. Helle Antwortansicht ebenfalls erfolgreich getestet und beide Ansichten visuell geprüft. Der erste Download-Test scheiterte nur an der Abfrage einer nach dem Scrollen nicht mehr sichtbaren Zeile; korrigierter Test erfolgreich.

Frischer Simulator-Build sowie signierter iPhone-Geräte-Build mit eingebetteter Watch-App erfolgreich. Download-Bedienung getestet; kein neuer vollständiger Großmodell-Download auf dem echten iPhone in diesem Lauf nachgewiesen.

## Watch

Entwicklermodus ist jetzt als aktiviert bestätigt. Gerätespezifischer signierter Build erfolgreich. Installation nach vorübergehenden CoreDevice-Verbindungsabbrüchen um 13:20 Uhr erfolgreich bestätigt; App-Start um 13:21 Uhr ebenfalls bestätigt. iPhone-Installation und Start ebenfalls erfolgreich. Drei Komplikations-Schnellzugriffe in vier Formen vorhanden; Tippen auf „Verlauf“ und Aufnahmestart/-stopp im Watch-Simulator erfolgreich geprüft. Hintergrund-Aufnahme und interaktive Pause/Stopp-Komplikationen noch nicht implementiert; gesonderter Plan vorhanden. Keine neuen realen Akku- oder Lifecycle-Messungen.
