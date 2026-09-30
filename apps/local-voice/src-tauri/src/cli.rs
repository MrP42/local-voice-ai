use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug, Clone, Default)]
#[command(name = "local-voice-ai", about = "Local Voice AI - lokale Sprach-KI")]
pub struct CliArgs {
    /// Start with the main window hidden
    #[arg(long)]
    pub start_hidden: bool,

    /// Disable the system tray icon
    #[arg(long)]
    pub no_tray: bool,

    /// Toggle transcription on/off (sent to running instance)
    #[arg(long)]
    pub toggle_transcription: bool,

    /// Toggle transcription with post-processing on/off (sent to running instance)
    #[arg(long)]
    pub toggle_post_process: bool,

    /// Cancel the current operation (sent to running instance)
    #[arg(long)]
    pub cancel: bool,

    /// Enable debug mode with verbose logging
    #[arg(long)]
    pub debug: bool,

    /// Transcribe this WAV (16 kHz mono) headlessly and exit. Runs the same
    /// batch transcription path as the app — no mic, no VAD, no download
    /// (the model must already be installed).
    #[arg(short = 'f', long, value_name = "WAV")]
    pub transcribe_file: Option<PathBuf>,

    /// Model id to load for --transcribe-file (default: the selected model).
    /// With --eval-notes: the local language model for this run only (catalog
    /// id, e.g. llm-qwen3.5-9b-q4; default: the configured provider/model).
    #[arg(long)]
    pub model: Option<String>,

    /// Hard-select the compute device for --transcribe-file by its registry
    /// index (see --list-devices). Omit to use the persisted accelerator
    /// setting. transcribe-cpp (whisper-family) models only.
    #[arg(long, value_name = "N")]
    pub device_index: Option<usize>,

    /// List the transcribe-cpp compute devices (with indices) and exit.
    #[arg(long)]
    pub list_devices: bool,

    /// List the available models (with ids) and exit. Pass an id to --model.
    /// Honors --json for machine-readable output.
    #[arg(long)]
    pub list_models: bool,

    /// Repeat the transcription N times (best_ms reports the fastest run).
    #[arg(long, value_name = "N")]
    pub repeat: Option<usize>,

    /// Emit --transcribe-file results as JSON.
    #[arg(long)]
    pub json: bool,

    /// Score the transcription of --transcribe-file against this phrase.
    /// Adds accuracy, a word-level diff and error counts to the output.
    /// Punctuation, capitalisation and ß/umlaut spellings are not counted as
    /// errors; number words versus digits ARE, because that is a real
    /// difference between models.
    #[arg(long, value_name = "TEXT")]
    pub reference: Option<String>,

    /// Write the result as JSON to this file.
    ///
    /// Needed because the release binary is built for the Windows GUI
    /// subsystem: its stdout is visible in a terminal but cannot be captured
    /// by a calling script, so a file is the only reliable channel back to an
    /// automated caller.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,

    /// Run --transcribe-file through the LIVE STREAMING path instead of batch,
    /// feeding the audio in real time as if it were being spoken, and report
    /// when text actually appeared. This is how streaming latency is measured
    /// without a microphone or a stopwatch. Needs a streaming-capable model.
    #[arg(long)]
    pub stream: bool,

    /// Run a headless TTS self-test against the local fish-speech server and
    /// exit: server sicherstellen, einen Satz synthetisieren, WAV validieren,
    /// Zeiten in ms melden. Honors --json and --out.
    #[arg(long)]
    pub tts_test: bool,

    /// Text for --tts-test (default: a short German sentence).
    #[arg(long, value_name = "TEXT")]
    pub tts_text: Option<String>,

    /// Reference voice id for --tts-test (a folder under <fish_dir>/references).
    /// Overrides the persisted tts_voice setting for this run only.
    #[arg(long, value_name = "ID")]
    pub tts_voice: Option<String>,

    /// Write the WAV produced by --tts-test to this file (audible evidence).
    #[arg(long, value_name = "FILE")]
    pub tts_out_wav: Option<PathBuf>,

    /// Import this file (audio/video/vtt/srt) as a meeting headlessly and
    /// exit. Runs the same import pipeline the UI command uses, then prints
    /// `MEETING_ID=<ulid>` and `DB=<path>` on stdout. Used by the M8
    /// acceptance harness (scripts/m8-verify.ps1).
    #[arg(long, value_name = "FILE")]
    pub import_meeting: Option<PathBuf>,

    /// Print one meeting's stored state as JSON (status, segment count,
    /// first/last segment times, audio paths, retention marker) and exit.
    /// Keeps the harness free of an external sqlite3 dependency.
    #[arg(long, value_name = "ID")]
    pub dump_meeting: Option<String>,

    /// P8a test hook (headless): steer the running processing job like the UI
    /// buttons do. Comma-separated `action@seconds` (pause, resume, stop),
    /// counted from the moment the job appears, e.g. `pause@4,resume@9,stop@14`.
    /// Works with --import-meeting and --continue-meeting; sandbox only
    /// (LVA_MEETINGS_DIR must be set).
    #[arg(long, value_name = "SCRIPT", hide = true)]
    pub job_script: Option<String>,

    /// P8a: write every meeting event of a headless run (state, progress,
    /// job end, script steps) as one JSON line to this file. Evidence for
    /// progress events and for pause/stop taking effect.
    #[arg(long, value_name = "FILE", hide = true)]
    pub job_events: Option<PathBuf>,

    /// P8a test hook (headless): continue a stopped (`cancelled`) meeting like
    /// the "Fortsetzen" button does and wait for the job. Sandbox only.
    #[arg(long, value_name = "ID", hide = true)]
    pub continue_meeting: Option<String>,

    /// Test hook for the crash-recovery scenario: fabricates an "app died
    /// mid recording" meeting — a row left on `recording` with a WAV whose
    /// RIFF/data sizes were never patched — from this 16 kHz mono WAV, then
    /// prints `MEETING_ID=<ulid>`. The next meetings run repairs it through
    /// the real `recover_orphans` path.
    ///
    /// Hidden from `--help`, and in RELEASE builds refused outright unless
    /// `LVA_HARNESS_DESTRUCTIVE=1` is set: it writes fabricated rows into
    /// whatever meetings database it finds, which on a user's machine is
    /// their real one (see `make_orphan_allowed` in lib.rs).
    #[arg(long, value_name = "WAV", hide = true)]
    pub make_orphan: Option<PathBuf>,

    // M2-P2c2
    /// Simulate a live meeting headlessly from WAV files (16 kHz mono PCM16):
    /// --mic is the microphone track, --system the system audio (loopback
    /// reference). Runs the real DSP thread (VAD, echo cancellation) and the
    /// real transcription, then prints one JSON object (transcripts per
    /// channel, echo-cancellation report, mic_aec.wav length, and
    /// ich_far_word_leak when --far-text/--near-text are given). Refuses to run
    /// without the sandbox LVA_MEETINGS_DIR.
    #[arg(long)]
    pub simulate_meeting: bool,

    /// Microphone track for --simulate-meeting.
    #[arg(long, value_name = "WAV")]
    pub mic: Option<PathBuf>,

    /// System-audio track (echo reference) for --simulate-meeting.
    #[arg(long, value_name = "WAV")]
    pub system: Option<PathBuf>,

    /// --simulate-meeting without echo cancellation (baseline).
    #[arg(long)]
    pub no_aec: bool,

    /// --simulate-meeting: the system track starts this much later than the
    /// microphone (a late loopback start; its first MS are dropped).
    #[arg(long, value_name = "MS")]
    pub system_delay_ms: Option<u64>,

    /// M2-P2d: after --simulate-meeting, run the final pass with this model
    /// (`auto`, `off` or a model id; same job as after a real stop) and add
    /// its result as `final` (live/final epoch, transcript per channel,
    /// timings) to the JSON.
    #[arg(long, value_name = "ID")]
    pub final_model: Option<String>,

    /// M7-P7b: after --simulate-meeting (and --final-model), create the AI
    /// meeting notes the way the automatic run after TranscriptFinal does
    /// (same decision, template and engine; the local llama-server is stopped
    /// at the end) and add `notes` (ran, ok, ms, error code) to the JSON.
    #[arg(long)]
    pub notes: bool,

    /// M7-P7b: local language model for --notes in this run only (catalog id,
    /// e.g. llm-qwen3.5-9b-q4); default: the configured provider/model.
    #[arg(long, value_name = "ID")]
    pub notes_model: Option<String>,

    /// Reference text of the far end (system track) for ich_far_word_leak.
    #[arg(long, value_name = "FILE")]
    pub far_text: Option<PathBuf>,

    /// Reference text of the near talker (microphone) for ich_far_word_leak.
    #[arg(long, value_name = "FILE")]
    pub near_text: Option<PathBuf>,

    // M2-P2b2
    /// --simulate-meeting in real time: the files are fed at wall-clock pace,
    /// so each segment's latency (emitted_at_ms - vad_end_ms) is what a live
    /// meeting would see. Reports latency p50/p95/max.
    #[arg(long)]
    pub realtime: bool,

    /// --simulate-meeting from benchmark scene folders instead of --mic/--system
    /// (repeatable; scenes are played back to back). Each folder holds the mic
    /// track (--scene-mic), system.wav and reference.json; the reference texts
    /// give the live WER and ich_far_word_leak.
    #[arg(long, value_name = "DIR")]
    pub scene: Vec<PathBuf>,

    /// Microphone file inside each --scene folder (default mic_echo.wav).
    #[arg(long, value_name = "FILE")]
    pub scene_mic: Option<String>,

    /// Open this document (txt/md/pdf/docx) in the read-aloud library and
    /// start playback — used by the Explorer context menu. Forwards to a
    /// running instance if there is one.
    #[arg(long, value_name = "FILE")]
    pub read_file: Option<PathBuf>,

    // M4-P4a
    /// Measure the meeting search index (list search, word search, hybrid
    /// search) on a synthetic sandbox database in the temp directory and exit.
    /// Sizes via --meetings and --chunks, runs per query via --repeat, output
    /// via --json/--out. Exit code 0 ok, 1 error, 3 when a p95 reaches 500 ms.
    /// Never touches the productive meetings.db.
    #[arg(long)]
    pub bench_search: bool,

    /// Number of synthetic meetings for --bench-search (default 500).
    #[arg(long, value_name = "N")]
    pub meetings: Option<usize>,

    /// Chunks per synthetic meeting for --bench-search (default 200).
    #[arg(long, value_name = "N")]
    pub chunks: Option<usize>,

    /// Directory in which --bench-search creates its temporary database
    /// (default: the system temp directory; needs about 1.2 GB for the
    /// default size).
    #[arg(long, value_name = "DIR")]
    pub bench_dir: Option<PathBuf>,

    // M4-P4b
    /// Rebuild the meeting search index (chunks, full-text, vectors through
    /// the embedding server, which is stopped at the end) and exit. Honours
    /// LVA_MEETINGS_DIR. Output via --json/--out. Exit code 0 ok (lexical only
    /// without the embedding model), 1 error, 3 when vectors are incomplete.
    #[arg(long)]
    pub reindex_meetings: bool,

    /// With --reindex-meetings: first import every meeting fixture (*.json
    /// with title, segments, notes) from this directory. Only allowed with
    /// LVA_MEETINGS_DIR set (sandbox), never on the productive database.
    #[arg(long, value_name = "DIR", hide = true)]
    pub seed_meetings: Option<PathBuf>,

    // M1-P1e
    /// Evaluate the AI meeting notes (acceptance AK3) on the synthetic
    /// fixtures in DIR (tests/fixtures/notes) with the configured language
    /// model and exit. Runs one fixture after the other in a sandbox store in
    /// the temp directory (never the productive meetings.db) and stops the
    /// local llama-server at the end. --model picks a local model for this run.
    /// Output via --json/--out. Exit 0 targets met, 3 missed, 1 error.
    #[arg(long, value_name = "DIR")]
    pub eval_notes: Option<PathBuf>,

    // P1k
    /// Evaluate the template choice ("Automatisch (nach Inhalt)") and the
    /// minutes on the synthetic fixtures in DIR (tests/fixtures/notes for the
    /// choice, tests/fixtures/minutes for long transcripts) with the configured
    /// language model and exit. Runs one fixture after the other in a sandbox
    /// store in the temp directory (never the productive meetings.db) and stops
    /// the local llama-server at the end. --model picks a local model for this
    /// run, --minutes-template a template instead of the automatic choice.
    /// Output via --json/--out. Exit 0 ok, 3 wrong template or a gap in the
    /// minutes, 1 error.
    #[arg(long, value_name = "DIR")]
    pub eval_minutes: Option<PathBuf>,

    /// With --eval-minutes: `auto` (default) or a template id such as
    /// builtin:vertrieb.
    #[arg(long, value_name = "ID")]
    pub minutes_template: Option<String>,

    // M4-P4f
    /// Evaluate the meeting chat (acceptance AK8) on the synthetic fixtures
    /// in DIR (tests/fixtures/chat with questions.json) with the configured
    /// language model and exit. Imports the meetings into a sandbox store in
    /// the temp directory (never the productive meetings.db), builds the
    /// search index including vectors (the embedding server is started and
    /// stopped), asks the questions one after the other and stops both
    /// llama-servers at the end. --model picks a local model for this run.
    /// Output via --json/--out. Exit 0 accuracy >= 0.85, 3 below, 1 error.
    #[arg(long, value_name = "DIR")]
    pub eval_chat: Option<PathBuf>,

    /// With --eval-chat: search by words only, without the embedding model
    /// (comparison run).
    #[arg(long)]
    pub lexical_only: bool,

    // M3-P3a
    /// Measure speaker diarization (DER, acceptance AK7) on every pair
    /// <name>.wav + <name>.rttm in DIR and exit. --model picks the diarization
    /// model (catalog id or GGUF path; default: the installed Sortformer).
    /// Weighted DER per group by file prefix: ami_test_*, other ami_*, rest
    /// (German). Output via --json/--out. Exit 0 targets met, 3 missed,
    /// 1 error, 2 bad input or model missing.
    #[arg(long, value_name = "DIR")]
    pub eval_diarization: Option<PathBuf>,

    /// --eval-diarization: collar in seconds around every reference boundary
    /// (default 0.25).
    #[arg(long, value_name = "S")]
    pub collar: Option<f64>,

    /// --eval-diarization: also write the hypothesis RTTMs (post-processed and
    /// raw) to this directory, e.g. for a cross-check with another scorer.
    #[arg(long, value_name = "DIR")]
    pub rttm_out: Option<PathBuf>,

    // M5-P5a
    /// Read a calendar (ICS file or address) like the sync service does, expand
    /// its series and print the events of the window, without touching the
    /// database, then exit. Honours --json/--out. The address is never printed
    /// (host only). Exit 0 read (warnings are listed), 1 read/parse error,
    /// 2 bad arguments. Default zone: LVA_CALENDAR_TZ, else the Windows zone.
    #[arg(long, value_name = "FILE_OR_URL")]
    pub calendar_dump: Option<String>,

    /// Window start for --calendar-dump: YYYY-MM-DD (00:00 UTC) or RFC 3339
    /// (default: 30 days before now).
    #[arg(long = "from", value_name = "DATE")]
    pub cal_from: Option<String>,

    /// Window end for --calendar-dump (default: 30 days after now).
    #[arg(long = "to", value_name = "DATE")]
    pub cal_to: Option<String>,

    // M5-P5c
    /// Watch the microphone usage log for --seconds and print which programs
    /// opened the microphone (same rules as the in-app hint: 5 s debounce, own
    /// exe, dead entries and webview2 filtered), then exit. Read-only; never
    /// opens the microphone. Honours --json/--out. Exit 0 watched, 1 registry
    /// unreadable, 2 bad arguments.
    #[arg(long)]
    pub detect_mic: bool,

    /// Duration for --detect-mic in seconds (default 15, at most 3600).
    #[arg(long, value_name = "N")]
    pub seconds: Option<u64>,

    /// --detect-mic: also report programs outside the meeting catalog.
    #[arg(long)]
    pub all_apps: bool,

    // M6-P6a
    /// Export one meeting to a file and exit: --export-meeting <ID> --format
    /// md|txt|docx|html|pdf|srt|vtt|json --out <FILE>. Honours LVA_MEETINGS_DIR;
    /// read-only (no model, no recording, no startup housekeeping). All parts
    /// are included; SRT/VTT hold the transcript only; audio is never
    /// exported. pdf renders through a hidden WebView2 window (Windows only,
    /// 20 s limit, error text starts with pdf_unavailable|pdf_timeout|
    /// pdf_low_memory|pdf_failed). Exit 0 ok, 1 error, 2 bad input (unknown
    /// meeting or format, no --out).
    #[arg(long, value_name = "ID")]
    pub export_meeting: Option<String>,

    /// Format for --export-meeting (default: from the --out extension).
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,

    // M6-P6e
    /// Run the local MCP server on stdin/stdout (JSON-RPC 2.0, one message per
    /// line) and exit when the client closes the pipe. Read-only access to the
    /// finished meetings, nothing else: no window, no models, no network. The
    /// meetings are only served while "Lokaler MCP-Server" is switched on in the
    /// settings. Honours LVA_MEETINGS_DIR and LVA_APPDATA_DIR (sandbox). stdout
    /// carries JSON-RPC only; diagnostics go to stderr. Exit 0 when the client
    /// closed the pipe, 1 on an I/O error.
    #[arg(long)]
    pub mcp: bool,

    // P6f (B13)
    /// Draft the follow-up e-mail of one meeting with the configured language
    /// model (--model picks a local one for this run) and print it as JSON
    /// (to, subject, body_text), then exit. SANDBOX ONLY: requires
    /// LVA_MEETINGS_DIR (prepare the meeting with --import-meeting), so it can
    /// never read the productive meetings.db. Stops the local llama-server at
    /// the end. Exit 0 draft written, 3 no content / empty answer, 1 error,
    /// 2 bad input (no sandbox, unknown meeting).
    #[arg(long, value_name = "ID")]
    pub followup_draft: Option<String>,

    // A1 (Goal Integrationen)
    /// Print the integrations register as JSON and exit: every integration with
    /// its direction, stored grants and the EFFECTIVE mode per capability and
    /// caller, the state of its secrets (present/missing/broken, never the
    /// content), counts of audit entries, pending approvals and provenance rows,
    /// and the calendar sources it mirrors. SANDBOX ONLY: requires
    /// LVA_MEETINGS_DIR, so it can never read the productive meetings.db (it
    /// opens, and therefore migrates, that sandbox store). Honours --json/--out.
    /// Exit 0 ok, 1 error, 2 no sandbox.
    #[arg(long)]
    pub integrations_dump: bool,

    // A2 (Goal Integrationen)
    /// Add a YouTube link as a meeting source headlessly and exit: one oEmbed
    /// request (title, channel; no API key, time-limited) creates a meeting with
    /// source `youtube`, records the register entry, the audit row and the
    /// provenance, prints `MEETING_ID=<ulid>` and, with --json/--out, the source
    /// as JSON. SANDBOX ONLY: requires LVA_MEETINGS_DIR, so it can never write to
    /// the productive meetings.db. In the sandbox LVA_YOUTUBE_OEMBED_URL may point
    /// to a local test endpoint. Exit 0 created, 1 error, 2 bad input (no sandbox,
    /// not a single-video link), 3 YouTube unreachable or video unavailable.
    #[arg(long, value_name = "URL")]
    pub add_youtube: Option<String>,
}
