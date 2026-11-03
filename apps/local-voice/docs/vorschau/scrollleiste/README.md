# Überflüssige Scrollleiste (Patrick 02.11.)

## Ursache
Unsichtbare Screenreader-Texte (`sr-only`, absolut) wie „Gehört.“ in der
Kurzbuch-Liste standen an ihrer statischen Stelle tief in einer Scrollliste.
Ohne positionierten Vorfahren entkamen sie dem Beschnitt der Liste und
verlängerten das **Dokument** (Scrollleiste ganz rechts, Titelleiste und
Navigation scrollten mit). „Wach halten“ setzt alle 45 s `position: relative`
auf die Lesespalte; dann landete derselbe Überhang **dort** (Leiste zwischen
Text und rechter Spalte). Daher der Wechsel.

## Messung (Playwright, Kurzbuch mit 12 gehörten Kernaussagen)

| Zustand | vorher: Dokument ragt über das Fenster | nachher |
|---|---|---|
| 1440×900, Liste aufgeklappt, Fokus unten | 126 px | 0 px |
| 1600×900 | 935 px | 0 px |
| 1024×900 (schmales Layout) | 1228 px, `html.scrollTop` = 8 | 0 px |

Lesespalte (`.reader-frame`) nachher in allen Fällen ohne Überlauf.
Headless-Chromium zeichnet keine Scrollleisten; die Bilder zeigen den Stand
nachher, der Beleg ist die Messung im Test
`tests/readaloud-translate.spec.ts` („Dokument und Rahmen scrollen nie“),
der ohne den Fix rot ist (126 px).
