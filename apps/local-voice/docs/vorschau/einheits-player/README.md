# Vorschau: Einheits-Player auf dem iPhone

Ein Player für Hören, Mitlesen und Lesen; Wechsel mit einem Tipp oben, Stelle bleibt.
Plan: `apps/apple/docs/einheits-player.md`. iPhone 390 × 844 pt, Faktor 2, Farben aus `VoicePalette`.
Erzeugt aus `vorschau.html` (Parameter `theme`, `screen`).

Beispiel: Fontane, „Effi Briest“ (gemeinfrei). Cover und Sprecherin sind erfunden.

| Bild | Inhalt |
| --- | --- |
| `00-detail-*` | Buchdetail mit **einem** Knopf „Weiter“ (vorher „Text vorlesen“ und „Hörbuch hören“ getrennt) |
| `01-hoeren-*` | Ansicht „Hören“: Cover, volle Bedienleiste mit Kapitelzeile und Buchfortschritt; Quelle-Knopf neu |
| `02-mitlesen-*` | „Lesen & Hören“: Satz hell, gesprochenes Wort kräftig (nur Hintergrund, kein Umbruchspringen), andere Absätze gedämpft, scrollt mit; Leiste schmal (ohne Kapitelzeile, Kapitel steht im Kopf) |
| `03-lesen-*` | „Lesen“: Wiedergabe pausiert, ruhiger Text, Leseposition dezent; Leiste nur Fortschritt + „Ab hier hören“ |
| `04-quelle-*` | Menü „Quelle“: Hörbuch oder Vorlesestimme an derselben Textstelle |
| `05-stimme-*` | Mitlesen mit Vorlesestimme: Wort exakt von der Systemstimme; Satz/Absatz vor und zurück |

Kopfzeile in allen Ansichten: Schließen · Titel/Kapitel · Aa (Darstellung) · Sprechblase (Buch-Gespräch „Fragen zum Buch“) · ⓘ (nur Hörbuch).
