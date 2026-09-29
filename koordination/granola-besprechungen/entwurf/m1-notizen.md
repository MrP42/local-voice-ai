# M1-Entwurf: Notizblock, KI-Notizen, Vorlagen (F01, F07–F10, F26)

Stand 29.09.2026 · Paket P1 (lv-architect) · Issue #59 · Grundlage: `recherche/ist-stand.md`, Code auf `feat/granola-besprechungen` (45f3a3b).
Pfade: RS = `apps/local-voice/src-tauri/src`, FE = `apps/local-voice/src`, FX = `apps/local-voice/src-tauri/tests/fixtures`.
Keine Messwerte in diesem Dokument: alle Zahlen sind Code-Fakten (mit Fundstelle) oder ausdrücklich als Annahme markiert, die P1e misst.

## 1. Architektur im Überblick

```
Aufnahmeseite (MeetingsSettings)          MeetingDetail
 RecorderCard (+Vorlage, +Hinweis)         Tabs: Notizen [Meine | KI] · Transkript · Protokoll
 LiveNotesPad ── NoteBlocksEditor ──┐       NotesTab ── NoteBlocksEditor / EnhancedNotesView
 LiveTranscript (unverändert)       │               └─ Quelle-Klick → Transkript-Tab + playSegment()
                                    ▼
commands/meeting_notes.rs (Notizen, Vorlagen, Aufgaben, Position)   commands/meeting_enhance.rs (KI)
                                    ▼                                          ▼
managers/meetings/notes/  model.rs · templates.rs · store_ext (in store.rs) · enhance.rs · assemble.rs · eval.rs
managers/meetings/llm_call.rs  (aus minutes.rs herausgelöst: Provider, JSON-Retry, Chunk-Retry)
                                    ▼
MeetingStore (meetings.db, EINE neue Migration)      llm_client::send_chat_completion_with_schema → llama-server / Anbieter
```

Leitentscheidungen:
1. **Nutzernotizen sind eine lebende Datei, keine Dokumentversion** → eigene Tabelle `meeting_notes` (eine Zeile je Besprechung, Blöcke als JSON, Revisionszähler). `meeting_documents` versioniert bei jedem `upsert_document` (store.rs:682); Autosave im Sekundentakt würde dort tausende Versionen erzeugen, und `MinutesView` filtert nach `kind` (MinutesView.tsx:47) – Mischen bräche Altlogik.
2. **KI-Notizen sind Dokumentversionen** → `meeting_documents`, `kind = "enhanced_notes"`, `body_format = "enhanced@1"`, Body = JSON (`EnhancedNotes`). Neue Version bei Erzeugen und bei „Anweisung anwenden"; Handbearbeitung ändert die jüngste Version in place (optimistische Sperre über `updated_at`).
3. **Nutzerstichpunkte wörtlich per Konstruktion, nicht per Modellgehorsam**: Das Modell referenziert Notizen nur über IDs (`"ref": "N3"`), den Text setzt Rust ein. Fehlende Notizen fügt `assemble` deterministisch ein. Damit ist „100 % wörtlich" eine Code-Garantie; das Eval prüft sie unabhängig von der Konstruktion nach.
4. **Protokoll erweitern statt Parallelstruktur**: Die LLM-Mechanik aus `minutes.rs` (`resolve_provider` :517, JSON-Retry `ask_for_minutes_json` :547, Chunk-Retry `ask_for_chunk` :615, `strip_code_fence` :505, `build_head` :644, `sorted_segments` :139) wandert generisch nach `meetings/llm_call.rs`; Protokoll und KI-Notizen nutzen sie beide. Das formale Protokoll bleibt als Tab/Export unverändert (keine Regression der ~95 Meeting-Tests), die Standard-Vorlage „Allgemein" ersetzt es im Alltag.
5. **Eigener Block-Editor statt Bibliothek**: Im Frontend existiert kein Rich-Text-Editor (package.json: nur `react-markdown`/`remark-gfm`; `TtsChipEditor.tsx` ist ein Textarea mit Chip-Overlay). Gebraucht wird je Block ein Zeitstempel und je Eintrag eine Herkunft – das ist ein Blockmodell, kein Fließtext. `NoteBlocksEditor` = Liste auto-wachsender Textareas mit Markdown-Kürzeln (`# `, `- `, `[ ] `), Enter = neuer Block, Backspace am Anfang = verschmelzen. Keine neue Abhängigkeit; TipTap/ProseMirror bleibt Option für M10 (Entscheidung E4).

## 2. Berührpunkte mit M2 (Live-Pfad-Umbau) – verbindlich

| # | Berührpunkt | Richtung | Vertrag |
|---|---|---|---|
| B1 | `MeetingEvent::State {meeting_id,status,paused}`, `::Segments {meeting_id, appended}`, `::Reset` (recorder.rs:97-120) | M1 liest | Form bleibt; M1 ändert das Enum nicht |
| B2 | **neu** `MeetingRecorderManager::position_ms(&self) -> Option<(String, u64)>` | M1 fügt ein (≈15 Zeilen in recorder.rs + Getter `frames_written()` in `wav_writer.rs`) | Liefert laufende Besprechungs-ID und Mikrofon-Audioposition in ms = Frames/16. Gleiche Zeitachse wie `StoredSegment.start_ms` (Pause komprimiert, recorder.rs:566-571). M2 muss den Getter bei Umbau der Sinks erhalten |
| B3 | `StoredSegment.segment_index` | M1 liest | eindeutig je Besprechung, stabil bis `clear_segments`; `start_ms` auf derselben Achse. VAD/kleinere Blöcke in M2 sind unkritisch, solange das gilt |
| B4 | `commands::meetings::meetings_stop` (commands/meetings.rs:55) | M1 (P1f) hängt nach `recorder.stop()` einen Auto-KI-Notizen-Start an | `stop()` kehrt erst zurück, wenn alle Segmente gespeichert sind (recorder.rs:629-641). M2 darf das nicht aufweichen |
| B5 | `MIGRATIONS` in store.rs:14 | M1 hängt GENAU EINEN Schritt an (Index 2) | M2 fügt keine meetings.db-Migration parallel hinzu, ohne auf M1 zu rebasen |
| B6 | `store.clear_segments` (store.rs:361) | M1 erhöht dort `segment_epoch` | von `retranscribe.rs` gerufen; M2 unberührt |
| B7 | RAM-Spitze nach Stopp: `stop()` lädt das Diktatmodell neu (recorder.rs:646), Auto-KI-Notizen starten llama-server | Risiko | `server.ensure` prüft RAM (`check_ram_for_start`, llm/server.rs:173); M1 wiederholt nicht bei RAM-Fehler (§8) |

Alle anderen M1-Änderungen liegen in neuen Dateien oder in `store.rs`, `minutes.rs`, `MeetingDetail.tsx`, `MeetingsSettings.tsx`, `RecorderCard.tsx` – nicht in `chunker.rs`, `transcription.rs`, `mic_capture.rs`, `loopback.rs`.

## 3. Datenmodell und Migration

Ein neuer Migrationsschritt (Index 2, nach `source_path`). Nur `CREATE`/`ADD COLUMN` mit Defaults – bestehende Zeilen bleiben bytegleich:

```sql
CREATE TABLE meeting_notes (
  meeting_id TEXT PRIMARY KEY, blocks_json TEXT NOT NULL DEFAULT '[]',
  revision INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
ALTER TABLE meetings ADD COLUMN template_id TEXT;              -- gewählte Vorlage (vor/während/nach)
ALTER TABLE transcripts ADD COLUMN segment_epoch INTEGER NOT NULL DEFAULT 0;  -- +1 je clear_segments
ALTER TABLE action_items ADD COLUMN document_id TEXT;          -- erzeugende KI-Notizen-Version
ALTER TABLE action_items ADD COLUMN entry_id TEXT;             -- Eintrag darin ("E7")
ALTER TABLE action_items ADD COLUMN assignee_label TEXT;       -- Freitext, humans-Tabelle folgt M9
ALTER TABLE action_items ADD COLUMN sources_json TEXT;         -- [segment_index,...]
CREATE INDEX idx_action_items_meeting ON action_items(meeting_id, deleted_at);
```

- **Vorlagen ohne Schemaänderung**: mitgelieferte Vorlagen haben die feste ID `builtin:<key>` (PK), werden bei jedem `open_at` per Upsert auf den Stand der App gebracht und sind schreibgeschützt (Bearbeiten = Duplizieren). `sections_json` trägt ab jetzt ein `TemplateSpec` (§4). Die vorhandene Zeile „Standardprotokoll" (ULID, `sections_json` = String-Array, store.rs:222) bleibt unangetastet; sie parst nicht als `TemplateSpec` und erscheint deshalb nicht in der Vorlagenwahl. `seed_default_template` bleibt für das Protokoll.
- **Löschen**: `soft_delete_meeting` setzt `deleted_at` zusätzlich auf `meeting_notes` und `action_items` der Besprechung (gleiche Transaktion).
- **Epoche**: `EnhancedNotes.segment_epoch` wird beim Erzeugen gespeichert; weicht sie von `transcripts.segment_epoch` ab (Neu-Transkription), sind die Quellen veraltet → UI deaktiviert Quellsprünge und bietet „Neu erzeugen" an.

## 4. Rust-Schnittstellen

`RS/managers/meetings/notes/model.rs` (alle `Serialize, Deserialize, specta::Type, Clone, Debug`):

```rust
#[serde(rename_all = "snake_case")] pub enum NoteBlockKind { Paragraph, Bullet, Heading, Todo }
pub struct NoteBlock { pub id: String /*ULID, im Frontend erzeugt*/, pub kind: NoteBlockKind,
    pub text: String, pub at_ms: Option<u64> /*Audioposition beim Anlegen; None = importiert/nach Stopp*/,
    pub checked: bool }
pub struct MeetingNotes { pub meeting_id: String, pub blocks: Vec<NoteBlock>, pub revision: u64, pub updated_at: i64 }

#[serde(rename_all = "snake_case")] pub enum SectionKind { Text, Tasks }
pub struct TemplateSection { pub id: String /*[a-z0-9_]{1,32}*/, pub title: String, pub instruction: String, pub kind: SectionKind }
pub struct TemplateSpec { pub version: u32 /*=1*/, pub context: String, pub sections: Vec<TemplateSection> }
pub struct TemplateInfo { pub id: String, pub title: String, pub builtin: bool, pub spec: TemplateSpec, pub updated_at: i64 }

#[serde(rename_all = "snake_case")] pub enum Origin { User, Ai }
pub struct EntryFlags { pub unsupported: bool /*KI ohne gültige Quelle*/, pub dropped_sources: u32,
    pub placed_by_fallback: bool, pub edited: bool }
pub struct EnhancedEntry { pub id: String /*"E1".. */, pub origin: Origin, pub text: String,
    pub note_id: Option<String>, pub source_segment_ids: Vec<u32>, pub assignee: Option<String>,
    pub due: Option<String>, pub flags: EntryFlags }
pub struct EnhancedSection { pub id: String, pub title: String, pub kind: SectionKind, pub entries: Vec<EnhancedEntry> }
pub struct EnhanceStats { pub user_notes_total: u32, pub user_notes_by_model: u32, pub user_notes_by_fallback: u32,
    pub ai_entries: u32, pub ai_entries_sourced: u32, pub dropped_source_ids: u32,
    pub chunks_total: u32, pub chunks_failed: Vec<u32>, pub single_pass: bool }
pub struct EnhancedNotes { pub format: String /*"enhanced@1"*/, pub template_id: Option<String>,
    pub template_title: String, pub segment_epoch: u32, pub sections: Vec<EnhancedSection>, pub stats: EnhanceStats }
pub struct ActionItem { pub id: String, pub meeting_id: String, pub text: String, pub status: String /*todo|done*/,
    pub assignee_label: Option<String>, pub document_id: Option<String>, pub entry_id: Option<String>,
    pub source_segment_ids: Vec<u32>, pub source: String /*ai|user|manual*/ }
```

`MeetingDocument` (store.rs:147) erhält additiv `template_id: Option<String>`, `updated_at: i64`.

Store (`impl MeetingStore`, store.rs – Paket P1a):
```rust
pub fn get_notes(&self, meeting_id: &str) -> Result<MeetingNotes>;                 // leer, wenn keine Zeile
pub fn save_notes(&self, meeting_id: &str, blocks: &[NoteBlock], base_revision: u64) -> Result<u64>; // Err "revision_conflict"
pub fn set_meeting_template(&self, meeting_id: &str, template_id: Option<&str>) -> Result<()>;
pub fn segment_epoch(&self, meeting_id: &str) -> Result<u32>;
pub fn insert_document(&self, meeting_id: &str, kind: &str, body_format: &str, body: &str,
    template_id: Option<&str>, metadata: Option<&str>) -> Result<String>;          // upsert_document delegiert hierher
pub fn get_document(&self, document_id: &str) -> Result<Option<MeetingDocument>>;
pub fn update_document_body(&self, document_id: &str, body: &str, expected_updated_at: i64) -> Result<i64>; // Err "stale_document"
pub fn list_template_infos(&self) -> Result<Vec<TemplateInfo>>;                    // nur parsebare Specs
pub fn save_template(&self, id: Option<&str>, title: &str, spec: &TemplateSpec) -> Result<TemplateInfo>; // builtin → Err "template_readonly"
pub fn delete_template(&self, id: &str) -> Result<()>;
pub fn replace_action_items(&self, meeting_id: &str, document_id: &str, items: &[ActionItem]) -> Result<Vec<ActionItem>>;
pub fn list_action_items(&self, meeting_id: &str) -> Result<Vec<ActionItem>>;       // nur jüngste Dokumentversion + manual
pub fn set_action_item_status(&self, id: &str, done: bool) -> Result<()>;
```
`save_notes` schreibt in EINER Transaktion `blocks_json`, `revision = revision+1`, `updated_at`; bei `base_revision != revision` Fehler ohne Schreiben. `updated_at` für Dokumente in Millisekunden (sonst kollidieren zwei Speicherungen in derselben Sekunde).

`notes/templates.rs`: `pub fn builtin_templates() -> Vec<(&'static str /*key*/, &'static str /*title*/, TemplateSpec)>`, `pub fn validate_spec(spec: &TemplateSpec) -> Result<(), String>` (1–10 Abschnitte, IDs eindeutig und `[a-z0-9_]`, Titel 1–60 Zeichen, Anweisung ≤ 500, Kontext ≤ 1000, höchstens ein `Tasks`-Abschnitt), `pub fn export_file(info) -> String` / `pub fn import_file(json: &str) -> Result<(String, TemplateSpec), String>` (Format `{"format":"lva-meeting-template@1","title":…,"spec":…}`, Dateiendung `.lvtemplate.json`, max. 64 KB).

`notes/enhance.rs` (P1b):
```rust
pub fn enhance_schema(spec: &TemplateSpec, local: bool) -> serde_json::Value;
pub fn enhance_system_prompt() -> String;
pub fn render_notes_for_prompt(blocks: &[NoteBlock]) -> String;                    // "N3 [12:05] - Text"
pub fn render_segments_for_prompt(segments: &[StoredSegment]) -> String;           // "S12 [03:15] Ich: Text"
pub fn single_pass_budget_chars(local: bool) -> usize;
pub async fn enhance_meeting(settings: &AppSettings, store: Arc<MeetingStore>, meeting_id: &str,
    template_id: Option<&str>, progress: impl Fn(u32, u32) + Send + Sync) -> Result<MeetingDocument, String>;
pub async fn apply_instruction(settings: &AppSettings, store: Arc<MeetingStore>, document_id: &str,
    instruction: &str) -> Result<MeetingDocument, String>;
pub fn apply_manual_edit(stored: &EnhancedNotes, edited: EnhancedNotes) -> Result<EnhancedNotes, String>;
pub fn enhanced_to_markdown(meeting_title: &str, notes: &EnhancedNotes) -> String;
pub struct EnhanceGuard;  // globaler Mutex: max. ein KI-Notizen-Lauf gleichzeitig → Err "enhance_busy"
```
`notes/assemble.rs` (P1b, rein, keine I/O):
```rust
pub struct RawEntry { pub r#ref: Option<String>, pub text: String, pub sources: Vec<String>,
                      pub assignee: Option<String>, pub due: Option<String> }
pub type RawEnhanced = std::collections::BTreeMap<String, Vec<RawEntry>>;           // section_id → Einträge
pub fn assemble(raw: RawEnhanced, spec: &TemplateSpec, notes: &[NoteBlock], protected: &[EnhancedEntry],
                segments: &[StoredSegment]) -> (Vec<EnhancedSection>, EnhanceStats);
```
`notes/eval.rs` (P1e): `pub fn metrics(out: &EnhancedNotes, notes: &[NoteBlock], segments: &[StoredSegment]) -> EvalMetrics` mit `user_preserved_ratio`, `ai_sourced_ratio`, `lexical_support_ratio` (Info: Anteil KI-Einträge mit ≥ 1 gemeinsamen Inhaltswort ≥ 4 Zeichen zu einer Quelle).

`recorder.rs` (P1c, Berührpunkt B2): `pub fn position_ms(&self) -> Option<(String, u64)>`.

## 5. Tauri-Commands und TS-Typen

`RS/commands/meeting_notes.rs` (P1c): `meeting_notes_get(meeting_id) -> MeetingNotes` · `meeting_notes_save(meeting_id, blocks: Vec<NoteBlock>, base_revision: u64) -> u64` · `meetings_recording_position() -> Option<RecordingPosition{meeting_id, position_ms}>` · `meetings_set_template(meeting_id, template_id: Option<String>)` · `meeting_templates_list() -> Vec<TemplateInfo>` · `meeting_templates_save(id: Option<String>, title, spec) -> TemplateInfo` · `meeting_templates_duplicate(id) -> TemplateInfo` · `meeting_templates_delete(id)` · `meeting_templates_export(id, path)` · `meeting_templates_import(path) -> TemplateInfo` · `action_items_list(meeting_id) -> Vec<ActionItem>` · `action_items_set_status(id, done: bool)`.

`RS/commands/meeting_enhance.rs` (P1b): `meeting_notes_enhance(meeting_id, template_id: Option<String>) -> MeetingDocument` · `meeting_notes_apply_instruction(document_id, instruction) -> MeetingDocument` · `meeting_notes_update_enhanced(document_id, notes: EnhancedNotes, expected_updated_at: i64) -> i64` · `meeting_notes_markdown(document_id) -> String`.

Event `MeetingNotesEvent` (`#[serde(tag="kind")]`, in `collect_events!` lib.rs:1612): `progress {meeting_id, step, total}` · `done {meeting_id, document_id}` · `failed {meeting_id, code}`; `code` ∈ `no_provider | no_model | memory_low | recording_active | enhance_busy | no_transcript | llm_failed | meeting_not_finished`. Das Frontend übersetzt Codes (Muster `MeetingEvent::Error`).

TS: Typen kommen aus `bindings.ts` (Debug-Build exportiert, lib.rs:1619; ohne Debug-Lauf von Hand nachziehen). Frontend-Helfer in `FE/lib/meetingNotes.ts`: `formatAt(ms)`, `parseSegmentRef`, `isStale(notes, epoch)`, `blockFromMarkdownPrefix(text)`.

## 6. Prompt- und Schema-Entwurf

System (Englisch wie `minutes_system_prompt`, minutes.rs:225; Ausgabe in Transkriptsprache):
```
You write meeting notes from (1) the user's own notes, ids N<k>, (2) a transcript, one segment per line, ids S<k>,
(3) a template with sections and an instruction per section.
- Put EVERY user note exactly once into the best-fitting section as {"ref":"N<k>","text":"","sources":[...]}.
  Never rewrite a user note; the app inserts its text. You may attach transcript sources to it.
- User notes show what mattered. Add AI entries that expand on them with facts from the transcript,
  placed directly after the note they belong to, and cover important points the user did not note.
- Every AI entry ("ref": null) MUST list the transcript segments it is based on in "sources" ("S12").
  If you cannot point to a segment, leave the statement out.
- Never invent names, numbers, dates or decisions. Unclear points go to an open-questions section if there is one.
- Assignee/due only if the transcript names them, else null.
- Same language as the transcript. Short factual sentences. No markdown. Reply with ONLY the JSON object.
```
User-Prompt: Kopf-Fakten wie `head_facts_block` (minutes.rs:242: Titel, Datum, Dauer, Kanäle) · `Zweck der Besprechung: <spec.context>` · Abschnittsliste `- <id> (<title>): <instruction>` · `Nutzernotizen:` + `render_notes_for_prompt` (Überschriften mit `#`, Aufgaben mit `[ ]/[x]`) · `Transkript:` + `render_segments_for_prompt` (sortiert wie `sorted_segments`).

Schema (dynamisch aus der Vorlage; strict: alle Felder Pflicht, `additionalProperties:false`, Muster wie `minutes_schema_for`):
```json
{"type":"object","additionalProperties":false,"required":["<sec1>","<sec2>"],
 "properties":{"<sec>":{"type":"array","items":{"type":"object","additionalProperties":false,
   "required":["ref","text","sources"],   // Tasks-Abschnitt zusätzlich "assignee","due" (nullable)
   "properties":{"ref":{"type":["string","null"]},"text":{"type":"string"},
                 "sources":{"type":"array","items":{"type":"string"}}}}}}}
```
Nur `local = true` (llama-server-Grammatik): `ref` mit `"pattern":"^N[0-9]+$"` bzw. im Anweisungsmodus `^(N|E)[0-9]+$`, `sources.items.pattern "^S[0-9]+$"`, `sources.maxItems: 8`. Cloud-strict erhält die Muster nicht (gleiche Begründung wie `minLength`, minutes.rs:151-157).
Abweichung vom Auftrag, bewusst: `origin` wird nicht vom Modell geliefert, sondern aus `ref` abgeleitet (ein Feld weniger, keine Widersprüche „origin=user, aber anderer Text" möglich). Gespeichert wird `{text, origin, source_segment_ids}` wie gefordert.

**Deterministische Nachprüfung `assemble`** (Reihenfolge fest, jede Regel ein Test):
1. Abschnitte, die die Vorlage nicht kennt, werden verworfen; fehlende sind leer.
2. `ref` bekannt und unbenutzt → `origin=user`, `text` = Notiztext **byte-genau**, `note_id` gesetzt; `checked` einer `Todo`-Notiz → Aufgabe `done`. Doppelte `ref` → spätere verworfen. Unbekannte `ref` mit Text → wie KI-Eintrag behandelt, ohne Text → verworfen.
3. KI-Eintrag: leerer Text → verworfen; `sources` „S<n>" → `n`, nur existierende `segment_index` bleiben, Rest zählt in `dropped_sources`; keine gültige Quelle → `flags.unsupported = true` (wird NICHT gelöscht, sonst wäre die 95-%-Metrik geschönt).
4. Nicht platzierte, nicht-leere Notizen → in Dokumentreihenfolge ans Ende des ersten `Text`-Abschnitts (`Todo`-Notizen in den `Tasks`-Abschnitt), `placed_by_fallback = true`.
5. Anweisungsmodus: geschützte Einträge (`origin=user`, per `E<k>` referenziert) behalten Text und Quellen; fehlende werden an ihrer alten Abschnittsposition wieder eingefügt.
6. IDs `E1..En` in Ausgabereihenfolge vergeben; `EnhanceStats` gefüllt.

**Einzeldurchlauf vs. Map-Reduce**: `single_pass_budget_chars(local)` – lokal aus `DEFAULT_CONTEXT_TOKENS = 8192` (llm/mod.rs:33) minus Ausgabereserve 2048 minus Prompt-Overhead 1000, mal 3 Zeichen/Token (**Annahme**, P1e misst sie über die Prompt-Tokens der Antworten und korrigiert die Konstante); entfernt 48 000 Zeichen (Annahme, konservativ). Darüber: Blöcke per `summarizer::chunk_text` an Zeilengrenzen (IDs bleiben ganz). Map je Block: gleiche Regeln, Ausgabe flache Liste `{section, ref, text, sources}`, übergeben werden nur Notizen mit `at_ms` im Zeitfenster des Blocks. Reduce: Teillisten als `sec | S12,S13 | Text` plus ALLE Notizen → Endschema. `assemble` prüft gegen die vollständige Segmentmenge; Blockausfälle wie bisher degradiert (`chunks_failed`, Hinweis im Dokument). Hinweis: bei 8192 Token Kontext läuft lokal fast jede Besprechung über 15 min im Map-Reduce-Pfad (Schätzung aus der Annahme oben); größerer Kontext wäre eine Änderung an `ensure_local` und ist NICHT Teil von M1 (siehe Risiken).

**Anweisung anwenden**: Eingabe = aktuelle KI-Notizen als `E<k> (user, unveränderlich)` bzw. `E<k> (ai) [S..]: Text`, die Freitext-Anweisung (≤ 500 Zeichen), dazu das Transkript, wenn es ins Budget passt, sonst nur die zitierten Segmente ± 1 Nachbar. Nutzereinträge bleiben unveränderlich (UI-Hinweis „Deine eigenen Notizen bleiben unverändert"); Ergebnis = neue Version.

**Aufgaben**: nach jeder neuen Version `replace_action_items`: Einträge aller `Tasks`-Abschnitte → Zeilen (`source` = ai/user nach Herkunft); Status `done` wird aus der Vorversion per normalisiertem Text (klein, Leerraum zusammengefasst) übernommen; Zeilen der Vorversion bekommen `deleted_at`, `manual`-Zeilen bleiben.

## 7. Mitgelieferte Vorlagen (P1a, `templates.rs`)

Jede Vorlage: `context` (1–2 Sätze Zweck) + Abschnitte mit Anweisung. Kurzform `Titel (Anweisung)`, `[T]` = `Tasks`-Abschnitt:
1. **allgemein** „Allgemein" – Zusammenfassung (3–5 Sätze: Anlass, Ergebnis) · Besprochene Punkte (je Thema ein Stichpunkt mit Kernaussage) · Entscheidungen (nur tatsächlich Beschlossenes) · [T] Aufgaben (wer, was, bis wann) · Offene Fragen.
2. **vertrieb** „Kundengespräch / Vertrieb" – Kunde & Ausgangslage · Bedarf & Schmerzpunkte (wörtliche Kundenaussagen bevorzugen) · Einwände · Budget, Zeitrahmen, Entscheider (nur Genanntes) · [T] Nächste Schritte · Einschätzung (nur aus Aussagen ableitbar, als solche kennzeichnen).
3. **eins_zu_eins** „1:1-Gespräch" – Stimmung & Themen der Person · Fortschritt seit letztem Mal · Hindernisse · Feedback (beidseitig, getrennt) · [T] Vereinbarungen.
4. **jour_fixe** „Jour fixe / Team" – Stand je Person oder Thema · Blocker · Entscheidungen · [T] Aufgaben · Themen fürs nächste Mal.
5. **kickoff** „Projekt-Kickoff" – Ziel & Erfolgskriterien · Umfang & Abgrenzung · Rollen & Zuständigkeiten · Meilensteine & Termine · Risiken & Annahmen · [T] Erste Schritte.
6. **interview** „Interview / Bewerbung" – Person & Werdegang (nur Genanntes) · Fragen & Antworten (Frage kurz, Antwort sinngemäß) · Stärken · Offene Punkte & Bedenken · [T] Weiteres Vorgehen.
7. **workshop** „Workshop / Brainstorming" – Fragestellung · Ideen (gruppiert, ohne Wertung) · Bewertung & Favoriten · Entscheidungen · [T] Aufgaben · Parkplatz.
8. **lenkungskreis** „Lenkungskreis / Entscheidung" – Entscheidungsvorlage & Optionen · Diskussion (Pro/Contra je Option) · Beschlüsse (Wortlaut, Gegenstimmen, Bedingungen) · Risiken & Eskalationen · [T] Aufträge.
Standardvorlage (Einstellung `meeting_default_template_id` = `None`) ist `builtin:allgemein`.

## 8. Ressourcen und Fehlerfälle

| Fall | Verhalten |
|---|---|
| Kein Anbieter / kein Modell | `resolve_provider` (minutes.rs:517) → `failed{no_provider/no_model}`; Knopf zeigt Meldung mit Verweis „Einstellungen → Sprachmodelle"; Auto-Lauf nach Stopp: einmaliger Toast, kein Retry |
| RAM zu knapp beim Start von llama-server | `server.ensure` → `check_ram_for_start` schlägt fehl → `memory_low`; **kein** Retry (reine Funktion `should_retry(err, free_mb) -> bool`: nie bei RAM-Fehler oder wenn `available_ram_mb()` unter der Reserve liegt) |
| Speicherwächter beendet llama-server mitten im Lauf | Transportfehler → `should_retry` prüft freien RAM → Abbruch `memory_low`, bereits fertige Map-Blöcke gehen verloren (bewusst einfach); Nutzernotizen unberührt |
| Aufnahme läuft, lokaler Anbieter | `recording_active` (STT und LLM konkurrieren; schützt M2-Latenz); entfernter Anbieter erlaubt |
| Besprechung nicht fertig (`recording/processing`) | `meeting_not_finished` wie minutes.rs:690-697; Notizblock bleibt editierbar |
| Zweiter Lauf parallel | `EnhanceGuard` → `enhance_busy` |
| Kein Transkript, aber Notizen | `no_transcript`; Notizen selbst bleiben nutzbar/exportierbar |
| Ungültiges JSON | 1 Retry mit Fehlertext (bestehende Mechanik), dann `llm_failed` |
| Neu-Transkription danach | Epoche weicht ab → Quellen „veraltet", Sprung aus, „Neu erzeugen" |
| Audio gelöscht (Aufbewahrung) | Quelle springt nur ins Transkript; Hinweis „Audio nicht mehr vorhanden" |
| Autosave-Konflikt (zwei Fenster) | `revision_conflict` → neu laden, lokale ungespeicherte Blöcke als Kopie anhängen (nie still verwerfen) |
| Absturz während Tippen | Verlust ≤ Entprellzeit 700 ms; Flush bei Blur, `visibilitychange`, Unmount und vor `meetings_stop` |
| Retention `AfterMinutes` | KI-Notizen lösen die Audio-Löschung **nicht** aus (nur `generate_minutes`, minutes.rs:857-872) – sonst wäre der Audio-Sprung nach Auto-KI-Notizen sofort tot (Entscheidung E1) |
| Vorlage gelöscht, Besprechung verweist darauf | Fallback Standardvorlage; Dokument behält `template_title` |
| Datenschutz (D9) | Kein Notiz-, Transkript- oder Ausgabetext im Log; nur Längen, Zähler, Codes |

## 9. UI-Skizze (Text)

**Aufnahmeseite** (breit ≥ 1024 px zweispaltig, sonst gestapelt): RecorderCard oben – neu: Vorlagen-Dropdown (Default aus Einstellung, „Vorlagen verwalten…" öffnet Dialog), Häkchen Systemton (Default aus Einstellung), während der Aufnahme Zeile „Hinweis für den Meeting-Chat [Kopieren]". Darunter links **LiveNotesPad** („Deine Notizen – Stichpunkte genügen"), rechts LiveTranscript. Jeder Block zeigt links grau `mm:ss` (Anlagezeit). Nach Stopp: Hinweis „KI-Notizen werden erstellt …" mit Fortschritt `2/5`, danach Link „Öffnen".
**MeetingDetail**: Tabs `Notizen · Transkript · Protokoll`. Tab Notizen: Umschalter `Meine Notizen | KI-Notizen`, rechts Vorlagen-Dropdown + „KI-Notizen erzeugen/neu erzeugen", Versionswahl, Export (md/docx/txt über `meetings_export_document`). KI-Notizen: Abschnittsüberschriften; Nutzertext normal (`text-text`), KI-Text grau (`text-text/60`, `data-origin="ai"`); je Eintrag rechts Lupen-Chips `03:15` (max. 3 + „+2"); Klick → Tab Transkript, Segment hervorgehoben (`data-segment-index`, 2 s Markierung), `scrollIntoView`, `playSegment()` (MeetingDetail.tsx:69). KI-Eintrag ohne Beleg: gelbes „ohne Beleg"-Symbol. Aufgabenabschnitt als Checkliste (Checkbox → `action_items_set_status`). Klick in einen Eintrag = bearbeiten (gleicher Block-Editor); gespeichert entprellt, Eintrag wird schwarz. Unten Eingabezeile „Anweisung an die KI (z. B. ‚kürzer', ‚Namen korrigieren: Maier → Meyer')" + „Anwenden". Importierte Besprechung: „Meine Notizen" ohne Zeitstempel, sonst identisch.
**Einstellungen** (bestehende Gruppe `meetings.title` in DictationTab.tsx:62, kein neuer Reiter): „Systemton standardmäßig aufnehmen" · „KI-Notizen nach dem Stopp automatisch erstellen" · „Standardvorlage".

## 10. F01 / F26 / Auto-Lauf

- **F01**: neues Setting `meeting_capture_system: bool`, serde-Default `true`. Heute ist das Häkchen reiner UI-Zustand (`useState(false)`, RecorderCard.tsx:35) und wird nie gespeichert – bestehende Nutzer erhalten den neuen Default ohne Migration; ab dann merkt sich die App die letzte Wahl. Wo kein Loopback verfügbar ist (Nicht-Windows), bleibt es wirkungslos wie bisher (Coder prüft die vorhandene Plattformbehandlung).
- **F26**: i18n-Text abhängig vom aktiven Anbieter: lokal → „Hinweis: Ich transkribiere diese Besprechung lokal auf meinem Rechner mit Local Voice AI, um Notizen zu erstellen. Es werden keine Daten an Dritte übertragen."; entfernter Anbieter → „… Für die Notizen wird der Text an <Anbieter> übermittelt." (der erste Satz wäre sonst falsch). Kopierknopf in RecorderCard und im Einwilligungsdialog.
- **Auto-Lauf**: Setting `meeting_auto_enhance: bool`, Default `true` (E2). In `meetings_stop` nach erfolgreichem `stop()`: wenn an → `tauri::async_runtime::spawn(enhance_meeting(…, meeting.template_id))`, Fehler nur als `MeetingNotesEvent::failed`. Import (`meetings_import_file`) in M1 ohne Auto-Lauf.

## 11. Testplan

Rust (`cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib <filter>`):
- Store/Migration: Altdaten-Test (DB mit `MIGRATIONS[..2]` anlegen, Zeilen in allen Tabellen einfügen, mit neuem Code öffnen → Zeilen unverändert, neue Spalten mit Default); Seeding idempotent (2× öffnen → 8 Builtins, Standardprotokoll unverändert, 9 Zeilen); `save_notes` Revision/Konflikt; Soft-Delete versteckt Notizen und Aufgaben; `clear_segments` erhöht Epoche; `update_document_body` Stale-Fehler.
- `assemble`: je Regel 1–6 mindestens ein Test, Byte-Gleichheit mit Umlauten/Emoji/führenden Leerzeichen, doppelte Refs, unbekannte Quellen, Fallback-Platzierung, geschützte Einträge.
- `enhance`: Schema je Vorlage gültig und strict-konform (`local=false` ohne `pattern`), Budget-Weiche, Map-Reduce mit Stub-LLM (vorhandener `spawn_llm_mock`, minutes.rs:1146) inkl. Blockausfall, `should_retry`-Tabelle, `EnhanceGuard`, `recording_active`, Retention bleibt unberührt.
- `minutes::` bleibt vollständig grün (Refactor auf `llm_call.rs`).
- `eval::metrics`: unabhängige Nachprüfung (manipulierte Ausgabe mit geändertem Nutzertext → `user_preserved_ratio < 1`).
Playwright (Attrappe `tests/workspace.spec.ts`, neue Datei `tests/meeting-notes.spec.ts` mit Meeting-Daten): Notizblock erzeugt Block mit Zeitstempel und ruft `meeting_notes_save` entprellt; KI-Einträge `data-origin`; Quell-Klick wechselt Tab, markiert Segment, ruft `playAt`; Checkbox ruft `action_items_set_status`; Vorlage duplizieren/bearbeiten; Hinweis kopieren.
Eval (AK3): 3 deutsche Fixtures in `FX/notes/` (synthetisch, keine realen Personen): `kundengespraech.json` (~10 min, Kanäle 0/1, 8 Nutzerstichpunkte), `jour_fixe.json` (~20 min, 10 Stichpunkte inkl. Überschrift und Todo), `lenkungskreis_lang.json` (> Einzeldurchlauf-Budget, erzwingt Map-Reduce, 12 Stichpunkte, davon 2 ohne `at_ms`). Format: `{title, template_key, segments: StoredSegment[], notes: NoteBlock[]}`. Befehl `--eval-notes <dir> [--json] [--out f]`: Sandbox-Store in Temp (nie die produktive meetings.db), LLM-Globals wie `initialize_core_logic` (lib.rs:229-258) installieren, llama-server am Ende stoppen, Fixtures seriell. Ausgabe je Fixture und gesamt; Exit 0 = Soll erfüllt, 3 = Soll verfehlt, 1 = Fehler.

## 12. Paketschnitt

Wellen: **W1** P1a → **W2** P1b ∥ P1c → **W3** P1d ∥ P1e ∥ P1f (P1e darf starten, sobald P1b gemergt ist).

**P1a – Fundament: Migration, Modell, Store, Vorlagenkatalog** · lv-coder-xhigh · M · Abh.: –
Scope: `RS/managers/meetings/store.rs` (Migration Index 2, Store-Funktionen §4, `MeetingDocument`-Felder, Soft-Delete, Epoche), neu `RS/managers/meetings/notes/{mod.rs,model.rs,templates.rs}`, `RS/managers/meetings/mod.rs` (`pub mod notes;`). Keine Commands, keine UI.
Akzeptanz: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib meetings::` → alle grün, darunter neu `store::tests::migration_keeps_legacy_rows`, `store::tests::builtin_templates_seed_idempotently`, `store::tests::notes_revision_conflict_rejects_write`, `notes::templates::tests::all_builtins_validate` (8 Vorlagen, je ≥ 1 Tasks-Abschnitt).

**P1b – KI-Notizen-Motor** · lv-coder-xhigh · L · Abh.: P1a
Scope: neu `RS/managers/meetings/llm_call.rs` (Herauslösung aus minutes.rs, Verhalten unverändert), `RS/managers/meetings/minutes.rs` (nutzt llm_call), neu `notes/{enhance.rs,assemble.rs}`, neu `RS/commands/meeting_enhance.rs`, `RS/commands/mod.rs`, `RS/lib.rs` (4 Commands + `MeetingNotesEvent`), `FE/bindings.ts` (nur diese Commands/Typen). `usage::Purpose` erhält `EnhancedNotes` (usage.rs:52).
Akzeptanz: `cargo test … --lib meetings::notes::` → grün (≥ 25 Tests, u. a. `assemble::tests::user_note_text_is_byte_identical`, `…::unknown_sources_are_dropped_and_flagged`, `enhance::tests::map_reduce_keeps_segment_ids_with_mock_llm`, `enhance::tests::no_retry_on_memory_low`); `cargo test … --lib meetings::minutes` → grün wie vor dem Refactor.

**P1c – Notizblock + Vorlagen-UI** · lv-coder · L · Abh.: P1a (parallel zu P1b)
Scope: `RS/managers/meetings/recorder.rs` (nur `position_ms`, B2) + `RS/audio_toolkit/audio/wav_writer.rs` (Getter), neu `RS/commands/meeting_notes.rs`, `RS/commands/mod.rs`, `RS/lib.rs` (Commands §5 ohne Enhance), `FE/bindings.ts`, neu `FE/components/settings/meetings/notes/{NoteBlocksEditor,LiveNotesPad,MyNotesView,TemplatePicker,TemplateManagerDialog}.tsx`, `useNotesAutosave.ts`, `FE/lib/meetingNotes.ts`, `MeetingsSettings.tsx` (Layout), `MeetingDetail.tsx` (Tab „Notizen" mit „Meine Notizen"), i18n `de`/`en` `meetings.notes.*`, `meetings.templates.*`, neu `tests/meeting-notes.spec.ts`.
Akzeptanz: `npx tsc --noEmit` → keine Fehler in berührten Dateien; `pnpm test:playwright -- tests/meeting-notes.spec.ts -g "Notizblock|Vorlagen"` → grün (Block mit `mm:ss`, entprelltes Save genau 1× nach Tippserie, Duplizieren eines Builtins); `cargo test … --lib meetings::recorder` → grün inkl. Positionstest.

**P1d – KI-Notizen-Ansicht: Quelle, Audio, Bearbeiten, Anweisung, Checkliste** · lv-coder · M · Abh.: P1b, P1c
Scope: neu `notes/{EnhancedNotesView,SourceChips,InstructionBar,TaskChecklist}.tsx`, `MeetingDetail.tsx` (Umschalter, Sprung-Handler, Segment-Markierung, Epoche/Audio-Hinweise), i18n, `tests/meeting-notes.spec.ts` (Block „KI-Notizen").
Akzeptanz: `pnpm test:playwright -- tests/meeting-notes.spec.ts -g "KI-Notizen"` → grün: Eintrag `data-origin="ai"` grau, Quell-Klick → Transkript-Tab, `[data-segment-index="12"]` markiert, Player-`playAt(195)` aufgerufen; Bearbeiten ruft `meeting_notes_update_enhanced`; Checkbox ruft `action_items_set_status`.

**P1e – Eval AK3** · lv-coder · M · Abh.: P1b
Scope: neu `FX/notes/*.json` (3 Fixtures), neu `notes/eval.rs`, `RS/cli.rs` (`--eval-notes`), `RS/lib.rs` (headless Zweig neben `run_headless_meetings`, lib.rs:736/1852).
Akzeptanz: `cargo test … --lib meetings::notes::eval` → grün (Metriken mit Stub-LLM gegen alle 3 Fixtures); danach mit Release-Binary und konfiguriertem lokalem Modell: `apps/local-voice/src-tauri/target/release/local-voice-ai.exe --eval-notes apps/local-voice/src-tauri/tests/fixtures/notes --json --out $env:TEMP\notes-eval.json` → Exit 0, `aggregate.user_preserved_ratio == 1.0`, `aggregate.ai_sourced_ratio >= 0.95`; Bericht nennt Modell, Laufzeit je Fixture, gemessene Zeichen/Token (kalibriert §6).

**P1f – Einstellungen, Auto-Lauf, Systemton, Hinweistext** · lv-coder · S–M · Abh.: P1b, P1c
Scope: `RS/settings.rs` (3 Settings + Defaults), `RS/shortcut/mod.rs` (Setter nach Muster `change_meeting_model_setting` :570), `RS/commands/meetings.rs` (Hook in `meetings_stop`, B4), `RS/lib.rs`, `FE/bindings.ts`, `FE/stores/settingsStore.ts`, `DictationTab.tsx` (Gruppe `meetings.title`), `RecorderCard.tsx` (Default Systemton, Vorlagenwahl, Hinweis kopieren), i18n.
Akzeptanz: `cargo test … --lib settings` → grün inkl. `defaults_enable_capture_system_and_auto_enhance` und Deserialisierung alter settings.json ohne die Felder; `pnpm test:playwright -- tests/meeting-notes.spec.ts -g "Hinweis|Systemton"` → grün.

**Konfliktstellen bei Parallelbetrieb**: `RS/lib.rs` (`collect_commands!` :1480ff, `collect_events!` :1612, headless-Zweig) – jedes Paket fügt einen eigenen, mit `// M1-P1x` kommentierten Block ein, Merge seriell durch den Orchestrator; `FE/bindings.ts` – gleiches Vorgehen, Debug-Export nach dem letzten Merge einmal neu erzeugen und diffen; `translation.json` de/en – Schlüssel nur unter `meetings.notes.*` (P1c), `meetings.enhanced.*` (P1d), `meetings.record.*`/`settings.meetings.*` (P1f), andere 20 Sprachen nicht anfassen (vorbestehend unvollständig); `RS/commands/mod.rs` – je eine Zeile; `MeetingDetail.tsx` – P1c vor P1d (seriell). M2-Konflikt nur über B2 (recorder.rs) und B5 (Migration).

## 13. Entscheidungen für Patrick und Risiken

- **E1 Aufbewahrung**: Default `AfterMinutes` löscht Audio nach dem Protokoll. Entwurf: KI-Notizen zählen nicht als Protokoll. Offen, ob der Default für neue Nutzer auf `Days(30)` wechseln soll, damit der Audio-Sprung (F08 „besser") dauerhaft wirkt.
- **E2 Auto-Lauf Default an**: startet nach jedem Stopp das lokale Modell (RAM, Lüfter). Alternative: Default an nur bei konfiguriertem Anbieter (ist ohnehin so: ohne Anbieter passiert nichts außer Hinweis).
- **E3 Nutzertext unveränderlich bei „Anweisung anwenden"** (z. B. „übersetze alles" betrifft nur KI-Text). Einfach und prüfbar; Alternative später per Handbearbeitung.
- **E4 Eigener Block-Editor** statt TipTap; Rich-Text (fett, Bilder) erst M10.
- Risiko Kontext 8192 lokal: Map-Reduce ist der Normalfall; Qualität der Quellen im Reduce-Schritt ist die Unbekannte, die P1e misst. Liegt `ai_sourced_ratio` unter 95 %, sind die Hebel in dieser Reihenfolge: Prompt/Beispiele → größere Blöcke mit höherem `context_tokens` (Änderung an `ensure_local`, eigenes Paket) → Nachbelegung unbelegter Einträge per zweitem kleinem Aufruf.
- Risiko Segment-IDs bei Whisper: ein Segment je 20-s-Block (ist-stand.md) → Quellen sind grob (±20 s); mit M2 (kleinere Blöcke) wird es feiner, ohne M1-Änderung.
