# P2 Lifecycle-Matrix – virtuelle Geräte

Programm: `d414228`; Simulatoren wie im Preflight. Kein Hardware-Nachweis.

| Fall | Aktueller Nachweis | Ergebnis / Grenze |
|---|---|---|
| iPhone Vordergrund | zwölf Qualitätsfälle, 20 Warm-/Kaltturns und 100er-Abschlusslauf | Watch → lokales STT/Antwort → Watch-Quittung |
| iPhone Hintergrund | `results/background.json` | 10 s ohne Antwort, anschließend gleiche Aufnahme beantwortet |
| iPhone gesperrt | `results/locked.json` | 06.09. bestanden: zehn Sekunden aufgeschoben, gleiche Aufnahme nach Öffnen beantwortet |
| Benutzer beendet iPhone-App | `results/force-quit.json` + visuell geprüfte Kartenbilder | 06.09. bestanden: tatsächliches Wegwischen, anschließend gleiche Aufnahme verarbeitet |
| Gesenktes Handgelenk | `results/wrist.json` | 06.09. bestanden: Recorder-Audio gespeichert und übernommen |
| Verbindung weg / zurück | frischer 100er-Lauf bestanden | Simulator-Shutdown ist kein physischer Funknachweis |
| App-Neustart bei Übertragung | frischer 100er-Lauf bestanden | zusätzlich 14 echte Prozessabbrüche an Speichergrenzen bestanden |
| Doppelte Nachrichten | `results/duplicate.json` | keine neuen Einträge, stabile Antwort-/Quittungsidentitäten |
| Mikrofon verweigert | `results/denied.json` | iPhone und Watch zeigen Ablehnung; keine falsche Aufnahmebestätigung |
| Wiedergabe unterbrochen | `results/interruption.json` | Handler nach echtem TTS-Start injiziert; gespeicherter Verlauf unverändert |
| Inferenz abbrechen | `results/native-cancellation-final.json` | native CPU-STT/Generierung abbrechbar, gleicher Auftrag fortgesetzt |
| Beschädigter Verlauf | drei frische iPhone-UI-Tests + Kerntests | sichtbar, exportierbar, gesunde Einträge weiterhin zugänglich |
| Kein Speicherplatz | injiziertes ENOSPC + temporäre Budgets | kein falsches Ack, keine automatische Original-Löschung |

Die Host-Sperre blockierte die drei Fensterfälle am 05.09.2026. Nach dem
Entsperren am 06.09. wurden sie frisch ausgeführt und bestanden. Die App-Quellen
sind gegenüber `d414228` unverändert. Externer Claude-Review bleibt offen.
