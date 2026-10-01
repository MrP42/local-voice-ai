# Pakete — Offene Issues abschliessen, neue Features, Version 0.21.0 (Goal: C:/Users/wolff/local-voice-project/.claude/worktrees/wt-goal/koordination/issues-abschluss/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| G1 | M1 | Leerer Eintrag im Projekt + Aufnahme/Import/Link füllen diesen Eintrag — lv-coder | AK1 | abgenommen | 307e7ac7 |
| G2a | M2 | #3 Segment-Modus über paste_guard, #9 Streaming mit Fokusprüfung — lv-coder-xhigh | Rust-Tests fail-closed, Issues kommentiert | abgenommen | 5d14bded |
| G2c | M2 | #8 Build-Fallstricke abstellen, #7 SBOM/Third-Party-Notices automatisch, #11 cargo-deny Triage (Upgrades + begründete Ignores als Vorschlag) — lv-coder | Skripte + cargo deny check | abgenommen | 5414e596 |
| G2e | M2 | Befund B4 (#66): Farbkontrast app-weit über Design-Token, axe color-contrast wieder an — lv-coder | axe ohne Ausnahme grün | abgenommen | 622fe493 |
| G2b | M2 | #15 M8-Backlog (Backend-Robustheit) + Modifier-Warten im Batch-Einfügepfad (Fund G2a) — lv-coder-xhigh | je Punkt Test | in_arbeit | |
| G2d | M2 | #29 Stimmen/Laufzeiten nur anbieten wenn startfähig (Windows), #5 Refinement optional freigeben, Katalog-Lizenzlücken (Piper-Stimmen/Runtimes, llama.cpp, LLMs, ASR „other“) schließen + Canary-Hinweis „nur nicht-kommerziell“ (Rest #7) — lv-coder | Tests + Windows-Prüfung + Notices --check | in_arbeit | |
| G2f | M2 | #10 Abnahme Browser/Word/VS Code per UI Automation (Skript), #6 Prüfskript Logdatei (Owner löscht) — lv-coder | Skript + Owner-Anleitung | offen | |
| G3 | M3 | U9 Projekt-Protokoll — lv-coder-xhigh | AK3 | offen | |
| G4 | M1 | Reiter umordnen (Wunsch Patrick 01.10.): Mitte = Transkript + Protokoll; rechts unter Bedienung = Notizen + KI-Notizen + Fragen; Live: Transkript Mitte, Notizblock rechts; schmal entsprechend; Persistenz/Tests anpassen; Kopf zeigt Teilnehmende (Anzahl + Namen) und bearbeitet sie per Popover (Personen P5d, Metadaten U7, Sprecher-Verknüpfung U8); Datum/Zeit in Kopf und Liste mit Jahr, ein einheitliches Format (Audit-Befund Datum); Reiter Protokoll/KI-Notizen kompakt: eine Symbol-Werkzeugzeile (Neu erzeugen, Kopieren, Herunterladen), Vorlage wählen/verwalten ins Menü ☰ neben ⓘ, Ablagepfad + „Erzeugt mit Vorlage“ in den Info-Dialog, Inhalt direkt unter den Reitern — lv-coder, nach G1 | Playwright meeting-layout/live-narrow angepasst + neue Reiter-Tests | in_arbeit | |
| G5 | M3 | Mehrsprachigkeit (Wunsch Patrick 01.10.): Spracherkennung vor/bei Transkription + Modellwahl je Sprache (Chip im Kopf, korrigierbar → Neu-Transkription); Übersetzung als neue Fassung (nie überschreiben, Wechsel per Fassungs-Chip, Satz-für-Satz-Vergleich mit Zeitmarken, treue Übersetzung mit Prüfung Zahlen/Namen/Satzanzahl, Herkunft); Protokoll mit Wahl der Grundlage (Original/Übersetzung) und Ausgabesprache, im Protokoll und Info-Dialog ausgewiesen — lv-coder-xhigh, nach G4 | Rust + Playwright meeting-translation | offen | |
