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

    /// Open this document (txt/md/pdf/docx) in the read-aloud library and
    /// start playback — used by the Explorer context menu. Forwards to a
    /// running instance if there is one.
    #[arg(long, value_name = "FILE")]
    pub read_file: Option<PathBuf>,
}
