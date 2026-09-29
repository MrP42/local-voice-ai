# Ist-Stand Besprechungen in Local Voice AI (Discovery 29.09.2026)

Erhoben per Explore-Agent (nur gelesen, Testzahlen = Grep-Zählungen). Pfade: RS = `apps/local-voice/src-tauri/src`, FE = `apps/local-voice/src`, DOC = `docs`.
UI-Name der Seite: „Aufnahmen" (Sidebar-ID `meetings`); Einstellungen: Bereich „Besprechungen".

**Kernbefund:** M8 (Aufnahme, Import, Protokoll, Export, Aufbewahrung) und Neu-Transkription sind gebaut. Es fehlen Notizen, Vorlagen, Diarisierung, Suche/Chat, Editor, Kalender, Meeting-Sync.

| Bereich | Ist | Lücke gegenüber Granola | Fundstelle |
|---|---|---|---|
| Start/Stopp | `meetings_start(title, consent, capture_system)`; Einwilligungsdialog Pflicht (§ 201 StGB); ein Meeting gleichzeitig, Diktat gesperrt; Pause verwirft Samples; Tray-/Overlay-Indikator; Stopp arbeitet Restblöcke ab | kein globaler Hotkey, kein Kalender-Auto-Start; Systemton-Häkchen standardmäßig aus | RS/managers/meetings/recorder.rs:67,280-306,549,580; RS/commands/meetings.rs:20-63; FE/components/settings/meetings/RecorderCard.tsx:35 |
| Audioquellen | Mikrofon (cpal), Systemton per WASAPI-Loopback auf Standard-Ausgabe (nur Windows); zwei WAVs `mic.wav`/`system.wav` 16 kHz mono i16, Kanal 0 „Ich", 1 „Gegenseite"; Start-Timeout 5 s, Watchdog | kein Systemton macOS; keine Einzel-App-Aufnahme; **keine Echo-Unterdrückung** (ohne Headset Doppelungen, vermutet) | RS/managers/meetings/mic_capture.rs; RS/audio_toolkit/audio/loopback.rs:51,144,256-287; recorder.rs:320-330,417-477 |
| VAD/Stille | kein VAD im Meeting-Pfad; Chunker schneidet ~20-s-Blöcke an leisester 200-ms-Stelle; jeder Block geht an STT | kein Stille-Gate → Whisper-Halluzinationen möglich | mic_capture.rs:1-11; chunker.rs:14-108 |
| Speicherung | SQLite `meetings.db` (`LVA_MEETINGS_DIR` überschreibbar); Ordner je ULID mit WAVs + `protokoll_<ts>.md`; Tabellen meetings, meeting_documents, transcripts(`segments_json`), transcript_deltas, speakers, humans, action_items, meeting_templates; Segment {text,start_ms,end_ms,channel,speaker_index?}; Standard-Aufbewahrung `AfterMinutes` löscht Audio nach Protokoll | speakers/humans/action_items/meeting_templates ungenutzt; keine Notiz-, Teilnehmer-, Ordner-/Tag-Tabellen; nach Standard-Aufbewahrung keine Neu-Transkription | RS/managers/meetings/mod.rs:20-38; store.rs:14-57,132,739; retention.rs:18-46; RS/settings.rs:782 |
| Absturzsicherheit | WAV-Header jede Sekunde; Delta-Commit je Block; `recover_orphans` für `recording`; Import 3 Versuche je Block, dann Lücken-Platzhalter | Kill in `processing` bleibt hängen; bis ~20 s/Kanal Verlust; Dauer nach Recovery fehlt (#15); Live-Kill/Drift ≥ 60 min nie gemessen | recorder.rs:686-733; wav_writer.rs:103-160; import.rs:324-361; DOC/m8-evidence/harness-report.md |
| Engines | `transcribe-cpp` (Whisper-GGUF, Parakeet/Canary/Cohere/Granite/Voxtral/Qwen3-ASR-GGUF; GPU nur Feature `gpu-vulkan`) + `transcribe-rs` (ONNX: Parakeet V2/V3 int8, Moonshine, SenseVoice, …; CPU); eigenes `meeting_model` + `meeting_language`; Parakeet V3 Deutsch 23× Echtzeit CPU | beide Kanäle seriell über einen Worker; `custom_words` nur Whisper; Modell-Entladung „Sofort" → Retry je Block | RS/managers/transcription.rs:225-236,554,815,1933,2186,2395; RS/catalog/catalog.json |
| Live vs. Ende | Pseudo-Live mit 20-s-Blöcken (Import 60 s); erstes Segment nach ~20 s; Parakeet echte Segmentzeiten, Whisper ein Segment je Block | kein Streaming, keine Zwischenergebnisse, keine Wortzeiten | recorder.rs:35,482-547; transcription.rs:2056-2070,2311; stats.rs:35-69 |
| Sprecher | nur Kanal-Label; `speaker_index` immer `None` | keine Diarisierung, kein Voiceprint; Import = Mischkanal; `multitalker-parakeet-streaming` im Katalog, nicht angebunden (ROADMAP E2) | stats.rs:23-35; recorder.rs:515; DOC/ROADMAP.md:61-69 |
| WER/Benchmarks | `--transcribe-file <wav> --reference … --json` → accuracy = 1−WER; deutsche Fixtures (Katja), M8-Fixtures | kein Meeting-Korpus (mehrere Sprecher, Überlappung, Fachvokabular), kein Modellvergleich | RS/selftest.rs:91-222; src-tauri/tests/german_transcription.rs |
| KI | Protokoll per Knopf; JSON-Schema summary/scope/decisions/tasks/next_steps/follow_ups/open_questions; Map-Reduce ab 16.000 Zeichen; Versionen bleiben; Anbieter = aktiver Post-Processing-Provider inkl. internem `llama-server` (Qwen3/3.5, Gemma3/4) | **keine Vorlagen, keine Nutzerstichpunkte, kein „Notizen anreichern"**, kein Prompt-Editor, Aufgaben nicht in `action_items`, nicht automatisch nach Stopp | RS/managers/meetings/minutes.rs; RS/managers/llm/mod.rs; RS/settings.rs:1105-1220 |
| UI | „Aufnahmen": RecorderCard (Pegel, Pause/Stopp), LiveTranscript, MeetingList (25er-Seiten, Import, D&D); MeetingDetail: Umbenennen, Player je Kanal, Tabs Transkript/Protokoll, Segment bearbeiten, Zeit→Audio, Export docx/txt/md, Neu-Transkription | kein Editor (Protokoll nur Vorschau), kein Notizen-Tab, keine Suche/Filter, keine Ordner/Tags; Detail lädt über `meetingsList(0,200)` | FE/components/settings/meetings/*; MeetingDetail.tsx; MinutesView.tsx |
| Suche/Chat/Integration | Export Word/TXT/MD/HTML-Zwischenablage; Import Audio/Video (ffmpeg), VTT/SRT; E2EE-Sync nur Seiten/Einstellungen | keine Volltext-/Vektorsuche, kein Chat, kein Kalender, kein SRT/JSON/PDF-Export | RS/managers/meetings/export.rs; RS/sync/collect.rs |
| Ressourcen | `process_guard` (RAM-Startprüfung 6 GB Reserve, Job-Objekt, Wächter < 2 GB stoppt TTS/llama-server); STT im Prozess ohne Schutz | kein GPU-Koordinator STT/LLM/TTS; Wächter kann laufende Protokollerzeugung beenden | RS/process_guard.rs; RS/managers/llm/server.rs |

## Zentrale Dateien
- `RS/managers/meetings/recorder.rs` Aufnahme-Orchestrierung · `store.rs` SQLite-Schema · `minutes.rs` Prompts/Map-Reduce/Rendering · `import.rs`, `retranscribe.rs` · `chunker.rs`, `stats.rs`, `retention.rs`, `export.rs`, `subtitle.rs`
- `RS/managers/meetings/mic_capture.rs`, `RS/audio_toolkit/audio/loopback.rs`, `wav_writer.rs`
- `RS/managers/transcription.rs` (`transcribe_segments`, Meeting-Modell/-Sprache)
- `RS/commands/meetings.rs` (16 Commands), `RS/lib.rs` (Registrierung ~1480, Recovery ~259, headless `--import-meeting`/`--dump-meeting`/`--make-orphan` ~723)
- `RS/managers/llm/server.rs`, `RS/llm_client.rs`, `RS/process_guard.rs`
- `FE/components/settings/meetings/*` (8 Komponenten), i18n `meetings.*` in de/en

## Tests und Befehle
- Rust: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib` (686 Tests, Probe 29.09.); Meetings ~95 Tests (`--lib meetings::`)
- WER: `cargo test --test german_transcription -- --nocapture` (überspringt ohne Modell)
- Harness: `scripts/make-m8-fixtures.ps1`, `scripts/m8-verify.ps1` (Release-Binary `npx tauri build --no-bundle`, Sandbox `LVA_MEETINGS_DIR`)
- Frontend: `npx tsc --noEmit`, `pnpm lint`, `pnpm test:playwright` (vite auf 1420; Attrappe in `tests/workspace.spec.ts:9-112`; 72 Tests, **keiner mit Meeting-Daten**)
- Spec: `DOC/superpowers/specs/2026-08-19-meetings-protokoll-design.md` (M8–M12: M9 Diarisierung, M10 Editor, M11 Sync, M12 iOS)
