---
thema: issues-abschluss
titel: Offene Issues abschliessen, neue Features, Version 0.21.0
state: EXECUTING
vorzustand: -
pausengrund: -
issue: 70
repo: MrP42/local-voice-ai
branch: feat/issues-abschluss
iteration: 1
erstellt: 2026-10-01
aktualisiert: 2026-10-01T18:38
---

# Goal: Offene Issues abschliessen, neue Features, Version 0.21.0

## Zielzustand
Alle offenen Issues des Repos sind umgesetzt und geschlossen oder mit belegtem Grund (Owner-Aktion, bewusste Ausgrenzung) aktualisiert, die neuen Features (leerer Eintrag im Projekt, Projekt-Protokoll, Integrationen Bündel 2, Workflow-Automation, Lokaler Agent, Bild-/Videoinhalte) laufen, und eine getestete Version 0.21.0 liegt als Installer vor. Erkennbar an `gh issue list --state open` (nur Owner-Restpunkte mit Kommentar), am Installer und an grünen Gesamttests auf dem gepushten Branch mit offenem PR.

## Scope
- **M1 (zuerst, Wunsch Patrick 01.10.):** In der Projekte-Spalte der Seite Aufnahmen einen leeren Eintrag („Neue Besprechung“) in einem Projekt anlegen, ohne Audio; darin dann Aufnahme starten, Datei importieren oder YouTube-Link einfügen.
- **M2 Ältere Issues:** #3 (Segment-Modus ohne Einfüge-Schutz), #9 (Streaming ohne Fokusprüfung), #15 (M8-Backlog), #8 (Build-Fallstricke), #11 (cargo-deny-Advisories, Triage mit Vorschlag), #5 (Refinement optional), #29 (Vorlesen: defekte Stimmen/Laufzeiten, Windows-Prüfung), #7 (SBOM/Notices), #10 (Abnahme Browser/Word/VS Code per UI Automation), #6 (alte Logdatei – Owner-Aktion mit Prüfskript), Befund B4 aus #66 (Farbkontrast app-weit).
- **M3 U9 Projekt-Protokoll** (#64): mehrere Aufnahmen eines Projekts per Häkchen gemeinsam protokollieren/zusammenfassen.
- **M4 #66 Bündel 2:** Seite „Integrationen“ (zwischen Modelle und Einstellungen), Rechte-Matrix, Mail/OneDrive/Obsidian/RAG, MCP/CLI steuernd (vorschlag.md auf feat/integrationen).
- **M5 #67 Workflow-Automation** (inkl. YouTube-Kanal-Workflow aus #65).
- **M6 #68 Lokaler Agent.**
- **M7 #69 Bild-/Videoinhalte** (Spike zuerst).
- **M8 Release 0.21.0:** Gesamttest, Installer, PR, Issues schließen/aktualisieren, Goal-Issues #59/#61/#64/#65/#66 abschließen soweit belegt.

## Non-Scope
- Mergen der PRs (#56–#58, #60, #62, #63, #21, #32): Owner-Aktion (Klassifikator blockt `gh pr merge`); Merge-Reihenfolge wird dokumentiert.
- Releases/Tags auf main ohne Patricks Freigabe.
- macOS-Builds (PR #32/#60 eines anderen Zugangs) – nur nicht brechen.

## Akzeptanzkriterien
- [x] AK1 — Leerer Eintrag: Playwright → im gewählten Projekt „Neue Besprechung“ anlegen (Plus/Kontextmenü), Eintrag erscheint ohne Audio im Projekt; darin „Aufnahme starten“, „Datei importieren“ und „Link einfügen“ füllen DIESEN Eintrag (keine zweite Besprechung); Rust-Test für leere Besprechung + spätere Quelle.
- [ ] AK2 — Ältere Issues: je Issue Befehl/Test als Beleg im Issue-Kommentar, Issue geschlossen; Owner-Punkte (#6 Löschen, #11 Ignore-Entscheidungen) mit Vorschlag und ausfüllfertigem Befehl kommentiert.
- [x] AK3 — U9: Playwright project-minutes grün; Rust-Test Mehrfach-Blocklogik; Quellen je Aufnahme mit Audio-Sprung.
- [ ] AK4 — #66 Bündel 2: AK7–AK11 aus koordination/integrationen/GOAL.md erfüllt.
- [ ] AK5 — #67: AK aus koordination/workflow-automation/GOAL.md erfüllt.
- [ ] AK6 — #68: AK aus koordination/lokaler-agent/GOAL.md erfüllt (oder nach Messung C1 begründet beendet).
- [ ] AK7 — #69: Spike-Messung lokal (OCR + Bildanalyse) dokumentiert; Folien eines Videos in der Besprechung sichtbar und im Protokoll verwendet.
- [ ] AK8 — Release: Version 0.21.0, Installer gebaut, `cargo test --lib`, tsc, volle Playwright-Suite grün; PR offen; `gh issue list --state open` nur noch Owner-Restpunkte mit Kommentar.

## Quality Gates
- [ ] QG1 — Rust komplett grün (`cargo test --lib`, 2x).
- [x] QG2 — Frontend: tsc 0, Playwright komplett grün.
- [x] QG3 — Systemschutz: neue Prozesse/Modelle hinter process_guard + RAM-Gate.
- [x] QG4 — i18n de+en, echte Umlaute (check_i18n_meetings.py).
- [x] QG5 — Sicherheitsreview für schreibende MCP/CLI-Werkzeuge und Workflow-Aktionen (Gate-Logik).
- [ ] QG6 — Doku/Hilfe aktualisiert, Handoff geschrieben.
- [ ] QG7 — Budget: Schätzung ~10,6 MTok (M1 0,3 · M2 1,5 · M3 0,4 · M4 1,7 · M5 2,6 · M6 1,6 · M7 2,0 · M8 0,5); Meldung 50/80 %.

## Constraints
- Basis `release/0.20.11`; Branch + PR, keine Formatierläufe über fremde Dateien, kein Installieren über Patricks App.
- Einstellungen nur in vorhandenen Gruppen (Ausnahme: Seite „Integrationen“, von Patrick gewünscht).
- Installer je Abnahmestand mit Patch +1 (0.20.12 … bis 0.21.0 zum Abschluss).

## Architekturprinzipien
- Bestehende Goals (#64, #66–#69) liefern Detail-AK und Architektur; dieses Goal bündelt Reihenfolge und Abschluss.

## Dependencies
- release/0.20.11 (alle bisherigen Pakete integriert).

## Risiken / Owner-Entscheidungen
- R1 Budget ~10,6 MTok – Vorschlag: gestaffelt freigeben (M1–M3 zuerst ~2,2 MTok). Owner: Patrick.
- R2 #6 Löschen alter Logdatei und #11 Ignore-Liste: Owner-Aktion, Vorschlag kommt fertig.
- R3 #10 Abnahme in Word/VS Code/Browser: automatisiert per UI Automation soweit möglich, Rest manuell durch Patrick.
- R4 Lizenz (G2d): Piper-Windows-Runtime bringt espeak-ng.dll (GPL-3.0-or-later) mit, nur Download auf Wunsch, Subprozess, nicht im Installer. Vorschlag: so lassen und Lizenztext beim Download ablegen. Owner: Patrick.
- R5 Lizenz (G2d): Piper-Stimmen Lessac/Ryan nicht-kommerziell, Thorsten/Amy/Alan/Alba/Kerstin abgeleitet und ungeklaert. Vorschlag: Hinweis "nur nicht-kommerziell" wie bei Canary. Owner: Patrick.

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M1 | Leerer Eintrag im Projekt (Installer 0.20.12) | offen |
| M2 | Ältere Issues geschlossen | offen |
| M3 | Projekt-Protokoll | offen |
| M4 | Seite Integrationen | offen |
| M5 | Workflow-Automation | offen |
| M6 | Lokaler Agent | offen |
| M7 | Bild-/Videoinhalte | offen |
| M8 | Release 0.21.0 | offen |

## Evidence
- 2026-10-01T11:15 AK1 erfüllt — G1 307e7ac7: meeting-empty-entry 13 passed, Rust empty 76 passed, Suite 579 passed
- 2026-10-01T18:38 AK3 erfüllt — G3 79417c0c: project-minutes.spec 15/15, minutes::project 51 + project_minutes_store 12 Rust-Tests, Quellen mit Audio-Sprung
- 2026-10-01T18:38 QG2 erfüllt — tsc 0 auf D:\lv-build\int; gezielte Playwright je Paket grün; volle Suite zuletzt 717 passed (A6) – auf Patricks Wunsch keine weiteren Volläufe
- 2026-10-01T18:38 QG5 erfüllt — Codex-Review review-qg5-codex.md: 4 Befunde B20-B23 → S1 5f9a6b91 behoben, je Test rot→grün; Integrationslauf c0e22923: cargo build ok, 1061 Tests grün
- 2026-10-01T18:38 QG3 erfüllt — Neue Prozesse/Modelle nur über process_guard/RAM-Gate/HeavyGate (D1 ffmpeg Job-Objekt, D3 mmproj im RAM-Gate, C2/C3 ensure_local, B1 HeavyGate); je Paket Fehlerfall-Tabelle
- 2026-10-01T18:38 QG4 erfüllt — check_i18n_meetings.py OK (de/en gleiche Schlüssel, echte Umlaute) nach D4/G5/K1/B6

## Blocker
-

## Entscheidungen
- 2026-10-01 Patrick (/goal-planner-worker): "schließe alle noch offenen punkte (issues) erfolgreich ab und liefer ein neues release aus" – Release 0.21.0 ist damit freigegeben. Annahmen (Vorschläge übernommen, widerrufbar): Agent-Router Qwen3.5-9B (E4B nur extract); mmproj-Download auf Wunsch im Katalog, Projektor nur für Folienaufträge; macOS ohne OCR; Lizenz: espeak-ng-Lizenztext beim Runtime-Download ablegen, Piper-Stimmen Lessac/Ryan und Ableitungen mit Hinweis "nur nicht-kommerziell". Arbeitsweise: Builds/Tests nur auf D:\lv-build, Tests minimal (Build + betroffene Gruppen + tsc), keine sichtbaren Fenster ohne Ankündigung.
- 2026-10-01 Patrick: "Vollgas" – maximale Parallelisierung, alle offenen Issues und Restpunkte; M4–M7 damit freigegeben. Neue Befunde: Sprecher-Dialog (G6), ASR-Wiederholungen (G7). Keine sichtbaren Fenster, solange Patrick am Rechner arbeitet.
- 2026-10-01 Patrick: G5 Mehrsprachigkeit/Übersetzung aufnehmen (vor 0.21.0); Rahmen dadurch ~3,2 MTok.
- 2026-10-01 Patrick: mp3lame LGPL als Ausnahme zulassen; Canary 1B mit Hinweis „nur nicht-kommerziell“ behalten; 12 unmaintained-Ignores übernehmen.
- 2026-10-01 Patrick: Reiter umordnen vor 0.21.0 – Mitte Transkript + Protokoll, rechts unter der Bedienung Notizen, KI-Notizen, Fragen; Kopf zeigt Teilnehmende (Anzahl, Namen) und macht sie dort bearbeitbar (G4).
- 2026-10-01 Patrick: Budget gestaffelt – jetzt M1–M3 + M8 Release (~2,7 MTok); M4–M7 je einzeln freigeben.
- 2026-10-01 Patrick (/goal-planner-worker): alle offenen Issues abschließen, neue Features umsetzen, testen, neue Version, Ergebnisse in GitHub; zuerst leerer Eintrag im Projekt.

## Nächste empfohlene Aktion
C4-Merge, dann B6-Merge, S1 abnehmen, Integrationslauf D:, Fenster-Block #10, Version 0.21.0, Installer, Merge #71 nach main, Tag app-v0.21.0

## Verlauf
- 2026-10-01T10:38 DISCOVERY — Goal State angelegt
- 2026-10-01T10:39 DISCOVERY (Runde 0) — Metadaten: issue=70
- 2026-10-01T10:39 READY (Runde 0) — Goal definiert, Issue #70
- 2026-10-01T10:39 PLANNING (Runde 1) — M1 zuerst
- 2026-10-01T10:39 EXECUTING (Runde 1) — G1 laeuft (wt-goal)
- 2026-10-01T11:59 EXECUTING (Runde 1) — G2b, G5, G2f (wt-u7), G3 (wt-g2c) laufen; G2d abgenommen
- 2026-10-01T16:49 EXECUTING (Runde 1) — Rechner auf Patricks Wunsch frei: keine Worker/Tests aktiv. Integrationslauf 97f3c24b: build ok, 3766/3767 (Katalog-Test, behoben 02b9a296)
- 2026-10-01T18:13 EXECUTING (Runde 1) — Budget-Meldung: Runde Abschluss geschätzt ~3 MTok, verbraucht ~3,1 MTok (B6 0,75, C3 0,5, C4 0,4, B8 0,39, D3 0,35, C5 0,35, K1 0,21, R0 0,16); Rest bis Release ~0,8 → Hochrechnung ~3,9 MTok (130 %)

