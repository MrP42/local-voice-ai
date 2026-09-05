# P2 Lifecycle-Matrix – virtuelle Geräte

Programm: `d414228`; Simulatoren wie im Preflight. Kein Hardware-Nachweis.

| Fall | Aktueller Nachweis | Ergebnis / Grenze |
|---|---|---|
| iPhone Vordergrund | zwölf Qualitätsfälle, 20 Warm-/Kaltturns und 100er-Abschlusslauf | Watch → lokales STT/Antwort → Watch-Quittung |
| iPhone Hintergrund | `results/background.json` | 10 s ohne Antwort, anschließend gleiche Aufnahme beantwortet |
| iPhone gesperrt | frischer UI-Lauf durch gesperrten Mac verhindert | P1-Nachweis vorhanden, keine frische P2-Bestätigung |
| Benutzer beendet iPhone-App | frischer Wegwischtest benötigt entsperrten Mac | P1-Nachweis vorhanden, Prozess-Kill ist kein Ersatz |
| Gesenktes Handgelenk | frischer Simulator-Fenstertest benötigt entsperrten Mac | P1-Nachweis vorhanden |
| Verbindung weg / zurück | frischer 100er-Lauf bestanden | Simulator-Shutdown ist kein physischer Funknachweis |
| App-Neustart bei Übertragung | frischer 100er-Lauf bestanden | zusätzlich 14 echte Prozessabbrüche an Speichergrenzen bestanden |
| Doppelte Nachrichten | `results/duplicate.json` | keine neuen Einträge, stabile Antwort-/Quittungsidentitäten |
| Mikrofon verweigert | `results/denied.json` | iPhone und Watch zeigen Ablehnung; keine falsche Aufnahmebestätigung |
| Wiedergabe unterbrochen | `results/interruption.json` | Handler nach echtem TTS-Start injiziert; gespeicherter Verlauf unverändert |
| Inferenz abbrechen | `results/native-cancellation-final.json` | native CPU-STT/Generierung abbrechbar, gleicher Auftrag fortgesetzt |
| Beschädigter Verlauf | drei frische iPhone-UI-Tests + Kerntests | sichtbar, exportierbar, gesunde Einträge weiterhin zugänglich |
| Kein Speicherplatz | injiziertes ENOSPC + temporäre Budgets | kein falsches Ack, keine automatische Original-Löschung |

Der Mac meldete beim neuen Sperrtasten-Test `IOConsoleLocked = Yes`.
XCTest konnte deshalb die Simulator-App nicht aktivieren. Diese Umgebungsblockade
wird nicht als bestandener Lifecycle-Fall ausgegeben. Eine Rückfrage zum Entsperren
ist offen; unabhängige Prüfungen wurden fortgesetzt.
