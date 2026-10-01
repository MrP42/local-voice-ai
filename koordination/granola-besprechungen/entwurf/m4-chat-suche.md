# M4-Entwurf: Chat, Suche, Ordner, Recipes (F11–F14, AK8)

Stand 29.09.2026 · Paket P4 (lv-architect) · Issue #59 · Branch `feat/granola-besprechungen` (0d82dac).
Pfade: RS = `apps/local-voice/src-tauri/src`, FE = `apps/local-voice/src`, FX = `apps/local-voice/src-tauri/tests/fixtures`,
SP = `C:\Users\wolff\lva-spikes\m4` (Spike-Code, nicht im Repo). Baut auf `entwurf/m1-notizen.md` (Segment-IDs, `llm_call.rs`,
Epoche) und `entwurf/m2-audio-stt.md` §3.2 (`TranscriptFinal`, `remap_sources`) auf.

## 1. Verifikation und Messungen (Spike, diese Maschine: RTX 4090, 32 logische Kerne, 64 GB; alle Läufe BelowNormal)

| # | Frage | Ergebnis | Beleg |
|---|---|---|---|
| V1 | FTS5 in rusqlite 0.37 `bundled`? | **ja**: SQLite 3.50.2, `ENABLE_FTS5` in `compile_options` | `SP/sqlspike` `features` |
| V2 | sqlite-vec statisch? | **ja**: Crate `sqlite-vec` 0.1.9 (MIT/Apache-2.0; neueste 0.1.10-alpha.4) kompiliert `sqlite-vec.c` per `cc` mit `SQLITE_CORE`, Registrierung per `sqlite3_auto_extension`; baut und läuft gegen libsqlite3-sys 0.35 (nur Linker-Hinweis .lib/.exp) | `SP/sqlspike/Cargo.toml` |
| V3 | Deutsch: unicode61 `remove_diacritics 2` vs. trigram | unicode61 findet nur ganze Wörter/Präfixe (`Kundenbetreuung` 0, `Kundenbetreuung*` 1, `budget` in „Marketingbudget" 0), dafür 2-Zeichen-Wörter (`KI` 1). trigram `remove_diacritics 1` findet Teilwörter und `gesprach`→„Gespräch", **aber nichts unter 3 Zeichen** (`KI` 0). ß↔ss faltet **keiner** (`Straße`≠`strasse`) | `sqlspike fts` |
| M1 | 100 000 Chunks (500 × 200, je ~1 000 Zeichen): Aufbau | Einfügen+f32-BLOB 5,8 s (554 MB) · FTS unicode61 2,7 s (**+34 MB**) · FTS trigram 20,5 s (**+324 MB**) · vec0 59,8 s (**+410 MB**) | `sqlspike perf 500 200 1024` |
| M2 | FTS-Abfrage top 50, `order by rank` | unicode61 seltenes Wort p95 0,3 ms; Wort in 80 % der Chunks p95 75 ms; 2-Wort-AND p95 136 ms · trigram seltener Teilstring p95 18–21 ms, häufiger p95 177 ms | dito (synthetischer Wortschatz mit 95 Wörtern = Worst Case) |
| M3 | Vektor-KNN k=50 über 100 000 × 1024 | **sqlite-vec vec0 p50 388 ms** (mit Metadatenfilter 355 ms) · Rust Brute-Force f32 1 Thread p50 52 ms, 4 Threads 23 ms, mit 5-%-Filter 2,8 ms · **int8 1 Thread p50 17 ms (102 MB RAM)** · BLOBs laden 711 ms (409 MB) | dito |
| V4 | llama-server b10938 Embeddings? | ja, `--embedding --pooling …`; Hilfe: „restrict to only support embedding use case; use only with dedicated embedding models". Chat-Anfrage an Embedding-Server → HTTP 500 ⇒ **zweiter Serverprozess nötig** | `SP/run-emb.ps1` |
| M4 | BGE-M3 Q8_0 (635 MB, MIT, 1024 Dim, sha256 950f4a8e…) | GPU: **44 Chunks/s** (302 Token/Chunk, 13 400 Tok/s), Abfrage 13 ms; CPU rein (`CUDA_VISIBLE_DEVICES=-1`): 4 Threads **1,0 Chunks/s**, 8 Threads 1,4; RAM-Spitze 1,9 GB. Achtung: CUDA-Build mit `-ngl 0` lagert große Matmuls trotzdem auf die GPU aus (14 Chunks/s) | `SP/embbench.py` |
| M5 | Qwen3-Embedding-0.6B Q8_0 (639 MB, Apache-2.0) | GPU 21,7 Chunks/s, CPU 0,5; RAM-Spitze **5,8 GB** ⇒ verworfen | dito |
| M6 | Chat 8 192 Kontext, ~4 450–5 600 Token Auszüge (28 Blöcke), 4 Fragen (3 Fakten, 1 unbeantwortbar) | Qwen3.5-9B GPU: Prefill ~7 500–8 000 Tok/s, Decode ~120 Tok/s, 0,4–0,7 s je Antwort, **4/4 richtig zitiert** · Qwen3-4B GPU 4/4, Prefill 16 000 Tok/s · Qwen3-4B **CPU** (8 Threads, Nebenlast): Prefill **53 Tok/s ⇒ 111 s** erste Frage, Folgefragen 4–13 s (Prompt-Cache) | `SP/chatbench.py` (Smoke, kein Eval) |
| M7 | Zeichen je Token Deutsch | Qwen3 **3,4**, Qwen3.5 **4,2**, BGE-M3 5,0 (gleicher Text) ⇒ M1-Annahme „3 Zeichen/Token" ist konservativ richtig | M4/M6 |

Deutsche Retrieval-Qualität nur aus Quelle: MIRACL-de nDCG@10 BGE-M3 57,6 vs. Qwen3-Emb-0.6B 54,2 (`recherche/lokaler-stack.md` §5.1).

**Entscheidungen aus dem Spike**
- **D1 Kein sqlite-vec.** vec0 ist 20× langsamer als int8-Brute-Force in Rust, verdoppelt die Vektorablage und ist pre-v1. Vektoren als f32-BLOB in einer Tabelle (Wahrheit), im RAM als int8-Cache (1 KB/Chunk); Suche int8 top 300 → f32-Nachbewertung der Kandidaten aus der DB → top 100.
- **D2 Zwei FTS5-Tabellen**: `…_fts_words` (unicode61 remove_diacritics 2) für BM25 im Chat und Wörter < 3 Zeichen; `…_fts_tri` (trigram remove_diacritics 1) für die Listen-Suche (Teilwort, Komposita). ß/ss: Rust erzeugt beide Schreibweisen als ODER-Varianten.
- **D3 BGE-M3 Q8_0 als einziges Embedding-Modell**, `--pooling cls`, eigener zweiter `llama-server` (Embedding-Modus). Qwen3-Emb-0.6B: langsamer, 3× RAM, schwächer auf Deutsch.
- **D4 Chat = fester Ablauf in Rust („Agent-light")** statt Tool-Calling durch das Modell: Suche → Shortlist (Breite) → Lesen (Tiefe) → höchstens 1 Wiederholung → Antwort → Coverage-Note. Begründung: 8 192 Kontext, 4–9B-Modelle, auf CPU kostet jede Runde ~1–2 min Prefill (M6).
- **D5 Zitate per Konstruktion**: Das Modell zitiert nur `[Q<k>]` (Auszugs-ID; im Smoke 8/8 korrekt). Rust bildet Q→(Besprechung, Epoche, Segmente) ab, verfeinert auf das beste Segment und nummeriert für die Anzeige um. Unbekannte IDs fallen weg und werden gezählt.

## 2. Architektur

```
Aufnahmen-Seite: MeetingList (+Suche, Filter, Ordner-Chips, Auswahl)  ── „Alle fragen" ──┐
MeetingDetail (+Chat-Seitenleiste Strg+J)   LiveChat unter LiveNotesPad (Aufnahme)      ChatPanel (ein Bauteil, 3 Scopes)
          ▼ commands/meeting_search.rs (Suche, Ordner)      commands/meeting_chat.rs (Chat, Threads, Recipes)
managers/meetings/search/  chunking.rs · index.rs(Store-Erw.) · vectors.rs · hybrid.rs · embed.rs · indexer.rs · bench.rs
managers/meetings/chat/    controller.rs · context.rs · prompt.rs · citations.rs · live.rs · recipes.rs · eval.rs
          ▼                                   ▼                                         ▼
meetings.db (Migration Index 3)   EMBED_SERVER: 2. LocalLlmServer (--embedding)   llm_call::resolve_provider (M1) → llama-server/Anbieter
```
Leitlinien: Index ist **abgeleitete Datei-interne Kopie** (jederzeit aus Transkript/Notizen neu erzeugbar, nie Quelle der Wahrheit). Lexikalischer Index sofort und billig; Vektoren nachgelagert im Hintergrund. Chat funktioniert ohne Vektoren (nur Stichwortsuche, in der Coverage-Note gesagt).

## 3. Datenmodell – Migration Index 3 (nach M1-Index 2)

```sql
CREATE TABLE meeting_chunks (
  id INTEGER PRIMARY KEY, meeting_id TEXT NOT NULL,
  source TEXT NOT NULL,            -- title | transcript | user_notes | ai_notes
  epoch INTEGER NOT NULL DEFAULT 0,-- transcripts.segment_epoch beim Indexieren (nur transcript)
  segment_ids TEXT NOT NULL DEFAULT '[]',  -- JSON [u32] (transcript; ai_notes: Quellen der Einträge)
  ref_keys TEXT NOT NULL DEFAULT '[]',     -- JSON: NoteBlock-IDs bzw. "E7" (ai_notes)
  document_id TEXT,                -- ai_notes: KI-Notizen-Version
  start_ms INTEGER, end_ms INTEGER, channel INTEGER,
  text TEXT NOT NULL,              -- Such-/Lesetext (Zeilen "S12 03:15 Ich: …" bei transcript)
  started_at INTEGER,              -- denormalisiert aus meetings (Zeitfilter ohne Join)
  created_at INTEGER NOT NULL);
CREATE INDEX idx_chunks_meeting ON meeting_chunks(meeting_id, source);
CREATE VIRTUAL TABLE meeting_chunks_fts_words USING fts5(text, content='meeting_chunks', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2');
CREATE VIRTUAL TABLE meeting_chunks_fts_tri USING fts5(text, content='meeting_chunks', content_rowid='id',
  tokenize='trigram remove_diacritics 1');
-- Trigger ai/ad/au auf meeting_chunks halten beide FTS-Tabellen synchron (Standardmuster external content)
CREATE TABLE meeting_chunk_vectors (chunk_id INTEGER PRIMARY KEY, model TEXT NOT NULL, dim INTEGER NOT NULL,
  vec BLOB NOT NULL);              -- f32 LE, L2-normalisiert; Trigger: löscht mit dem Chunk
CREATE TABLE meeting_index_state (meeting_id TEXT PRIMARY KEY, transcript_epoch INTEGER, transcript_rev INTEGER,
  notes_revision INTEGER, enhanced_doc_id TEXT, enhanced_updated_at INTEGER, title TEXT,
  embed_model TEXT, embedded_at INTEGER, status TEXT NOT NULL DEFAULT 'pending', error TEXT, updated_at INTEGER NOT NULL);
CREATE TABLE meeting_folders (id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT, sort INTEGER NOT NULL DEFAULT 0,
  parent_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);  -- parent_id reserviert, v1 flach
CREATE TABLE meeting_folder_items (folder_id TEXT NOT NULL, meeting_id TEXT NOT NULL, added_at INTEGER NOT NULL,
  PRIMARY KEY (folder_id, meeting_id));                           -- n:m wie Granola
CREATE INDEX idx_folder_items_meeting ON meeting_folder_items(meeting_id);
CREATE TABLE chat_recipes (id TEXT PRIMARY KEY, title TEXT NOT NULL, spec_json TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);                 -- builtin:<key> wie Vorlagen
CREATE TABLE chat_threads (id TEXT PRIMARY KEY, scope_json TEXT NOT NULL, meeting_id TEXT, title TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
CREATE TABLE chat_messages (id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
  citations_json TEXT, coverage_json TEXT, created_at INTEGER NOT NULL);
CREATE INDEX idx_chat_messages_thread ON chat_messages(thread_id, created_at);
```
- Nur `CREATE`: Altzeilen unberührt. Bestehende Besprechungen bekommen `status='pending'` erst beim ersten Indexer-Lauf (Backfill), nicht in der Migration (Migration bleibt schnell).
- **Löschen**: `soft_delete_meeting` löscht in derselben Transaktion `meeting_chunks` (→ FTS/Vektoren per Trigger), `meeting_index_state`, `meeting_folder_items`, Threads mit `meeting_id` samt Nachrichten (hart). Globale Threads behalten ihren Text; Zitate auf gelöschte Besprechungen zeigt die UI als „gelöscht" ohne Sprung.
- **Aufbewahrung**: Audio-Löschung ändert nichts am Index (Text bleibt); Zitat springt dann nur ins Transkript (wie M1 §8).
- Größe (Hochrechnung aus M1): 60-min-Besprechung ≈ 70 000 Zeichen ≈ 60 Chunks; 500 Besprechungen ≈ 30 000 Chunks ≈ 123 MB f32-Vektoren, ≈ 110 MB trigram, 31 MB int8-RAM-Cache.

## 4. Rust-Schnittstellen

`search/chunking.rs` (rein, keine I/O):
```rust
#[derive(Serialize, Deserialize, specta::Type, Clone, Copy, Debug, PartialEq)] #[serde(rename_all="snake_case")]
pub enum ChunkSource { Title, Transcript, UserNotes, AiNotes }
pub struct ChunkHead { pub title: String, pub started_at: Option<i64>, pub folder_names: Vec<String> }
pub struct ChunkDraft { pub source: ChunkSource, pub epoch: u32, pub segment_ids: Vec<u32>, pub ref_keys: Vec<String>,
    pub document_id: Option<String>, pub start_ms: Option<u64>, pub end_ms: Option<u64>, pub channel: Option<u8>,
    pub text: String /*für FTS + Prompt*/, pub embed_text: String /*"Besprechung: <Titel>, <Datum>\n"+text*/ }
pub const TARGET_CHARS: usize = 1_200; pub const MAX_CHARS: usize = 1_800; pub const PAUSE_BREAK_MS: u64 = 2_000;
pub fn chunk_transcript(segs: &[StoredSegment], epoch: u32, head: &ChunkHead) -> Vec<ChunkDraft>; // nach start_ms, Bruch an Pause/Kanalwechsel ab TARGET, hart bei MAX, 1 Segment Überlappung
pub fn chunk_user_notes(blocks: &[NoteBlock], head: &ChunkHead) -> Vec<ChunkDraft>;               // an Überschriften, ≤ TARGET
pub fn chunk_enhanced(document_id: &str, notes: &EnhancedNotes, head: &ChunkHead) -> Vec<ChunkDraft>; // je Abschnitt, Quellen vereinigt
pub fn fts_query_words(q: &str) -> Option<String>;   // Stoppwörter raus, Terme quoten, ≥5 Zeichen zusätzlich "term"*, ß/ss-Varianten, OR
pub fn fts_query_trigram(q: &str) -> Option<String>; // Terme ≥3 Zeichen quoten, AND; None → Wortsuche
```
Store-Erweiterung (`search/index.rs`, `impl MeetingStore`):
```rust
pub fn replace_meeting_chunks(&self, meeting_id: &str, sources: &[ChunkSource], drafts: &[ChunkDraft], state: &IndexState) -> Result<Vec<i64>>; // EINE Tx; nur genannte Quellen ersetzt
pub fn index_state(&self, meeting_id: &str) -> Result<Option<IndexState>>;
pub fn stale_meetings(&self, limit: u32) -> Result<Vec<String>>;        // Zustand ≠ aktuelle Epoche/Revision/Dokument/Titel
pub fn chunks_without_vectors(&self, model: &str, limit: u32) -> Result<Vec<(i64, String)>>;
pub fn put_vectors(&self, model: &str, rows: &[(i64, Vec<f32>)]) -> Result<()>;
pub fn search_words(&self, fts: &str, scope: &[String], limit: u32) -> Result<Vec<(i64, f64)>>;   // bm25
pub fn search_meetings(&self, query: &str, filter: &MeetingFilter, offset: u32, limit: u32) -> Result<MeetingSearchPage>;
pub fn get_chunks(&self, ids: &[i64]) -> Result<Vec<ChunkRow>>;
pub fn resolve_scope(&self, scope: &ScopeFilter) -> Result<Vec<String>>;  // Ordner, Person, Zeitraum, Auswahl → IDs (nur live)
pub fn folders_list(&self) -> Result<Vec<Folder>>; pub fn folder_save(&self, id: Option<&str>, name: &str, color: Option<&str>) -> Result<Folder>;
pub fn folder_delete(&self, id: &str) -> Result<()>; pub fn set_meeting_folders(&self, meeting_id: &str, folder_ids: &[String]) -> Result<()>;
pub fn recipes_list(&self) -> Result<Vec<RecipeInfo>>; pub fn recipe_save(…) -> Result<RecipeInfo>; pub fn recipe_delete(&self, id: &str) -> Result<()>;
pub fn thread_create/thread_append/thread_list(scope)/thread_get/thread_delete
```
`search/vectors.rs`: `pub struct VectorIndex { model, dim, ids: Vec<i64>, meeting_ix: Vec<u32>, q8: Vec<i8>, scale: Vec<f32> }` · `load(store, model)` · `upsert(&mut self, rows)` · `remove_meeting(&mut self, id)` · `topk(&self, q: &[f32], k: usize, mask: Option<&[bool]>) -> Vec<(i64, f32)>` (int8, 1 Thread) · `rescore(store, q, cands, k)` (f32). Global `RwLock<Option<VectorIndex>>`, bei 10 min ohne Nutzung verworfen.
`search/hybrid.rs`: `pub fn rrf(lists: &[Vec<(i64, f64)>], k: f64 /*60*/, top: usize) -> Vec<(i64, f64)>`; `pub async fn hybrid_search(store, embed: &dyn Embedder, query, scope_ids, top) -> HybridResult { hits, lexical_only: bool }`.
`search/embed.rs`:
```rust
#[async_trait] pub trait Embedder: Send + Sync { async fn embed(&self, texts: &[String], kind: EmbedKind) -> Result<Vec<Vec<f32>>, EmbedError>; fn model_id(&self) -> &str; }
pub enum EmbedKind { Query, Document }            // BGE-M3 ohne Anweisung; Qwen3 bräuchte "Instruct:…" (nicht gewählt)
pub enum EmbedError { NoModel, MemoryLow, Busy, Failed(String) }
pub struct LlamaEmbedder;                          // nutzt llm::ensure_embedding()
```
`llm/server.rs` (additiv): `StartOptions.embedding: Option<EmbeddingOpts { pooling: &'static str /*"cls"*/, parallel: u32 /*2*/, below_normal: bool }>` → Argumente `--embedding --pooling cls -b 2048 -ub 2048 -np 2`; auf CPU-Backend `-t 4`; Windows-Prozessklasse BELOW_NORMAL (Creation-Flag `0x4000`). `llm/mod.rs`: zweites Global `EMBED_SERVER`, `pub async fn ensure_embedding(model_id) -> Result<String,String>` (RAM-Gate wie `ensure`), `pub fn stop_embedding()`; Speicherwächter in `lib.rs:242` stoppt auch `EMBED_SERVER`.
`search/indexer.rs`:
```rust
pub enum IndexJob { Meeting(String), Deleted(String), Backfill, EmbedPending }
pub struct MeetingIndexer { tx: std::sync::mpsc::Sender<IndexJob> }
impl MeetingIndexer { pub fn spawn(store: Arc<MeetingStore>, embed: Arc<dyn Embedder>, gates: Arc<dyn IndexGates>) -> Self; pub fn submit(&self, job: IndexJob); }
pub trait IndexGates: Send + Sync { fn recording_active(&self) -> bool; fn processing_active(&self) -> bool; fn enhance_running(&self) -> bool; }
```
Lexikalische Stufe sofort (ein Thread, ~ms je Besprechung). Vektorstufe nur wenn alle Gates frei, Einstellung an, Modell vorhanden; Chargen à 16 Chunks, nach jeder Charge Gates neu prüfen; Server nach 5 min Leerlauf stoppen. Auslöser: `TranscriptFinal` (M2) bzw. Status `ready` ohne M2, Neu-Transkription fertig, `meeting_notes_save` (entprellt 30 s), neue/bearbeitete KI-Notizen-Version, Umbenennen, Import fertig, Löschen, App-Start (`Backfill`).

`chat/`:
```rust
pub enum ChatScope { Meeting { meeting_id: String }, Global { filter: ScopeFilter } }
pub struct ScopeFilter { pub meeting_ids: Option<Vec<String>>, pub folder_id: Option<String>, pub person: Option<String>, pub from: Option<i64>, pub to: Option<i64> }
pub struct ChatRequest { pub request_id: String, pub thread_id: Option<String>, pub scope: ChatScope, pub question: String, pub recipe: Option<RecipeCall> }
pub struct Citation { pub n: u32, pub meeting_id: String, pub meeting_title: String, pub started_at: Option<i64>, pub source: ChunkSource,
    pub epoch: u32, pub segment_index: Option<u32>, pub start_ms: Option<u64>, pub ref_key: Option<String>, pub quote: String /*≤ 200 Z.*/ }
pub struct Coverage { pub meetings_in_scope: u32, pub meetings_with_hits: u32, pub meetings_read: u32, pub excerpts_read: u32,
    pub lexical_only: bool, pub rounds: u8, pub truncated: bool, pub live: bool, pub dropped_citations: u32 }
pub struct ChatAnswer { pub thread_id: String, pub message_id: String, pub text: String /*mit [1]…*/, pub citations: Vec<Citation>,
    pub coverage: Coverage, pub not_found: bool, pub provider_local: bool }
pub async fn ask(settings: &AppSettings, store: Arc<MeetingStore>, embed: Arc<dyn Embedder>, live: Option<LiveSnapshot>,
    req: ChatRequest, on_delta: impl Fn(&str) + Send + Sync) -> Result<ChatAnswer, ChatError>;
pub fn context_budget_tokens(ctx_tokens: u32, backend_cpu: bool, local: bool) -> u32;  // 8192 GPU ⇒ ~5 200; CPU ⇒ min(…, 2 000); Anbieter ⇒ 12 000
pub fn postprocess(raw: &str, excerpts: &[Excerpt]) -> (String, Vec<Citation>, u32 /*dropped*/, bool /*not_found*/); // rein
pub fn refine_segment(sentence: &str, excerpt: &Excerpt) -> Option<u32>;  // größte Inhaltswort-Überlappung, Gleichstand → frühestes
```
`llm_client.rs`: neu `send_chat_completion_stream(purpose, provider, api_key, model, messages, on_delta) -> Result<String,String>` (SSE, `stream: true`; bei Fehler vor erstem Token Rückfall auf nicht-streamend). `usage::Purpose::Chat` (Zähler, kein Text). Thinking aus wie in `llm_call` (Qwen3.5: `enable_thinking=false`).

## 5. Commands, Events, TS-Typen

`commands/meeting_search.rs`: `meetings_search(query: String, filter: MeetingFilter, offset: u32, limit: u32) -> MeetingSearchPage` (`MeetingFilter {folder_id?, from?, to?, source?, has_notes?}`, `MeetingSearchPage {items: [{meeting: Meeting, snippet?: string, hit_source?: ChunkSource}], total}`) · `meeting_folders_list/save/delete` · `meetings_set_folders(meeting_id, folder_ids)` · `meeting_index_status() -> IndexStatus {pending, lexical_done, embedded, total, model_ready, running}` · `meeting_embedding_model_download()` (Katalogeintrag `emb-bge-m3-q8`, Muster LLM-Downloads).
`commands/meeting_chat.rs`: `meeting_chat_ask(req: ChatRequest) -> ChatAnswer` (Deltas per Event) · `meeting_chat_cancel(request_id)` · `meeting_chat_threads(scope) -> ThreadInfo[]` · `meeting_chat_thread(id) -> ChatMessage[]` · `meeting_chat_thread_delete(id)` · `chat_recipes_list/save/delete/duplicate`.
Event `MeetingChatEvent` (`tag="kind"`, in `collect_events!`): `delta {request_id, text}` · `stage {request_id, stage: searching|reading|answering, round}` · `failed {request_id, code}`; `code` ∈ `no_provider | no_model | memory_low | recording_active_cpu | chat_busy | empty_scope | llm_failed | cancelled`. Event `MeetingIndexEvent::progress {done,total}` für die Einstellungszeile. TS aus `bindings.ts` (von Hand nachziehen); Helfer `FE/lib/meetingChat.ts`: `splitAnswer(text) → (Text|Cite)[]`, `formatCoverage(c, t)`, `recipePlaceholders(spec)`.

## 6. Chat-Pipeline

**Budget** (M6/M7): lokal `ctx 8192 − Antwort 1 024 − System+Recipe ≤ 900 − Verlauf ≤ 1 000` ⇒ ~5 200 Token ≈ 17 700 Zeichen (3,4 Z./Token, schlechtester Messwert). CPU-Backend: 2 000 Token (≈ 40 s Prefill bei 53 Tok/s) + Hinweis „CPU: weniger Auszüge gelesen". Prompt-Reihenfolge `[System][Auszüge][Verlauf][Frage]`; Folgefragen im selben Thread behalten alte Auszüge vorn (Prompt-Cache von llama-server, M6: Folgefragen 4–13 s statt 111 s auf CPU).

**Je Besprechung (fertig)**: (1) Passt alles (Nutzernotizen + jüngste KI-Notizen + Transkript) ins Budget → komplett lesen, keine Suche. (2) Sonst: KI-Notizen als Überblick (Breite, mit ihren Quellen) + hybride Suche nur in dieser Besprechung, Auszüge mit ±1 Nachbar-Chunk bis Budget.
**Live (Aufnahme läuft)**: kein Index, keine Embeddings. `chat/live.rs` nimmt `LiveSnapshot {segments, notes, position_ms}` aus Store + B2 `position_ms`: letzte 8 min komplett (Frage „Was habe ich verpasst?"), ältere Segmente per In-Memory-BM25 (rein, keine DB), Nutzernotizen immer. Lokales LLM während der Aufnahme nur mit GPU-Backend (STT-Latenz AK5); CPU → `recording_active_cpu` (Anbieter extern: erlaubt). Zitate tragen die Live-Epoche; nach `TranscriptFinal` werden gespeicherte Zitate beim Anzeigen per `remap_sources` (M2/P2d) umgesetzt, ohne M2 als „veraltet" markiert.
**Global (alle / Ordner / Person / Zeitraum / Auswahl)**:
1. `resolve_scope` → Besprechungs-IDs. Person v1 = Name in `speakers.display_name` (ab M3 gefüllt) ODER Wortsuche im Titel/Notizen/Transkript; M5 ergänzt Kalender-Teilnehmer ohne Schnittstellenänderung.
2. Suche: Wort-FTS top 100 + Vektor top 100 (int8 top 300 → f32) → RRF (k=60) → top 40 Chunks.
3. Shortlist (Breite): Score je Besprechung = max + 0,3·Summe Rest; top 8 Besprechungen als Karte (Titel, Datum, Ordner, erster KI-Notizen-Abschnitt ≤ 200 Z.).
4. Lesen (Tiefe): Chunks der Shortlist, je Besprechung ≤ 3, absteigend, bis Budget; KI-Notizen-Chunks bevorzugt vor Transkript bei gleichem Rang (Granola: Notizen ~10× günstiger).
5. Antwort; enthält sie `KEIN_BELEG` und gibt es ungelesene Kandidaten (Ränge 9–20) → **eine** Wiederholung mit diesen.
6. Coverage-Note deterministisch: „Durchsucht: 132 Besprechungen (Ordner Vertrieb, 01.–30.09.). Gelesen: 6 Besprechungen, 11 Stellen. 14 weitere mit Treffern nicht gelesen – Frage eingrenzen." (+ „nur Stichwortsuche", wenn ohne Vektoren).

**Prompt** (System Deutsch, weil Antwortsprache = Frage; Beispiel-Satz aus dem Smoke M6 bewährt):
```
Du beantwortest Fragen zu Besprechungen des Nutzers. Nutze NUR die Auszüge unten.
Belege jede Aussage direkt dahinter mit der Auszugs-ID in eckigen Klammern, z. B. [Q3]. Mehrere: [Q3][Q7].
Erfinde keine Namen, Zahlen, Termine. Wenn die Auszüge die Frage nicht beantworten, schreibe genau: KEIN_BELEG
Antworte knapp, in der Sprache der Frage, ohne Überschriften. Datum der Besprechung angeben, wenn mehrere beteiligt sind.
```
User: `Heute: <Datum>` · `Überblick:` Karten `B2 "Titel" 12.09.2026 · Ordner Vertrieb · <Kurzfassung>` · `Auszüge:` je `[Q5] B2 · Transkript 03:15–04:40` + Zeilen `S12 03:15 Ich: …` (Notizen: `[Q6] B2 · Meine Notizen` / `KI-Notizen, Abschnitt Entscheidungen`) · Verlauf · `Frage: …`. Freitext statt JSON-Schema, damit gestreamt werden kann.
**Nachbearbeitung `postprocess`** (jede Regel ein Test): `[Q<k>]`-Marker (auch `[Q3, Q7]`, `(Q3)`) erkennen; unbekannte k verwerfen → `dropped_citations`; je Satz das Segment per `refine_segment` wählen (nur Segmente des Auszugs); Anzeige-Nummern `[1]…` in Reihenfolge des ersten Auftretens, gleiche Quelle = gleiche Nummer; `KEIN_BELEG` (auch mit Satzzeichen/Leerraum) → `not_found=true`, Text durch i18n ersetzt, Zitate leer; Antwort ohne jedes Zitat und nicht `not_found` → Hinweis „ohne Beleg" (wie M1).

## 7. Recipes (F13) mit Variablen

`RecipeSpec { version: 1, prompt: String /*mit {{name}}*/, variables: Vec<RecipeVar>, scope: RecipeScope /*meeting|global|any*/, live_ok: bool }`, `RecipeVar { name: [a-z_]{1,24}, label, kind: text|person|folder|date_from|date_to|meeting, required, default? }`. `render_recipe(spec, values) -> Result<(String, ScopeFilter-Patch), RecipeError>`: `person/folder/date_*` setzen zusätzlich den Filter (Variable wirkt auf Text UND Suche); fehlende Pflichtvariable → Fehler vor dem LLM-Aufruf; unbekannte `{{x}}` in `validate_spec` abgelehnt; Prompt ≤ 2 000 Z., ≤ 6 Variablen. Aufruf im Chat per `/` (Menü) oder Knopfleiste; Ausfüllen inline als Chips. Mitgeliefert (`builtin:<key>`, schreibgeschützt, Duplizieren wie Vorlagen): „Was habe ich verpasst?" (meeting, live) · „Was sollte ich jetzt fragen?" (meeting, live) · „Follow-up-E-Mail an {{empfaenger}}" · „Offene Aufgaben von {{person}} seit {{date_from}}" · „Entscheidungen im Ordner {{folder}}" · „Einwände und Bedenken von Kunden" · „Vorbereitung auf das Gespräch mit {{person}}". Export/Import wie Vorlagen (`lva-chat-recipe@1`).

## 8. UI-Skizze (kein neuer Menüpunkt, kein neuer Reiter)

**Aufnahmen → MeetingList**: Kopfzeile Suchfeld („Besprechungen durchsuchen …", 250 ms Entprellung, `meetings_search`) + Filter-Chips (Zeitraum, Quelle, „mit Notizen") + Ordner-Chips „Alle · Vertrieb · Projekte · +". Treffer zeigen Snippet mit Markierung (FTS5 `snippet()`); ohne Suchtext bisherige Liste (25er-Seiten). Kontextmenü je Besprechung „In Ordner …" (Mehrfachwahl), Drag auf Ordner-Chip; Ordner umbenennen/löschen im Chip-Kontextmenü (Besprechungen bleiben). Auswahlmodus (Checkboxen) → „Auswahl fragen". Knopf „Alle Besprechungen fragen" öffnet das ChatPanel als Seitenleiste mit Scope-Chips (aktueller Ordner/Filter vorbelegt).
**MeetingDetail**: Knopf „Fragen" (Strg+J) öffnet rechts das ChatPanel (Scope = diese Besprechung); die Tabs bleiben sichtbar, damit der Zitatsprung daneben landet. **Aufnahmeseite**: unter LiveNotesPad eingeklappte Zeile „Frage zur laufenden Besprechung" (Recipes „Was habe ich verpasst?", „Was sollte ich fragen?").
**ChatPanel**: Verlauf (Threads je Scope, Liste oben), Antwort gestreamt, Zitate als Chips `[1]` mit Tooltip (Besprechung, Datum, `03:15`, Zitat); Klick → MeetingDetail (bei global erst öffnen) → Tab Transkript → Segment markiert + `playSegment()` (P1d-Sprung-Handler wiederverwenden); Notizen-Zitat → Tab Notizen, Block markiert. Unter jeder Antwort grau die Coverage-Note. Externer Anbieter: gelbe Leiste „Antworten erzeugt <Anbieter>. Ausschnitte aus deinen Besprechungen werden dorthin übertragen." + einmalige Bestätigung je Anbieter. **Einstellungen** (Gruppe „Besprechungen"): „Semantische Suche (lädt 635 MB)" mit Download und Indexstand „412 / 500 Besprechungen".

## 9. Ressourcen und Datenschutz

- Embedding-Server: RAM-Spitze 1,9 GB (M4), VRAM-Bedarf nicht gemessen (Modell 635 MB + Puffer, Schätzung ≤ 1,5 GB; P4b misst). Start nur über `ensure_embedding` mit `check_ram_for_start`, Job-Objekt-Deckel, Wächter-Stopp. Nie gleichzeitig mit Aufnahme, Enddurchlauf (`processing`) oder KI-Notizen-Lauf (Gates); CPU-Backend 4 Threads, BelowNormal. Nachholen von 500 Besprechungen (~30 000 Chunks): GPU ~11 min, CPU ~8 h verteilt auf Leerlauf (M4-Rate); Fortschritt sichtbar, jederzeit abbrechbar.
- Chat-LLM: vorhandener `llama-server` über `resolve_provider` (M1). Beide Server gleichzeitig möglich (Qwen3.5-9B 5,5 GB + BGE 1,9 GB RAM, M4/M6); beim Chat wird der Embedding-Server nur für die Abfrage gebraucht und nach 5 min Leerlauf gestoppt.
- Vektor-Cache ≤ 1 KB/Chunk im RAM, verfällt nach 10 min.
- Nichts verlässt den Rechner: Embeddings immer lokal; Index in `meetings.db` (gleicher Ort, gleiche Sandbox `LVA_MEETINGS_DIR`). Kein Frage-, Antwort- oder Auszugstext im Log (nur Längen, Zähler, Codes; wie M1 D9). Weiche Löschung der Besprechung löscht Index und Besprechungs-Chats hart.

## 10. Fehlerfälle

| Fall | Verhalten |
|---|---|
| Embedding-Modell fehlt / Server startet nicht / RAM | Chat und Suche lexikalisch; `lexical_only=true` in Coverage; Indexer markiert `status='lexical'`, versucht beim nächsten Leerlauf erneut (kein Retry-Sturm: Backoff 10 min) |
| Kein LLM-Anbieter / Modell | `no_provider`/`no_model`; Suche funktioniert weiter |
| Frage während Aufnahme, lokales LLM auf CPU | `recording_active_cpu` mit Hinweis; mit GPU oder externem Anbieter erlaubt |
| Zweite Frage während einer läuft | `chat_busy` (ein Chat-Lauf gleichzeitig; KI-Notizen-Lauf hat Vorrang über `EnhanceGuard`) |
| Leerer Scope (Ordner ohne Besprechungen) | `empty_scope` vor jedem LLM-Aufruf |
| Modell zitiert unbekannte ID / gar nicht | verworfen und gezählt / Hinweis „ohne Beleg" |
| Neu-Transkription / Enddurchlauf während Index | Indexer vergleicht Epoche vor dem Schreiben; veraltet → Job neu einreihen |
| Besprechung gelöscht während Chat | Zitate darauf fallen beim Speichern weg; Antwort bleibt |
| Index beschädigt / Schema-Drift | `meeting_reindex_all` (Einstellungen) löscht `meeting_chunks` und baut neu; Quelle der Wahrheit unberührt |
| SQLite gesperrt (Recorder schreibt) | rusqlite-Standard `busy_timeout` 5 s (inner_connection.rs:119); Indexer-Transaktion je Besprechung (< 100 ms), nie über mehrere |
| Abbruch durch Nutzer | `meeting_chat_cancel` bricht Stream ab, nichts gespeichert |

## 11. Testplan

Rust `--lib meetings::search` / `meetings::chat`: Migration (DB mit `MIGRATIONS[..3]` + Altzeilen → unverändert, neue Tabellen leer); Chunking (Pause-/Kanalbruch, MAX hart, Überlappung, Umlaute, leere Segmente); FTS (Kompositum per trigram, `KI` per words, ß/ss, Treffer nur live/Scope); Trigger (Löschen entfernt FTS + Vektoren); `soft_delete_meeting` räumt Index/Ordner/Threads; RRF (Gleichstand, Einzel-Liste); int8-topk gleich f32-topk auf festen Vektoren (Recall@10 = 1,0 nach Rescoring); Indexer-Gates mit Fake-Embedder (Aufnahme → keine Vektoren; Epoche geändert → Neuindex); `postprocess`-Regeln; `refine_segment`; Budget-Tabelle; Live-Fenster; Recipe-Validierung/Rendering; Controller mit `spawn_llm_mock` (minutes.rs) inkl. Wiederholungsrunde und `KEIN_BELEG`.
Playwright (Attrappe): `tests/meeting-search.spec.ts` (Suche ruft `meetings_search` entprellt, Snippet, Ordner-Chip filtert, „In Ordner" ruft `meetings_set_folders`), `tests/meeting-chat.spec.ts` (Delta-Events rendern, Zitat-Chip → Transkript-Tab + `playAt`, Coverage sichtbar, externer Anbieter zeigt Leiste, Recipe-Variable als Chip).
Performance: `--bench-search --meetings 500 --chunks 200 --json` (synthetische Sandbox-DB in Temp): p95 `ui_search_ms`, `hybrid_ms` (Abfragevektor fest, ohne Embedding-Aufruf), `lexical_ms`; Exit 3 bei p95 ≥ 500.
**Eval AK8** `--eval-chat <dir> [--json] [--out f] [--lexical-only]`: Sandbox-Store, Fixtures importieren, Index inkl. Vektoren synchron aufbauen (startet/stoppt Embedding-Server), Fragen seriell, danach beide Server stoppen. Fixtures: die 3 aus P1e (`FX/notes/`) + 2 neue in `FX/chat/` (`nordlicht_folgetermin.json` – zweites Gespräch mit demselben Kunden, widersprüchlicher Termin; `projekt_statusrunde.json` – Zahlen, Namen, Aufgaben) mit Feld `folder` und `participants`. `FX/chat/questions.json` ≥ 24 Fragen: 10 je Besprechung, 8 global (davon 3 über zwei Besprechungen), 3 mit Ordner/Person/Zeitraum-Filter, 3 unbeantwortbar. Je Frage `{id, scope, question, expect: {all_of: [[Synonyme…]…], meeting, segments: [..]} | {not_found: true}}`. Richtig = alle Schlüsselgruppen im normalisierten Text (klein, ß→ss, Ziffern/Zahlwörter) **und** ≥ 1 Zitat, dessen Chunk ein erwartetes Segment enthält; unbeantwortbar = `not_found` ohne Zitate. Ausgabe je Frage + `aggregate.accuracy`, Laufzeit, Modell; Exit 0 bei ≥ 0,85, 3 darunter, 1 Fehler.

## 12. Paketschnitt

Wellen: **W1** P4a → **W2** P4b ∥ P4c ∥ P4d → **W3** P4e ∥ P4f.

**P4a – Index-Fundament** · lv-coder-xhigh (Migration) · L · Abh.: P1a (abgenommen)
Scope: `RS/managers/meetings/store.rs` (Migration Index 3, `soft_delete_meeting` erweitert), neu `RS/managers/meetings/search/{mod,chunking,index,vectors,hybrid,bench}.rs`, `meetings/mod.rs` (`pub mod search;`), `RS/cli.rs` + `RS/lib.rs` (`--bench-search`, eigener `// M4-P4a`-Block). Keine Commands, kein Embedding-Server.
Akzeptanz: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib meetings::` → grün, ≥ 20 neue Tests, darunter `store::tests::migration_3_keeps_m1_rows`, `search::index::tests::trigram_finds_compound_part`, `…::words_finds_two_letter_term`, `…::soft_delete_purges_chunks_fts_vectors`, `search::vectors::tests::int8_rescored_matches_f32_top10`; `target/release/local-voice-ai.exe --bench-search --meetings 500 --chunks 200 --json` → Exit 0, `ui_search_p95_ms < 500`, `hybrid_p95_ms < 500`.

**P4b – Embedding-Server + Indexer** · lv-coder-xhigh (Prozess-/Speicherschutz) · L · Abh.: P4a; P2a für `TranscriptFinal` (ohne: Auslöser Status `ready`)
Scope: `RS/managers/llm/{server.rs,mod.rs}` (Embedding-Modus, `EMBED_SERVER`, `ensure_embedding`), `RS/lib.rs` (Wächter :242, Indexer-Start, Backfill, Event), `RS/catalog/catalog.json` (`emb-bge-m3-q8`, sha256 `950f4a8e5e19477a6d3c26d2f162233c20002c601f75e4b002e3239997821167`, 634 553 760 B, gpustack/bge-m3-GGUF, MIT), `llm/runtime.rs` (Download-Zweck), neu `search/{embed,indexer}.rs`, `RS/settings.rs` (`meeting_semantic_search: bool`, Default true, wirkt erst mit Modell), neu `RS/commands/meeting_search.rs` (nur `meeting_index_status`, `meeting_embedding_model_download`), Auslöser-Aufrufe in `commands/meetings.rs`/`meeting_notes.rs`/`meeting_enhance.rs` (je eine Zeile `indexer.submit`), `--reindex-meetings`.
Akzeptanz: `cargo test … --lib meetings::search::indexer` → ≥ 8 Tests grün (Gates, Epoche, Löschen, Backoff); `cargo test … --lib llm::server` grün inkl. `embedding_args_contain_pooling_cls`; `local-voice-ai.exe --reindex-meetings --json` mit `LVA_MEETINGS_DIR`=Sandbox aus 3 P1e-Fixtures → `chunks > 0`, `vectors == chunks`, `embed_server_running=false` am Ende; Bericht nennt VRAM/RAM-Spitze und Was-passiert-bei-vollem-RAM.

**P4c – Chat-Motor** · lv-coder-xhigh (Zitat-Korrektheit = Gate-Logik) · L · Abh.: P4a, P1b (`llm_call.rs`); Embedder als Trait → parallel zu P4b
Scope: neu `RS/managers/meetings/chat/{mod,controller,context,prompt,citations,live,recipes}.rs`, `RS/llm_client.rs` (Stream-Funktion), `RS/managers/usage.rs` (`Purpose::Chat`), neu `RS/commands/meeting_chat.rs`, `commands/mod.rs`, `lib.rs` (Commands + `MeetingChatEvent`), `FE/bindings.ts` (nur diese Typen), Recipes-Builtins + Store-Funktionen Threads/Recipes.
Akzeptanz: `cargo test … --lib meetings::chat` → ≥ 25 Tests grün, u. a. `citations::tests::unknown_ids_dropped_and_counted`, `…::renumbers_in_order_of_first_use`, `…::kein_beleg_sets_not_found`, `…::refine_picks_overlapping_segment`, `context::tests::cpu_budget_smaller`, `live::tests::recent_window_always_included`, `controller::tests::second_round_on_not_found_with_mock_llm`, `recipes::tests::person_variable_sets_filter`; `cargo test … --lib llm_client` grün.

**P4d – Suche, Filter, Ordner in der Liste** · lv-coder · M · Abh.: P4a
Scope: `RS/commands/meeting_search.rs` (Suche, Ordner-Commands), `lib.rs`, `FE/bindings.ts`, `FE/components/settings/meetings/MeetingList.tsx`, neu `…/meetings/search/{SearchBar,FilterChips,FolderChips,FolderPickerDialog}.tsx`, i18n de/en `meetings.search.*`, `meetings.folders.*`, neu `tests/meeting-search.spec.ts`.
Akzeptanz: `npx tsc --noEmit` ohne Fehler in berührten Dateien; `pnpm test:playwright -- tests/meeting-search.spec.ts` → grün (entprellter Aufruf genau 1× je Tippserie, Snippet mit `<mark>`, Ordner-Chip setzt `folder_id`, Mehrfach-Zuordnung).

**P4e – Chat-Oberfläche** · lv-coder · L · Abh.: P4c, P4d, P1d (Sprung-Handler), P1c (LiveNotesPad)
Scope: neu `FE/components/settings/meetings/chat/{ChatPanel,ChatMessage,CitationChip,CoverageNote,RecipeMenu,RecipeManagerDialog,ScopeChips}.tsx`, `FE/lib/meetingChat.ts`, `MeetingDetail.tsx` (Seitenleiste, Strg+J), `MeetingsSettings.tsx` (Live-Zeile), `MeetingList.tsx` (Auswahl/„fragen"), `DictationTab.tsx` (Einstellungszeile semantische Suche), i18n `meetings.chat.*`, `meetings.recipes.*`, neu `tests/meeting-chat.spec.ts`.
Akzeptanz: `pnpm test:playwright -- tests/meeting-chat.spec.ts` → grün (Deltas, Zitat → `[data-segment-index]` markiert + `playAt`, Coverage, Anbieter-Leiste, Recipe-Variable, Abbrechen ruft `meeting_chat_cancel`).

**P4f – Eval AK8** · lv-coder · M · Abh.: P4b, P4c, P1e (Fixtures)
Scope: neu `FX/chat/{nordlicht_folgetermin,projekt_statusrunde,questions}.json`, neu `chat/eval.rs`, `RS/cli.rs` + `lib.rs` (`--eval-chat`).
Akzeptanz: `cargo test … --lib meetings::chat::eval` → grün (Metrik mit Stub-LLM: perfekte Antworten 1,0; falsches Zitat 0); danach Release-Binary mit lokalem Modell: `local-voice-ai.exe --eval-chat apps/local-voice/src-tauri/tests/fixtures/chat --json --out $env:TEMP\chat-eval.json` → Exit 0, `questions >= 20`, `meetings >= 5`, `aggregate.accuracy >= 0.85`; Bericht mit Modell, Laufzeit je Frage, Vergleichslauf `--lexical-only`.

## 13. Konfliktstellen mit laufenden Paketen

| Datei | Andere Pakete | Regel |
|---|---|---|
| `store.rs` MIGRATIONS | P1a (Index 2, fertig); M3 plant ggf. `speakers`/`humans`-Änderungen | M4 belegt **Index 3**; wer später kommt, rebased und nimmt 4. P2 fügt keine Migration hinzu (M2 B5) |
| `store.rs` `soft_delete_meeting`, `replace_segments` | P2d (replace_segments, Epoche+1) | P4a ändert nur `soft_delete_meeting`; Index reagiert auf Epoche, nicht auf `replace_segments` direkt |
| `lib.rs` (Commands, Events, headless, Wächter :242) | P1b–P1f, P2a, P2b2, P2d, P2e | je Paket eigener `// M4-P4x`-Block, seriell mergen |
| `cli.rs` | P1e `--eval-notes`, P2b2 `--simulate-meeting` | neue Flags nur anhängen |
| `bindings.ts`, `commands/mod.rs` | alle UI-/Command-Pakete | von Hand, nur eigene Typen; nach letztem Merge Debug-Export diffen |
| `MeetingDetail.tsx` | P1c, P1d | P4e erst nach P1d |
| `MeetingsSettings.tsx`, `DictationTab.tsx`, `settings.rs` | P1c, P1f | P4b/P4e nach P1f; Settings-Felder additiv mit serde-Default |
| `MeetingList.tsx` | – (nur P4d, danach P4e) | seriell P4d → P4e |
| `catalog.json` | P2a (Silero-VAD-Eintrag möglich) | Eintrag ans Ende des Arrays, Merge von Hand |
| `llm/server.rs`, `llm/mod.rs` | P1b nutzt `ensure_local` unverändert | P4b rein additiv (`embedding: None` = heutiges Verhalten) |
| `usage.rs` Purpose, `llm_client.rs` | P1b (`EnhancedNotes`) | je eine Enum-Zeile; Stream-Funktion neu, bestehende unverändert |
| Events | P2a/P2d `TranscriptFinal` | P4b abonniert; bis P2d gemergt ist Status `ready` der Auslöser |
| i18n de/en | P1c/P1d/P1f | nur `meetings.search.*`, `.folders.*`, `.chat.*`, `.recipes.*` |

## 14. Entscheidungen für Patrick und Risiken

- **E6 Semantische Suche lädt 635 MB** (BGE-M3): Default „an, sobald heruntergeladen"; Download per Knopf in „Besprechungen", nicht automatisch. Ohne: Chat nur Stichwortsuche (AK8 wird mit Vektoren gemessen, `--lexical-only` als Vergleich).
- **E7 Live-Chat mit lokalem LLM nur bei GPU** (Schutz AK5). Alternative: auch CPU mit Warnung.
- **E8 Chats werden gespeichert** (Threads je Scope); Granola speichert Live-Chats nicht. Alternative: Live-Chats flüchtig.
- **E9 Ordner flach, n:m** (Granola: eine Verschachtelungsebene); `parent_id` reserviert.
- Risiko 8 192 Kontext: globale Fragen über viele Besprechungen („alle Einwände seit Juli") sehen höchstens ~14 Auszüge; Coverage-Note macht das sichtbar. Hebel: größerer `context_tokens` für GPU-Rechner (eigenes Paket, betrifft auch M1), Map-Reduce über Besprechungskarten.
- Risiko Zitatqualität kleiner Modelle bei 24 echten Fragen: Smoke (M6) war 8/8, aber mit künstlich klaren Auszügen. Hebel: `refine_segment`, Few-shot-Beispiel, Qwen3.5-9B als Mindestmodell für Chat.
- Risiko synthetische Perf-Messung: Wortschatz 95 Wörter ist schlechter als echte Sprache (häufige Terme), Werte gelten als obere Schranke.
- Offen: harte Löschung von Besprechungen (heute nur weich; Transkripttext bleibt in `transcripts`) – außerhalb M4, betrifft Index nicht.
