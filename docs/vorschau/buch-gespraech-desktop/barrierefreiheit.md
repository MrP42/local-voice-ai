# Buch-Gespräch Desktop: Barrierefreiheit (lva#309)

Gemessen mit Playwright (`apps/local-voice/tests/booktalk.spec.ts`, Test „Kontraste der Spaltenfarben“):
`getComputedStyle`, tatsächlich dahinterliegende Fläche (halbtransparente Füllungen werden über die
Vorfahren gemischt), WCAG-2.x-Formel. Text braucht 4,5:1, der Rand von Bedienelementen 3:1.

## Bereichsknopf

- Beschriftung nennt die konkrete Grenze: „Bis S. 4 geschützt“ (Seite der letzten Seitenmarke vor der
  Lesegrenze), ohne Seitenmarken „Bis Kap. 2 geschützt“ (Kapitelindex + 1), bei Grenze 0 „Bis zum Anfang
  geschützt“, nur ohne Marken und ohne Inhaltsverzeichnis „Bis hier geschützt“.
- `aria-label`: „Bereich: bis Seite 4 geschützt. Ändern“, „… bis Kapitel 2 …“, „… bis zum Anfang …“.
- Ansage nach dem Zurückwechseln („Bereich: nur bis Seite 4 geschützt“) und die Rückfrage („Bisher
  geschützt bis Seite 4. Das kann die Handlung vorwegnehmen …“) nennen dieselbe Grenze.

## Bedienziele

- Schließen-„×“ im Blatt (390 px): vorher 24 × 24 px, jetzt mindestens 44 × 44 px
  (`.kidoc-sheet button { min-width: 44px; min-height: 44px }`); Playwright prüft die `boundingBox`.

## Kontraste

| Element | Hell | Dunkel | Soll |
|---|---|---|---|
| Text (Antwort) | 10,05 | 10,01 | 4,5 |
| Text (eigene Frage) | 13,46 | 13,15 | 4,5 |
| Nebentext (leerer Verlauf, Frageart, Fundort, Vorschau-Hinweis) | 10,05 / 8,71 / 9,06 / 10,05 | 10,01 / 9,04 / 8,59 / 10,01 | 4,5 |
| Belegchip (Text auf Akzentfläche) | 5,38 | 5,75 | 4,5 |
| Vorschau-Badge Text | 10,05 | 10,01 | 4,5 |
| Vorschau-Badge Rand | 7,09 (vorher 2,93) | 10,09 | 3 |
| Bereichsknopf Text (Akzent auf Akzent hell) | 4,67 | 4,99 | 4,5 |
| Bereichsknopf Rand | 5,38 | 5,52 | 3 |
| Bereichsknopf „Ganzes Buch“ Text | 12,01 | 9,64 | 4,5 |
| Bereichsknopf „Ganzes Buch“ Rand | 7,09 | 10,09 | 3 |
| Eingabefeld Rand | 3,19 (vorher 1,56) | 3,94 (vorher 1,82) | 3 |

Behoben: Rand des Vorschau-Badges (volle Bernsteinfarbe statt 60 %) und Rand des Eingabefelds
(`--bt-edge`, 70 % Mittelgrau statt 30 %).
