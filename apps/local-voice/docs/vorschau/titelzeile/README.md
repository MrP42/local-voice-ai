# Titelzeile in den Modulen entfernt (Patrick 02.11.2026)

Vorher (`vorher/`, main cbb88d7) und nachher (`nachher/`, dieser Branch), gleiche
Testdaten, Playwright-Browser ohne Tauri-Fensterknöpfe.

| Bild | Was sich ändert |
|------|-----------------|
| `*-1440-*` | Keine Zeile „Verlauf“/„Vorlesen“ … mehr im Inhalt. ⓘ, Suche und Aktionen (z. B. „Hinzufügen“, „Aufnahmeordner öffnen“, Rückgängig/Wiederherstellen im Bearbeiten-Modus) stehen rechts in der Titelleiste vor den Fensterknöpfen. Der Inhalt beginnt ca. 48 px höher. |
| `*-390-*` | Unter 640 px reicht der Platz in der Titelleiste nicht; die Werkzeuge bleiben als schmale Zeile ohne Titel über dem Inhalt. Module ohne Werkzeuge haben keine Zeile. |

Barrierefreiheit: Der Modulname bleibt als unsichtbare Überschrift (`h1#page-title`,
`sr-only`), der Abschnitt ist weiter per `aria-labelledby` benannt, die Werkzeugleiste
hat `role="toolbar"` mit dem Modulnamen als Bezeichnung.

Herkunft: Bis 01.11. stand der Titel in der Titelleiste; ADR-0027 Variante A (lva#197)
hat ihn in einen Modulkopf im Inhalt verlegt, damit die Titelleiste nur App und
Fensterknöpfe zeigt. Jetzt: Titel nur in der Navigation, Werkzeuge in der Titelleiste.
