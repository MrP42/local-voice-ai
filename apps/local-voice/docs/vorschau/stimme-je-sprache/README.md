# Vorschau: Stimme je Buchsprache (lva#352)

Stimmenauswahl im Reader, englisches Buch, Phonon. Runde 2 nach Codex A1–A4.

| Bild | Zustand |
|---|---|
| `01-alle-buecher-1440-{light,dark}.png` | Schalter aus: „Standardstimme für Bücher auf Englisch“. |
| `02-nur-dieses-buch-1440-{light,dark}.png` | Schalter an: „Eigene Stimme für dieses Buch (Englisch)“. |
| `03-buchstimme-fehlt-1440-{light,dark}.png` | Buchstimme gibt es nur auf dem iPhone: Schalter bleibt an, Zeile erklärt den Ersatz durch die Standardstimme. |

Schalter: Spur mit Knopf, `role="switch"`, `aria-checked`, Beschriftung
„Nur für dieses Buch“ (Desktop und iPhone gleich), Erklärtext per
`aria-describedby`. Klick lässt die Liste offen.
iPhone: SwiftUI-`Toggle` „Nur für dieses Buch“ oben in der Stimmenliste,
Fußtext mit denselben drei Wortlauten.
