# Pakete — Vorlesen-Oberflaeche: einheitlich, kompakt, anpassbar (Goal: koordination/ui-vorlesen-kompakt/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

Codebereiche in `TtsSettings.tsx` je Paket getrennt (parallel): P2 = Inhalt von `<aside className="tts-controls">`
(ohne das aside-Tag selbst), P3 = `<details className="workspace-disclosure">`, P4 = `nextExportName` + Aufrufe,
P5 = Rahmen (Wrapper, aside-Tag, FilesSidebar-Einbau, persistente Zustände oben).

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| P0 | M1 | Werkzeug: Testport je Worktree (`LV_DEV_PORT`), Basislinie | Playwright 72/72 auf Port 1610, tsc 0 | abgenommen | dd22dd8 |
| P1 | M1 | UI-Audit + Screenshot-Spec (lv-architect, wt-ui-p1, Port 1611) | AK1-Tabelle vollständig mit Messwerten; `SCREENS_DIR=… playwright test readaloud-screens` erzeugt Vorher-PNGs; ohne SCREENS_DIR übersprungen | in_arbeit | - |
| P2 | M2 | Symbolleiste: Symbol-Knöpfe, Menü ☰, Tooltips, eigene Symbole (lv-coder, wt-ui-p2, Port 1612) | AK2, AK3, AK4 per `tests/readaloud-toolbar.spec.ts`; Gesamtsuite grün | in_arbeit | - |
| P3 | M2 | Fehler „Ausdruck & Sprechstil → Alle“ überlagert Text (lv-coder, wt-ui-p3, Port 1613) | AK10 per Playwright; Gesamtsuite grün | abgenommen | 534f66c |
| P4 | M2 | Dateiname kurz + Dateiliste mit Zeitstempel (lv-coder, wt-ui-p4, Port 1614) | AK8, AK9; Gesamtsuite grün | abgenommen (B1 vom Planer nachgezogen) | c0da046 |
| P5 | M3 | Layout gestapelt/nebeneinander + ziehbare Spalten (lv-coder, wt-ui-p5, Port 1615) | AK5 (Struktur), AK6, AK7 per `tests/readaloud-layout.spec.ts`; Gesamtsuite grün | abgenommen | cb9501d |
