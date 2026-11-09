# Reader: Kapitel-Sprung, Wiedergabe mittig, Stimmen nach Buchsprache

Vorschau für die Codex-Abnahme (Patrick 01.11.2026). 1920×1050, englisches Buch,
Phonon mit deutschen und englischen Stimmen; hell und dunkel.

| Bild | Inhalt |
|---|---|
| `1-leiste-mittig-*.png` | Wiedergabe mittig, Kapitel links (anklickbar), Werkzeuge rechts |
| `2-kapitelliste-*.png` | Kapitelliste mit Seitenzahlen, aktuelles Kapitel markiert |
| `3-stimmen-passend-*.png` | Stimmenauswahl ohne Hover-Blase, Filter „Passend (Englisch)“ vorgewählt |
| `3b-gewaehlte-stimme-nah-*.png` | Gewählte Stimme ungekürzt mit Sprache („Phonon · Standardstimme · Englisch“), Knopf in der Leiste mit Kürzel „EN“ |
| `4-stimmen-alle-*.png` | Filter „Alle“ |
| `5-lesevollbild-im-fenster-1200-*.png` | Lese-Vollbild füllt das Fenster (1200×800) unter der Titelleiste; Fensterknöpfe (Minimieren/Maximieren/Schließen) bleiben sichtbar, das Fenster ist nicht maximiert |
| `6-schmal-390-*.png` | 390 px: Leiste einzeilig wie bisher, Kapitel im Inhalt links |

## Nachweis Fensterzustand (Codex-Must-fix 2)

`tests/readaloud-reader.spec.ts` › „Vollbild-Knopf füllt das Fenster, ohne es zu maximieren; F11 für echtes Vollbild“:

- Lese-Vollbild: `document.fullscreenElement === null`, kein Aufruf von
  `plugin:window|maximize`, `toggle_maximize`, `set_fullscreen`, `set_size`;
  Lesespalte ab y = 48 px (unter der Titelleiste), Fensterknöpfe sichtbar,
  Knopf „Maximieren“ (nicht „Wiederherstellen“).
- F11: `document.fullscreenElement` ist die Lesespalte (Fullscreen-API des
  WebViews); Knopf beendet es; auch dabei kein Fensterbefehl.
