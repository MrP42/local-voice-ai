# Vorschlag Goal „Integrationen“ — YouTube zuerst, dann Register für Kalender, Mail, Speicher, Wissen und Agenten

Grundlage: `recherche/muster-und-quellen.md` (Stand 30.09.2026, inkl. §9 YouTube und §10 Provenienz).
Für den Planer zum Übertragen in `GOAL.md`. Zusatz Patrick vom 30.09.: YouTube ist die **erste** Integration und
das erste lieferbare Paket-Bündel; Provenienz ist Querschnitt für A, B und C.

## Zielzustand
Ein YouTube-Link lässt sich in „Aufnahmen“ einfügen und wird zur Quelle: Video in der App ansehen, Untertitel (falls
verfügbar) und eigene Transkription nebeneinander vergleichen, eine Fassung wählen oder per KI zusammenführen und
zusammenfassen — und zu jedem erzeugten Inhalt zeigt ein Rechtsklick „Herkunft“ Modell, Token, Dauer, Zeitpunkt,
Quellen, Konfidenz und Auslöser. Danach verwaltet eine neue Seite „Integrationen“ zwischen „Modelle“ und
„Einstellungen“ beliebig viele Verbindungen (YouTube, Kalender, Postfächer, Ordner/OneDrive, Obsidian-Vault,
WAI-Wissensbasis, Agentenzugänge) mit Richtung und einem Recht je Fähigkeit (aus / fragen / erlaubt), und externe
Agenten steuern die App über MCP und `local-voice-ai.exe ctl` nur im Rahmen dieser Rechte, jede Aktion im Audit-Log.

## Scope
**Bündel 1 — zuerst lieferbar (A1–A3):**
- Fundament: Register-Kern (Datenmodell, Übernahme der `calendar_sources`, Rechte, Audit) und **Provenienz**
  (`provenance`-Tabelle, Verweis auf `usage_event`, Rückfall auf `generation_metadata_json`).
- YouTube-Quelle: Link (Video) in Aufnahmen einfügen → Besprechung mit Quelle `youtube`, Metadaten (Titel, Kanal,
  Vorschaubild, Dauer), Ansehen in der App im eingebetteten YouTube-Player; Audio-/Dateiweg je Owner-Entscheidung E1.
- Untertitel (manuell/automatisch, Sprachwahl) laden, sofern der entschiedene Weg das erlaubt; unabhängig davon
  eigene Transkription; **Vergleichsansicht** (Wort-Diff), Fassung wählen oder **KI-Zusammenführung** (Schema-gebunden,
  mit Provenienz); Zusammenfassung über den vorhandenen KI-Notizen-/Protokollpfad.
- Kontextmenü „Herkunft“ an Transkript, KI-Notizen, Protokoll, Zusammenfassung (Dialog).

**Bündel 2 (A4–A8):**
- Seite „Integrationen“: Liste, Katalog mit Assistent je Art, Detail (Richtung, Fähigkeiten-Matrix, Test, Protokoll),
  Freigabedialog; Umzug von Kalender-UI und MCP-Schalter.
- Microsoft-365-Konto (Kalender lesen/schreiben, Mail senden, OneDrive), SMTP-Postfach (App-Passwort), Ordner
  (lokal/OneDrive-Sync), Obsidian-Vault (AI-OS-Frontmatter), WAI-Wissensbasis (MCP-Client, suchen/lesen).
- Agentensteuerung: Named Pipe zur laufenden App, Client-Token, Rechte je Werkzeug; MCP schreibend (Aufnahme
  starten/stoppen, Datei transkribieren, Session/Besprechung anlegen, Vorlesen-Seite + Audio, YouTube-Link als Quelle
  anlegen) und `ctl`-CLI.

## Non-Scope
- **Playlist** (Folgeschritt, eigenes Paket A9 nach Bündel 1, nicht im Budget).
- Werbung im eingebetteten Player blockieren oder verändern (Developer Policies, BGH I ZR 131/23).
- Bündeln von yt-dlp/Deno im Installer (siehe E1).
- Workflow-Engine (Goal B), lokaler LLM-Agent (Goal C), Google-OAuth, IMAP-Lesen, `wissen:write` im AI-OS-Repo,
  Webhook- und „eigener MCP-Server“-Integration (nach B verschoben), macOS-Kanal, DLP-Klassen.

## Akzeptanzkriterien
- [ ] AK1 — Fundament: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib integrations:: provenance::` → ≥ 30 Tests grün, u. a. Migrationstest (Fixture mit 2 `calendar_sources` → 2 Integrationen gleicher ID, zweiter Start ohne Dubletten) und Provenienz: Protokoll-Erzeugung legt einen Eintrag mit Modell, Token, Dauer, `usage_event_id` an; alte Dokumente liefern Herkunft aus `generation_metadata_json`.
- [ ] AK2 — YouTube-Quelle: Playwright `youtube-source.spec.ts` → Link `https://www.youtube.com/watch?v=…` (auch `youtu.be/…`, `shorts/…`) in Aufnahmen einfügen erzeugt eine Besprechung mit Quelle „YouTube“, Titel und Kanal; ungültige/Playlist-Links → verständliche Meldung; Rust-Test für Link-Normalisierung (≥ 10 Fälle).
- [ ] AK3 — Ansehen: Installer, echtes Video → Player in der Inhaltsspalte spielt ab, Position springt beim Klick auf ein Transkript-Segment; kein verschachteltes iframe, keine Veränderung der Werbung (Code-Review-Punkt).
- [ ] AK4 — Untertitel + eigene Transkription: für ein Video mit Untertiteln liegen beide Fassungen an derselben Besprechung (Quelle je Fassung sichtbar); ohne Untertitel → nur eigene Fassung mit Hinweis; Sprachauswahl bei mehreren Spuren (Rust-Tests gegen Fixture-VTT, manuell 1 Video).
- [ ] AK5 — Vergleich/Zusammenführen: Playwright → Diff-Ansicht markiert Einfügungen/Löschungen wortweise; „Fassung wählen“ setzt das aktive Transkript; „Zusammenführen“ erzeugt eine dritte Fassung mit Provenienz (Quellen = beide Fassungen, Modell, Token); Rust-Test: Zusammenführung verwirft Ausgaben, die das Schema verletzen oder > 20 % Text erfinden (Längen-/Überdeckungsprüfung).
- [ ] AK6 — Zusammenfassung + Herkunft: Zusammenfassung eines YouTube-Videos wird erzeugt; Rechtsklick „Herkunft“ auf Transkript, Zusammenfassung und Protokoll öffnet Dialog mit Modell, Token, Dauer, Zeitpunkt, Quellen, Konfidenz (falls vorhanden) und Auslöser (Playwright).
- [ ] AK7 — Seite Integrationen: Playwright `integrations.spec.ts` → Eintrag zwischen „Modelle“ und „Einstellungen“; Katalog ≥ 7 Arten; Ordner-Integration anlegen, Richtung/Fähigkeitsmodus ändern übersteht Neuladen; Kalenderquellen als Karten; MCP-Schalter unter Einstellungen > Besprechungen durch Verweis ersetzt, `meeting_mcp_enabled` wirkt unverändert.
- [ ] AK8 — Konten und Ziele: `--lib integrations::m365 integrations::smtp integrations::folder integrations::obsidian integrations::wissen` → ≥ 30 Tests gegen Test-Server/Sandbox (Scopes nur für eingeschaltete Fähigkeiten, `sendMail`, OneDrive-Upload klein und per Upload-Session, 401→Refresh; SMTP an Test-Server; Pfad-Sandbox gegen `..`/Junction; Vault-Notiz mit Frontmatter-Golden; `wissen_suchen` mit Scope-Fehler-Meldung); manuell Patrick: Testmail, Datei in OneDrive, Suche in der Wissensbasis.
- [ ] AK9 — Agentenbrücke: `--lib agent_bridge::` → ≥ 15 Tests (Pipe nur aktueller Benutzer, Remote abgewiesen, Token ungültig/zurückgezogen → abgelehnt + Audit, „aus“ → Werkzeug fehlt in `tools/list`, „fragen“ → Freigabe, 30 s ohne Antwort → `pending` + ID).
- [ ] AK10 — MCP/CLI schreibend: `python apps/local-voice/scripts/mcp_smoke.py --write` gegen Release-Binary + laufende Sandbox-App → Exit 0 (`transcribe_file` → `meeting_id`; `tts_page_create` + `tts_render_audio` → WAV; `add_youtube_source` → Besprechung; `start_recording` ohne Einwilligung in der App startet nie eine Aufnahme); `ctl status --json` Exit 0, ohne App Exit 2, Werkzeug „aus“ Exit 3.
- [ ] AK11 — Audit: alle Aktionen aus AK8–AK10 in `--audit-dump --json` und in der UI; Aufbewahrung gedeckelt (Test).
- [ ] AK12 — Anfassbar: nach Bündel 1 Installer (Patch +1) mit YouTube-Ablauf und Screenshots (Player, Diff, Herkunft) — **erste Abnahme durch Patrick**; nach Bündel 2 Installer + Screenshots (Liste, Katalog, Rechte-Matrix, Freigabe, Audit).

## Quality Gates
- [ ] QG1 — `cargo test --lib` gesamt grün (Sandbox, `CARGO_BUILD_JOBS=8`); neue Dateien ohne neue Clippy-Warnungen.
- [ ] QG2 — `npx tsc --noEmit` Exit 0; Playwright-Suite grün; eslint/prettier nur berührte Dateien.
- [ ] QG3 — i18n de + en, echte Umlaute.
- [ ] QG4 — Externes Sicherheitsreview für Rechte, Pipe, Token, Freigabe (A1, A7, A8) und für den YouTube-Dateiweg (Prozessaufruf, Pfade).
- [ ] QG5 — Systemschutz: jeder Kindprozess (z. B. externes yt-dlp, falls E1 so entschieden) über `process_guard`; Modellstarts über das RAM-Gate; Aufruf-Obergrenzen je Agent-Client.
- [ ] QG6 — Doku: `docs/INTEGRATIONEN.md` (YouTube inkl. Rechtshinweis, Rechte, Agentenzugang mit `claude mcp add`-Beispiel), Hilfe-Texte, Handoff.
- [ ] QG7 — Budget 2,7 MTok (Spanne 2,3–3,5); Meldung bei 50 % und 80 %, harter Stopp bei 150 %.

## Architektur-Skizze

### Module (Anschluss an vorhandene Muster)
```
src-tauri/src/managers/provenance/   mod.rs (record/list), model.rs            ← Querschnitt A/B/C
src-tauri/src/managers/integrations/
  mod.rs, model.rs, store.rs, grants.rs (effective_mode, rein), audit.rs, approvals.rs
  kinds/youtube.rs   Link-Normalisierung, oEmbed-Metadaten, Untertitel-/Dateiweg (Adapter je E1)
  kinds/{m365.rs, smtp.rs, folder.rs, obsidian.rs, wissen.rs}; ics/graph delegieren an managers/calendar
managers/meetings/: neue Quelle `youtube` (meetings.source), Transkript-Fassungen (s. u.), Merge-Aufruf über llm_call
managers/calendar/secret.rs → Namensraum je Integration (Kalender-Präfix bleibt, kein Neu-Login)
src-tauri/src/agent_bridge/  pipe.rs (DACL aktueller Benutzer, PIPE_REJECT_REMOTE_CLIENTS), protocol.rs, tools.rs
src-tauri/src/mcp/tools.rs   + schreibende Werkzeuge (leiten über die Pipe an die laufende App)
src-tauri/src/cli.rs         + `ctl <verb>`, `--integrations-dump`, `--audit-dump`
src/components/integrations/*, src/components/workspace/meetings/youtube/* (Player, Diff), Kontextmenü „Herkunft“
```

### Datenmodell (meetings.db, Migrationen nach dem aktuellen Index)
```sql
-- Provenienz (Querschnitt): je erzeugtem Inhalt ein oder mehrere Einträge
CREATE TABLE provenance (id TEXT PRIMARY KEY, subject_kind TEXT NOT NULL CHECK (subject_kind IN
  ('transcript','transcript_variant','document','summary','knowledge_note','tts_audio','export','run_output')),
  subject_id TEXT NOT NULL, subject_revision INTEGER, created_at INTEGER NOT NULL,
  operation TEXT NOT NULL,               -- stt, subtitles_import, merge, summary, minutes, notes, relevance, reconcile, factcheck, …
  actor_kind TEXT NOT NULL CHECK (actor_kind IN ('user','auto','workflow','agent_external','agent_local')),
  actor_ref TEXT,                        -- workflow-/run-/client-ID
  provider TEXT, model_id TEXT, model_label TEXT,
  usage_event_id INTEGER,                -- Verweis in usage.db (Kosten/Preise bleiben dort)
  prompt_tokens INTEGER, completion_tokens INTEGER, duration_ms INTEGER,   -- Kopie für Offline-Anzeige
  sources_json TEXT NOT NULL DEFAULT '[]', -- [{kind: youtube|subtitle|meeting|transcript|rag|vault|web, ref, title, url}]
  confidence REAL, params_json TEXT);
CREATE INDEX provenance_subject ON provenance(subject_kind, subject_id);
-- Transkript-Fassungen (Untertitel / eigene STT / Zusammenführung); das aktive Transkript bleibt in `transcripts`
CREATE TABLE transcript_variants (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, kind TEXT NOT NULL CHECK (kind IN
  ('subtitles_manual','subtitles_auto','stt','merged')), language TEXT, segments_json TEXT NOT NULL,
  created_at INTEGER NOT NULL, active INTEGER NOT NULL DEFAULT 0);
-- Register (wie gehabt)
CREATE TABLE integrations (id TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
  direction TEXT NOT NULL CHECK (direction IN ('read','write','both')), config_json TEXT NOT NULL DEFAULT '{}',
  account_hint TEXT, data_class TEXT, created_at INTEGER, updated_at INTEGER, last_ok_at INTEGER, last_error TEXT);
CREATE TABLE integration_grants (integration_id TEXT NOT NULL REFERENCES integrations(id) ON DELETE CASCADE,
  capability TEXT NOT NULL, caller TEXT NOT NULL CHECK (caller IN ('workflow','agent_external','agent_local')),
  mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')), PRIMARY KEY (integration_id, capability, caller));
CREATE TABLE agent_clients (id TEXT PRIMARY KEY, label TEXT NOT NULL, token_hash TEXT NOT NULL,
  created_at INTEGER, last_used_at INTEGER, revoked_at INTEGER);
CREATE TABLE agent_tool_grants (client_id TEXT NOT NULL REFERENCES agent_clients(id) ON DELETE CASCADE,
  tool TEXT NOT NULL, mode TEXT NOT NULL CHECK (mode IN ('off','ask','allow')), PRIMARY KEY (client_id, tool));
CREATE TABLE audit_log (id INTEGER PRIMARY KEY, ts INTEGER NOT NULL, caller TEXT NOT NULL, integration_id TEXT,
  capability TEXT, target TEXT, outcome TEXT NOT NULL CHECK (outcome IN ('ok','denied','error','pending')), detail_json TEXT);
CREATE TABLE approvals (id TEXT PRIMARY KEY, created_at INTEGER, caller TEXT, tool_or_capability TEXT,
  args_preview TEXT, state TEXT CHECK (state IN ('pending','approved','denied','expired')), decided_at INTEGER);
```
`usage.rs`: `Purpose` um `TranscriptMerge`, `Relevance`, `Reconcile`, `FactCheck`, `AgentRoute`, `Extract` erweitern
(feste Liste bleibt); Aufrufer reichen die von `record()` gelieferte ID an `provenance::record` weiter.

### Rechte
- Wirksamer Modus = min(Richtung erlaubt die Fähigkeit?, `integration_grants[Aufrufer]`, bei externen Agenten
  zusätzlich `agent_tool_grants`). Patrick in der UI braucht keine Freigabe.
- Standard: Lesendes `allow` für `workflow`, Schreibendes `ask`, externe Agenten alles `off` bis zur Freigabe.
- „Aufnahme starten“ nie `allow`-fähig: die App zeigt immer den Einwilligungsdialog (§ 201 StGB).
- YouTube-Dateiweg (falls E1 = B′) nur mit eingeschalteter Fähigkeit `media.fetch` und Hinweisdialog beim ersten Mal.

### Schnittstellen (Entwurf)
```rust
pub fn provenance::record(conn: &Connection, e: NewProvenance) -> anyhow::Result<String>;
pub fn provenance::list(conn: &Connection, kind: SubjectKind, id: &str) -> anyhow::Result<Vec<ProvenanceEntry>>;
pub fn youtube::normalize_link(raw: &str) -> Result<YoutubeRef, LinkError>;   // Video | Playlist(→ A9) | Invalid
pub async fn youtube::metadata(r: &YoutubeRef) -> Result<VideoMeta, IntegrationError>;   // oEmbed, ohne Schlüssel
pub fn effective_mode(i: &Integration, cap: Capability, caller: Caller, g: &GrantSet) -> GrantMode;
#[tauri::command] meetings_add_youtube(url: String, session_id: Option<String>) -> Result<Meeting, String>
#[tauri::command] transcript_variants(meeting_id: String) -> Result<Vec<TranscriptVariant>, String>
#[tauri::command] transcript_variant_activate(id: String) / transcript_variants_merge(meeting_id: String, a: String, b: String)
#[tauri::command] provenance_get(subject_kind: SubjectKind, subject_id: String) -> Result<Vec<ProvenanceEntry>, String>
#[tauri::command] integrations_list / integrations_catalog / integration_create / integration_update / integration_delete
#[tauri::command] integration_set_grant / integration_test / integration_connect_m365 / audit_list
#[tauri::command] agent_client_create(label) -> (AgentClient, String /*Token einmalig*/) / approvals_pending / approval_decide
```
MCP-Werkzeuge neu: `add_youtube_source`, `start_recording`, `stop_recording`, `transcribe_file`, `create_session`,
`create_meeting`, `tts_page_create`, `tts_render_audio`, `get_action_status`, `get_provenance`.

## Paketschnitt (je 250–300 kTok)
| Paket | Bündel | Scope | Akzeptanztest | Abh. | Worker |
|---|---|---|---|---|---|
| A1 | 1 | Fundament: Register-Kern (Migration, Kalender-Übernahme, Grants, Audit, Approvals, Geheimnis-Namensraum, `--integrations-dump`) + Provenienz (Tabelle, API, `Purpose`-Erweiterung, Einbau in Protokoll/KI-Notizen/Zusammenfassung/STT) | AK1 | – | lv-coder-xhigh |
| A2 | 1 | YouTube-Quelle: Link-Normalisierung, oEmbed, Besprechung mit Quelle `youtube`, eingebetteter Player mit Segment-Sprung, Adapter für Dateiweg nach E1, minimale YouTube-Karte im Register | AK2, AK3 | A1, aufnahmen-ui M2 | lv-coder |
| A3 | 1 | Untertitel + Fassungen + Diff + Zusammenführen + Zusammenfassung + Kontextmenü „Herkunft“; Installer Bündel 1 | AK4, AK5, AK6, AK12 (1) | A2 | lv-coder-xhigh |
| A4 | 2 | Seite Integrationen (Liste, Katalog, Detail, Rechte-Matrix, Audit-Ansicht, Freigabedialog), Umzug Kalender/MCP | AK7 | A1 | lv-coder |
| A5 | 2 | Microsoft-365-Konto: Scopes je Fähigkeit, Mail senden, OneDrive, Termin-Notiz; Follow-up-Mail „senden über“ | AK8 (m365) | A1 | lv-coder-xhigh |
| A6 | 2 | SMTP, Ordner (Sandbox), Obsidian-Vault, WAI-Wissensbasis; Export „ablegen in“ | AK8 (Rest) | A1, A4 | lv-coder |
| A7 | 2 | Agentenbrücke: Named Pipe, Client-Token, Werkzeug-Rechte, Freigaben, `ctl`-CLI | AK9 | A1 | lv-coder-xhigh |
| A8 | 2 | MCP schreibend (inkl. `add_youtube_source`, `get_provenance`), Protokollversion 2026-07-28 prüfen, `mcp_smoke.py --write`, Audit-Dump; Doku, Sicherheitsreview, Installer Bündel 2 | AK10, AK11, AK12 (2) | A7, aufnahmen-ui M3 | lv-coder-xhigh + Planer |
| A9 | später | Playlist: Links auflösen, je Video eine Besprechung in einer Session, Fortschritt | – | A3 | – (nicht im Budget) |

## Budget
8 Pakete × ~275 kTok = 2,2 MTok + Reviews/Nacharbeit ~20 % → **2,7 MTok** (Spanne 2,3–3,5), unverändert gegenüber dem
ersten Entwurf; dafür wurden Webhook und „eigener MCP-Server“ nach Goal B verschoben und SMTP/Ordner/Obsidian/Wissen
in ein Paket (A6) gelegt. Bündel 1 allein: ≈ 1,0 MTok. A9 Playlist: +0,25 MTok, falls gewünscht.

## Risiken mit Vorschlag
- R1 **Rechtsrisiko YouTube-Download** (OLG Hamburg 5 U 54/23: Rolling Cipher = wirksame Schutzmaßnahme; ToS verbietet
  Download) → E1; Standard ist der ToS-konforme Player; kein Bündeln von yt-dlp; vor Verteilung an Dritte anwaltlich
  prüfen.
- R2 **Brüchigkeit yt-dlp** (PO-Token, SABR, Deno-Pflicht) → nur als extern installiertes Werkzeug mit Versionsanzeige,
  Fehler klar melden („Werkzeug veraltet – bitte aktualisieren“), nie still scheitern.
- R3 Loopback-Mitschnitt bei Weg A enthält Werbung und läuft in Echtzeit → Werbeabschnitte bleiben im Rohaudio;
  Hinweis in der UI; Untertitel dann nicht verfügbar (API nur für eigene Videos).
- R4 KI-Zusammenführung erfindet Text → Schema, Überdeckungsprüfung gegen beide Fassungen, Provenienz, Diff zur Kontrolle (AK5).
- R5 Konflikt mit aufnahmen-ui (Inhaltsspalte, Sessions, Sidebar) → A2 nach aufnahmen-ui M2, A8 nach M3; Player als
  eigenständige Komponente in der Inhaltsspalte.
- R6 Migration Kalenderquellen → 1:1-ID, `calendar_sources` bleibt, Backup, Migrationstest.
- R7 Graph-Scopes fehlen in der Entra-Registrierung → E5 vor A5.
- R8 Prompt-Injection über Video-/Besprechungsinhalte an Agenten → Schreibendes „fragen“, Freigabe in der App, keine
  freien Empfänger.
- R9 MCP-Spec 2026-07-28 (`_meta` je Anfrage) → Versionsverhandlung mit Claude Code und Codex in A8 testen.

## Owner-Entscheidungen (Patrick)
- **E1 YouTube-Weg** — Optionen: (A) nur eingebetteter Player, ToS-konform, mit Werbung, Transkript per Loopback-Mitschnitt
  in Echtzeit; (B′) zusätzlich ein **von dir selbst installiertes** yt-dlp (+ Deno) als externes Werkzeug, Pfad in der
  YouTube-Integration, Schalter „privat/experimentell“, Standard aus, nicht im Installer gebündelt → werbefrei lokal
  ansehen, schnell transkribieren, Untertitel; (C) yt-dlp bündeln. **Empfehlung: A als Standard, B′ nur für deine
  eigene Nutzung hinter dem Schalter; C nein.** Vor Weitergabe der App an Dritte: Rechtsprüfung.
- E2 Register und Provenienz in `meetings.db` (gleiche Migrationskette, MCP liest mit) — **Empfehlung: ja**.
- E3 Standardrechte: schreibend „fragen“, externe Agenten „aus“, Aufnahme nie ohne Einwilligungsdialog — **Empfehlung: so**.
- E4 Wissen in den RAG über den Vault statt neuem `wissen:write`-Endpunkt — **Empfehlung: Vault jetzt**.
- E5 Entra-App um `Calendars.ReadWrite`, `Mail.Send`, `Files.ReadWrite` erweitern — **Empfehlung: vor A5**.
- E6 Kalender- und MCP-Einstellungen ziehen auf die neue Seite (alter Ort nur Verweis) — **Empfehlung: ja**.
- E7 Datenklasse für Vault-Notizen aus Besprechungen/Videos: `confidential` (Besprechungen), `internal` (öffentliche
  Videos) — **Empfehlung: so**.

## Abhängigkeiten
- **aufnahmen-ui** (läuft): A2 braucht das Spaltengerüst (M2), A8 die Sessions (M3). A1 ist reines Backend und kann
  sofort parallel starten.
- **Goal B** braucht A1 (Grants, Audit, Approvals, Provenienz), A3 (YouTube-Transkript/Zusammenfassung als Aktionen),
  A5/A6 (Mail, Ordner, Vault, Wissen), A7 (Agenten).
- **Goal C** braucht A1 (Aufrufer `agent_local`, Provenienz mit Konfidenz) und A6 (Vault/Wissen).
- Basis: `feat/granola-besprechungen` (P5 Kalender, P6a/b Export, P6c Mail, P6e MCP, M7 KI-Notizen).
