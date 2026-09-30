# Vorschlag Goal „Workflow-Automation“ — Besprechungen, Ordner und Integrationen automatisieren

Grundlage: `recherche/engines-und-trigger.md` (Stand 30.09.2026). Setzt Goal „Integrationen“ (A) voraus.

## Zielzustand
Ein Modul „Automationen“ führt lokal definierte Abläufe aus Auslöser, Bedingungen und Aktionen aus — etwa „Termin im
Kalender X beginnt → aufzeichnen → Transkript → Protokoll → Mail an mich bzw. an die Teilnehmenden“ oder „Datei im
OneDrive-Eingangsordner → transkribieren → Protokoll als Word in OneDrive ablegen und mailen“ — mit Warteschlange,
Wiederholung, Freigaben und Laufprotokoll, ohne Abo und ohne fremde Laufzeit. Erkennbar an den zwei mitgelieferten
Vorlagen, die im Installer mit einem echten Termin und einer echten Datei durchlaufen, und daran, dass externe Agenten
Abläufe per MCP/CLI starten und ihren Status abfragen können.

## Scope
- Engine in Rust im App-Prozess: JSON-Definition `lva-workflow@1`, lineare Schritte mit Bedingungen, Vorlagen-
  Variablen (`{{meeting.title}}`), Warteschlange in SQLite, Idempotenz, Wiederholung, Wiederaufnahme nach Neustart,
  Freigabe-Zustand, Trockenlauf, Laufprotokoll.
- Auslöser: Termin beginnt/endet (Kalender aus dem Register, Filter Kalender/Titel/Teilnehmende), Besprechung
  fertig (Transkript final, KI-Notizen fertig, Protokoll fertig), Datei in Ordner (lokal/OneDrive-Sync), Zeitplan
  (täglich/wöchentlich), manuell, Agent (MCP/CLI).
- Aktionen: Aufnahme starten (mit Einwilligung) und stoppen, Datei importieren/transkribieren, KI-Notizen und
  Protokoll (Vorlage) erzeugen, Export (Word/PDF/Markdown) in Ordner-Integration, Mail senden (Empfängerregel ich /
  Teilnehmende / alle / feste Liste) oder Entwurf, Notiz in Termin schreiben, Obsidian-Notiz, Wissenssuche, lokale
  Windows-Mitteilung, Vorlesen-Seite + Audio, Webhook (z. B. n8n), Warten.
- Bedingungen: Kalender-ID, Teilnehmende intern/extern (Domänenliste), Dauer, Titel/Schlagwort, Vorlage, Ergebnis
  eines Vorschritts.
- Oberfläche: Liste, formularbasierter Editor (Auslöser → Schritte), Vorlagen, Laufprotokoll, Freigaben, JSON
  importieren/exportieren.
- Agenten: MCP/CLI `list_workflows`, `run_workflow`, `get_run`; n8n-Brücke (Webhook hinaus, Eingangsordner herein).

## Non-Scope
- Freier Graph-Editor (Canvas), Schleifen, parallele Zweige.
- Ausführung bei geschlossener App (Windows-Dienst); die App muss laufen (Tray/Autostart).
- Graph-Webhooks (brauchen öffentlichen HTTPS-Endpunkt); OneDrive nur über Sync-Ordner, Graph-delta optional später.
- Einbetten von n8n/Activepieces/Node-RED; Loopback-HTTP-Eingang für n8n (später, eigenes Paket).
- Lokaler LLM-Agent als Schritt (Goal C, hier nur Schnittstelle vorsehen).

## Akzeptanzkriterien
- [ ] AK1 — Engine: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib workflows::` → ≥ 30 Tests grün: Schema-Validierung, Bedingungen, Variablen ohne Code-Ausführung, Retry nur bei vorübergehenden Fehlern, Idempotenz (gleicher Auslöser zweimal → ein Lauf), Wiederaufnahme nach simuliertem Absturz ohne doppelte Außenwirkung.
- [ ] AK2 — Trockenlauf: `local-voice-ai.exe --workflow-run vorlage-besprechung.json --dry-run --json` (Sandbox) → JSON mit jedem Schritt, geplanter Wirkung und Rechte-Ergebnis; keine Datei, keine Mail, kein Modellstart.
- [ ] AK3 — Kalender-Auslöser: Test mit fester Uhr und Test-Kalender → „Termin beginnt“ feuert genau einmal je Termin und Ablauf, nicht für ganztägige/abgesagte, nicht während laufender Aufnahme; nutzt denselben Takt wie `calendar::reminder` (kein zweiter Poller).
- [ ] AK4 — Einwilligung: Ablauf mit „Aufnahme starten“ → ohne Bestätigung im Hinweisfenster startet keine Aufnahme; Lauf steht auf „wartet auf Freigabe“ und läuft nach Klick weiter (Playwright + Rust-Test).
- [ ] AK5 — Ordner-Auslöser: `--lib workflows::trigger::folder` → Datei, die in Teilen geschrieben wird, wird erst nach Stabilität verarbeitet; Umbenennen/Konfliktkopie erzeugt keinen zweiten Lauf; Dateiledger verhindert Doppelverarbeitung nach Neustart (≥ 10 Tests).
- [ ] AK6 — Vorlage „Eingangsordner → Word“: Sandbox-Ablauf mit Test-WAV im Eingangsordner → `.docx` im Zielordner (ZIP mit `word/document.xml`, Titel und Umlaute enthalten), Laufprotokoll „ok“.
- [ ] AK7 — Vorlage „Termin → Protokoll → Mail“: mit `--simulate-meeting`-Aufnahme und Test-SMTP/Test-Graph → Mail geht an die Empfängerregel des Kalenders (Test: Kalender A → nur ich; Kalender B → alle Teilnehmenden); Regel „fragen“ → Mail erst nach Freigabe.
- [ ] AK8 — Rechte: Aktion auf Integration mit Modus „aus“ → Schritt „abgelehnt“, Lauf endet sauber, Audit-Eintrag (A); Modus „fragen“ → Freigabe-Eintrag mit Vorschau (Empfänger, Betreff, Anhang).
- [ ] AK9 — Oberfläche: Playwright `automations.spec.ts` → Ablauf aus Vorlage anlegen, Auslöser/Bedingung/Schritt ändern, speichern, Trockenlauf anzeigen, Laufprotokoll und Freigabe bedienen; JSON exportieren und wieder importieren ergibt denselben Ablauf.
- [ ] AK10 — Agenten: `mcp_smoke.py --workflows` → `list_workflows`, `run_workflow` (Trockenlauf), `get_run` liefern erwartete Felder; `ctl workflow run <id> --json` → Exit 0; ohne Recht Exit 3.
- [ ] AK11 — Systemschutz: zwei gleichzeitig ausgelöste Abläufe mit Transkription laufen nacheinander (Test auf Warteschlange); bei RAM-Gate „zu wenig Speicher“ wartet der Schritt statt zu scheitern oder das System zu belasten.
- [ ] AK12 — Anfassbar: Screenshots (Liste, Editor, Lauf, Freigabe) und je ein echter Lauf beider Vorlagen im Installer (Patch +1), von Patrick abgenommen.

## Quality Gates
- [ ] QG1 — `cargo test --lib` gesamt grün; neue Dateien ohne neue Clippy-Warnungen.
- [ ] QG2 — `npx tsc --noEmit` Exit 0, Playwright-Suite grün, eslint/prettier nur berührte Dateien.
- [ ] QG3 — i18n de + en, echte Umlaute.
- [ ] QG4 — Sicherheitsreview (extern) für Rechte-/Freigabe-/Idempotenzlogik und Mail-Empfängerregeln.
- [ ] QG5 — Systemschutz: schwere Schritte seriell, RAM-Gate vor Modellstart, `process_guard` für jeden Kindprozess, Laufprotokoll-Aufbewahrung gedeckelt.
- [ ] QG6 — Doku `docs/AUTOMATIONEN.md` (Vorlagen, Bausteine, n8n-Brücke mit Beispiel), Hilfe-Text, Handoff.
- [ ] QG7 — Budget 2,6 MTok (Spanne 2,2–3,4); Meldung bei 50 % und 80 %, Stopp bei 150 %.

## Architektur-Skizze

### Module
```
src-tauri/src/managers/workflows/
  mod.rs       WorkflowManager (State): Start, Wiederaufnahme offener Läufe, Takt
  model.rs     Workflow, Trigger, Step, Condition, RunState (serde + specta), Schema lva-workflow@1
  expr.rs      Vorlagen-Variablen + Vergleiche (eigener kleiner Parser, keine Code-Ausführung)
  store.rs     Tabellen in meetings.db (nächste Migration nach A1)
  queue.rs     Warteschlange: seriell für Klasse „schwer“ (STT/LLM/TTS), leichte Schritte direkt
  runner.rs    Schrittausführung, Retry, Idempotenz, Freigabe über integrations::approvals
  trigger/{calendar.rs (hängt an calendar::service-Takt + reminder-Regeln), meeting_events.rs,
           folder.rs (notify-Crate, Debounce, Stabilität, Ledger), schedule.rs, manual.rs}
  actions/{recording.rs, import.rs, notes.rs, minutes.rs, export.rs, mail.rs, calendar_note.rs,
           obsidian.rs, notify.rs (tauri-plugin-notification, neu), tts.rs, webhook.rs, wait.rs}
  templates/   zwei Vorlagen als JSON (eingebettet)
commands/workflows.rs; src/components/automations/*; mcp/tools.rs + agent_bridge/tools.rs erweitert
```

### Datenmodell
```sql
CREATE TABLE workflows (id TEXT PRIMARY KEY, name TEXT NOT NULL, enabled INTEGER NOT NULL,
  definition_json TEXT NOT NULL, schema_version INTEGER NOT NULL, dry_run INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER, updated_at INTEGER);
CREATE TABLE workflow_runs (id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, trigger_key TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('queued','running','awaiting_approval','done','failed','cancelled')),
  context_json TEXT, started_at INTEGER, ended_at INTEGER, error TEXT,
  UNIQUE (workflow_id, trigger_key));                        -- Idempotenz
CREATE TABLE workflow_run_steps (run_id TEXT NOT NULL, step_id TEXT NOT NULL, attempt INTEGER NOT NULL,
  state TEXT NOT NULL, output_json TEXT, error TEXT, started_at INTEGER, ended_at INTEGER,
  PRIMARY KEY (run_id, step_id, attempt));
CREATE TABLE workflow_file_ledger (path_key TEXT PRIMARY KEY, size INTEGER, mtime INTEGER,
  content_hash TEXT, run_id TEXT, seen_at INTEGER);
```
Beispiel-Definition (gekürzt):
```json
{"schema":"lva-workflow@1","name":"Kundentermin protokollieren",
 "trigger":{"type":"calendar.event_starting","integration":"cal-graph-1","lead_min":1,"only_meetings":true},
 "steps":[
  {"id":"rec","action":"recording.start","params":{"stop":"event_end"}},
  {"id":"min","action":"meeting.minutes","params":{"template":"kunde"}},
  {"id":"doc","action":"export.document","params":{"format":"docx","target":"folder-onedrive-protokolle"}},
  {"id":"mail","action":"mail.send","when":"{{trigger.calendar}} == 'cal-graph-1'",
   "params":{"via":"m365-1","to":"participants","attach":"{{steps.doc.path}}"}}]}
```

### Rechte
Jede Aktion nennt Integration + Fähigkeit; `integrations::grants::effective_mode(…, Caller::Workflow)` entscheidet;
„fragen“ → `awaiting_approval` + lokale Mitteilung. Neue Abläufe starten im Trockenlauf, bis Patrick „scharf
schalten“ wählt. Empfänger kommen nur aus der Regel (ich/Teilnehmende/alle/feste Liste), nie aus Freitext eines
Modells.

### Schnittstellen (Entwurf)
```rust
pub trait Action { fn capability(&self) -> Option<(IntegrationRef, Capability)>; fn heavy(&self) -> bool;
  async fn run(&self, ctx: &RunCtx, params: &Value, dry: bool) -> Result<StepOutput, StepError>; }
pub enum StepError { Transient(String), Permanent(String), Denied, AwaitingApproval(String) }
#[tauri::command] workflows_list / workflow_save(def: Value) / workflow_delete(id) / workflow_run(id, dry: bool)
#[tauri::command] workflow_runs(filter) / workflow_run_detail(id) / workflow_templates()
```

## Paketschnitt (je 250–300 kTok)
| Paket | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|
| B1 | Engine-Kern: Modell, Schema, `expr`, Store, Warteschlange, Retry, Idempotenz, Wiederaufnahme, `--workflow-run --dry-run` | AK1, AK2, AK11 | A1 | lv-coder-xhigh |
| B2 | Auslöser Kalender, Besprechungsereignisse, Zeitplan, manuell; Einwilligungsweg | AK3, AK4 | B1 | lv-coder-xhigh |
| B3 | Ordner-Auslöser (Debounce, Stabilität, Ledger, OneDrive-Platzhalter) + Aktion Import/Transkription | AK5 | B1 | lv-coder |
| B4 | App-Aktionen: Notizen, Protokoll, Export in Ordner, TTS, lokale Mitteilung, Warten; Vorlage „Eingangsordner → Word“ | AK6 | B1, A4 | lv-coder |
| B5 | Integrations-Aktionen: Mail (Empfängerregeln, Freigabe), Termin-Notiz, Obsidian, Webhook; Vorlage „Termin → Mail“ | AK7, AK8 | B2, A3, A4, A5 | lv-coder-xhigh |
| B6 | Oberfläche Automationen: Liste, Formular-Editor, Vorlagen, Lauf, Freigaben, JSON-Import/Export | AK9 | B1, A2 | lv-coder |
| B7 | Agenten + n8n-Brücke: MCP/CLI-Werkzeuge, Beispiel-n8n-Workflow über Webhook/Austauschordner, Doku | AK10 | B1, A6 | lv-coder |
| B8 | Abnahme: Ende-zu-Ende beider Vorlagen, Sicherheitsreview, Screenshots, Installer | AK12, QG4, QG6 | alle | Planer + lv-architect |

## Budget
8 × ~275 kTok = 2,2 MTok + ~20 % → **2,6 MTok** (Spanne 2,2–3,4). Minimalschnitt: B1, B2, B4, B5, B6 (≈ 1,6 MTok).

## Risiken mit Vorschlag
- R1 Einwilligung/§ 201 StGB bei Auto-Aufnahme → Ein-Klick-Bestätigung zur Startzeit als Standard (AK4); Vorab-Bestätigung nur als bewusste Owner-Option mit Audit.
- R2 Falsche Mail an falsche Empfänger → Standard „Entwurf zur Freigabe“, automatisch nur „an mich“; Empfänger deterministisch; Trockenlauf beim Anlegen.
- R3 Kurzfristige Termine verpasst (Sync-Intervall) → Intervall sichtbar machen; bei Aufnahme-Abläufen Sync-Takt kurz vor vollen/halben Stunden verdichten (Vermutung, in B2 messen).
- R4 OneDrive-Eigenheiten (Teil-Schreibvorgänge, Platzhalter, Konfliktkopien) → Stabilitätsfenster + Ledger + Tests mit echtem Sync-Ordner in B3.
- R5 GPU/RAM-Überlast durch parallele Abläufe → Klasse „schwer“ seriell, RAM-Gate, Rückstau statt Abbruch.
- R6 Überschneidung mit vorhandener Automatik (`meeting_auto_enhance`, Erinnerung/Hinweisfenster) → vorhandene Automatik bleibt; Engine-Auslöser nutzen dieselben Ereignisse, keine Doppelung (Test: KI-Notizen laufen einmal).
- R7 Scope-Creep Richtung Zapier → nur lineare Abläufe, Bausteine erweiterbar über das `Action`-Trait.

## Owner-Entscheidungen (Patrick)
- E1 Eigene Rust-Engine im App-Prozess, n8n nur als Brücke (Webhook/Austauschordner), kein Einbetten — **Empfehlung: ja** (Lizenz, keine Docker-Pflicht, Systemschutz).
- E2 Auto-Aufnahme: Aufnahme startet erst nach Ein-Klick-Bestätigung im Hinweisfenster — **Empfehlung: ja**; Vorab-Bestätigung je Kalender nur, wenn du es ausdrücklich willst.
- E3 Mail-Automatik: an mich automatisch, an Teilnehmende/alle nur nach Freigabe (je Ablauf änderbar) — **Empfehlung: so**.
- E4 Ort in der Oberfläche: eigener Navigationseintrag „Automationen“ direkt unter „Integrationen“ oder Reiter auf der Seite Integrationen — **Empfehlung: eigener Eintrag** (Arbeitsbereich mit Läufen, keine Einstellung).
- E5 App muss laufen (Tray/Autostart), kein Windows-Dienst — **Empfehlung: ja**.
- E6 OneDrive über den lokalen Sync-Ordner, Graph-delta später — **Empfehlung: ja**.

## Abhängigkeiten
- **Goal A**: A1 (Grants, Audit, Approvals) vor B1; A3/A4/A5 vor B4/B5; A6 vor B7; A2 vor B6 (gemeinsame Bausteine).
- **Goal C** liefert später den Schritttyp `agent.*`; B1 sieht dafür das `Action`-Trait vor.
- **aufnahmen-ui**: Sessions-Modell (M4-Ordner) als Ziel „Besprechung in Session X ablegen“; keine UI-Überschneidung.
- Vorhanden: Kalender P5a/P5b/P5f, Export P6a/P6b, Mail P6c, KI-Notizen/Protokoll (M7, P1k).
