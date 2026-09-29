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

    /// Reference text of the far end (system track) for ich_far_word_leak.
    #[arg(long, value_name = "FILE")]
    pub far_text: Option<PathBuf>,

    /// Reference text of the near talker (microphone) for ich_far_word_leak.
    #[arg(long, value_name = "FILE")]
    pub near_text: Option<PathBuf>,

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

    // M6-P6a
    /// Export one meeting to a file and exit: --export-meeting <ID> --format
    /// md|txt|docx|html|srt|vtt|json --out <FILE>. Honours LVA_MEETINGS_DIR;
    /// read-only (no model, no recording, no startup housekeeping). All parts
    /// are included; SRT/VTT hold the transcript only; audio is never
    /// exported. Exit 0 ok, 1 error, 2 bad input (unknown meeting or format,
    /// no --out).
    #[arg(long, value_name = "ID")]
    pub export_meeting: Option<String>,

    /// Format for --export-meeting (default: from the --out extension).
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,
}
