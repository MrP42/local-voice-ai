# Pakete — Aufnahmen-Oberflaeche: Spalten, Sessions, eine Scrollbar (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-aui/koordination/aufnahmen-ui/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| U2 | M2 | Spaltengerüst Variante B (PageShell fill, 3 Bereiche, Griffe, Klappleisten, eine Scrollbar, Persistenz, Transkript flex statt max-h-96) — lv-coder | playwright meeting-layout: AK2, AK4 (4 Viewports), AK8 | abgenommen | 94dd0569 |
| U3 | M3 | Projekte-Spalte aus meeting_folders (oberste Ebene, n:m, Strg+Ziehen fügt hinzu), anlegen/umbenennen/löschen/sortieren, Suche/Filter im Projekt (Filterchips → Popover), „Als Nächstes“ unten, Migrationstest — lv-coder | cargo meetings::folders + playwright meeting-projects → AK3 | in_arbeit | |
| U4 | M4 | Kompakter Kopf (Titel inline, Status-Chips, Details-Dialog), Symbolzeile + Menü ☰ (Neu transkribieren-Dialog, Vorlage, Verschieben, Löschen), Aufnahmekarte → Startdialog, Player-Breite, Fortschritt als Zeile — lv-coder | playwright meeting-header → AK5 (≤120 px bei 1366), Tooltips | in_arbeit | |
