# Vorschau D1: Seite „Ablage“ (Epic #395, ADR-0029)

Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel`.
Grundlage: Spezifikation `docs/superpowers/specs/2026-11-08-dokumentenablage.md` §2–§4.
32 Bilder, je hell und dunkel, alle aus EINEM Lauf über dieselbe `vorschau.html` (`belege.json` trägt deren SHA-256).
Maß-Belege je Bild (Playwright): `scrollX` und Spaltenüberlauf 0; `unter44` misst jede sichtbare Bedienfläche
(Links, Knöpfe, Eingabefeld, Auswahlliste, Schalter, Reiter, Optionsfelder) in Breite UND Höhe und ist überall leer.
Bei Dialog/Blatt zusätzlich: Fokus beim Öffnen, Fokus bleibt bei Tab im Dialog, Escape schließt und gibt den Fokus an
den Auslöser zurück. Suche: Pfeiltaste wechselt den Reiter.

Bedienung: Ordner- und Dateizeilen sind Links, Suche ist ein echtes Eingabefeld, Datenklasse eine echte Auswahlliste,
Schalter `button role=switch` (Fläche 52×44), KI-Freigabe als Optionsgruppe (Pfeiltasten), Suchreiter `role=tab`
(Pfeiltasten, Pos1/Ende). Dialog und Blatt sind `aria-modal`, halten den Fokus und schließen mit Escape.
Kontrast: Grün im Dunkelmodus #6ccb6c (5,8:1 auf Chip-Grund).


| Bild | Breiten | Zeigt |
|---|---|---|
| `01-liste` | 2560, 1440, 768, 390 | Ordnerbaum (eigene Ordner, verknüpfte Ordner nur lesen, Papierkorb), Dateiliste mit Sync-Zustand, KI-Freigabe und Datenklasse; rechts Vorschau, „Für die KI“ (geerbt, nur Auszüge), Textfassung, Fassungen, Details. ≤ 1023 px Details als Blatt (Dialog) |
| `02-hochladen` | 1440, 390 | 1440: Drag & Drop mit Zielangabe und Hochladeliste. 390: Hochladeliste als Blatt mit Prüfung, Namenskollision → „Als neue Fassung“ oder „Beide behalten“ (nie überschreiben), Textfassung im Hintergrund |
| `03-ordner` | 1440, 390 | Ordner-Einstellungen: KI-Freigabe An/Aus/Gesperrt (vererbt, „Gesperrt“ gewinnt auch in Projekten, gerätelokal), Datenklasse, Nur dieses Gerät, KI-Entwürfe erlaubt |
| `04-suche` | 1440, 390 | Lokale Suche mit Auszügen und Fundstelle (Seite, Absatz, Teil); Bereich Ganze Ablage / Dieser Ordner / Verknüpft auf allen Breiten |
| `05-konflikt` | 1440, 390 | Gleichzeitige Änderung: beide Fassungen und die gemeinsame Vorgängerfassung; Wahl auch mobil im Blatt. KI nutzt bis zur Wahl die gemeinsame Vorgängerfassung, ohne gemeinsame Fassung die Datei gar nicht |
| `06-papierkorb` | 1440, 390 | 30 Tage, geräteübergreifend, Wiederherstellen; voller Name als Titel und im Knopfnamen; KI nutzt gelöschte Dateien nicht |
| `07-leer` | 1440, 390 | Leerer Ordner, ein Satz, keine Vorschlagsblasen |

Nicht in D1: Index und Suche (D2, hier nur gezeigt), KI-Zugriff (D3), Sync (D4). Die Leiste der App
(Start/Chat-Umbau aus dem Assistent-Thread) ist hier nur angedeutet.
