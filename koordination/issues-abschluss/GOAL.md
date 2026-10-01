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
aktualisiert: 2026-10-01T10:39
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
- [ ] AK1 — Leerer Eintrag: Playwright → im gewählten Projekt „Neue Besprechung“ anlegen (Plus/Kontextmenü), Eintrag erscheint ohne Audio im Projekt; darin „Aufnahme starten“, „Datei importieren“ und „Link einfügen“ füllen DIESEN Eintrag (keine zweite Besprechung); Rust-Test für leere Besprechung + spätere Quelle.
- [ ] AK2 — Ältere Issues: je Issue Befehl/Test als Beleg im Issue-Kommentar, Issue geschlossen; Owner-Punkte (#6 Löschen, #11 Ignore-Entscheidungen) mit Vorschlag und ausfüllfertigem Befehl kommentiert.
- [ ] AK3 — U9: Playwright project-minutes grün; Rust-Test Mehrfach-Blocklogik; Quellen je Aufnahme mit Audio-Sprung.
- [ ] AK4 — #66 Bündel 2: AK7–AK11 aus koordination/integrationen/GOAL.md erfüllt.
- [ ] AK5 — #67: AK aus koordination/workflow-automation/GOAL.md erfüllt.
- [ ] AK6 — #68: AK aus koordination/lokaler-agent/GOAL.md erfüllt (oder nach Messung C1 begründet beendet).
- [ ] AK7 — #69: Spike-Messung lokal (OCR + Bildanalyse) dokumentiert; Folien eines Videos in der Besprechung sichtbar und im Protokoll verwendet.
- [ ] AK8 — Release: Version 0.21.0, Installer gebaut, `cargo test --lib`, tsc, volle Playwright-Suite grün; PR offen; `gh issue list --state open` nur noch Owner-Restpunkte mit Kommentar.

## Quality Gates
- [ ] QG1 — Rust komplett grün (`cargo test --lib`, 2x).
- [ ] QG2 — Frontend: tsc 0, Playwright komplett grün.
- [ ] QG3 — Systemschutz: neue Prozesse/Modelle hinter process_guard + RAM-Gate.
- [ ] QG4 — i18n de+en, echte Umlaute (check_i18n_meetings.py).
- [ ] QG5 — Sicherheitsreview für schreibende MCP/CLI-Werkzeuge und Workflow-Aktionen (Gate-Logik).
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
-

## Blocker
-

## Entscheidungen
- 2026-10-01 Patrick: Reiter umordnen vor 0.21.0 – Mitte Transkript + Protokoll, rechts unter der Bedienung Notizen, KI-Notizen, Fragen; Kopf zeigt Teilnehmende (Anzahl, Namen) und macht sie dort bearbeitbar (G4).
- 2026-10-01 Patrick: Budget gestaffelt – jetzt M1–M3 + M8 Release (~2,7 MTok); M4–M7 je einzeln freigeben.
- 2026-10-01 Patrick (/goal-planner-worker): alle offenen Issues abschließen, neue Features umsetzen, testen, neue Version, Ergebnisse in GitHub; zuerst leerer Eintrag im Projekt.

## Nächste empfohlene Aktion
M1 briefen.

## Verlauf
- 2026-10-01T10:38 DISCOVERY — Goal State angelegt
- 2026-10-01T10:39 DISCOVERY (Runde 0) — Metadaten: issue=70
- 2026-10-01T10:39 READY (Runde 0) — Goal definiert, Issue #70
- 2026-10-01T10:39 PLANNING (Runde 1) — M1 zuerst
- 2026-10-01T10:39 EXECUTING (Runde 1) — G1 laeuft (wt-goal)

