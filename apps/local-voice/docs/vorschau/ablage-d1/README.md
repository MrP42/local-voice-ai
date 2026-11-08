# Vorschau D1: Seite „Ablage“ (Epic #395, ADR-0029)

Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel`.
Grundlage: Spezifikation `docs/superpowers/specs/2026-11-08-dokumentenablage.md` §2–§4.
32 Bilder, je hell und dunkel. Maß-Belege in `belege.json` (Playwright): `scrollX` und Spaltenüberlauf
überall 0, `unter44` überall leer (Knöpfe, Zeilen, Reiter, Schalterzeilen, Leiste).

| Bild | Breiten | Zeigt |
|---|---|---|
| `01-liste` | 2560, 1440, 768, 390 | Ordnerbaum (eigene Ordner, verknüpfte Ordner nur lesen, Papierkorb), Dateiliste mit Sync-Zustand, KI-Freigabe und Datenklasse; rechts Vorschau, „Für die KI“ (geerbt, nur Auszüge), Textfassung, Fassungen, Details. ≤ 1023 px Details als Blatt |
| `02-hochladen` | 1440, 390 | Drag & Drop mit Zielangabe; Liste mit Prüfung, Namenskollision → neue Fassung oder beide behalten (nie überschreiben), Textfassung im Hintergrund. 390: nur Ablagefläche (Hochladeliste als Blatt folgt im Bau) |
| `03-ordner` | 1440, 390 | Ordner-Einstellungen: KI-Freigabe (vererbt, Unterordner „aus“ gewinnt, gerätelokal), Datenklasse, Nur dieses Gerät, KI-Entwürfe erlaubt |
| `04-suche` | 1440, 390 | Lokale Suche mit Auszügen, Fundstelle (Seite/Abschnitt), Bereich Ganze Ablage / Ordner / verknüpfte Ordner; Hinweis auf den Assistenten |
| `05-konflikt` | 1440, 390 | Gleichzeitige Änderung auf zwei Geräten: beide Fassungen erhalten, Wahl; KI nutzt bis dahin die zuletzt geprüfte Fassung |
| `06-papierkorb` | 1440, 390 | 30 Tage, geräteübergreifend, Wiederherstellen; KI nutzt gelöschte Dateien nicht |
| `07-leer` | 1440, 390 | Leerer Ordner, ein Satz, keine Vorschlagsblasen |

Nicht in D1: Index und Suche (D2, hier nur gezeigt), KI-Zugriff (D3), Sync (D4). Die Leiste der App
(Start/Chat-Umbau aus dem Assistent-Thread) ist hier nur angedeutet.
