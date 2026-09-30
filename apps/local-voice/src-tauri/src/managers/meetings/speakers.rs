//! M3-P3b: Sprecher einer Besprechung - Labels, Turns, Zuordnung, Namen.
//!
//! ```text
//! Enddurchlauf / Import / Neu-Transkription:
//!   collect_turns   je Kanal laut Plan (diarize_tracks) Turns holen: gespeicherte
//!                   (speaker_hints_json) wiederverwenden, sonst Sortformer
//!                   (RAM-Tor, ein Modell zur Zeit) -> TurnSet
//!   assign          Wort -> Sprecher, Segmente teilen (diarize::assign)
//!   store           Segmente + speaker_hints_json + speakers-Zeilen in EINER
//!                   Transaktion (store::SpeakerWrite)
//! ```
//!
//! Sprechernamen stehen in `speakers` (nicht im Transkript) und ueberstehen
//! darum `clear_segments`/Neu-Transkription: die neuen Segmente bekommen ihre
//! Sprecher aus den gespeicherten Turns, Nummer und Name bleiben gleich. Wird
//! neu diarisiert (fehlende Turns, anderes Modell), ordnet [`remap_speakers`]
//! alte und neue Sprecher ueber die Zeitueberlappung zu; Namen wandern mit.
//!
//! Fehlerfaelle (Entwurf `m3-sprecher.md` 4) und ihre Absicherung:
//! - Korrektur von Hand waehrend des Laufs: Schreiben nur mit der Revision des
//!   Schnappschusses (`Conflict`); [`apply_to_stored`] rechnet die Zuordnung
//!   mit frischem Stand neu (Turns sind zeitbasiert, bleiben gueltig), sonst
//!   bleibt das Transkript wie es ist.
//! - Abbruch/Absturz mitten im Vorgang: Turns, Segmente und Zeilen werden erst
//!   am Ende in EINER Transaktion geschrieben; Recovery startet den Job neu,
//!   ein zweiter Lauf aendert nichts (idempotent, keine zweite Epoche).
//!   Eine neue Aufnahme bricht ueber [`ChannelDiarizer::cancelled`] ab.
//! - Absturz im nativen Modell (kein Kindprozess, der App-Prozess stirbt): die
//!   Marke `metadata_json.diarize.state = running` bremst die Absturzschleife:
//!   nach [`MAX_ATTEMPTS`] unfertigen Laeufen wird der Schritt uebersprungen.
//! - voller Speicher/Datentraeger: RAM-Tor inkl. PCM-Groesse VOR dem Einlesen,
//!   `try_reserve` fuer den Puffer, Kanaele ueber [`MAX_AUDIO_MS`] entfallen;
//!   eine volle Platte laesst die Transaktion komplett scheitern (Live bleibt).
//! - Modell fehlt / Einstellung `off` / Audio geloescht / kaputte WAV: der
//!   Kanal wird uebersprungen (Bericht `skipped`), Labels bleiben wie bisher.
//!
//! Dieses Modul kennt weder Tauri noch `settings`: die App-Anbindung
//! (`AppDiarizer`) steht in `final_pass.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use log::{info, warn};
use serde::Serialize;
use serde_json::{json, Value};

use super::diarize::assign::assign_segments;
use super::diarize::engine::DIARIZE_RAM_NEED_MB;
use super::diarize::{der, DiarizeError, Turn, DEFAULT_GAP_MS, DEFAULT_PAD_MS, MIN_AUDIO_MS};
use super::final_pass::{final_tracks, wav_duration_ms, Track};
use super::recorder::CHANNEL_MIC;
use super::stats::label_for_channel;
use super::store::{Meeting, MeetingStore, ReplaceError, SpeakerWrite, StoredSegment};
use super::MIC_AEC_FILE;

/// Import-Spur ohne Kanaltrennung (wie `import.rs`).
const CHANNEL_MIXED: u8 = 2;
/// Version von `speaker_hints_json`.
pub const HINTS_VERSION: u32 = 1;
/// Modellname in `speaker_hints_json` (Sortformer 4spk-v2.1, Q8).
pub const HINTS_MODEL_NAME: &str = "sortformer-4spk-v2.1-q8";
/// `metadata_json`-Schluessel des Berichts und der Absturzmarke.
pub const REPORT_KEY: &str = "diarize";
/// `metadata_json`-Schluessel: "Mehrere Personen am Mikrofon" (Kanal 0 diarisieren).
pub const DIARIZE_MIC_KEY: &str = "diarize_mic";
/// So viele unfertige Laeufe (Absturz im Modell) duldet der Schritt, danach
/// wird er fuer diese Besprechung uebersprungen.
pub const MAX_ATTEMPTS: u32 = 2;
/// Laengster Kanal, der diarisiert wird (4 h = ~0,9 GB Puffer). Alles darueber
/// wird uebersprungen, statt den Speicher zu sprengen.
pub const MAX_AUDIO_MS: u64 = 4 * 60 * 60 * 1000;
/// Alt -> neu (`remap_speakers`): mindestens so viele ms Ueberlappung ...
pub const REMAP_MIN_OVERLAP_MS: u64 = 500;
/// ... und so viel Anteil der Sprechzeit des alten Sprechers.
pub const REMAP_MIN_SHARE: f64 = 0.2;
/// Wie oft [`apply_to_stored`] bei einer Korrektur von Hand neu ansetzt.
const APPLY_ATTEMPTS: usize = 3;

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/// Anzeigenamen der Sprecher einer Besprechung. Ersetzt
/// `stats::label_for_channel` an allen Aufrufstellen: Name, sonst
/// "Gegenseite 2" / "Raum 1" / "Person 1" / "Ich".
#[derive(Clone, Debug, Default)]
pub struct SpeakerDirectory {
    names: HashMap<(u8, u32), String>,
    /// Es gibt einen Systemton (Gegenseite): Kanal-0-Sprecher heissen dann
    /// "Raum n" (mehrere Personen am Mikrofon), sonst "Person n" (Praesenz).
    has_remote: bool,
}

impl SpeakerDirectory {
    pub fn new(names: impl IntoIterator<Item = ((u8, u32), String)>, has_remote: bool) -> Self {
        Self {
            names: names
                .into_iter()
                .filter(|(_, n)| !n.trim().is_empty())
                .map(|(k, n)| (k, n.trim().to_string()))
                .collect(),
            has_remote,
        }
    }

    /// Ohne Namen, aus den Segmenten (Tests, Aufrufer ohne Store).
    pub fn from_segments(segments: &[StoredSegment]) -> Self {
        Self {
            names: HashMap::new(),
            has_remote: segments.iter().any(|s| s.channel == 1),
        }
    }

    /// Namen und Kanalform aus dem Store. Nie ein Fehler: ein unlesbarer Store
    /// ergibt Standardlabels (die Anzeige darf am Namen nicht scheitern).
    pub fn load(store: &MeetingStore, meeting_id: &str) -> Self {
        let names = store
            .speaker_rows(meeting_id)
            .map(|rows| {
                rows.into_iter()
                    .filter_map(|r| r.display_name.map(|n| ((r.channel, r.speaker_index), n)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|e| {
                warn!("meetings: speaker names not readable ({meeting_id}): {e}");
                Vec::new()
            });
        let has_system = store
            .get_meeting(meeting_id)
            .ok()
            .flatten()
            .is_some_and(|m| m.system_audio_path.is_some());
        let has_remote = has_system
            || store
                .get_segments(meeting_id)
                .map(|s| s.iter().any(|s| s.channel == 1))
                .unwrap_or(false);
        Self::new(names, has_remote)
    }

    /// Der eingetragene Name (getrimmt), falls es einen gibt.
    pub fn name(&self, channel: u8, speaker_index: u32) -> Option<&str> {
        self.names
            .get(&(channel, speaker_index))
            .map(String::as_str)
    }

    pub fn label(&self, segment: &StoredSegment) -> String {
        self.label_for(segment.channel, segment.speaker_index)
    }

    pub fn label_for(&self, channel: u8, speaker_index: Option<u32>) -> String {
        let Some(n) = speaker_index else {
            return label_for_channel(channel);
        };
        if let Some(name) = self.name(channel, n) {
            return name.to_string();
        }
        let base = match channel {
            1 => "Gegenseite",
            0 if self.has_remote => "Raum",
            0 | 2 => "Person",
            _ => "Sprecher",
        };
        format!("{base} {n}")
    }
}

// ---------------------------------------------------------------------------
// speaker_hints_json
// ---------------------------------------------------------------------------

/// `speaker_hints_json` -> Turns je Kanal. Kaputte Eintraege entfallen; ein
/// unlesbares Ganzes ergibt eine leere Tabelle.
pub fn turns_from_hints(hints: &Value) -> BTreeMap<u8, Vec<Turn>> {
    let mut out = BTreeMap::new();
    let Some(channels) = hints.get("channels").and_then(Value::as_object) else {
        return out;
    };
    for (key, list) in channels {
        let (Ok(channel), Some(list)) = (key.parse::<u8>(), list.as_array()) else {
            continue;
        };
        let turns: Vec<Turn> = list
            .iter()
            .filter_map(|t| {
                let t = t.as_array()?;
                let start_ms = t.first()?.as_u64()?;
                let end_ms = t.get(1)?.as_u64()?;
                let speaker = u32::try_from(t.get(2)?.as_u64()?).ok()?;
                (end_ms > start_ms && speaker > 0).then_some(Turn {
                    start_ms,
                    end_ms,
                    speaker,
                })
            })
            .collect();
        out.insert(channel, turns);
    }
    out
}

/// Neuer Inhalt von `speaker_hints_json`: Turns der uebergebenen Kanaele
/// ersetzen die gespeicherten, alle anderen Kanaele und unbekannte Schluessel
/// (z. B. spaetere Erweiterungen) bleiben stehen.
pub fn hints_with_turns(
    existing: Option<&str>,
    model: &str,
    gap_ms: u64,
    pad_ms: u64,
    channels: &BTreeMap<u8, Vec<Turn>>,
) -> String {
    let mut root = existing
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    let mut merged = turns_from_hints(&Value::Object(root.clone()));
    for (channel, turns) in channels {
        merged.insert(*channel, turns.clone());
    }
    let channels_json: serde_json::Map<String, Value> = merged
        .iter()
        .map(|(channel, turns)| {
            let rows: Vec<Value> = turns
                .iter()
                .map(|t| json!([t.start_ms, t.end_ms, t.speaker]))
                .collect();
            (channel.to_string(), Value::Array(rows))
        })
        .collect();
    root.insert("v".into(), json!(HINTS_VERSION));
    root.insert("model".into(), json!(model));
    root.insert(
        "params".into(),
        json!({ "gap_ms": gap_ms, "pad_ms": pad_ms }),
    );
    root.insert("channels".into(), Value::Object(channels_json));
    Value::Object(root).to_string()
}

/// M3-P3c (Zusammenfuehren): in `speaker_hints_json` gehoeren alle Turns von
/// `from` im Kanal `channel` jetzt `into`. Alles andere (Modell, Parameter,
/// andere Kanaele, unbekannte Schluessel) bleibt unangetastet; ein unlesbarer
/// Text kommt unveraendert zurueck.
pub fn rewrite_turn_speaker(hints: &str, channel: u8, from: u32, into: u32) -> String {
    let Ok(mut root) = serde_json::from_str::<Value>(hints) else {
        return hints.to_string();
    };
    let list = root
        .get_mut("channels")
        .and_then(|c| c.get_mut(channel.to_string()))
        .and_then(Value::as_array_mut);
    if let Some(list) = list {
        for turn in list.iter_mut().filter_map(Value::as_array_mut) {
            if turn.get(2).and_then(Value::as_u64) == Some(u64::from(from)) {
                turn[2] = json!(into);
            }
        }
    }
    root.to_string()
}

// ---------------------------------------------------------------------------
// Namen ueber eine neue Diarisierung hinweg
// ---------------------------------------------------------------------------

fn speakers_of(turns: &[Turn]) -> Vec<u32> {
    let mut v: Vec<u32> = turns.iter().map(|t| t.speaker).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Ordnet alte Sprecher neuen zu (alt -> neu): 1:1 nach maximaler
/// Zeitueberlappung (derselbe Hungarian-Code wie beim DER). Nur Paare mit
/// echter Ueberlappung zaehlen ([`REMAP_MIN_OVERLAP_MS`] und
/// [`REMAP_MIN_SHARE`] der Sprechzeit des alten Sprechers): ein Name wird nie
/// an einen Sprecher gehaengt, der nur zufaellig ein paar Millisekunden
/// mitspricht. Alte Sprecher ohne Partner fehlen im Ergebnis; neue ohne
/// Partner behalten ihre Nummer aus der neuen Diarisierung.
pub fn remap_speakers(old_turns: &[Turn], new_turns: &[Turn]) -> HashMap<u32, u32> {
    let old = speakers_of(old_turns);
    let new = speakers_of(new_turns);
    if old.is_empty() || new.is_empty() {
        return HashMap::new();
    }
    let mut overlap = vec![vec![0i64; new.len()]; old.len()];
    let mut old_total = vec![0u64; old.len()];
    for a in old_turns {
        let i = old.binary_search(&a.speaker).unwrap_or(0);
        old_total[i] += a.end_ms.saturating_sub(a.start_ms);
        for b in new_turns {
            let ov = a
                .end_ms
                .min(b.end_ms)
                .saturating_sub(a.start_ms.max(b.start_ms));
            if ov > 0 {
                let j = new.binary_search(&b.speaker).unwrap_or(0);
                overlap[i][j] += ov as i64;
            }
        }
    }
    der::max_assignment_pairs(&overlap)
        .into_iter()
        .filter(|&(i, j)| {
            let ov = overlap[i][j] as u64;
            ov >= REMAP_MIN_OVERLAP_MS && ov as f64 >= REMAP_MIN_SHARE * old_total[i] as f64
        })
        .map(|(i, j)| (old[i], new[j]))
        .collect()
}

// ---------------------------------------------------------------------------
// Plan: welche Kanaele werden diarisiert
// ---------------------------------------------------------------------------

/// "Mehrere Personen am Mikrofon" (`metadata_json.diarize_mic`).
pub fn diarize_mic(metadata: Option<&Value>) -> bool {
    metadata
        .and_then(|m| m.get(DIARIZE_MIC_KEY))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Kanaele, die diarisiert werden (Entwurf 3.2), nur vorhandene Dateien:
/// - Live mit Systemton: Kanal 1 (Gegenseite); Kanal 0 ("Ich") nur mit dem
///   Haekchen `diarize_mic` (dann bevorzugt `mic_aec.wav`, siehe `final_tracks`).
/// - Live ohne Systemton (Praesenz): Kanal 0.
/// - Import (Audio/Video): die eine gemischte Spur, Kanal 2.
/// - Untertitel: keine Audiodaten, nichts zu tun.
pub fn diarize_tracks(meeting: &Meeting, metadata: Option<&Value>) -> Vec<Track> {
    match meeting.source.as_str() {
        "live" => {
            let want_mic = meeting.system_audio_path.is_none() || diarize_mic(metadata);
            final_tracks(meeting, metadata)
                .into_iter()
                .filter(|t| t.channel != CHANNEL_MIC || want_mic)
                .collect()
        }
        "import" => meeting
            .mic_audio_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .map(|path| {
                vec![Track {
                    path,
                    channel: CHANNEL_MIXED,
                }]
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Diarisierer (Schnittstelle) und Turns holen
// ---------------------------------------------------------------------------

/// Die Sprechertrennung, wie dieser Schritt sie braucht. Produktion:
/// `final_pass::AppDiarizer` (Sortformer); Tests: Attrappen.
pub trait ChannelDiarizer {
    /// Modellname fuer `speaker_hints_json` und den Bericht.
    fn model_name(&self) -> String;
    /// Nachbearbeitung (Luecken, Pad) fuer `speaker_hints_json`.
    fn params(&self) -> (u64, u64) {
        (DEFAULT_GAP_MS, DEFAULT_PAD_MS)
    }
    /// `Some(Grund)`, wenn gar nicht diarisiert werden kann oder soll
    /// (`disabled`, `model_missing`): gespeicherte Turns werden trotzdem genutzt.
    fn unavailable(&self) -> Option<&'static str> {
        None
    }
    /// RAM-Tor VOR dem Einlesen des Kanals; `need_mb` = Modell + Puffer.
    fn check_ram(&mut self, need_mb: u64) -> Result<(), String>;
    /// Turns eines Kanals (16 kHz mono f32), Sprecher 1-basiert.
    fn diarize(&mut self, channel: u8, pcm: &[f32]) -> Result<Vec<Turn>, DiarizeError>;
    /// Abbruch von aussen (neue Aufnahme, Stopp durch den Nutzer).
    fn cancelled(&self) -> bool {
        false
    }
    /// P8a: Fortschritt: `done_ms` von `total_ms` Audio der jetzt zu
    /// berechnenden Kanaele sind fertig (je Kanal ein Modelllauf, darin gibt es
    /// keinen Zwischenstand). Standard: nichts.
    fn progress(&mut self, _done_ms: u64, _total_ms: u64) {}
    /// Modell freigeben (vor dem Laden des End-STT-Modells).
    fn release(&mut self) {}
}

/// Turns aller diarisierten Kanaele einer Besprechung.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnSet {
    /// Kanal -> Turns (Sprecher 1-basiert). Ein Kanal mit leerer Liste wurde
    /// diarisiert, aber es fehlt Sprache.
    pub channels: BTreeMap<u8, Vec<Turn>>,
    /// Kanaele, die JETZT neu berechnet wurden (nicht aus dem Speicher geholt).
    pub fresh: BTreeSet<u8>,
    pub model: String,
    pub gap_ms: u64,
    pub pad_ms: u64,
}

/// Was aus einem Kanal wurde (Teil des Berichts).
#[derive(Clone, Debug, Serialize)]
pub struct ChannelReport {
    pub channel: u8,
    /// `model` (jetzt berechnet) oder `stored` (aus `speaker_hints_json`).
    pub source: &'static str,
    pub speakers: usize,
    pub turns: usize,
    pub audio_ms: u64,
    pub model_ms: u64,
    /// Grund, warum der Kanal nicht diarisiert wurde.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<&'static str>,
}

/// Bericht des Sprecher-Schritts: nur Zahlen und Gruende, kein Text aus dem
/// Transkript (Log, `metadata_json.diarize`, `metadata_json.final_pass`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct StepReport {
    /// `done`, `skipped`, `cancelled` oder (Absturzmarke) `running`/`aborted`.
    pub state: String,
    pub model: Option<String>,
    /// Laeufe seit dem letzten sauberen Ende (Absturzbremse).
    pub attempts: u32,
    pub channels: Vec<ChannelReport>,
    pub assigned: usize,
    pub unassigned: usize,
    pub split_added: usize,
    /// Das Mikrofon (Kanal 0) wird diarisiert, aber es gibt keine Spur ohne
    /// Echo (`mic_aec.wav`): die Gegenseite kann im Mikrofon mitsprechen. Die
    /// Anzeige rat dann zu Kopfhoerern oder Echo-Unterdrueckung (Entwurf 4).
    pub mic_without_aec: bool,
    /// Die Segmente wurden geschrieben (sonst: nichts zu aendern oder Fehler).
    pub applied: bool,
    pub wall_ms: u64,
}

/// Der Lauf wurde von aussen abgebrochen.
#[derive(Debug, PartialEq, Eq)]
pub struct Cancelled;

fn distinct_speakers(turns: &[Turn]) -> usize {
    speakers_of(turns).len()
}

fn skip_reason(e: &DiarizeError) -> &'static str {
    match e {
        DiarizeError::ModelMissing(_) => "model_missing",
        DiarizeError::LowMemory(_) => "low_memory",
        DiarizeError::WrongModel(_) | DiarizeError::Load(_) | DiarizeError::Run(_) => "failed",
        DiarizeError::Cancelled => "cancelled",
    }
}

/// Grosse eines 16-kHz-f32-Puffers in MB (aufgerundet).
fn pcm_mb(audio_ms: u64) -> u64 {
    (audio_ms * 16 * 4).div_ceil(1024 * 1024)
}

/// 16-kHz-Mono-WAV als f32. 16 kHz/mono/PCM16 (jede Aufnahme der App) wird
/// gestreamt; alles andere (ein importiertes WAV in fremdem Format) laeuft
/// ueber den tolerant lesenden Import-Pfad. Ein kaputter Rest beendet die Spur
/// (Absturz mitten im Schreiben), ein nicht bereitstellbarer Puffer ist ein
/// Fehler statt eines Abbruchs des Prozesses.
pub fn read_channel_pcm(path: &std::path::Path) -> Result<Vec<f32>, String> {
    let mut reader =
        hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = reader.spec();
    if (spec.sample_rate, spec.channels, spec.bits_per_sample) == (16_000, 1, 16)
        && spec.sample_format == hound::SampleFormat::Int
    {
        let mut pcm: Vec<f32> = Vec::new();
        pcm.try_reserve_exact(reader.duration() as usize)
            .map_err(|_| "not enough memory for the channel".to_string())?;
        for s in reader.samples::<i16>() {
            match s {
                Ok(v) => pcm.push(f32::from(v) / 32_768.0),
                Err(_) => break,
            }
        }
        return Ok(pcm);
    }
    drop(reader);
    let samples = super::import::read_wav_i16_mono_16k(path)?;
    let mut pcm: Vec<f32> = Vec::new();
    pcm.try_reserve_exact(samples.len())
        .map_err(|_| "not enough memory for the channel".to_string())?;
    pcm.extend(samples.iter().map(|&v| f32::from(v) / 32_768.0));
    Ok(pcm)
}

/// Absturzmarke: `metadata_json.diarize.state = running`, solange das Modell
/// rechnet. Faellt der Wert (Panik), steht danach `aborted`; stirbt der
/// Prozess, bleibt `running` stehen und der naechste Lauf zaehlt mit.
struct RunMark<'a> {
    store: &'a MeetingStore,
    meeting_id: &'a str,
    armed: bool,
}

impl<'a> RunMark<'a> {
    fn start(store: &'a MeetingStore, meeting_id: &'a str, attempts: u32) -> Self {
        let mark = json!({ "state": "running", "attempts": attempts });
        if let Err(e) = store.set_metadata_key(meeting_id, REPORT_KEY, mark) {
            warn!("meetings: diarization marker not stored: {e}");
        }
        Self {
            store,
            meeting_id,
            armed: true,
        }
    }

    /// Sauberes Ende: den Bericht statt der Marke schreiben.
    fn finish(mut self, report: &StepReport) {
        self.armed = false;
        write_report(self.store, self.meeting_id, report);
    }
}

impl Drop for RunMark<'_> {
    fn drop(&mut self) {
        if self.armed {
            let mark = json!({ "state": "aborted", "attempts": 0 });
            let _ = self
                .store
                .set_metadata_key(self.meeting_id, REPORT_KEY, mark);
        }
    }
}

pub fn write_report(store: &MeetingStore, meeting_id: &str, report: &StepReport) {
    match serde_json::to_value(report) {
        Ok(v) => {
            if let Err(e) = store.set_metadata_key(meeting_id, REPORT_KEY, v) {
                warn!("meetings: diarization report not stored: {e}");
            }
        }
        Err(e) => warn!("meetings: diarization report not serializable: {e}"),
    }
}

/// Unfertige Laeufe aus einem frueheren Absturz (`state == running`).
fn previous_attempts(metadata: Option<&Value>) -> u32 {
    let Some(d) = metadata.and_then(|m| m.get(REPORT_KEY)) else {
        return 0;
    };
    if d.get("state").and_then(Value::as_str) != Some("running") {
        return 0;
    }
    d.get("attempts")
        .and_then(Value::as_u64)
        .map_or(1, |a| a.min(u64::from(u32::MAX)) as u32)
}

/// Turns fuer alle Kanaele des Plans. Gespeicherte Turns
/// (`use_stored`, `speaker_hints_json`) werden wiederverwendet und nie neu
/// berechnet; fehlende rechnet `diarizer`. Jeder Kanal scheitert fuer sich:
/// ein Fehler ueberspringt ihn (Bericht), der Rest laeuft weiter. Nur ein
/// Abbruch von aussen beendet alles (`Err(Cancelled)`, nichts wurde
/// geschrieben). `Ok(None)`: kein Kanal brauchbar. Das Modell ist am Ende
/// wieder freigegeben.
pub fn collect_turns(
    store: &MeetingStore,
    meeting: &Meeting,
    metadata: Option<&Value>,
    tracks: &[Track],
    use_stored: bool,
    diarizer: &mut dyn ChannelDiarizer,
    report: &mut StepReport,
) -> Result<Option<TurnSet>, Cancelled> {
    let started = Instant::now();
    let (gap_ms, pad_ms) = diarizer.params();
    let mut set = TurnSet {
        model: diarizer.model_name(),
        gap_ms,
        pad_ms,
        ..Default::default()
    };
    report.model = Some(set.model.clone());

    if use_stored {
        match store.speaker_hints(&meeting.id) {
            Ok(Some(text)) => match serde_json::from_str::<Value>(&text) {
                Ok(v) => {
                    for (channel, turns) in turns_from_hints(&v) {
                        report.channels.push(ChannelReport {
                            channel,
                            source: "stored",
                            speakers: distinct_speakers(&turns),
                            turns: turns.len(),
                            audio_ms: 0,
                            model_ms: 0,
                            skipped: None,
                        });
                        set.channels.insert(channel, turns);
                    }
                }
                Err(e) => warn!("meetings: speaker hints unreadable ({}): {e}", meeting.id),
            },
            Ok(None) => {}
            Err(e) => warn!("meetings: speaker hints not read ({}): {e}", meeting.id),
        }
    }

    let todo: Vec<&Track> = tracks
        .iter()
        .filter(|t| !set.channels.contains_key(&t.channel))
        .collect();
    let skip = |report: &mut StepReport, channel: u8, why: &'static str, audio_ms: u64| {
        report.channels.push(ChannelReport {
            channel,
            source: "model",
            speakers: 0,
            turns: 0,
            audio_ms,
            model_ms: 0,
            skipped: Some(why),
        });
    };

    // Absturzbremse: ein frueherer Lauf ist im Modell gestorben.
    let attempts = previous_attempts(metadata);
    let mut mark: Option<RunMark<'_>> = None;
    let brake = !todo.is_empty() && attempts >= MAX_ATTEMPTS;
    if brake {
        warn!(
            "meetings: diarization of {} skipped, {attempts} earlier runs did not finish",
            meeting.id
        );
    }
    let unavailable = diarizer.unavailable();
    // P8a: Gesamtdauer der Kanaele, die das Modell rechnen wird.
    let total_ms: u64 = if unavailable.is_some() || brake {
        0
    } else {
        todo.iter()
            .filter_map(|t| wav_duration_ms(&t.path))
            .filter(|ms| (MIN_AUDIO_MS..=MAX_AUDIO_MS).contains(ms))
            .sum()
    };
    let mut done_ms: u64 = 0;

    for track in todo {
        let channel = track.channel;
        if channel == CHANNEL_MIC
            && track.path.file_name().and_then(|n| n.to_str()) != Some(MIC_AEC_FILE)
        {
            report.mic_without_aec = true;
        }
        if let Some(why) = unavailable {
            skip(report, channel, why, 0);
            continue;
        }
        if brake {
            skip(report, channel, "crash_loop", 0);
            continue;
        }
        if diarizer.cancelled() {
            report.state = "cancelled".into();
            diarizer.release();
            if let Some(mark) = mark.take() {
                mark.finish(report);
            }
            return Err(Cancelled);
        }
        let Some(audio_ms) = wav_duration_ms(&track.path) else {
            skip(report, channel, "unreadable", 0);
            continue;
        };
        if audio_ms < MIN_AUDIO_MS {
            // Weniger als 2 s: keine Sprache, kein Modell, 0 Sprecher.
            set.channels.insert(channel, Vec::new());
            set.fresh.insert(channel);
            skip(report, channel, "too_short", audio_ms);
            continue;
        }
        if audio_ms > MAX_AUDIO_MS {
            skip(report, channel, "too_long", audio_ms);
            continue;
        }
        if let Err(e) = diarizer.check_ram(DIARIZE_RAM_NEED_MB + pcm_mb(audio_ms)) {
            warn!("meetings: diarization of channel {channel} not started: {e}");
            skip(report, channel, "low_memory", audio_ms);
            continue;
        }
        if mark.is_none() {
            mark = Some(RunMark::start(store, &meeting.id, attempts + 1));
            report.attempts = attempts + 1;
        }
        let pcm = match read_channel_pcm(&track.path) {
            Ok(p) => p,
            Err(e) => {
                warn!("meetings: channel {channel} not readable for diarization: {e}");
                skip(report, channel, "unreadable", audio_ms);
                continue;
            }
        };
        let run = Instant::now();
        diarizer.progress(done_ms, total_ms);
        let result = diarizer.diarize(channel, &pcm);
        drop(pcm);
        // Ein abgebrochener Kanal ist nicht "fertig": der Balken springt nicht auf 100 %.
        if !matches!(result, Err(DiarizeError::Cancelled)) {
            done_ms += audio_ms;
            diarizer.progress(done_ms, total_ms);
        }
        match result {
            Ok(turns) => {
                report.channels.push(ChannelReport {
                    channel,
                    source: "model",
                    speakers: distinct_speakers(&turns),
                    turns: turns.len(),
                    audio_ms,
                    model_ms: run.elapsed().as_millis() as u64,
                    skipped: None,
                });
                set.channels.insert(channel, turns);
                set.fresh.insert(channel);
            }
            Err(DiarizeError::Cancelled) => {
                report.state = "cancelled".into();
                diarizer.release();
                if let Some(mark) = mark.take() {
                    mark.finish(report);
                }
                return Err(Cancelled);
            }
            Err(e) => {
                warn!("meetings: diarization of channel {channel} failed: {e}");
                skip(report, channel, skip_reason(&e), audio_ms);
            }
        }
    }
    diarizer.release();
    report.wall_ms += started.elapsed().as_millis() as u64;
    let state = if set.channels.is_empty() {
        "skipped"
    } else {
        "done"
    };
    report.state = state.into();
    if let Some(mark) = mark.take() {
        mark.finish(report);
    } else if brake {
        write_report(store, &meeting.id, report);
    }
    if set.channels.is_empty() {
        return Ok(None);
    }
    Ok(Some(set))
}

// ---------------------------------------------------------------------------
// Zuordnen und speichern
// ---------------------------------------------------------------------------

/// Ergebnis von [`apply_to_stored`].
#[derive(Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Geschrieben; `epoch` ist die Epoche danach, `epoch_bumped` ob Segmente
    /// geteilt wurden.
    Applied { epoch: u32, epoch_bumped: bool },
    /// Nichts zu aendern (zweiter Lauf, keine Sprecher gefunden).
    Unchanged,
    /// Das Transkript wurde von Hand veraendert, auch nach neuem Ansatz.
    Conflict,
    /// Store-Fehler; das Transkript ist unveraendert.
    Failed(String),
}

fn present_speakers(segments: &[StoredSegment]) -> Vec<(u8, u32)> {
    let mut v: Vec<(u8, u32)> = segments
        .iter()
        .filter_map(|s| s.speaker_index.map(|n| (s.channel, n)))
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Die Sprecherdaten zu `segments`: neues `speaker_hints_json` (Turns der
/// Kanaele aus `set`, Rest bleibt), Zeilen fuer alle vorkommenden Sprecher
/// und fuer jeden neu berechneten Kanal die Zuordnung alt -> neu (Namen
/// wandern mit, siehe [`remap_speakers`]).
pub fn speaker_write(
    store: &MeetingStore,
    meeting_id: &str,
    set: &TurnSet,
    segments: &[StoredSegment],
) -> SpeakerWrite {
    let existing = match store.speaker_hints(meeting_id) {
        Ok(text) => text,
        Err(e) => {
            warn!("meetings: speaker hints not read ({meeting_id}): {e}");
            None
        }
    };
    let old_turns = existing
        .as_deref()
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .map(|v| turns_from_hints(&v))
        .unwrap_or_default();
    let mut fresh = BTreeMap::new();
    for channel in &set.fresh {
        let new = set.channels.get(channel).map(Vec::as_slice).unwrap_or(&[]);
        let map = old_turns
            .get(channel)
            .filter(|old| !old.is_empty())
            .map(|old| remap_speakers(old, new));
        fresh.insert(*channel, map);
    }
    SpeakerWrite {
        hints_json: hints_with_turns(
            existing.as_deref(),
            &set.model,
            set.gap_ms,
            set.pad_ms,
            &set.channels,
        ),
        present: present_speakers(segments),
        fresh,
    }
}

/// Neu nummerieren, wenn Segmente hinzugekommen sind: in der vorhandenen
/// Reihenfolge, ab 0 (die Epoche steigt dann).
fn renumber(segments: &mut [StoredSegment]) {
    for (i, s) in segments.iter_mut().enumerate() {
        s.segment_index = i as u32;
    }
}

/// Wendet `set` auf das GESPEICHERTE Transkript an (Live = Ende, Import,
/// Neu-Transkription): zuordnen, an Sprecherwechseln teilen, Segmente,
/// `speaker_hints_json` und `speakers` in einer Transaktion schreiben. Bei
/// einer Korrektur von Hand waehrend des Laufs (Revision) wird mit dem neuen
/// Stand bis zu [`APPLY_ATTEMPTS`]-mal neu angesetzt. Ein zweiter Aufruf mit
/// denselben Turns aendert nichts (`Unchanged`, keine zweite Epoche).
pub fn apply_to_stored(
    store: &MeetingStore,
    meeting_id: &str,
    set: &TurnSet,
    report: &mut StepReport,
) -> ApplyOutcome {
    apply_to_stored_with(store, meeting_id, set, report, &mut || {})
}

/// [`apply_to_stored`] mit einem Haken direkt nach dem Schnappschuss (Tests
/// stellen dort eine Korrektur von Hand nach; die Produktion uebergibt nichts).
fn apply_to_stored_with(
    store: &MeetingStore,
    meeting_id: &str,
    set: &TurnSet,
    report: &mut StepReport,
    after_snapshot: &mut dyn FnMut(),
) -> ApplyOutcome {
    for _ in 0..APPLY_ATTEMPTS {
        let snapshot = match store.transcript_snapshot(meeting_id) {
            Ok(s) => s,
            Err(e) => return ApplyOutcome::Failed(e.to_string()),
        };
        after_snapshot();
        let before = snapshot.segments.len();
        let (mut segments, stats) = assign_segments(snapshot.segments.clone(), &set.channels);
        let split = segments.len() != before;
        if split {
            renumber(&mut segments);
        }
        let write = speaker_write(store, meeting_id, set, &segments);
        if !split && segments == snapshot.segments && !needs_write(store, meeting_id, &write) {
            report.assigned = stats.assigned;
            report.unassigned = stats.unassigned;
            return ApplyOutcome::Unchanged;
        }
        match store.update_segments_with_speakers(
            meeting_id,
            &segments,
            snapshot.revision,
            split,
            &write,
        ) {
            Ok(epoch) => {
                report.assigned = stats.assigned;
                report.unassigned = stats.unassigned;
                report.split_added = stats.split_added;
                report.applied = true;
                return ApplyOutcome::Applied {
                    epoch,
                    epoch_bumped: split,
                };
            }
            Err(ReplaceError::Conflict { .. }) => {
                info!(
                    "meetings: transcript changed during the speaker step ({meeting_id}), retrying"
                );
            }
            Err(ReplaceError::Store(e)) => return ApplyOutcome::Failed(e),
        }
    }
    ApplyOutcome::Conflict
}

/// Muesste `write` noch etwas aendern (Hinweise, fehlende Zeilen)?
fn needs_write(store: &MeetingStore, meeting_id: &str, write: &SpeakerWrite) -> bool {
    // Gleiche Hinweise heissen: die neuen Turns sind die alten, der Abgleich
    // alt -> neu ist die Identitaet und aendert keine Zeile.
    let same_hints =
        matches!(store.speaker_hints(meeting_id), Ok(Some(t)) if t == write.hints_json);
    if !same_hints {
        return true;
    }
    let rows = store.speaker_rows(meeting_id).unwrap_or_default();
    write.present.iter().any(|(c, n)| {
        !rows
            .iter()
            .any(|r| r.channel == *c && r.speaker_index == *n)
    })
}

/// Ein Durchgang fuer ein gespeichertes Transkript (Import, Neu-Transkription):
/// Plan, Turns holen (gespeicherte zuerst), zuordnen, schreiben. Der Bericht
/// steht danach in `metadata_json.diarize`. Ein Fehler in irgendeinem Schritt
/// laesst das Transkript unveraendert; das Ergebnis meldet nur, ob geschrieben
/// wurde (`Applied`), damit der Aufrufer die Anzeige neu laden kann.
pub fn step_on_stored(
    store: &MeetingStore,
    meeting_id: &str,
    diarizer: &mut dyn ChannelDiarizer,
    use_stored: bool,
) -> (StepReport, ApplyOutcome) {
    let started = Instant::now();
    let mut report = StepReport {
        state: "skipped".into(),
        ..Default::default()
    };
    let meeting = match store.get_meeting(meeting_id) {
        Ok(Some(m)) if m.deleted_at.is_none() => m,
        _ => return (report, ApplyOutcome::Failed("meeting not found".into())),
    };
    let metadata = store.metadata_json(meeting_id).ok().flatten();
    let tracks = diarize_tracks(&meeting, metadata.as_ref());
    let collected = collect_turns(
        store,
        &meeting,
        metadata.as_ref(),
        &tracks,
        use_stored,
        diarizer,
        &mut report,
    );
    let outcome = match collected {
        Ok(Some(set)) => apply_to_stored(store, meeting_id, &set, &mut report),
        Ok(None) => ApplyOutcome::Unchanged,
        Err(Cancelled) => ApplyOutcome::Unchanged,
    };
    report.wall_ms = started.elapsed().as_millis() as u64;
    write_report(store, meeting_id, &report);
    (report, outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus, TranscriptDelta};
    use crate::managers::transcription::WordTime;
    use std::cell::RefCell;
    use std::path::Path;

    fn turn(start_ms: u64, end_ms: u64, speaker: u32) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn word(text: &str, start_ms: u64, end_ms: u64) -> WordTime {
        WordTime {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    /// Ein Segment mit Wortzeiten (je `step` ms ab `from`) und Text aus den Woertern.
    fn seg(index: u32, channel: u8, from: u64, texts: &[&str], step: u64) -> StoredSegment {
        let words: Vec<WordTime> = texts
            .iter()
            .enumerate()
            .map(|(i, t)| word(t, from + i as u64 * step, from + (i as u64 + 1) * step))
            .collect();
        StoredSegment {
            segment_index: index,
            text: texts.join(" "),
            start_ms: from,
            end_ms: from + texts.len() as u64 * step,
            channel,
            speaker_index: None,
            words: Some(words),
        }
    }

    fn plain(index: u32, channel: u8, start_ms: u64, end_ms: u64) -> StoredSegment {
        StoredSegment {
            segment_index: index,
            text: format!("s{index}"),
            start_ms,
            end_ms,
            channel,
            speaker_index: None,
            words: None,
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        store: MeetingStore,
        id: String,
    }

    fn fixture(source: MeetingSource) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("meetings.db")).unwrap();
        let m = store.create_meeting("t", source, Some(0)).unwrap();
        store.set_status(&m.id, MeetingStatus::Processing).unwrap();
        Fixture {
            _dir: dir,
            store,
            id: m.id,
        }
    }

    fn put(f: &Fixture, segments: Vec<StoredSegment>) {
        f.store
            .append_delta(
                &f.id,
                &TranscriptDelta {
                    new_segments: segments,
                },
            )
            .unwrap();
    }

    fn set(channels: &[(u8, Vec<Turn>)], fresh: &[u8]) -> TurnSet {
        TurnSet {
            channels: channels.iter().cloned().collect(),
            fresh: fresh.iter().copied().collect(),
            model: HINTS_MODEL_NAME.into(),
            gap_ms: DEFAULT_GAP_MS,
            pad_ms: DEFAULT_PAD_MS,
        }
    }

    // ---- Labels -------------------------------------------------------------

    #[test]
    fn labels_use_the_name_else_the_channel_and_number() {
        let dir = SpeakerDirectory::new(
            [
                ((1, 2), "  Anna Berg ".to_string()),
                ((1, 3), "   ".to_string()),
            ],
            true,
        );
        let mut s = plain(0, 1, 0, 1);
        assert_eq!(
            dir.label(&s),
            "Gegenseite",
            "ohne Sprechertrennung wie bisher"
        );
        s.speaker_index = Some(1);
        assert_eq!(dir.label(&s), "Gegenseite 1");
        s.speaker_index = Some(2);
        assert_eq!(dir.label(&s), "Anna Berg", "Name getrimmt");
        s.speaker_index = Some(3);
        assert_eq!(dir.label(&s), "Gegenseite 3", "leerer Name zaehlt nicht");
        assert_eq!(dir.label_for(0, None), "Ich");
        assert_eq!(dir.label_for(2, None), "Aufnahme");
        assert_eq!(dir.label_for(2, Some(1)), "Person 1", "Import");
        assert_eq!(
            dir.label_for(0, Some(2)),
            "Raum 2",
            "mehrere Personen am Mikrofon + Systemton"
        );
        let presence = SpeakerDirectory::new([], false);
        assert_eq!(presence.label_for(0, Some(1)), "Person 1", "Praesenz");
        assert_eq!(presence.label_for(7, Some(1)), "Sprecher 1");
        assert_eq!(presence.label_for(7, None), "Kanal 7");
    }

    #[test]
    fn the_directory_loads_names_and_the_channel_shape_from_the_store() {
        let f = fixture(MeetingSource::Live);
        put(&f, vec![plain(0, 1, 0, 1_000)]);
        f.store.set_speaker_name(&f.id, 1, 2, Some("Anna")).unwrap();
        f.store.set_speaker_name(&f.id, 0, 1, Some("Ben")).unwrap();
        let dir = SpeakerDirectory::load(&f.store, &f.id);
        assert_eq!(dir.name(1, 2), Some("Anna"));
        assert_eq!(
            dir.label_for(0, Some(2)),
            "Raum 2",
            "Kanal 1 hat Segmente: es gibt eine Gegenseite"
        );
        // Unbekannte Besprechung: Standardlabels statt Fehler.
        let none = SpeakerDirectory::load(&f.store, "gibt-es-nicht");
        assert_eq!(none.label_for(1, Some(1)), "Gegenseite 1");
        // Einen Namen loeschen.
        f.store.set_speaker_name(&f.id, 1, 2, None).unwrap();
        assert_eq!(SpeakerDirectory::load(&f.store, &f.id).name(1, 2), None);
    }

    // ---- speaker_hints_json ---------------------------------------------------

    #[test]
    fn hints_round_trip_and_keep_other_channels_and_unknown_keys() {
        let first = hints_with_turns(
            None,
            "m",
            1_000,
            100,
            &[(1u8, vec![turn(0, 1_000, 1), turn(1_500, 2_500, 2)])]
                .into_iter()
                .collect(),
        );
        let v: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["model"], "m");
        assert_eq!(v["params"]["gap_ms"], 1_000);
        assert_eq!(v["channels"]["1"][1], json!([1_500, 2_500, 2]));
        // Kanal 0 kommt dazu, Kanal 1 und ein fremder Schluessel bleiben.
        let mut with_extra: Value = serde_json::from_str(&first).unwrap();
        with_extra["rejected"] = json!({ "1": ["human"] });
        let second = hints_with_turns(
            Some(&with_extra.to_string()),
            "m2",
            900,
            50,
            &[(0u8, vec![turn(0, 500, 1)])].into_iter().collect(),
        );
        let v: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(v["model"], "m2");
        assert_eq!(v["rejected"]["1"][0], "human");
        let turns = turns_from_hints(&v);
        assert_eq!(turns[&1].len(), 2);
        assert_eq!(turns[&0], vec![turn(0, 500, 1)]);
        // Kaputte Eintraege entfallen.
        let broken = json!({ "channels": { "1": [[0, 100, 1], [5], "x", [100, 50, 1], [0, 10, 0]], "zz": [[0, 1, 1]] } });
        assert_eq!(turns_from_hints(&broken)[&1], vec![turn(0, 100, 1)]);
        assert!(turns_from_hints(&json!("kein Objekt")).is_empty());
    }

    // ---- remap_speakers -----------------------------------------------------

    #[test]
    fn remap_follows_a_permutation() {
        let old = vec![
            turn(0, 10_000, 1),
            turn(10_000, 30_000, 2),
            turn(30_000, 40_000, 3),
        ];
        // Dieselben Personen, andere Nummern (Ankunftsreihenfolge anders).
        let new = vec![
            turn(0, 10_500, 3),
            turn(10_000, 30_000, 1),
            turn(30_000, 40_000, 2),
        ];
        let map = remap_speakers(&old, &new);
        assert_eq!(map.get(&1), Some(&3));
        assert_eq!(map.get(&2), Some(&1));
        assert_eq!(map.get(&3), Some(&2));
    }

    #[test]
    fn remap_with_an_extra_or_a_missing_speaker() {
        let old = vec![turn(0, 12_000, 1), turn(12_000, 20_000, 2)];
        // Ein dritter Sprecher kommt dazu: die beiden alten behalten ihre Partner.
        let more = vec![
            turn(0, 12_000, 1),
            turn(12_000, 20_000, 2),
            turn(20_000, 30_000, 3),
        ];
        let map = remap_speakers(&old, &more);
        assert_eq!((map.get(&1), map.get(&2)), (Some(&1), Some(&2)));
        assert_eq!(map.len(), 2, "der neue Sprecher 3 hat keinen alten Namen");
        // Das Modell fasst 2 in 1 zusammen: Sprecher 1 hat die groessere
        // Ueberlappung (12 s gegen 8 s), Sprecher 2 hat keinen Partner mehr.
        let fewer = vec![turn(0, 20_000, 1)];
        let map = remap_speakers(&old, &fewer);
        assert_eq!(map.get(&1), Some(&1));
        assert_eq!(map.get(&2), None);
        assert!(remap_speakers(&[], &more).is_empty());
        assert!(remap_speakers(&old, &[]).is_empty());
    }

    #[test]
    fn remap_never_attaches_a_name_to_a_chance_overlap() {
        let old = vec![turn(0, 60_000, 1)];
        // 300 ms Zufallsueberlappung: keine Zuordnung.
        let tiny = vec![turn(59_700, 90_000, 1)];
        assert!(remap_speakers(&old, &tiny).is_empty());
        // 10 s bei 60 s Sprechzeit (17 % < 20 %): ebenfalls keine.
        let small = vec![turn(50_000, 70_000, 1)];
        assert!(remap_speakers(&old, &small).is_empty());
        // 20 s bei 60 s (33 %): ja.
        let fair = vec![turn(40_000, 70_000, 1)];
        assert_eq!(remap_speakers(&old, &fair).get(&1), Some(&1));
    }

    // ---- Plan ---------------------------------------------------------------

    fn meeting_with(f: &Fixture, mic: Option<&Path>, system: Option<&Path>) -> Meeting {
        f.store
            .set_audio_paths(
                &f.id,
                mic.and_then(Path::to_str),
                system.and_then(Path::to_str),
                Some(4_000),
            )
            .unwrap();
        f.store.get_meeting(&f.id).unwrap().unwrap()
    }

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn the_plan_follows_the_recording_kind() {
        let f = fixture(MeetingSource::Live);
        let d = f._dir.path();
        let mic = touch(d, "mic.wav");
        let system = touch(d, "system.wav");
        let ch = |m: &Meeting, md: Option<&Value>| -> Vec<u8> {
            diarize_tracks(m, md).iter().map(|t| t.channel).collect()
        };
        // Online mit Systemton: nur die Gegenseite.
        let online = meeting_with(&f, Some(&mic), Some(&system));
        assert_eq!(ch(&online, None), vec![1]);
        // Mit dem Haekchen "Mehrere Personen am Mikrofon" auch das Mikrofon,
        // und zwar die Spur ohne Echo, wenn es sie gibt.
        let md = json!({ "diarize_mic": true });
        assert_eq!(ch(&online, Some(&md)), vec![0, 1]);
        let aec = touch(d, "mic_aec.wav");
        let tracks = diarize_tracks(&online, Some(&md));
        assert_eq!(tracks[0].path, aec);
        assert!(
            !diarize_mic(Some(&json!({ "diarize_mic": "ja" }))),
            "nur ein echtes bool zaehlt"
        );
        // Praesenz (kein Systemton): das Mikrofon.
        let presence = meeting_with(&f, Some(&mic), None);
        assert_eq!(ch(&presence, None), vec![0]);
        // Fehlt die Datei, gibt es nichts zu tun.
        std::fs::remove_file(&system).unwrap();
        let gone = meeting_with(&f, Some(&mic), Some(&system));
        assert!(
            ch(&gone, None).is_empty(),
            "nur Kanal 0 ohne Haekchen + mit Systempfad: nichts"
        );
    }

    #[test]
    fn imports_get_one_mixed_track_and_subtitles_none() {
        let f = fixture(MeetingSource::Import);
        let wav = touch(f._dir.path(), "import.wav");
        let m = meeting_with(&f, Some(&wav), None);
        let tracks = diarize_tracks(&m, None);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].channel, 2);
        let sub = fixture(MeetingSource::Subtitle);
        let m = sub.store.get_meeting(&sub.id).unwrap().unwrap();
        assert!(diarize_tracks(&m, None).is_empty());
    }

    // ---- Turns holen --------------------------------------------------------

    /// Attrappe: feste Turns je Kanal, zaehlt Aufrufe, steuerbare Fehler.
    struct Fake {
        progress: Vec<(u64, u64)>,
        turns: BTreeMap<u8, Vec<Turn>>,
        calls: RefCell<Vec<u8>>,
        ram_error: Option<String>,
        run_error: Option<fn() -> DiarizeError>,
        unavailable: Option<&'static str>,
        cancel_after_calls: Option<usize>,
        released: usize,
    }

    impl Fake {
        fn new(turns: &[(u8, Vec<Turn>)]) -> Self {
            Self {
                progress: Vec::new(),
                turns: turns.iter().cloned().collect(),
                calls: RefCell::new(Vec::new()),
                ram_error: None,
                run_error: None,
                unavailable: None,
                cancel_after_calls: None,
                released: 0,
            }
        }
    }

    impl ChannelDiarizer for Fake {
        fn model_name(&self) -> String {
            "fake".into()
        }
        fn unavailable(&self) -> Option<&'static str> {
            self.unavailable
        }
        fn check_ram(&mut self, need_mb: u64) -> Result<(), String> {
            assert!(
                need_mb >= DIARIZE_RAM_NEED_MB,
                "Modellbedarf ist immer dabei"
            );
            self.ram_error.clone().map_or(Ok(()), Err)
        }
        fn diarize(&mut self, channel: u8, pcm: &[f32]) -> Result<Vec<Turn>, DiarizeError> {
            assert!(!pcm.is_empty());
            self.calls.borrow_mut().push(channel);
            if let Some(e) = self.run_error {
                return Err(e());
            }
            Ok(self.turns.get(&channel).cloned().unwrap_or_default())
        }
        fn cancelled(&self) -> bool {
            self.cancel_after_calls
                .is_some_and(|n| self.calls.borrow().len() >= n)
        }
        fn progress(&mut self, done_ms: u64, total_ms: u64) {
            self.progress.push((done_ms, total_ms));
        }
        fn release(&mut self) {
            self.released += 1;
        }
    }

    fn write_wav(path: &Path, ms: u64) {
        let mut w = crate::audio_toolkit::audio::StreamingWavWriter::create(path, 16_000).unwrap();
        w.append(&vec![1_000i16; (ms * 16) as usize]).unwrap();
        w.finalize().unwrap();
    }

    /// Live-Besprechung mit mic.wav und system.wav (je `ms` lang).
    fn live(ms: u64) -> (Fixture, Meeting) {
        let f = fixture(MeetingSource::Live);
        let mic = f._dir.path().join("mic.wav");
        let system = f._dir.path().join("system.wav");
        write_wav(&mic, ms);
        write_wav(&system, ms);
        let m = meeting_with(&f, Some(&mic), Some(&system));
        (f, m)
    }

    fn collect(
        f: &Fixture,
        m: &Meeting,
        fake: &mut Fake,
        use_stored: bool,
    ) -> (Result<Option<TurnSet>, Cancelled>, StepReport) {
        let md = f.store.metadata_json(&m.id).unwrap();
        let tracks = diarize_tracks(m, md.as_ref());
        let mut report = StepReport::default();
        let r = collect_turns(
            &f.store,
            m,
            md.as_ref(),
            &tracks,
            use_stored,
            fake,
            &mut report,
        );
        (r, report)
    }

    #[test]
    fn collect_turns_runs_the_model_per_channel_and_releases_it() {
        let (f, m) = live(4_000);
        f.store
            .set_metadata_key(&f.id, "diarize_mic", json!(true))
            .unwrap();
        let mut fake = Fake::new(&[
            (0, vec![turn(0, 4_000, 1)]),
            (1, vec![turn(0, 2_000, 1), turn(2_000, 4_000, 2)]),
        ]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        let set = r.unwrap().unwrap();
        assert_eq!(*fake.calls.borrow(), vec![0, 1]);
        assert_eq!(set.channels.len(), 2);
        assert_eq!(set.fresh, BTreeSet::from([0, 1]));
        assert_eq!(set.model, "fake");
        assert_eq!(fake.released, 1, "Modell vor dem End-STT frei");
        assert_eq!(report.state, "done");
        assert_eq!(report.channels[1].speakers, 2);
        // Der Bericht steht (statt der Absturzmarke) in metadata_json.
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(md["diarize"]["state"], "done");
        assert_eq!(md["diarize"]["attempts"], 1);
    }

    /// P8a: je Kanal vor und nach dem Modelllauf ein Stand, mit der
    /// Gesamtdauer der Kanaele, die wirklich gerechnet werden.
    #[test]
    fn collect_turns_reports_progress_per_channel_against_the_total() {
        let (f, m) = live(4_000);
        f.store
            .set_metadata_key(&f.id, "diarize_mic", json!(true))
            .unwrap();
        let mut fake = Fake::new(&[
            (0, vec![turn(0, 4_000, 1)]),
            (1, vec![turn(0, 4_000, 1)]),
        ]);
        let (r, _) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_some());
        assert_eq!(
            fake.progress,
            vec![(0, 8_000), (4_000, 8_000), (4_000, 8_000), (8_000, 8_000)]
        );
    }

    #[test]
    fn a_channel_that_fails_still_advances_the_progress() {
        let (f, m) = live(4_000);
        let mut fake = Fake::new(&[]);
        fake.run_error = Some(|| DiarizeError::Run("kaputt".into()));
        let (_r, _) = collect(&f, &m, &mut fake, true);
        assert_eq!(fake.progress.last(), Some(&(4_000, 4_000)), "Balken bleibt nicht stehen");
    }

    #[test]
    fn a_cancelled_channel_never_reports_itself_as_done() {
        let (f, m) = live(4_000);
        let mut fake = Fake::new(&[]);
        fake.run_error = Some(|| DiarizeError::Cancelled);
        let (r, _) = collect(&f, &m, &mut fake, true);
        assert_eq!(r, Err(Cancelled));
        assert_eq!(
            fake.progress,
            vec![(0, 4_000)],
            "nur der Start, kein Sprung auf 100 % nach dem Abbruch"
        );
    }

    #[test]
    fn nothing_is_reported_when_the_model_will_not_run() {
        let (f, m) = live(4_000);
        let mut fake = Fake::new(&[]);
        fake.unavailable = Some("disabled");
        let (_r, _) = collect(&f, &m, &mut fake, true);
        assert!(fake.progress.is_empty());
    }

    #[test]
    fn stored_turns_are_reused_and_never_recomputed() {
        let (f, m) = live(4_000);
        let stored = set(&[(1, vec![turn(0, 4_000, 1)])], &[1]);
        let mut report = StepReport::default();
        put(&f, vec![seg(0, 1, 0, &["a", "b", "c", "d"], 500)]);
        assert!(matches!(
            apply_to_stored(&f.store, &f.id, &stored, &mut report),
            ApplyOutcome::Applied { .. }
        ));
        let mut fake = Fake::new(&[(1, vec![turn(0, 1_000, 9)])]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        let got = r.unwrap().unwrap();
        assert!(fake.calls.borrow().is_empty(), "kein Modelllauf");
        assert_eq!(got.channels[&1], vec![turn(0, 4_000, 1)]);
        assert!(got.fresh.is_empty());
        assert_eq!(report.channels[0].source, "stored");
        // Ohne `use_stored` wird neu gerechnet.
        let (r, _) = collect(&f, &m, &mut fake, false);
        assert_eq!(r.unwrap().unwrap().channels[&1], vec![turn(0, 1_000, 9)]);
    }

    #[test]
    fn a_channel_that_cannot_run_is_skipped_and_the_others_still_run() {
        let (f, m) = live(4_000);
        f.store
            .set_metadata_key(&f.id, "diarize_mic", json!(true))
            .unwrap();
        // Kanal 0 ohne Datei: uebersprungen, Kanal 1 laeuft.
        std::fs::remove_file(f._dir.path().join("mic.wav")).unwrap();
        let mut fake = Fake::new(&[(1, vec![turn(0, 4_000, 1)])]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_some());
        assert_eq!(*fake.calls.borrow(), vec![1]);
        let _ = report;

        // Zu wenig RAM: kein Kanal gelesen, kein Modelllauf, Ergebnis None.
        let (f, m) = live(4_000);
        let mut fake = Fake::new(&[(1, vec![turn(0, 4_000, 1)])]);
        fake.ram_error = Some("Zu wenig freier Arbeitsspeicher".into());
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_none());
        assert!(fake.calls.borrow().is_empty());
        assert_eq!(report.channels[0].skipped, Some("low_memory"));
        assert_eq!(report.state, "skipped");
        // Es gibt keine Absturzmarke, weil kein Modell lief.
        assert!(f
            .store
            .metadata_json(&f.id)
            .unwrap()
            .is_none_or(|m| m["diarize"].is_null()));

        // Modell fehlt / abgeschaltet: uebersprungen, ohne Audio zu lesen.
        let mut fake = Fake::new(&[]);
        fake.unavailable = Some("model_missing");
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_none());
        assert_eq!(report.channels[0].skipped, Some("model_missing"));

        // Laufzeitfehler des Modells: uebersprungen, weiter.
        let mut fake = Fake::new(&[]);
        fake.run_error = Some(|| DiarizeError::Run("kaputt".into()));
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_none());
        assert_eq!(report.channels[0].skipped, Some("failed"));
        assert_eq!(fake.released, 1);
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(
            md["diarize"]["state"], "skipped",
            "Marke ist durch den Bericht ersetzt"
        );
    }

    #[test]
    fn short_and_endless_channels_never_reach_the_model() {
        // Unter 2 s: 0 Sprecher, aber "diarisiert" (Namen bleiben Sache des Abgleichs).
        let (f, m) = live(1_500);
        let mut fake = Fake::new(&[(1, vec![turn(0, 1_000, 1)])]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        let got = r.unwrap().unwrap();
        assert!(got.channels[&1].is_empty());
        assert!(fake.calls.borrow().is_empty());
        assert_eq!(report.channels[0].skipped, Some("too_short"));
        // RAM-Bedarf waechst mit der Laenge (PCM-Groesse zaehlt).
        assert_eq!(pcm_mb(60 * 60 * 1000), 220);
        assert!(pcm_mb(MAX_AUDIO_MS) < 1_000);
    }

    #[test]
    fn cancelling_leaves_the_store_untouched() {
        let (f, m) = live(4_000);
        f.store
            .set_metadata_key(&f.id, "diarize_mic", json!(true))
            .unwrap();
        put(&f, vec![seg(0, 1, 0, &["a", "b"], 500)]);
        let before = f.store.transcript_snapshot(&f.id).unwrap();
        let mut fake = Fake::new(&[(0, vec![turn(0, 4_000, 1)]), (1, vec![turn(0, 4_000, 1)])]);
        fake.cancel_after_calls = Some(1); // nach Kanal 0 kommt die neue Aufnahme
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert_eq!(r, Err(Cancelled));
        assert_eq!(
            *fake.calls.borrow(),
            vec![0],
            "Kanal 1 wird nicht mehr gerechnet"
        );
        assert_eq!(report.state, "cancelled");
        assert_eq!(fake.released, 1);
        let after = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(after.segments, before.segments);
        assert_eq!(after.revision, before.revision);
        assert_eq!(
            f.store.speaker_hints(&f.id).unwrap(),
            None,
            "keine Teil-Turns"
        );
        assert!(f.store.speaker_rows(&f.id).unwrap().is_empty());
        // Die Absturzmarke ist durch den Bericht ersetzt (nicht `running`/`aborted`).
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(md["diarize"]["state"], "cancelled");
        // Ein Abbruch VOR dem ersten Kanal schreibt nicht einmal die Marke.
        let (f2, m2) = live(4_000);
        let mut fake = Fake::new(&[]);
        fake.cancel_after_calls = Some(0);
        let (r, _) = collect(&f2, &m2, &mut fake, true);
        assert_eq!(r, Err(Cancelled));
        assert!(f2
            .store
            .metadata_json(&f2.id)
            .unwrap()
            .is_none_or(|md| md["diarize"].is_null()));
    }

    #[test]
    fn a_crashed_earlier_run_is_retried_once_then_skipped() {
        let (f, m) = live(4_000);
        // Absturz im Modell: die Marke steht auf running (Versuch 1).
        f.store
            .set_metadata_key(
                &f.id,
                "diarize",
                json!({ "state": "running", "attempts": 1 }),
            )
            .unwrap();
        let mut fake = Fake::new(&[(1, vec![turn(0, 4_000, 1)])]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_some(), "zweiter Versuch");
        assert_eq!(report.attempts, 2);
        // Wieder abgestuerzt (Versuch 2 unfertig): jetzt Schluss.
        f.store
            .set_metadata_key(
                &f.id,
                "diarize",
                json!({ "state": "running", "attempts": 2 }),
            )
            .unwrap();
        let mut fake = Fake::new(&[(1, vec![turn(0, 4_000, 1)])]);
        let (r, report) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_none());
        assert!(fake.calls.borrow().is_empty(), "kein dritter Absturz");
        assert_eq!(report.channels[0].skipped, Some("crash_loop"));
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(md["diarize"]["state"], "skipped");
        // Danach (Bericht statt Marke) ist die Bremse wieder frei.
        let mut fake = Fake::new(&[(1, vec![turn(0, 4_000, 1)])]);
        let (r, _) = collect(&f, &m, &mut fake, true);
        assert!(r.unwrap().is_some());
    }

    #[test]
    fn a_panic_during_the_run_does_not_leave_the_marker_running() {
        let (f, m) = live(4_000);
        struct Boom;
        impl ChannelDiarizer for Boom {
            fn model_name(&self) -> String {
                "boom".into()
            }
            fn check_ram(&mut self, _: u64) -> Result<(), String> {
                Ok(())
            }
            fn diarize(&mut self, _: u8, _: &[f32]) -> Result<Vec<Turn>, DiarizeError> {
                panic!("nativer Fehler nachgestellt");
            }
        }
        let md = f.store.metadata_json(&f.id).unwrap();
        let tracks = diarize_tracks(&m, md.as_ref());
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut report = StepReport::default();
            let _ = collect_turns(
                &f.store,
                &m,
                md.as_ref(),
                &tracks,
                true,
                &mut Boom,
                &mut report,
            );
        }));
        assert!(caught.is_err());
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(md["diarize"]["state"], "aborted");
    }

    // ---- Zuordnen und speichern ---------------------------------------------

    #[test]
    fn applying_splits_segments_bumps_the_epoch_and_writes_everything_at_once() {
        let f = fixture(MeetingSource::Live);
        // Ein Block mit zwei Sprechern auf Kanal 1, ein "Ich"-Segment auf Kanal 0.
        put(
            &f,
            vec![
                seg(0, 0, 0, &["Hallo", "zusammen"], 500),
                seg(
                    1,
                    1,
                    1_000,
                    &[
                        "Guten", "Tag", "Frau", "Berg", "Ja", "gerne", "danke", "schoen",
                    ],
                    500,
                ),
            ],
        );
        let before = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(before.epoch, 0);
        let ts = set(
            &[(1, vec![turn(1_000, 3_000, 1), turn(3_000, 5_000, 2)])],
            &[1],
        );
        let mut report = StepReport::default();
        let out = apply_to_stored(&f.store, &f.id, &ts, &mut report);
        assert_eq!(
            out,
            ApplyOutcome::Applied {
                epoch: 1,
                epoch_bumped: true
            }
        );
        let after = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(after.epoch, 1);
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.segments.len(), 3);
        assert_eq!(
            after
                .segments
                .iter()
                .map(|s| s.segment_index)
                .collect::<Vec<_>>(),
            vec![0, 1, 2],
            "neu nummeriert"
        );
        let ch1: Vec<_> = after
            .segments
            .iter()
            .filter(|s| s.channel == 1)
            .map(|s| s.speaker_index)
            .collect();
        assert_eq!(ch1, vec![Some(1), Some(2)]);
        assert_eq!(
            after.segments[0].speaker_index, None,
            "Ich bleibt ohne Sprecher"
        );
        assert_eq!(report.split_added, 1);
        // Hinweise und Zeilen stehen mit im selben Schritt.
        let hints: Value =
            serde_json::from_str(&f.store.speaker_hints(&f.id).unwrap().unwrap()).unwrap();
        assert_eq!(hints["channels"]["1"][0], json!([1_000, 3_000, 1]));
        let rows = f.store.speaker_rows(&f.id).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| (r.channel, r.speaker_index))
                .collect::<Vec<_>>(),
            vec![(1, 1), (1, 2)]
        );
        // Ein zweiter Lauf mit denselben Turns aendert nichts (Recovery, kein zweiter Epochensprung).
        let again = apply_to_stored(&f.store, &f.id, &ts, &mut StepReport::default());
        assert_eq!(again, ApplyOutcome::Unchanged);
        let end = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!((end.epoch, end.revision), (after.epoch, after.revision));
    }

    #[test]
    fn without_a_split_the_epoch_stays() {
        let f = fixture(MeetingSource::Live);
        put(&f, vec![plain(0, 1, 0, 2_000), plain(1, 1, 3_000, 5_000)]);
        let ts = set(&[(1, vec![turn(0, 2_500, 1), turn(2_600, 5_000, 2)])], &[1]);
        let out = apply_to_stored(&f.store, &f.id, &ts, &mut StepReport::default());
        assert_eq!(
            out,
            ApplyOutcome::Applied {
                epoch: 0,
                epoch_bumped: false
            }
        );
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(
            snap.segments
                .iter()
                .map(|s| (s.segment_index, s.speaker_index))
                .collect::<Vec<_>>(),
            vec![(0, Some(1)), (1, Some(2))],
            "Indizes unveraendert, also bleiben Belege gueltig"
        );
    }

    #[test]
    fn a_hand_correction_during_the_run_is_kept_and_the_step_retries_on_the_new_state() {
        let f = fixture(MeetingSource::Live);
        put(&f, vec![plain(0, 1, 0, 2_000), plain(1, 1, 3_000, 5_000)]);
        let ts = set(&[(1, vec![turn(0, 2_500, 1), turn(2_600, 5_000, 2)])], &[1]);
        // Die Korrektur passiert nach dem ersten Schnappschuss: das Schreiben
        // scheitert einmal an der Revision, der zweite Ansatz sieht sie.
        let db = f.store.db_path().to_path_buf();
        let id = f.id.clone();
        let mut calls = 0;
        let out = apply_to_stored_with(
            &f.store,
            &f.id,
            &ts,
            &mut StepReport::default(),
            &mut || {
                calls += 1;
                if calls == 1 {
                    let other = MeetingStore::open_at(&db).unwrap();
                    other.update_segment_text(&id, 1, "von Hand").unwrap();
                }
            },
        );
        assert_eq!(calls, 2, "ein Konflikt, ein neuer Ansatz");
        assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.segments[1].text, "von Hand", "Korrektur bleibt");
        assert_eq!(snap.segments[1].speaker_index, Some(2));
    }

    #[test]
    fn endless_hand_corrections_end_in_a_conflict_and_the_transcript_stays() {
        let f = fixture(MeetingSource::Live);
        put(&f, vec![plain(0, 1, 0, 2_000)]);
        let ts = set(&[(1, vec![turn(0, 2_000, 1)])], &[1]);
        let db = f.store.db_path().to_path_buf();
        let id = f.id.clone();
        let mut calls = 0;
        let out = apply_to_stored_with(
            &f.store,
            &f.id,
            &ts,
            &mut StepReport::default(),
            &mut || {
                calls += 1;
                let other = MeetingStore::open_at(&db).unwrap();
                other
                    .update_segment_text(&id, 0, &format!("Korrektur {calls}"))
                    .unwrap();
            },
        );
        assert_eq!(out, ApplyOutcome::Conflict);
        assert_eq!(calls, APPLY_ATTEMPTS);
        let snap = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(snap.segments[0].text, format!("Korrektur {APPLY_ATTEMPTS}"));
        assert_eq!(
            snap.segments[0].speaker_index, None,
            "nichts halb geschrieben"
        );
        assert_eq!(f.store.speaker_hints(&f.id).unwrap(), None);
    }

    #[test]
    fn a_broken_store_leaves_the_transcript_untouched() {
        let f = fixture(MeetingSource::Live);
        put(&f, vec![plain(0, 1, 0, 2_000)]);
        let ts = set(&[(1, vec![turn(0, 2_000, 1)])], &[1]);
        f.store.soft_delete_meeting(&f.id).unwrap();
        let out = apply_to_stored(&f.store, &f.id, &ts, &mut StepReport::default());
        assert!(matches!(out, ApplyOutcome::Failed(_)), "{out:?}");
    }

    /// AK: Namen ueberstehen die Neu-Transkription - auch wenn neu diarisiert
    /// und anders nummeriert wird.
    #[test]
    fn retranscribe_keeps_speaker_names() {
        let f = fixture(MeetingSource::Live);
        // Erste Fassung: zwei Sprecher auf Kanal 1 (Anna 1..20 s, Ben 20..40 s).
        let turns_v1 = vec![turn(0, 20_000, 1), turn(20_000, 40_000, 2)];
        put(
            &f,
            vec![plain(0, 1, 0, 19_000), plain(1, 1, 21_000, 39_000)],
        );
        let v1 = set(&[(1, turns_v1.clone())], &[1]);
        assert!(matches!(
            apply_to_stored(&f.store, &f.id, &v1, &mut StepReport::default()),
            ApplyOutcome::Applied { .. }
        ));
        f.store
            .set_speaker_name(&f.id, 1, 1, Some("Anna Berg"))
            .unwrap();
        f.store
            .set_speaker_name(&f.id, 1, 2, Some("Ben Kaya"))
            .unwrap();

        // Neu-Transkription mit anderem Modell: andere Segmentgrenzen, andere
        // Texte, KEIN Sprecher. Die Namen stehen in `speakers`, nicht im Text.
        f.store.clear_segments(&f.id).unwrap();
        assert_eq!(
            f.store.transcript_snapshot(&f.id).unwrap().segments.len(),
            0
        );
        put(
            &f,
            vec![
                seg(0, 1, 2_000, &["Guten", "Tag", "allerseits"], 1_000),
                seg(
                    1,
                    1,
                    18_000,
                    &["und", "jetzt", "Ben", "bitte", "du", "weiter", "gut"],
                    1_000,
                ),
                seg(
                    2,
                    1,
                    30_000,
                    &["danke", "Anna", "so", "machen", "wir", "das"],
                    1_000,
                ),
            ],
        );
        // Diarisiert wird nicht neu: die gespeicherten Turns tragen die Sprecher.
        let f_meeting = f.store.get_meeting(&f.id).unwrap().unwrap();
        let mut fake = Fake::new(&[]);
        fake.unavailable = Some("disabled");
        let mut report = StepReport::default();
        let stored = collect_turns(
            &f.store,
            &f_meeting,
            None,
            &[],
            true,
            &mut fake,
            &mut report,
        )
        .unwrap()
        .expect("gespeicherte Turns");
        assert!(stored.fresh.is_empty(), "nichts neu berechnet");
        let out = apply_to_stored(&f.store, &f.id, &stored, &mut report);
        assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");

        let dir = SpeakerDirectory::load(&f.store, &f.id);
        let segs = f.store.get_segments(&f.id).unwrap();
        let labels: Vec<String> = segs.iter().map(|s| dir.label(s)).collect();
        // "und jetzt" (18..20 s) gehoert noch Anna, "Ben bitte du weiter gut" (20..25 s) Ben.
        assert_eq!(labels.first().map(String::as_str), Some("Anna Berg"));
        assert_eq!(
            labels.last().map(String::as_str),
            Some("Ben Kaya"),
            "{labels:?}"
        );
        assert!(
            labels.iter().all(|l| l == "Anna Berg" || l == "Ben Kaya"),
            "{labels:?}"
        );
        let names: Vec<_> = f
            .store
            .speaker_rows(&f.id)
            .unwrap()
            .into_iter()
            .map(|r| r.display_name)
            .collect();
        assert_eq!(
            names,
            vec![Some("Anna Berg".to_string()), Some("Ben Kaya".to_string())]
        );

        // Zweiter Fall: neu diarisiert, das Modell nummeriert anders (Ben ist jetzt
        // Sprecher 1, Anna 2) und findet einen dritten Sprecher am Ende.
        let turns_v2 = vec![
            turn(0, 20_000, 2),
            turn(20_000, 34_000, 1),
            turn(34_000, 40_000, 3),
        ];
        let v2 = set(&[(1, turns_v2)], &[1]);
        let out = apply_to_stored(&f.store, &f.id, &v2, &mut StepReport::default());
        assert!(matches!(out, ApplyOutcome::Applied { .. }), "{out:?}");
        let dir = SpeakerDirectory::load(&f.store, &f.id);
        let segs = f.store.get_segments(&f.id).unwrap();
        let first = segs.first().unwrap();
        assert_eq!(
            first.speaker_index,
            Some(2),
            "Anna heisst jetzt Sprecher 2 ..."
        );
        assert_eq!(dir.label(first), "Anna Berg", "... und behaelt ihren Namen");
        let ben = segs
            .iter()
            .find(|s| s.text.starts_with("und jetzt Ben") || s.text.starts_with("Ben"))
            .unwrap();
        assert_eq!(dir.label(ben), "Ben Kaya");
        let third = segs.last().unwrap();
        assert_eq!(third.speaker_index, Some(3));
        assert_eq!(
            dir.label(third),
            "Gegenseite 3",
            "der neue Sprecher hat noch keinen Namen"
        );
        // Die Hinweise tragen jetzt die neuen Turns.
        let hints: Value =
            serde_json::from_str(&f.store.speaker_hints(&f.id).unwrap().unwrap()).unwrap();
        assert_eq!(hints["channels"]["1"][0], json!([0, 20_000, 2]));
    }

    #[test]
    fn a_speaker_that_vanishes_in_a_new_run_takes_the_name_with_it() {
        let f = fixture(MeetingSource::Live);
        put(
            &f,
            vec![plain(0, 1, 0, 11_000), plain(1, 1, 13_000, 19_000)],
        );
        let v1 = set(
            &[(1, vec![turn(0, 12_000, 1), turn(12_000, 20_000, 2)])],
            &[1],
        );
        apply_to_stored(&f.store, &f.id, &v1, &mut StepReport::default());
        f.store.set_speaker_name(&f.id, 1, 2, Some("Ben")).unwrap();
        // Das Modell fasst beide zusammen: Ben hat keinen Partner mehr.
        let v2 = set(&[(1, vec![turn(0, 20_000, 1)])], &[1]);
        apply_to_stored(&f.store, &f.id, &v2, &mut StepReport::default());
        let rows = f.store.speaker_rows(&f.id).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].speaker_index, 1);
        assert_eq!(
            rows[0].display_name, None,
            "kein Name an den falschen Sprecher"
        );
    }

    #[test]
    fn step_on_stored_diarizes_an_import_end_to_end() {
        let f = fixture(MeetingSource::Import);
        let wav = f._dir.path().join("import.wav");
        write_wav(&wav, 6_000);
        meeting_with(&f, Some(&wav), None);
        put(
            &f,
            vec![seg(
                0,
                2,
                0,
                &["Eins", "zwei", "drei", "vier", "fuenf", "sechs"],
                1_000,
            )],
        );
        let mut fake = Fake::new(&[(2, vec![turn(0, 3_000, 1), turn(3_000, 6_000, 2)])]);
        let (report, out) = step_on_stored(&f.store, &f.id, &mut fake, true);
        assert!(
            matches!(
                out,
                ApplyOutcome::Applied {
                    epoch_bumped: true,
                    ..
                }
            ),
            "{out:?}"
        );
        assert_eq!(report.state, "done");
        assert_eq!(report.split_added, 1);
        let segs = f.store.get_segments(&f.id).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(
            SpeakerDirectory::load(&f.store, &f.id).label(&segs[1]),
            "Person 2"
        );
        // Der Bericht steht in den Metadaten.
        let md = f.store.metadata_json(&f.id).unwrap().unwrap();
        assert_eq!(md["diarize"]["assigned"], 2);
        assert_eq!(md["diarize"]["channels"][0]["speakers"], 2);
    }

    #[test]
    fn step_on_stored_without_a_model_changes_nothing() {
        let f = fixture(MeetingSource::Import);
        let wav = f._dir.path().join("import.wav");
        write_wav(&wav, 6_000);
        meeting_with(&f, Some(&wav), None);
        put(&f, vec![seg(0, 2, 0, &["Eins", "zwei"], 1_000)]);
        let before = f.store.transcript_snapshot(&f.id).unwrap();
        let mut fake = Fake::new(&[]);
        fake.unavailable = Some("disabled");
        let (report, out) = step_on_stored(&f.store, &f.id, &mut fake, true);
        assert_eq!(out, ApplyOutcome::Unchanged);
        assert_eq!(report.channels[0].skipped, Some("disabled"));
        let after = f.store.transcript_snapshot(&f.id).unwrap();
        assert_eq!(
            (after.epoch, after.revision, after.segments),
            (before.epoch, before.revision, before.segments)
        );
    }

    #[test]
    fn the_pcm_reader_streams_the_recording_format_and_survives_a_torn_tail() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.wav");
        write_wav(&p, 1_000);
        let pcm = read_channel_pcm(&p).unwrap();
        assert_eq!(pcm.len(), 16_000);
        assert!((pcm[10] - 1_000.0 / 32_768.0).abs() < 1e-6);
        // Absturz mitten im Schreiben: der Header sagt mehr, als da ist.
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, &bytes[..bytes.len() - 1_001]).unwrap();
        let torn = read_channel_pcm(&p).unwrap();
        assert!(torn.len() < 16_000 && torn.len() > 15_000, "{}", torn.len());
        assert!(read_channel_pcm(&d.path().join("gibt-es-nicht.wav")).is_err());
        // Fremdes Format (44,1 kHz stereo) geht ueber den Import-Leser.
        let q = d.path().join("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&q, spec).unwrap();
        for _ in 0..44_100 {
            w.write_sample(3_000i16).unwrap();
            w.write_sample(3_000i16).unwrap();
        }
        w.finalize().unwrap();
        let mono = read_channel_pcm(&q).unwrap();
        assert!((15_900..=16_100).contains(&mono.len()), "{}", mono.len());
    }
}
