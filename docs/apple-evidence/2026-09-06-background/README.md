# Watch-Verarbeitung ohne geöffnetes iPhone

Das Update ist auf dem iPhone installiert. Watch-Aufnahmen werden beim Empfang in einem von iOS gewährten Hintergrund-Zeitfenster verarbeitet. Ein sichtbares iPhone-Fenster ist dafür nicht mehr erforderlich. Übrige Aufträge werden zusätzlich für einen späteren BGProcessingTask angemeldet. Diesen Zeitpunkt bestimmt iOS.

## Auf echten Geräten bestätigt

iPhone 15 Pro Max und Apple Watch Ultra 3, 06.09.2026:

- Die iPhone-App blieb im kontrollierten Gerätetest 90 Sekunden im Hintergrund.
- Die echte Watch sendete eine vorbereitete 4,73-Sekunden-Sprachaufnahme über den normalen dauerhaften Transport. Die Mikrofonaufnahme wurde für diesen reproduzierbaren Test umgangen.
- Apple SpeechTranscriber transkribierte lokal in **0,57 Sekunden**.
- Apple Foundation Models erzeugte die lokale Antwort in **2,63 Sekunden**.
- Die App protokollierte ausdrücklich den Hintergrundzustand beim Abschluss.
- Die Watch bestätigte den Empfang der gespeicherten Antwort. Vom Sichern der Watch-Testaufnahme bis zum Antwortempfang vergingen **4,74 Sekunden**. Ein automatischer TTS-Start wurde in dieser Probe nicht erfasst.
- Das iPhone wurde dafür nicht wieder geöffnet.

77 Core-Tests bestanden. Ein Simulator-Bedienungstest bestätigt zusätzlich mit kontrollierten Testanbietern, dass das Ergebnis vor der Rückkehr in den Vordergrund entsteht. Ein echter iPhone-Bedienungstest bestätigt das 90-Sekunden-Hintergrundfenster. Frischer signierter Geräte-Build und Installation erfolgreich.

## Verhalten bei Unterbrechungen

Bei abgelaufener Laufzeit wird die lokale Inferenz abgebrochen; bestätigte Aufnahmen und bereits fertige Transkripte bleiben erhalten. Systemunterbrechungen verbrauchen nicht das Fehlerbudget für tatsächlich fehlgeschlagene Anbieteraufrufe. Antworten werden vor Ende des Laufzeitfensters an die dauerhafte WatchConnectivity-Zustellung übergeben; doppelte Zustellungen bleiben idempotent.

Aufnahmen und Modelle bleiben verschlüsselt, sind aber nach dem ersten Entsperren seit dem Neustart auch bei gesperrtem Bildschirm zugänglich. Auch am gesperrten iPhone wurde die Testaufnahme lokal verarbeitet und von der Watch quittiert: Sperre vor und nach dem Versuch per Geräteabfrage bestätigt, Abschluss im Hintergrund protokolliert. Die Zustellung benötigte in dieser Probe ungefähr 85 Sekunden; anschließend STT 3,46 Sekunden und Antworterzeugung 6,11 Sekunden. Das bestätigt aufgeschobene Verarbeitung ohne erneutes Öffnen, keine garantierte Sofortantwort.

iOS kann zusätzliche Laufzeit verweigern oder geplante Aufträge verzögern. Ein BGProcessingTask ist registriert und wird bei Restarbeit angefordert; ein natürlich vom System ausgelöster geplanter Lauf ist hier noch nicht nachgewiesen. Nach ausdrücklichem Wegwischen der iPhone-App lässt sich ein erneuter Hintergrundstart nicht garantieren. Dann bleibt die Aufnahme gespeichert. Kein unbegrenzter Echtzeitbetrieb und keine neue Akkumessung zugesagt.

Diese Änderung betrifft kurze Watch-/Sprachnotizen. Lange Medienimporte und Modell-Downloads haben weiterhin ihren separat dokumentierten Vordergrundablauf. Interaktive Hintergrundaufnahme direkt aus einer Watch-Komplikation bleibt ein eigener offener Ausbau.

## Apple-Quellen

- [Watch-Nachrichten können die iPhone-App im Hintergrund wecken](https://developer.apple.com/videos/play/wwdc2021/10003/)
- [Begrenzte zusätzliche Hintergrundlaufzeit](https://developer.apple.com/documentation/uikit/extending-your-app-s-background-execution-time)
- [Geplante Hintergrundverarbeitung](https://developer.apple.com/documentation/backgroundtasks/bgprocessingtaskrequest)
- [iOS-Grenzen, insbesondere nach erzwungenem Beenden](https://developer.apple.com/forums/thread/685525)
