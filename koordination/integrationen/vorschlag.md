# Vorschlag Goal „Integrationen“ — Register für Kalender, Mail, Speicher, Wissen und Agenten

Grundlage: `recherche/muster-und-quellen.md` (Stand 30.09.2026). Für den Planer zum Übertragen in `GOAL.md`.

## Zielzustand
Eine neue Seite „Integrationen“ zwischen „Modelle“ und „Einstellungen“ verwaltet beliebig viele Verbindungen —
Kalender (ICS, Microsoft 365), Postfächer (Microsoft 365, SMTP), Ordner (lokal, OneDrive-Sync), Obsidian-Vault,
WAI-Wissensbasis und Agentenzugänge (MCP/CLI) — mit Richtung und einem Recht je Fähigkeit (aus / fragen / erlaubt);
die bisherigen Kalenderquellen, der MCP-Schalter, die Follow-up-Mail und der Export nutzen dieses Register statt
eigener Einstellungen. Erkennbar daran, dass ein externer Agent per MCP oder `local-voice-ai.exe ctl` eine Datei
transkribieren oder eine Vorlesen-Seite mit Audio anlegen kann, nur wenn das Werkzeug freigegeben ist, und jede
solche Aktion im Audit-Protokoll der Seite steht.

## Scope
- Register-Kern: Datenmodell, Migration der vorhandenen `calendar_sources`, verallgemeinerter DPAPI-Geheimnisspeicher,
  Rechte (Richtung × Fähigkeit × Aufrufer), Audit-Log, Freigabe-Warteschlange.
- Seite „Integrationen“: Liste, Katalog „Integration hinzufügen“ mit Assistent je Art, Detail (Richtung,
  Fähigkeiten-Matrix, Verbindung testen, Protokoll), Freigabedialog.
- Integrationsarten: ICS-Kalender (vorhanden), Microsoft-365-Konto (Kalender lesen/schreiben, Mail senden, OneDrive
  lesen/schreiben), SMTP-Postfach (senden, App-Passwort), Ordner (lokal / OneDrive-Sync), Obsidian-Vault (Notizen
  schreiben mit AI-OS-Frontmatter), WAI-Wissensbasis (MCP-Client, suchen/lesen), Agentenzugang (MCP/CLI-Clients mit
  Token), individuell: Webhook (HTTP POST) und eigener MCP-Server (HTTP).
- Agentensteuerung: Named-Pipe-Kanal zur laufenden App; MCP-Werkzeuge schreibend (Aufnahme starten/stoppen, Datei
  transkribieren, Session anlegen, Besprechung anlegen/importieren, Vorlesen-Seite anlegen, Audio erzeugen) und
  `ctl`-CLI mit JSON-Ausgabe und Exit-Codes.
- Überführung: Kalender-UI und MCP-Schalter ziehen auf die neue Seite; Follow-up-Mail bietet „senden über …“;
  Export bietet „ablegen in …“.

## Non-Scope
- Workflow-Engine, Trigger, Automationen (Goal B) und lokaler LLM-Agent (Goal C).
- Google-OAuth (Kalender/Gmail/Drive) — Gmail läuft über SMTP mit App-Passwort; OAuth als Folgepaket.
- IMAP-Lesen von Postfächern, Graph-Mail-Lesen (nur Senden in diesem Goal).
- Schreib-Endpunkt `wissen:write` im AI-OS-Repo (Wissen fließt über den Vault ein).
- macOS-Kanal (Unix-Socket, Keychain) — Schnittstelle so schneiden, dass er später passt.
- DLP-Klassen („geschäftlich“/„privat“ nicht mischen) — nur Datenfeld vorsehen.

## Akzeptanzkriterien
- [ ] AK1 — Register-Kern: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib integrations::` → ≥ 25 Tests grün, darunter Migrationstest: eine Fixture-DB mit 2 `calendar_sources` ergibt 2 Integrationen gleicher ID; zweiter Start erzeugt keine Dubletten.
- [ ] AK2 — Dump: `local-voice-ai.exe --integrations-dump --json` (Sandbox `LVA_MEETINGS_DIR`) → JSON mit Art, Richtung, Fähigkeiten, Modus je Aufrufer; Test prüft, dass kein Geheimnis (Token-/Passwortmuster, ICS-URL) enthalten ist.
- [ ] AK3 — Seite: Playwright `integrations.spec.ts` → Navigationseintrag „Integrationen“ steht zwischen „Modelle“ und „Einstellungen“; Katalog zeigt ≥ 8 Arten; Ordner-Integration anlegen, Richtung „nur lesen“ und Fähigkeit „fragen“ setzen, Neuladen → Zustand erhalten; löschen mit Rückfrage.
- [ ] AK4 — Überführung: Playwright → Kalenderquellen erscheinen als Karten und lassen sich dort synchronisieren; der MCP-Schalter unter Einstellungen > Besprechungen ist durch einen Verweis ersetzt; `meeting_mcp_enabled` wirkt unverändert (Rust-Test).
- [ ] AK5 — Microsoft 365: `--lib integrations::m365` → ≥ 12 Tests gegen Test-HTTP-Server (Scopes nur für eingeschaltete Fähigkeiten, `sendMail`, OneDrive-Upload klein und per Upload-Session, Termin-Notiz, 401→Refresh, 403→Klartextmeldung); manuell Patrick: Testmail an sich, Datei im OneDrive-Ordner, Notiz im Termin.
- [ ] AK6 — SMTP + Ordner: `--lib integrations::smtp integrations::folder` → Versand an lokalen Test-SMTP-Server; Pfad-Sandbox lehnt `..`, absolute Fremdpfade, Junction/Symlink nach außen ab (≥ 10 Tests).
- [ ] AK7 — Obsidian + Wissen: Golden-Test → Notiz mit `title, tags, context_area, data_class, sensitivity, tier` im Sandbox-Vault, kein Überschreiben ohne Recht; `wissen_suchen` gegen Test-MCP-Server liefert Treffer, falscher Scope → verständliche Meldung; manuell: Suche gegen die laufende Wissensbasis.
- [ ] AK8 — Agentenbrücke: `--lib agent_bridge::` → ≥ 15 Tests: Pipe nur für aktuellen Benutzer, entfernte Clients abgewiesen, ungültiger/zurückgezogener Token → abgelehnt + Audit, Modus „aus“ → Werkzeug fehlt in `tools/list`, „fragen“ → Freigabe, keine Antwort in 30 s → `pending` mit Freigabe-ID.
- [ ] AK9 — MCP schreibend: `python apps/local-voice/scripts/mcp_smoke.py --write` gegen Release-Binary + laufende Sandbox-App → Exit 0: `transcribe_file` (Test-WAV) liefert `meeting_id`; `tts_page_create` + `tts_render_audio` liefern WAV-Pfad; `start_recording` ohne Einwilligung in der App führt nie zu einer laufenden Aufnahme.
- [ ] AK10 — CLI: `local-voice-ai.exe ctl status --json` und `ctl transcribe <wav> --json` → Exit 0 mit JSON; ohne laufende App Exit 2 und Hinweis; Werkzeug „aus“ → Exit 3.
- [ ] AK11 — Audit: alle Aktionen aus AK5–AK10 stehen in `--audit-dump --json` und in der UI-Ansicht; Aufbewahrung gedeckelt (Test: älteste Einträge werden über der Grenze gelöscht).
- [ ] AK12 — Anfassbar: Screenshots (Liste, Katalog, Detail mit Rechte-Matrix, Freigabedialog, Audit) als Artefakt; Installer mit Patch-Version +1 von Patrick abgenommen.

## Quality Gates
- [ ] QG1 — `cargo test --lib` gesamt grün (Sandbox, `CARGO_BUILD_JOBS=8`); vorbestehendes Clippy-Rot bleibt, neue Dateien ohne neue Warnungen.
- [ ] QG2 — `npx tsc --noEmit` Exit 0; Playwright-Suite grün; eslint/prettier nur berührte Dateien.
- [ ] QG3 — i18n de + en für alle neuen Schlüssel, echte Umlaute.
- [ ] QG4 — Sicherheitsreview (extern, Codex) für Rechteprüfung, Pipe, Token, Freigabe (A1, A6, A7) — das sind Gate-/Sicherheitslogik im Sinne der Budgetregel.
- [ ] QG5 — Systemschutz: kein neuer Kindprozess ohne `process_guard`; Werkzeuge mit Modellstart gehen durch das RAM-Gate; Aufruf-Obergrenze je Client/Minute.
- [ ] QG6 — Doku: `docs/INTEGRATIONEN.md` (Arten, Rechte, Agentenzugang einrichten mit Beispiel `claude mcp add`), Hilfe-Text der Seite, Handoff.
- [ ] QG7 — Budget: 2,7 MTok (Spanne 2,3–3,5); Meldung bei 50 % und 80 %, harter Stopp bei 150 %.

## Architektur-Skizze

### Module (Anschluss an vorhandene Muster)
```
src-tauri/src/managers/integrations/
  mod.rs        IntegrationManager (State), Start: Migration + Übernahme calendar_sources
  model.rs      Integration, IntegrationKind, Direction, Capability, Caller, GrantMode (serde + specta)
  store.rs      SQL in meetings.db (rusqlite_migration, nächster Index nach CALENDAR_MIGRATION)
  grants.rs     effective_mode(integration, capability, caller) -> GrantMode   (rein, testbar)
  audit.rs      append/list/prune
  approvals.rs  Freigabe-Warteschlange (pending/approved/denied/expired)
  kinds/{ics.rs → delegiert an calendar::, m365.rs (Graph: calendar/mail/files), smtp.rs (lettre, Lizenz prüfen),
         folder.rs (Pfad-Sandbox), obsidian.rs (Frontmatter-Vertrag AI-OS), wissen.rs (HTTP-MCP-Client), webhook.rs}
managers/calendar/secret.rs → Namensraum je Integration (Entropy-Präfix alt für Kalender beibehalten, kein Neu-Login)
src-tauri/src/agent_bridge/  pipe.rs (Named Pipe, DACL aktueller Benutzer, PIPE_REJECT_REMOTE_CLIENTS),
                             protocol.rs (JSON-Zeilen, versioniert), tools.rs (Werkzeug → App-Funktion + Capability)
src-tauri/src/mcp/tools.rs   + schreibende Werkzeuge: leiten über Pipe an die laufende App weiter
src-tauri/src/cli.rs          + Unterbefehl `ctl <verb>` (Client der Pipe), `--integrations-dump`, `--audit-dump`
src-tauri/src/commands/integrations.rs, src/components/integrations/*, Sidebar-Eintrag `integrations`
```

### Datenmodell (meetings.db, eine Migration)
```sql
CREATE TABLE integrations (id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1, direction TEXT NOT NULL CHECK (direction IN ('read','write','both')),
  config_json TEXT NOT NULL DEFAULT '{}',   -- nie Geheimnisse; Geheimnis unter secrets/<id>.bin
  account_hint TEXT, data_class TEXT, created_at INTEGER, updated_at INTEGER,
  last_ok_at INTEGER, last_error TEXT);
-- kind ics/graph: id == calendar_sources.id (1:1, calendar_sources bleibt Betriebstabelle der Termine)
CREATE TABLE integration_grants (integration_id TEXT NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
  capability TEXT NOT NULL, caller TEXT NOT NULL CHECK (caller IN ('workflow','agent_external','agent_local')),
  mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')), PRIMARY KEY (integration_id, capability, caller));
CREATE TABLE agent_clients (id TEXT PRIMARY KEY, label TEXT NOT NULL, token_hash TEXT NOT NULL,
  created_at INTEGER, last_used_at INTEGER, revoked_at INTEGER);
CREATE TABLE agent_tool_grants (client_id TEXT NOT NULL REFERENCES agent_clients(id) ON DELETE CASCADE,
  tool TEXT NOT NULL, mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')), PRIMARY KEY (client_id, tool));
CREATE TABLE audit_log (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, caller TEXT NOT NULL, integration_id TEXT,
  capability TEXT, target TEXT, outcome TEXT NOT NULL CHECK (outcome IN ('ok','denied','error','pending')),
  detail_json TEXT);  -- Ziel/Details gekürzt, nie Inhalt oder Geheimnis
CREATE TABLE approvals (id TEXT PRIMARY KEY, created_at INTEGER, caller TEXT, tool_or_capability TEXT,
  args_preview TEXT, state TEXT CHECK (state IN ('pending','approved','denied','expired')), decided_at INTEGER);
```

### Rechte
- Wirksamer Modus = min(Richtung erlaubt die Fähigkeit?, `integration_grants` für Aufrufer, bei externen Agenten
  zusätzlich `agent_tool_grants`). Die UI selbst (Patrick klickt) braucht keine Freigabe.
- Standard für neue Integrationen: lesende Fähigkeiten `allow` für `workflow`, alles Schreibende `ask`; für
  `agent_external` alles `off` bis zur Freigabe.
- **Aufnahme starten** ist nie `allow`-fähig: die App zeigt immer ihren Einwilligungsdialog (§ 201 StGB).
- Pfade (transcribe_file, Ordner) nur innerhalb freigegebener Ordner-Integrationen oder mit Freigabe.

### Schnittstellen (Entwurf)
```rust
pub fn effective_mode(i: &Integration, cap: Capability, caller: Caller, grants: &GrantSet) -> GrantMode;
pub trait IntegrationKindImpl { fn capabilities(&self) -> &'static [Capability];
  async fn test(&self, i: &Integration) -> Result<TestReport, IntegrationError>; }
#[tauri::command] integrations_list() -> Result<Vec<Integration>, String>
#[tauri::command] integrations_catalog() -> Vec<IntegrationKindInfo>
#[tauri::command] integration_create(kind: IntegrationKind, label: String, config: serde_json::Value) -> Result<Integration, String>
#[tauri::command] integration_update(id: String, patch: IntegrationPatch) -> Result<Integration, String>
#[tauri::command] integration_delete(id: String) -> Result<(), String>
#[tauri::command] integration_set_grant(id: String, capability: Capability, caller: Caller, mode: GrantMode) -> Result<(), String>
#[tauri::command] integration_test(id: String) -> Result<TestReport, String>
#[tauri::command] integration_connect_m365(id: String) -> Result<(), String>   // PKCE-Fluss aus graph.rs
#[tauri::command] agent_client_create(label: String) -> Result<(AgentClient, String /*Token, einmalig*/), String>
#[tauri::command] approvals_pending() / approval_decide(id: String, approve: bool)
#[tauri::command] audit_list(filter: AuditFilter) -> Result<Vec<AuditEntry>, String>
```
MCP-Werkzeuge (neu, `destructiveHint`/`readOnlyHint` gesetzt, Prüfung trotzdem serverseitig): `start_recording`,
`stop_recording`, `transcribe_file`, `create_session`, `create_meeting`, `tts_page_create`, `tts_render_audio`,
`get_action_status`. Bindings (`bindings.ts`) wie im Repo von Hand nachziehen.

## Paketschnitt (je 250–300 kTok)
| Paket | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|
| A1 | Register-Kern: Migration, Übernahme Kalender, Grants, Audit, Approvals, Geheimnis-Namensraum, `--integrations-dump` | AK1, AK2, AK11 (Kern) | – | lv-coder-xhigh (Migration) |
| A2 | Seite Integrationen: Navigation, Liste, Katalog, Assistent-Gerüst, Detail mit Rechte-Matrix, Audit-Ansicht; Kalender/MCP-Umzug | AK3, AK4 | A1 | lv-coder |
| A3 | Microsoft-365-Konto: Scopes je Fähigkeit, Mail senden, OneDrive ablegen, Termin-Notiz | AK5 | A1 | lv-coder-xhigh |
| A4 | SMTP-Postfach, Ordner-Integration (Sandbox), Webhook; Follow-up-Mail „senden über“, Export „ablegen in“ | AK6 | A1, A2 | lv-coder |
| A5 | Obsidian-Vault (Frontmatter-Vertrag) + WAI-Wissensbasis (HTTP-MCP-Client) + eigener MCP-Server | AK7 | A1 | lv-coder |
| A6 | Agentenbrücke: Named Pipe, Client-Token, Werkzeug-Rechte, Freigabedialog, `ctl`-CLI | AK8, AK10 | A1 | lv-coder-xhigh |
| A7 | MCP schreibend: Werkzeuge, Protokollversion 2026-07-28 prüfen, `mcp_smoke.py --write` | AK9 | A6, aufnahmen-ui M3 (Sessions) | lv-coder-xhigh |
| A8 | Abnahme: Doku, Hilfe, Sicherheitsreview, Screenshots, Installer | AK12, QG4, QG6 | alle | Planer + lv-architect |

## Budget
8 Pakete × ~275 kTok = 2,2 MTok + Reviews/Nacharbeit ~20 % → **2,7 MTok** (Spanne 2,3–3,5).
Minimalschnitt, falls knapp: A1, A2, A4, A6, A7 (≈ 1,7 MTok) — M365-Schreiben und Wissen folgen.

## Risiken mit Vorschlag
- R1 Migration der Kalenderquellen beschädigt Termine/Links → 1:1-ID, `calendar_sources` bleibt; Migrationstest mit Kopie echter Struktur; Backup vor Migration (Muster `settings_store.json.bak`).
- R2 Graph-Scopes: Entra-Registrierung fehlt `Mail.Send`/`Files.ReadWrite`/`Calendars.ReadWrite` → Owner-Aufgabe vor A3; ohne sie bleibt A3 bei Lesen.
- R3 Prompt-Injection über Besprechungsinhalte an externe Agenten (Rule of Two) → Schreibendes standardmäßig „fragen“, Freigabe in der App, keine freien Empfänger.
- R4 MCP-Spec-Wechsel (2026-07-28: `_meta` je Anfrage, zustandslos) bricht ältere/neuere Clients → Versionsverhandlung testen (Claude Code, Codex) in A7.
- R5 Konflikt mit aufnahmen-ui (Sidebar, Sessions-API) → A2 nach aufnahmen-ui M2 mergen; A7 nutzt deren Sessions-Modell (M4-Ordner).
- R6 Zwei Modellstarts (headless-CLI + App) sprengen VRAM → `ctl` spricht immer mit der laufenden App, nie eigener Modellstart.
- R7 Umfang „keine Mengenbegrenzung“ → keine künstliche Grenze, aber Sync/Tests je Integration zeitlich gestaffelt.

## Owner-Entscheidungen (Patrick)
- E1 Register in `meetings.db` (gleiche Migrationskette, MCP liest mit) statt eigener Datenbank — **Empfehlung: ja**.
- E2 Wissen in den RAG über den Vault (Frontmatter + AI-OS-Index) statt neuem `wissen:write`-Endpunkt — **Empfehlung: Vault jetzt, Endpunkt später im AI-OS-Goal**.
- E3 Standardrechte: schreibend „fragen“, externe Agenten „aus“; Aufnahme starten nie ohne Einwilligungsdialog — **Empfehlung: so**.
- E4 Entra-App um `Calendars.ReadWrite`, `Mail.Send`, `Files.ReadWrite` erweitern (Patrick im Azure-Portal) — **Empfehlung: vor A3**.
- E5 Google-OAuth später; Gmail über SMTP-App-Passwort — **Empfehlung: so**.
- E6 Kalender- und MCP-Einstellungen ziehen ganz auf die neue Seite (alter Ort nur Verweis) — **Empfehlung: ja**.
- E7 Datenklasse je Integration (Standard für Vault-Notizen aus Besprechungen: `confidential`) — **Empfehlung: confidential**, weil Aussagen Dritter; Maschinen-Keys sehen sie dann nicht (gewollt).

## Abhängigkeiten
- **aufnahmen-ui** (läuft): Sidebar und Sessions (= M4-Ordner, R2 dort). A1/A3/A5/A6 sind Backend und können parallel
  laufen; A2 nach aufnahmen-ui M2, A7 nach aufnahmen-ui M3.
- **Goal B** braucht A1 (Grants/Audit/Approvals) und die Integrationsarten aus A3–A5 als Aktionen.
- **Goal C** braucht A1 (Aufrufer `agent_local`) und A5 (Vault/Wissen).
- Basis: `feat/granola-besprechungen` inkl. P5/P6-Stand (Kalender, MCP, Mail, Export).
