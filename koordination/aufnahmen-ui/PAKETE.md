# Pakete — Aufnahmen-Oberflaeche: Spalten, Sessions, eine Scrollbar (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-aui/koordination/aufnahmen-ui/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| U2 | M2 | Spaltengerüst Variante B (PageShell fill, 3 Bereiche, Griffe, Klappleisten, eine Scrollbar, Persistenz, Transkript flex statt max-h-96) — lv-coder | playwright meeting-layout: AK2, AK4 (4 Viewports), AK8 | abgenommen | 94dd0569 |
| U3 | M3 | Projekte-Spalte aus meeting_folders (oberste Ebene, n:m, Strg+Ziehen fügt hinzu), anlegen/umbenennen/löschen/sortieren, Suche/Filter im Projekt (Filterchips → Popover), „Als Nächstes“ unten, Migrationstest — lv-coder | cargo meetings::folders + playwright meeting-projects → AK3 | abgenommen | 89f39ae5 |
| U4 | M4 | Kompakter Kopf (Titel inline, Status-Chips, Details-Dialog), Symbolzeile + Menü ☰ (Neu transkribieren-Dialog, Vorlage, Verschieben, Löschen), Aufnahmekarte → Startdialog, Player-Breite, Fortschritt als Zeile — lv-coder | playwright meeting-header → AK5 (≤120 px bei 1366), Tooltips | abgenommen | 89f39ae5 |
| U5 | M5 | Live + Import + schmal: laufende Aufnahme = gewählte Besprechung (Notizblock Mitte, Live-Transkript rechts, LiveChatRow in Reiter Fragen), ein Import-Weg (Symbol + Ablage auf Inhaltsspalte in gewähltes Projekt), Startdialog nutzt useSelectedProject, „Nächste Termine“ in RecorderCard entfernen (NextUp links), schmal < 620 px, Flaky-Tests Chat/Notizen unter Last härten — lv-coder-xhigh | playwright meeting-live-narrow → AK6 (480×800 ohne Scrollen), AK7 | abgenommen | 3a385469 |
| U6 | M6 | Barrierefreiheit (Tastatur, Fokus, Rollen), tote i18n-Schlüssel, Hilfe-Text, Doku — lv-coder | meeting-a11y 14 passed, Suite 466 grün; axe-Paket fehlt (Freigabe devDependency offen) | abgenommen | 30f66788 |
| U7 | M7 | Import-Warteschlange + parallele Transkriptionen (1–3, RAM/VRAM-Gate) + Metadaten bearbeiten (Titel, Beschreibung, Datum, Teilnehmende, Projekte) — lv-coder-xhigh | Rust Queue-Tests, Playwright meeting-queue/meeting-metadata | abgenommen | 5dc70add |
| U8 | M8 | Sprecher: Benennen überall sichtbar, Namensvorschlag aus Anrede im Transkript (mit Bestätigung), eigener Name für Mikrofon/„Ich“ — lv-coder | Playwright meeting-speakers-names, Rust Anrede-Erkennung | abgenommen | 9e55d4e8 |
| U9 | M9 | Projekt-Protokoll: Aufnahmen eines Projekts per Häkchen wählen, gemeinsames Protokoll/Zusammenfassung mit Vorlage (Automatisch), Quellen je Aufnahme + Audio-Sprung, Provenienz, im Projekt gespeichert — lv-coder-xhigh, nach U7 | Rust Mehrfach-Blocklogik, Playwright project-minutes | offen | |
