# Buch-Gespräch – Nachweise Barrierefreiheit (Vorschau, Runde 2)

Gemessen mit Playwright/Chromium an `index.html` (Skript im PR-Bericht), Stand 05.11.2026.

## Kontrast (WCAG 2.2, Ziel Text ≥ 4,5:1, Bedienelemente ≥ 3:1)

| Paar | Hell | Dunkel |
|---|---|---|
| Text / Fläche | 17,22 | 13,28 |
| Nebentext / Fläche | 6,69 | 7,30 |
| Nebentext / Fläche 2 (Belegkarte, Hinweisleiste) | 6,41 | 6,55 |
| Akzent / Akzent hell (Belegchip, Bereichsknopf) | 4,67 | 4,99 |
| Akzent / Fläche (Links, Bereichsknopf-Rand) | 5,38 | 5,75 |
| Buchtext / Papier | 16,94 | 12,24 |
| Buchtext / Sprungmarkierung | 15,20 | 7,61 |
| iOS-Akzent / Karte („Weiterlesen“, Alert-Knöpfe) | 5,57 | 4,66 |
| Weiß / iOS-Knopf („Fragen“, Sprechen) | 5,80 | 5,62 |
| Weiß / Akzent-Knopf (Sprechen Desktop, aktiver Chip) | 5,38 | 5,75 |
| Weiß / Live-Knopf (Stopp) | 6,07 | 6,07 |
| Live-Welle / Fläche | 6,07 | 5,49 |

Änderung gegenüber Runde 1: eigene Füllfarben für Knöpfe (`--accent-fill`, `--ios-fill`, `--live-fill`), weil Weiß auf dem hellen Dunkelmodus-Akzent nur 2,6:1 erreichte; iOS-Akzent hell `#0066cc` statt `#007aff` (4,02:1).

## Touchflächen

- iPhone: 33 sichtbare Bedienelemente in i1–i5 gemessen, **alle ≥ 44 × 44 pt** (Sprechen 72 pt, „Fragen“ 44 pt hoch, Bereichsknopf 44 pt, Belegkarten-Knöpfe 44–48 pt, Menüzeilen 48 pt, Alert-Knöpfe 44 pt).
- Desktop breit: Bedienelemente ≥ 30 px, Sprechen 52 px. Desktop schmal (< 880 px) nutzt das Bottom Sheet mit 44 px wie lva#183.

## Dynamic Type (iOS)

- i5 zeigt AX1 (~ 21 pt Fließtext): Kopf bricht in zwei Zeilen, Belegkarten-Knöpfe stehen untereinander, Verlauf scrollt, nichts wird abgeschnitten. Umsetzung mit `@ScaledMetric`, `ViewThatFits` für die Knopfreihe, keine festen Höhen außer Mindesthöhen.

## VoiceOver / Screenreader

| Element | Beschriftung | Ansage bei Zustandswechsel |
|---|---|---|
| Sprechen-Knopf | „Frage sprechen“ / im Gespräch „Zuhören beenden“ | „Hört zu“ |
| Live-Text | Rolle Live-Region (polite) | Text erst bei Frageende vollständig |
| Denkt nach | — | „Sucht im Buch“ |
| Antwort | Rolle Artikel, Inhalt + „2 Belege“ | „Antwort fertig, 2 Belege“ |
| Belegchip | „Beleg Seite 2, Zitat öffnen“ | — |
| Bereichsknopf | „Bereich: bis Seite 4 geschützt. Ändern“ | nach Änderung „Ganzes Buch einbezogen“ |
| Anton spricht | „Antwort wird vorgelesen“ + Aktion „Antwort anhalten“ | — |
| Vorlesen-Hinweis | „Vorlesen pausiert bei Seite 4. Weiterlesen“ | — |

Bei laufendem VoiceOver spricht die App die Antwort nicht zusätzlich mit Anton, solange VoiceOver selbst vorliest (Einstellung „Antworten mit Anton sprechen“, Standard an; bei VoiceOver Rückfrage beim ersten Mal).

## Tastatur (Desktop)

- Leertaste halten = sprechen (nur wenn der Fokus nicht im Textfeld liegt), `G` = Gespräch an/aus, `Esc` beendet Zuhören bzw. Antwort, `Strg+Enter` sendet Text.
- Tab-Reihenfolge: Bereichsknopf → Menü → Verlauf (Belegchips, Karten-Knöpfe) → Sprechen → Textfeld → Senden.
- Fokus sichtbar (2 px Akzentring), Fokusfalle im Bestätigungsdialog „Spätere Kapitel einbeziehen?“, Esc schließt ohne Erweiterung.
