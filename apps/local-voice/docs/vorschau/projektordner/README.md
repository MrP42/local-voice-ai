# Vorschau: Buch-Ordner im Vorlesen und appweite Projekte (Runde 2)

Spec: `docs/superpowers/specs/2026-11-07-buch-ordner-und-projekte.md`.
Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel&ebene=ordner|liste|reader`.

Alle Bilder liegen in hell und dunkel vor (`-hell`, `-dunkel`). 48 Bilder.

| Bild | Breiten | Zeigt |
|---|---|---|
| `01-buchordner` | 2560, 1440, 768; 390 als `-ordner`, `-liste`, `-reader` | Vorlesen nach Aufnahmen-Muster. KI-Dokumente nennen ihre Fassung |
| `02-alle` | 1440, 768; 390 `-reader` | Ordner „Alle“, ausgewählter loser Text; Reader, Kapitel und Player zeigen diesen Text |
| `03-projekt` | 2560, 1440, 768, 390 | Projektseite, Schalter „Als Kontext“ je Mitgliedschaft; 4 eingeschaltete Einträge = 16 300 Token |
| `04-ablage` | 1440 | Buchdetail „Dateien im Buch-Ordner“ |
| `05-migration` | 1440, 390 | Dialog „Bibliothek neu ordnen“ |
| `06-leer` | 1440, 390 | Leere Bibliothek, keine Projekte |
| `06-projektleer` | 1440 | Leeres Projekt |
| `06-fehlt` | 1440 | Fehlende Quelle: nichts gelöscht, Suchen / aus Sicherung / aus Buch entfernen |
| `06-ohneziel` | 1440, 390 | Kein aktives Projekt: Ziel sichtbar „kein Projekt“, Zuordnung zur Fassung bleibt |
| `06-migfehler` | 1440, 390 | Migration angehalten (alter und neuer Ordner vorhanden), nichts überschrieben |

## Maß-Belege (`belege.json`)

Je Bild per Playwright `getBoundingClientRect` gemessen:

- `scrollX`: waagrechter Überlauf, überall 0.
- `unter44`: sichtbare Bedienelemente (Knöpfe, Zeilen, Reiter, Schalter, Ausgabeziel) unter 44 × 44 px. Die Liste ist überall leer.
- `kopf`: Ausgabeziel in der Kopfzeile, `schalter`: Kontext-Schalter, `zurueck`: Zurück-Knöpfe mobil, jeweils mit Breite und Höhe.
- `kontrastGedaempft`: Kontrast des gedämpften Texts zur Fläche, mindestens 6,19 : 1 (WCAG AA verlangt 4,5 : 1).

Korrekturen aus Codex Runde 1:

- Bild „Alle“: Der Reader zeigt jetzt den ausgewählten Text.
- Der Kontextzähler nennt jetzt 4 Einträge.
- Das Ausgabeziel ist jetzt ein Knopf mit 44 px.
- Der Kontext-Schalter ist eine 44 px hohe Schaltfläche.
- Kapitelzeilen sind jetzt 44 px hoch.
- Die eingeklappte Leiste bei 768 px ist 64 px breit.
