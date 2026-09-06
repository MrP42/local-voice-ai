# P2 Lifecycle-Matrix – abgeschlossen im Simulatorumfang

Abnahme 06.09.2026, finaler Programmstand `a9fbf65`. Abbruchlog auf `71a1d5a` mit
identischem Abbruchcode; Build-Suite und gezielter letzter Setup-Test siehe Rohdaten.
Alle folgenden Nachweise stehen in `results/`. Kein physischer Hardware-Nachweis.

| Fall | Nachweis | Ergebnis / Grenze |
|---|---|---|
| iPhone Vordergrund | warm-cold-after-review.json; local-100-after-review.json | 20 Einzelturns + 100 Aufträge vollständig beantwortet |
| iPhone Hintergrund | background-after-review.json | 10 s aufgeschoben, dieselbe Aufnahme danach verarbeitet |
| iPhone gesperrt | locked-after-review.json | Native Simulator-Sperre, danach gleiche Identität verarbeitet |
| Benutzer beendet iPhone-App | force-quit-after-review.json | Richtige Karte visuell geprüft, nativ weggewischt; Original zunächst auf Watch, beide Dateien nach Wiederaufnahme geprüft |
| Gesenktes Handgelenk | wrist-after-review.json | Echter Recorder vor Always On gestartet; gespeichert und identisch übertragen |
| Verbindung weg / zurück | local-100-after-review.json | Simulator-Shutdown/Wiederverbindung, kein physischer Funknachweis |
| Neustart während Übertragung | local-100-after-review.json; storage-crashes-after-review.json | 100er-Audit + 14 Prozessabbrüche bestanden |
| Doppelte Nachrichten | duplicate-after-review.json | Keine neuen Einträge, stabile Identitäten im vollständigen Verlauf |
| Mikrofon verweigert | denied-after-review.json | Beide Geräte, keine falsche Aufnahmebestätigung |
| Audiowiedergabe unterbrochen | interruption-after-review.json | Notification nach tatsächlichem TTS-Start injiziert; Originale/Verlauf erhalten |
| Inferenz abbrechen | native-cancellation-after-review.json | STT/Generierung tatsächlich abgebrochen; gleicher Auftrag ohne erneutes STT fortgesetzt |
| Beschädigung / Setupfehler | build-and-tests.json | 59 Kerntests, vier iPhone-UI-Tests plus finaler Setup-Retry; gesunde Einträge nutzbar |
| Kein Speicherplatz | build-and-tests.json; storage-crashes-after-review.json | ENOSPC injiziert, Budgets geprüft, kein falsches Ack/keine Original-Löschung |

Sperre/Wrist/Force-Quit wurden nach den Review-Korrekturen frisch bestanden.
Gespeicherte Antwort bedeutet nicht vollständig abgespielte Antwort. Im 100er-Lauf
sind 100 Antworten und 96 tatsächliche TTS-Starts belegt. Historische Fehlversuche
bleiben erhalten; keine ausstehende Benutzeraktion.
