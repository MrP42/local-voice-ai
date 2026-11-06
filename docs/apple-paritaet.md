# Funktionsparität Windows → Mac → iOS

Die Funktionsentwicklung läuft überwiegend in der Windows-App. Bei jedem großen
Release (Minor-Sprung, z. B. `app-v0.23.0`) wird diese Matrix aktualisiert und
Mac sowie iOS ziehen nach. Lokale Zwischenstände (0.22.1, 0.22.2 …) brauchen
keinen Abgleich.

- **Mac** ist derselbe Tauri-Build für macOS. Parität entsteht beim Bau; offen
  bleiben nur plattformspezifische Teile (Schlüsselbund, Signatur, Mikrofonrechte).
- **iOS/watchOS** ist nativ (SwiftUI, `apps/apple`) und wird portiert.

Status: **gleich** · **angepasst** (bewusst mobil zugeschnitten) · **offen** ·
**nicht geplant** (mit Grund).

| Bereich | Funktion | Windows | Mac | iOS | Issue / Bemerkung |
|---|---|---|---|---|---|
| Diktieren | Push-to-talk, lokales STT | gleich | gleich | angepasst | iPhone/Watch-PTT bis 30 s |
| Sprachgespräch | Voicebot mit Antwort | gleich | gleich | angepasst | kurze lokale Antwort, Watch liest vor |
| Besprechungen | Medienimport, lokales STT | gleich | gleich | gleich | `MeetingArchive` |
| Besprechungen | Protokoll (Desktop-Schema, belegt) | gleich | gleich | gleich | `MeetingMinutes` |
| Besprechungen | Aufnahme in der App mit Live-Transkript | gleich | gleich | offen | iOS-Plan P3 |
| Besprechungen | Live-Stand, To-dos, offene Fragen während der Besprechung | offen | offen | offen | iOS-Plan P1/P3; Desktop folgt aus demselben Schema |
| Besprechungen | Einwilligungshinweis vor Aufnahme | gleich | gleich | offen | iOS-Plan P1/P3/P5 |
| Besprechungen | Endlauf mit großem Modell | gleich | gleich | offen | iOS-Plan P7 |
| Besprechungen | Sprecher, Hörprobe, Redeanteile | gleich | gleich | offen | lva#242; iOS danach |
| Besprechungen | Chat zur Besprechung, Suche | gleich | gleich | offen | nach P3 |
| Übernahme | Teilen-Menü, „Öffnen in“, Sprachmemos | – | – | offen | iOS-Plan P4 |
| Übernahme | Watch-Besprechungsaufnahme | – | – | offen | iOS-Plan P5 |
| Übernahme | Kurzbefehle, Aktionstaste | – | – | offen | iOS-Plan P6 |
| Vorlesen | Text vorlesen, Stimmen | gleich | gleich | angepasst | lva#210 |
| Bibliothek | Bücher, Hörbücher, Mitlesen | gleich | gleich | angepasst | lva#210 (P0–P3 gemergt) |
| Bibliothek | PDF/EPUB-Import mit Formatierung | gleich | gleich | offen | lva#149, lva#210 |
| Bibliothek | Sync zwischen Geräten | gleich | gleich | angepasst | lva#210 P5/P6 |
| Buch-KI | Zusammenfassungen, Begriffe, Notizen, Chat | gleich | gleich | offen | lva#181 |
| Übersetzung | Buchübersetzung | gleich | gleich | nicht geplant | Rechenlast; Ergebnis per Sync |
| Stimmen | Stimmen klonen | gleich | angepasst | nicht geplant | nur Desktop |

## Ablauf je großem Release

1. Vor dem Tag `app-v0.X.0` diese Matrix gegen die Änderungen seit dem letzten
   großen Release abgleichen (`git log app-v0.(X-1).0..HEAD`).
2. Für jede neue Windows-Funktion eine Zeile ergänzen; iOS-Lücken als Issue mit
   Label `apple` anlegen oder begründet als „nicht geplant“ markieren.
3. Mac-Build und Apple-Build mit derselben Marketingversion `0.X.0` (siehe
   `apps/apple/RELEASING.md`).
