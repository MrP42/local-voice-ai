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
| G2b | M2 | #15 M8-Backlog (Backend-Robustheit) + Modifier-Warten im Batch-Einfügepfad (Fund G2a) — lv-coder-xhigh | je Punkt Test | abgenommen | 1d7a2e55 |
| G2d | M2 | #29 Stimmen/Laufzeiten nur anbieten wenn startfähig (Windows), #5 Refinement optional freigeben, Katalog-Lizenzlücken (Piper-Stimmen/Runtimes, llama.cpp, LLMs, ASR „other“) schließen + Canary-Hinweis „nur nicht-kommerziell“ (Rest #7) — lv-coder | Tests + Windows-Prüfung + Notices --check | abgenommen | 465faeaa |
| G2f | M2 | #10 Abnahme Browser/Word/VS Code per UI Automation (Skript), #6 Prüfskript Logdatei (Owner löscht) — lv-coder | Skript + Owner-Anleitung | abgenommen (Live-Lauf #10 offen) | 3d9d9ac1 |
| G3 | M3 | U9 Projekt-Protokoll — lv-coder-xhigh | AK3 | abgenommen | 79417c0c |
| G4 | M1 | Reiter umordnen (Wunsch Patrick 01.10.): Mitte = Transkript + Protokoll; rechts unter Bedienung = Notizen + KI-Notizen + Fragen; Live: Transkript Mitte, Notizblock rechts; schmal entsprechend; Persistenz/Tests anpassen; Kopf zeigt Teilnehmende (Anzahl + Namen) und bearbeitet sie per Popover (Personen P5d, Metadaten U7, Sprecher-Verknüpfung U8); Datum/Zeit in Kopf und Liste mit Jahr, ein einheitliches Format (Audit-Befund Datum); Reiter Protokoll/KI-Notizen kompakt: eine Symbol-Werkzeugzeile (Neu erzeugen, Kopieren, Herunterladen), Vorlage wählen/verwalten ins Menü ☰ neben ⓘ, Ablagepfad + „Erzeugt mit Vorlage“ in den Info-Dialog, Inhalt direkt unter den Reitern — lv-coder, nach G1 | Playwright meeting-layout/live-narrow angepasst + neue Reiter-Tests | abgenommen | a86d5a34 |
| G5 | M3 | Mehrsprachigkeit (Wunsch Patrick 01.10.): Spracherkennung vor/bei Transkription + Modellwahl je Sprache (Chip im Kopf, korrigierbar → Neu-Transkription); Übersetzung als neue Fassung (nie überschreiben, Wechsel per Fassungs-Chip, Satz-für-Satz-Vergleich mit Zeitmarken, treue Übersetzung mit Prüfung Zahlen/Namen/Satzanzahl, Herkunft); Protokoll mit Wahl der Grundlage (Original/Übersetzung) und Ausgabesprache, im Protokoll und Info-Dialog ausgewiesen — lv-coder-xhigh, nach G4 | Rust + Playwright meeting-translation | abgenommen | 49f0c2f0 |
| G6 | M2 | Sprecher-Dialog: Speichern wirkt im Transkript, Anhören → Stopp (Patrick 01.10.) — lv-coder | Playwright rot→grün | abgenommen (Nachtest Installer) | 507a2feb |
| G7 | M2 | ASR-Wiederholungsschleifen (if if if, s s s, I I I) – Decoder + Nachfilter hallucination.rs — lv-coder-xhigh | Rust ≥12 Fälle, Messung echtes Audio | abgenommen | 5c6bca8f |
| A4 | M4 | #66 Seite Integrationen (Liste, Katalog, Rechte-Matrix, Audit, Freigaben, Kalender/MCP-Umzug) — lv-coder | AK7 integrations:: integrations.spec | abgenommen | a77920e3 |
| B1 | M5 | #67 Workflow-Engine-Kern + Trockenlauf-CLI — lv-coder-xhigh | workflows:: ≥30 Tests, --dry-run JSON | abgenommen | c850b73a |
| C1 | M6 | #68 Eval-Harness + Messung lokaler Modelle — lv-architect | Bericht eval-2026-10-01.md | abgenommen | 3f66fbdd |
| D0 | M7 | #69 Spike OCR/Bildanalyse/Folien — lv-architect | Spike-Bericht mit Messung | abgenommen | koordination/bild-video/spike |
| A7 | M4 | #66 Agentenbrücke (Named Pipe, Token, Werkzeug-Rechte, Freigaben, ctl) — lv-coder-xhigh | AK9 agent_bridge:: ≥15 | abgenommen (QG5-Review offen) | ec72a83b |
| A5 | M4 | #66 Microsoft-365-Konto (OAuth PKCE, Mail, OneDrive, Termin-Notiz) — lv-coder-xhigh | AK8 m365 ≥12 Tests | abgenommen (Owner: Entra-Registrierung, Testmail) | 7b9656dc |
| A6 | M4 | #66 SMTP, Ordner-Sandbox, Obsidian, Wissensbasis, Export ablegen — lv-coder | AK8 Rest, AK11 | abgenommen (Owner: Testmail, Vault, Wissens-Schlüssel) | fa0e5870 |
| D1 | M7 | #69 Folienerkennung Kern (dHash, meeting_slides, Job-Phase) — lv-coder-xhigh | cargo test slides | abgenommen | fc478c2e |
| G8 | M8 | Wackelige Tests unter Last: Ursachen beheben (SQLite busy_timeout, Zeitabhängigkeit, Prozess-Timeouts) — lv-coder-xhigh | 3x volle Suite unter Last grün | abgenommen | 09ca4341 |
| B2 | M5 | #67 Auslöser Kalender/Ereignisse/Zeitplan/manuell, Arbeiter in lib.rs, Einwilligungsweg — lv-coder-xhigh | AK3, AK4 | abgenommen (Abnahme Hinweisfenster im Installer) | c850b73a |
| D2 | M7 | #69 OCR je Folie (Windows-OCR), Text-Dubletten — lv-coder | cargo test slides::ocr | abgenommen | 219043ea |
| D4 | M7 | #69 Folien-Oberfläche (Leiste, Sprung, Ausblenden, Großansicht, Folien erkennen) — lv-coder | meeting-slides.spec | abgenommen | cc097c92 |
| B3 | M5 | #67 Auslöser Ordner (Stabilität, Ledger, OneDrive) und YouTube-Kanal (RSS) + Aktion Import — lv-coder | AK5, AK11 Auslöser | abgenommen | effa3670 |
| D5 | M7 | #69 Folien in Protokoll, KI-Notizen, Chat, Suche ([Folie n · mm:ss], Belege) — lv-coder-xhigh | cargo test minutes notes search slides | abgenommen | a1c20633 |
| B4 | M5 | #67 App-Aktionen (Notizen, Protokoll, Zusammenfassung, Export, TTS, Mitteilung, Warten) + Vorlage Eingangsordner → Word — lv-coder | AK6 | abgenommen (Toast im Installer prüfen) | cf388ec1 |
| A8 | M4 | #66 MCP schreibend (transcribe_file, tts, add_youtube_source, get_provenance, start/stop_recording mit Einwilligung), mcp_smoke --write, --audit-dump — lv-coder-xhigh | AK10, AK11 | in_arbeit | |
| B5 | M5 | #67 Integrations-Aktionen Mail (Empfängerregeln, Freigabe), Termin-Notiz, Webhook (n8n); Vorlage Termin → Mail — lv-coder-xhigh | AK7, AK8 | in_arbeit | |
| B7 | M5 | #67 Oberfläche Automationen (Liste, Editor, Vorlagen, Lauf mit Herkunft, Freigaben, JSON-Import/Export) — lv-coder | AK9 automations.spec | abgenommen (B7n Einwilligung im Freigabedialog) | bd8d7a6f |
| C2 | M6 | #68 Agent-Laufzeit (Schema, Denken aus, Validierung, Retry, Rückfall) + agent.extract (To-dos/Fristen/Entscheidungen mit Belegen) + Provenienz — lv-coder-xhigh | AK3, AK4 | in_arbeit | |
| B7n | M5 | Einwilligung zur Aufnahme im allgemeinen Freigabedialog nur mit demselben Häkchen wie im Hinweisfenster (oder dorthin verweisen) — lv-coder | Playwright + Rust: ohne Häkchen keine Freigabe | in_arbeit | |
