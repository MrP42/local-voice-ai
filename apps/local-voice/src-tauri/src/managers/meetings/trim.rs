//! lva#522: ein Bereich einer Aufnahme. Zwei Wege, ein Typ:
//!
//! - **A: Bereich transkribieren** (`retranscribe.rs`): jede Spur wird auf
//!   `[start_ms, end_ms)` geschnitten, nur das wird transkribiert. Die Segmente
//!   tragen weiter die ABSOLUTEN Zeiten der Aufnahme (Versatz `start_ms`), damit
//!   Player, Sprung zur Zeitmarke und Notizen auf derselben Achse bleiben.
//! - **B: Aufnahme zuschneiden** ([`trim_meeting`]): Die Audiodateien werden auf den
//!   Bereich gekuerzt, und alles, was Zeiten der Aufnahme traegt, wandert mit (Zeitachse
//!   beginnt danach bei 0:00). Die Aufnahme ist danach endgueltig kuerzer; die alten
//!   Dateien bleiben aber als Sicherung liegen (siehe unten).
//!
//! Ablauf von B nach dem Muster von `parts.rs`: die neuen WAVs entstehen in einem
//! frischen Unterordner `trim-<ulid>/` des Besprechungsordners, IMMER mit festen
//! Namen je Spur (`mic.wav`, `system.wav`, dazu die geschnittene `mic_aec.wav`:
//! `final_pass` und `derived_audio_paths` finden sie ueber den Namen neben der
//! `mic.wav`). Die Namen kommen nie aus den alten Pfaden: zwei Quellen wie `A/mic.wav`
//! und `B/MIC.WAV` wuerden unter Windows (ohne Gross-/Kleinschreibung) auf dieselbe
//! Datei zeigen und die zweite Spur die erste ueberschreiben. Ordner und Dateien
//! entstehen relativ zu Verzeichnis-Handles (`create_new`, nie ueberschreiben), auch die
//! Fehlerbereinigung laeuft nur ueber dieselben Handles (Modul `folder`). DANN schreibt
//! EINE Transaktion alles um ([`MeetingStore::apply_trim`]). Scheitert etwas vor dem
//! Commit, verschwindet der neue Ordner und alles bleibt, wie es war.
//!
//! Der Zuschnitt LOESCHT KEINE alten Dateien (Entscheidung zu #528): mic/system/mic_aec
//! bzw. der Ordner eines frueheren Zuschnitts bleiben unangetastet liegen und stehen im
//! selben Commit dauerhaft in `metadata_json["trim_superseded"]`. Das Freigeben des
//! Speichers kommt mit einem Folge-PR. Stuerzt die App zwischen dem Schreiben der WAVs
//! und dem Commit ab, bleibt ein unbenutzter Ordner `trim-<ulid>/` liegen (nur
//! Speicher, kein Datenverlust).
//!
//! Bestandsaufnahme: was in der Datenbank Zeiten der Aufnahme (ms) traegt und wie
//! B damit umgeht:
//!
//! | Daten | Behandlung |
//! |---|---|
//! | `transcripts.segments_json`, alle `transcript_variants` (auch geloeschte) | filtern (Ueberlappung), klemmen, `- start_ms`, Woerter ebenso, `segment_index` neu 0..n |
//! | `speaker_hints_json` (Turns je Kanal), Transkript und Fassungen | filtern, klemmen, verschieben |
//! | Uebersetzungs-Fassungen: `meta_json.flagged[].segment_index` | auf die neuen Indizes umgeschrieben, ausserhalb entfaellt der Eintrag |
//! | `transcript_deltas` | geloescht (Protokoll des Aufbaus, wie beim Zusammenfuehren) |
//! | `meeting_notes.blocks_json` (`at_ms`) | im Bereich: verschoben; ausserhalb: `None` (Position unbekannt, Notiz bleibt) |
//! | `meeting_parts` (Teile) | verschieben/klemmen, Teile ausserhalb entfallen; bleibt hoechstens einer, entfaellt die Liste |
//! | `meeting_slides.occurrences_json` (ms im Video) | verschoben/geklemmt; eine Folie ohne Auftreten im Bereich wird weich geloescht |
//! | `meeting_chunks`, `meeting_index_state` (Suche) | geloescht, der Indexer baut neu |
//! | `meetings.duration_ms`, `mic/system_audio_path` | neu |
//! | `meetings.metadata_json.trim_superseded` | neu: die alten Audiodateien (`path`, `trim_id`, `channel`, `at`), die dieser Zuschnitt ersetzt hat und die liegen bleiben (siehe `folder`) |
//! | `meetings.metadata_json.aec.file` | bleibt (die `mic_aec.wav` ist mit demselben Bereich mitgeschnitten); scheitert deren Schnitt, entfaellt der Verweis |
//! | `meetings.metadata_json.speaker_merges` (gelten nur fuer ihre Epoche) | auf die neue Epoche umgeschrieben: Segmente ueber die neuen Nummern, Turns ueber die neuen Positionen, ausserhalb Entfallenes entfaellt, ein Eintrag ohne Rest entfaellt |
//! | `transcripts.segment_epoch` / `content_revision` | +1: Belege aus KI-Notizen, Protokollen und Aufgaben (`source_segment_ids`) gelten damit als veraltet (derselbe Weg wie nach dem Zusammenfuehren) |
//!
//! Bewusst NICHT angepasst: Protokolle und andere Dokumente (`meeting_documents`,
//! Fliesstext mit Zeitangaben), Aufgaben (`action_items`), Projekt-Protokolle
//! (`project_minutes`, Belege `segment_index`/`start_ms`) und Chat-Verlaeufe sind
//! aus dem Transkript abgeleitet und tragen keine Zeiten als Datenfeld, das ein
//! Leser gegen das Audio laeuft - sie zeigen nach der Epoche-Erhoehung als veraltet
//! und lassen sich neu erzeugen. `started_at`/`ended_at` bleiben (Datum der
//! Besprechung); `metadata_json.timeline` (Versatz zwischen Mikrofon und Systemton)
//! bleibt, weil beide Spuren am selben Punkt geschnitten werden.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use log::{info, warn};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use ulid::Ulid;

use super::job::JobGuard;
use super::notes::model::NoteBlock;
use super::parts::MeetingPart;
use super::slides::SlideOccurrence;
use super::store::{MeetingStore, StoredSegment};

mod folder;

#[cfg(test)]
mod tests;

/// Abtastrate der Besprechungs-WAVs und der Transkription (Samples je ms = 16).
const RATE: u64 = 16_000;
const SAMPLES_PER_MS: u64 = RATE / 1_000;

/// Ein Bereich der Aufnahme in Millisekunden, `[start_ms, end_ms)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TimeRange {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl TimeRange {
    /// `start_ms < end_ms`, sonst `invalid_range`.
    pub fn validate(self) -> Result<Self, String> {
        if self.start_ms < self.end_ms {
            Ok(self)
        } else {
            Err("invalid_range".to_string())
        }
    }

    /// Laenge des Bereichs.
    pub fn len_ms(self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

/// Provenienz: `params` um den Bereich ergaenzen (nur, wenn es einen gab).
pub fn params_with_range(mut params: Value, range: Option<TimeRange>) -> Value {
    if let (Some(range), Some(map)) = (range, params.as_object_mut()) {
        map.insert(
            "range".into(),
            serde_json::json!({ "start_ms": range.start_ms, "end_ms": range.end_ms }),
        );
    }
    params
}

// ---------------------------------------------------------------------------
// Samples (16 kHz mono)
// ---------------------------------------------------------------------------

fn sample_index(ms: u64) -> usize {
    usize::try_from(ms.saturating_mul(SAMPLES_PER_MS)).unwrap_or(usize::MAX)
}

/// Schneidet eine Spur auf `range`. Eine kuerzere Spur wird am Ende geklemmt, liegt
/// der Anfang hinter ihrem Ende, bleibt sie leer. Der Aufrufer ueberspringt leere Spuren.
pub fn cut_in_place(samples: &mut Vec<i16>, range: TimeRange) {
    let end = sample_index(range.end_ms).min(samples.len());
    let start = sample_index(range.start_ms).min(end);
    samples.truncate(end);
    samples.drain(..start);
}

/// Liest eine Spur als 16-kHz-Mono nur im Bereich. Die Normalform (16 kHz, mono,
/// 16 Bit) wird angesprungen und gelesen, ohne die Aufnahme ganz in den
/// Speicher zu holen (918 Minuten waeren knapp 1,7 GB); alles andere liest
/// und rechnet um wie bisher und schneidet danach.
pub fn read_track_range(path: &Path, range: TimeRange) -> Result<Vec<i16>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("WAV nicht lesbar: {e}"))?;
    let spec = reader.spec();
    let normal = spec.sample_rate == RATE as u32
        && spec.channels == 1
        && spec.sample_format == hound::SampleFormat::Int
        && spec.bits_per_sample == 16;
    if !normal {
        drop(reader);
        let mut all = super::import::read_wav_i16_mono_16k(path)?;
        cut_in_place(&mut all, range);
        return Ok(all);
    }
    let mut reader = reader;
    let total = u64::from(reader.duration());
    let end = range.end_ms.saturating_mul(SAMPLES_PER_MS).min(total);
    let start = range.start_ms.saturating_mul(SAMPLES_PER_MS).min(end);
    if start == end {
        return Ok(Vec::new());
    }
    reader
        .seek(start as u32)
        .map_err(|e| format!("WAV nicht lesbar: {e}"))?;
    let count = usize::try_from(end - start).unwrap_or(usize::MAX);
    reader
        .samples::<i16>()
        .take(count)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("WAV nicht lesbar: {e}"))
}

// ---------------------------------------------------------------------------
// Zeiten: reine Funktionen
// ---------------------------------------------------------------------------

/// Liegt `[start, end)` (auch leer) im Bereich?
fn overlaps(start: u64, end: u64, range: TimeRange) -> bool {
    if end > start {
        start < range.end_ms && end > range.start_ms
    } else {
        start >= range.start_ms && start < range.end_ms
    }
}

/// Auf den Bereich klemmen und um `start_ms` nach vorn schieben.
fn shifted(start: u64, end: u64, range: TimeRange) -> (u64, u64) {
    let s = start.max(range.start_ms);
    let e = end.min(range.end_ms).max(s);
    (s - range.start_ms, e - range.start_ms)
}

/// Segmente im Bereich: behalten, wer ihn ueberlappt; auf den Bereich geklemmt,
/// um `start_ms` verschoben, Woerter ebenso (Woerter ausserhalb entfallen; bleiben
/// keine, hat das Segment keine Wortzeiten). `segment_index` laeuft neu ab 0.
/// Gibt zusaetzlich zurueck, welcher alte Index welcher neue wurde.
pub fn trim_segments(
    segments: Vec<StoredSegment>,
    range: TimeRange,
) -> (Vec<StoredSegment>, HashMap<u32, u32>) {
    let mut map = HashMap::new();
    let mut out = Vec::new();
    for mut s in segments {
        if !overlaps(s.start_ms, s.end_ms, range) {
            continue;
        }
        let (start, end) = shifted(s.start_ms, s.end_ms, range);
        s.start_ms = start;
        s.end_ms = end;
        s.words = s
            .words
            .map(|words| {
                words
                    .into_iter()
                    .filter(|w| overlaps(w.start_ms, w.end_ms, range))
                    .map(|mut w| {
                        let (ws, we) = shifted(w.start_ms, w.end_ms, range);
                        w.start_ms = ws;
                        w.end_ms = we;
                        w
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|words| !words.is_empty());
        let new_index = out.len() as u32;
        map.insert(s.segment_index, new_index);
        s.segment_index = new_index;
        out.push(s);
    }
    (out, map)
}

/// `speaker_hints_json`: die Turns je Kanal (`[start_ms, end_ms, sprecher]`)
/// filtern, klemmen und verschieben; alles andere im Objekt bleibt. Kaputte
/// Turns entfallen (wie beim Lesen); ein unlesbares Ganzes bleibt unberuehrt.
pub fn trim_hints(hints: &str, range: TimeRange) -> String {
    trim_hints_mapped(hints, range).0
}

/// Wie [`trim_hints`], dazu je Kanal (Schluessel wie in `channels`), welche alte
/// Position eines Turns welche neue wurde (fuer die Zusammenfuehrungen).
pub fn trim_hints_mapped(hints: &str, range: TimeRange) -> (String, TurnMap) {
    let mut positions = TurnMap::new();
    let Ok(mut root) = serde_json::from_str::<Value>(hints) else {
        return (hints.to_string(), positions);
    };
    let Some(channels) = root.get_mut("channels").and_then(Value::as_object_mut) else {
        return (hints.to_string(), positions);
    };
    for (channel, list) in channels.iter_mut() {
        let Some(turns) = list.as_array() else {
            continue;
        };
        let mut map = HashMap::new();
        let mut kept: Vec<Value> = Vec::new();
        for (at, turn) in turns.iter().enumerate() {
            let Some(turn) = turn.as_array() else {
                continue;
            };
            let (Some(start), Some(end)) = (
                turn.first().and_then(Value::as_u64),
                turn.get(1).and_then(Value::as_u64),
            ) else {
                continue;
            };
            if !overlaps(start, end, range) {
                continue;
            }
            let (s, e) = shifted(start, end, range);
            let mut out = turn.clone();
            out[0] = Value::from(s);
            out[1] = Value::from(e);
            map.insert(at, kept.len());
            kept.push(Value::Array(out));
        }
        *list = Value::Array(kept);
        positions.insert(channel.clone(), map);
    }
    (root.to_string(), positions)
}

/// Je Kanal: alte Position eines Turns -> neue Position.
pub type TurnMap = HashMap<String, HashMap<usize, usize>>;

/// Notizen: eine Position im Bereich wandert mit, eine ausserhalb wird `None`
/// ("Position unbekannt"); die Notiz selbst bleibt immer.
pub fn trim_notes(blocks: Vec<NoteBlock>, range: TimeRange) -> Vec<NoteBlock> {
    blocks
        .into_iter()
        .map(|mut b| {
            b.at_ms = b
                .at_ms
                .filter(|ms| *ms >= range.start_ms && *ms < range.end_ms)
                .map(|ms| ms - range.start_ms);
            b
        })
        .collect()
}

/// Teile (Anzeige der zusammengefuehrten Dateien): verschieben und klemmen, Teile
/// ausserhalb entfallen, `part_index` neu. Bleibt hoechstens ein Teil, ist die
/// Liste leer (eine Besprechung ohne Zeilen besteht aus einem Teil).
pub fn trim_parts(parts: Vec<MeetingPart>, range: TimeRange) -> Vec<MeetingPart> {
    let mut out: Vec<MeetingPart> = parts
        .into_iter()
        .filter(|p| overlaps(p.offset_ms, p.offset_ms + p.duration_ms, range))
        .map(|mut p| {
            let (start, end) = shifted(p.offset_ms, p.offset_ms + p.duration_ms, range);
            p.offset_ms = start;
            p.duration_ms = end - start;
            p
        })
        .collect();
    if out.len() <= 1 {
        return Vec::new();
    }
    for (i, p) in out.iter_mut().enumerate() {
        p.part_index = i as u32;
    }
    out
}

/// Auftreten einer Folie: im Bereich verschoben/geklemmt, sonst weg.
pub fn trim_occurrences(list: Vec<SlideOccurrence>, range: TimeRange) -> Vec<SlideOccurrence> {
    list.into_iter()
        .filter(|o| overlaps(o.start_ms, o.end_ms, range))
        .map(|o| {
            let (start_ms, end_ms) = shifted(o.start_ms, o.end_ms, range);
            SlideOccurrence { start_ms, end_ms }
        })
        .collect()
}

/// Pruefbericht einer Uebersetzung (`meta_json`): die markierten Saetze folgen
/// ihren neuen Indizes, ausserhalb entfallen sie. Alles andere bleibt.
pub fn remap_flagged(meta: &str, index_map: &HashMap<u32, u32>) -> String {
    let Ok(mut root) = serde_json::from_str::<Value>(meta) else {
        return meta.to_string();
    };
    let Some(flagged) = root.get_mut("flagged").and_then(Value::as_array_mut) else {
        return meta.to_string();
    };
    let kept: Vec<Value> = flagged
        .iter()
        .filter_map(|entry| {
            let old = u32::try_from(entry.get("segment_index")?.as_u64()?).ok()?;
            let new = *index_map.get(&old)?;
            let mut entry = entry.clone();
            entry["segment_index"] = Value::from(new);
            Some(entry)
        })
        .collect();
    *flagged = kept;
    root.to_string()
}

/// `metadata_json`: der Verweis auf `mic_aec.wav` entfaellt (die Spur wurde nicht
/// mitgeschnitten). `true`, wenn etwas geaendert wurde.
fn drop_aec_file(root: &mut Value) -> bool {
    let Some(aec) = root.get_mut("aec").and_then(Value::as_object_mut) else {
        return false;
    };
    if aec.remove("file").is_none() {
        return false;
    }
    aec.insert("wav_kept".into(), Value::Bool(false));
    true
}

/// Was die Zusammenfuehrungen von Sprechern (`speaker_merges`) fuer den Umzug
/// brauchen: ihre Epoche vor dem Zuschnitt, wohin Segmente (Nummern) und Turns
/// (Positionen je Kanal) des Transkripts wanderten.
pub struct MergeRemap {
    pub old_epoch: u32,
    pub segments: HashMap<u32, u32>,
    pub turns: TurnMap,
}

/// Zusammenfuehrungen der alten Epoche auf die neue Epoche umschreiben: Segmente
/// und Turns folgen ihren neuen Nummern und Positionen, was ausserhalb des
/// Bereichs lag, entfaellt; blieb von einem Eintrag, der etwas hatte, nichts
/// uebrig, entfaellt er (es gibt nichts mehr zu trennen). Eintraege anderer
/// Epochen und unlesbare bleiben unberuehrt (sie gelten ohnehin nicht).
/// `true`, wenn etwas geaendert wurde.
fn remap_merges(root: &mut Value, ctx: &MergeRemap) -> bool {
    use super::speaker_merges::{MergeRecord, MERGES_KEY};
    let Some(list) = root.get_mut(MERGES_KEY).and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    let old = std::mem::take(list);
    *list = old
        .into_iter()
        .filter_map(|value| {
            let Ok(mut rec) = serde_json::from_value::<MergeRecord>(value.clone()) else {
                return Some(value);
            };
            if rec.epoch != ctx.old_epoch {
                return Some(value);
            }
            changed = true;
            let had = !rec.segments.is_empty() || !rec.turns.is_empty();
            rec.segments = rec
                .segments
                .iter()
                .filter_map(|s| ctx.segments.get(s).copied())
                .collect();
            let turns = ctx.turns.get(&rec.channel.to_string());
            rec.turns = rec
                .turns
                .iter()
                .filter_map(|at| turns.and_then(|m| m.get(at)).copied())
                .collect();
            if had && rec.segments.is_empty() && rec.turns.is_empty() {
                return None;
            }
            rec.epoch = ctx.old_epoch.saturating_add(1);
            serde_json::to_value(&rec).ok()
        })
        .collect();
    changed
}

/// `metadata_json` nach dem Zuschnitt: der Verweis auf `mic_aec.wav` entfaellt,
/// wenn die Spur nicht mitgeschnitten wurde (`aec_kept` false), die
/// Zusammenfuehrungen ziehen auf die neue Epoche um, und die alten Dateien, die liegen
/// bleiben, werden als `trim_superseded` vermerkt (siehe `folder`). `true`, wenn sich
/// etwas aenderte.
fn trimmed_metadata(
    root: &mut Value,
    aec_kept: bool,
    merges: Option<&MergeRemap>,
    superseded: &[folder::Superseded],
) -> bool {
    let mut changed = false;
    if !aec_kept {
        changed |= drop_aec_file(root);
    }
    if let Some(ctx) = merges {
        changed |= remap_merges(root, ctx);
    }
    changed |= folder::push_superseded(root, superseded);
    changed
}

// ---------------------------------------------------------------------------
// Audio
// ---------------------------------------------------------------------------

/// Laenge einer WAV in ms (aus dem Kopf), mit ihrer Abtastrate.
fn wav_len_ms(path: &Path) -> Result<u64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("wav_unreadable: {e}"))?;
    let rate = u64::from(reader.spec().sample_rate.max(1));
    Ok(u64::from(reader.duration()) * 1_000 / rate)
}

/// Was [`cut_wav`] geschrieben hat: Frames und die Abtastrate der Datei.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cut {
    frames: u64,
    sample_rate: u32,
}

impl Cut {
    fn len_ms(self) -> u64 {
        self.frames * 1_000 / u64::from(self.sample_rate.max(1))
    }
}

/// Schneidet `range` aus `src` in die neue Datei `name` im Zuschnittsordner `out`, im
/// Format des Originals (Kanaele, Abtastrate, Bittiefe, Ganzzahl oder Gleitkomma); der
/// Abschnitt wird in der Abtastrate der DATEI gerechnet. Ein Bereich hinter dem Ende
/// ergibt eine gueltige WAV ohne Samples. Die Datei entsteht mit `create_new` ueber das
/// Handle des Ordners: gibt es sie schon, ist das ein Fehler (`wav_write_failed`) und sie
/// bleibt unberuehrt. Eine halb geschriebene Datei raeumt [`folder::NewDir::discard`]
/// ueber dasselbe Handle weg; hier wird nie ein Pfad geloescht.
fn cut_wav(
    src: &Path,
    out: &mut folder::NewDir,
    name: &'static str,
    range: TimeRange,
) -> Result<Cut, String> {
    let mut reader = hound::WavReader::open(src).map_err(|e| format!("wav_unreadable: {e}"))?;
    let sample_rate = reader.spec().sample_rate.max(1);
    let file = out.create_file(name)?;
    let frames = cut_wav_into(&mut reader, std::io::BufWriter::new(file), range)?;
    Ok(Cut {
        frames,
        sample_rate,
    })
}

fn cut_wav_into<R, W>(
    reader: &mut hound::WavReader<R>,
    out: W,
    range: TimeRange,
) -> Result<u64, String>
where
    R: std::io::Read + std::io::Seek,
    W: std::io::Write + std::io::Seek,
{
    let spec = reader.spec();
    let rate = u64::from(spec.sample_rate.max(1));
    let channels = u64::from(spec.channels.max(1));
    let total = u64::from(reader.duration());
    let end = (range.end_ms.saturating_mul(rate) / 1_000).min(total);
    let start = (range.start_ms.saturating_mul(rate) / 1_000).min(end);
    let mut writer =
        hound::WavWriter::new(out, spec).map_err(|e| format!("wav_write_failed: {e}"))?;
    let wanted = (end - start) * channels;
    let mut written = 0u64;
    if wanted > 0 {
        reader
            .seek(start as u32)
            .map_err(|e| format!("wav_unreadable: {e}"))?;
        let take = usize::try_from(wanted).unwrap_or(usize::MAX);
        match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Float, _) => {
                for s in reader.samples::<f32>().take(take) {
                    let s = s.map_err(|e| format!("wav_unreadable: {e}"))?;
                    writer
                        .write_sample(s)
                        .map_err(|e| format!("wav_write_failed: {e}"))?;
                    written += 1;
                }
            }
            (hound::SampleFormat::Int, _) => {
                for s in reader.samples::<i32>().take(take) {
                    let s = s.map_err(|e| format!("wav_unreadable: {e}"))?;
                    writer
                        .write_sample(s)
                        .map_err(|e| format!("wav_write_failed: {e}"))?;
                    written += 1;
                }
            }
        }
    }
    writer
        .finalize()
        .map_err(|e| format!("wav_write_failed: {e}"))?;
    Ok(written / channels)
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Was [`MeetingStore::apply_trim`] schreibt.
pub struct TrimWrite<'a> {
    pub meeting_id: &'a str,
    /// ULID des neuen Ordners `trim-<ULID>` dieses Zuschnitts: sie steht in den Vermerken
    /// der ersetzten Dateien. Die neuen Dateien muessen darin liegen.
    pub trim_id: &'a str,
    /// Die Pfade, die zugeschnitten wurden (stehen sie nicht mehr so da: `trim_changed`).
    pub old_mic: Option<&'a str>,
    pub old_system: Option<&'a str>,
    /// Die entechote Kopie neben der alten `mic.wav`, falls es sie gab (nur fuer den Vermerk).
    pub old_aec: Option<&'a str>,
    pub new_mic: Option<&'a str>,
    pub new_system: Option<&'a str>,
    pub duration_ms: u64,
    /// Der wirksame Bereich (auf die Laenge der Aufnahme geklemmt).
    pub range: TimeRange,
    /// `mic_aec.wav` wurde mitgeschnitten und liegt neben der neuen `mic.wav`
    /// (sonst entfaellt der Verweis in `metadata.aec`).
    pub aec_kept: bool,
}

/// Status, Mikrofon- und Systempfad, `metadata_json` einer Besprechung.
type MeetingRow = (String, Option<String>, Option<String>, Option<String>);

fn io(e: &dyn std::fmt::Display) -> String {
    format!("store_failed: {e}")
}

impl MeetingStore {
    /// Schreibt den Zuschnitt in EINER Transaktion (siehe Modulkopf), auch den Vermerk
    /// der ersetzten alten Audiodateien (`trim_superseded`). Es wird nichts geloescht.
    pub fn apply_trim(&self, w: &TrimWrite<'_>) -> Result<(), String> {
        // Die Herkunft muss stimmen, bevor irgendetwas geschrieben wird: eine gueltige
        // ULID, und die neuen Dateien liegen in genau diesem Ordner.
        let trim_id =
            folder::TrimId::parse(w.trim_id).ok_or_else(|| "store_failed: trim_id".to_string())?;
        let own_dir = trim_id.dir_name();
        let in_own_dir = |new: &str| {
            Path::new(new)
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&own_dir))
        };
        if [w.new_mic, w.new_system]
            .into_iter()
            .flatten()
            .any(|new| !in_own_dir(new))
        {
            return Err("store_failed: trim_id".into());
        }
        let mut conn = self.get_connection().map_err(|e| io(&e))?;
        let now = Utc::now().timestamp();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| io(&e))?;
        let id = w.meeting_id;
        let range = w.range;

        let row: Option<MeetingRow> = tx
            .query_row(
                "SELECT status, mic_audio_path, system_audio_path, metadata_json FROM meetings
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| io(&e))?;
        let Some((status, mic, system, metadata)) = row else {
            return Err("meeting_not_found".into());
        };
        if matches!(status.as_str(), "recording" | "processing" | "queued") {
            return Err("meeting_busy".into());
        }
        if mic.as_deref() != w.old_mic || system.as_deref() != w.old_system {
            return Err("trim_changed".into());
        }
        // Zustand VOR jeder Aenderung: unlesbare Metadaten und eine angehaltene
        // Neu-Transkription verbieten den Zuschnitt (die Transaktion rollt zurueck).
        let mut root = trim_state(&tx, id, metadata)?;
        let superseded: Vec<folder::Superseded> = [
            (folder::Channel::Mic, mic.as_deref()),
            (folder::Channel::System, system.as_deref()),
            (folder::Channel::MicAec, w.old_aec),
        ]
        .into_iter()
        .filter_map(|(channel, path)| {
            Some(folder::Superseded {
                path: path?.to_string(),
                trim_id: trim_id.clone(),
                channel,
                at: now,
            })
        })
        .collect();

        tx.execute(
            "UPDATE meetings SET mic_audio_path = ?1, system_audio_path = ?2, duration_ms = ?3,
                 updated_at = ?4 WHERE id = ?5",
            params![w.new_mic, w.new_system, w.duration_ms as i64, now, id],
        )
        .map_err(|e| io(&e))?;
        let mut merges: Option<MergeRemap> = None;

        // Transkript: Segmente und Turns; Deltas weg; Epoche und Revision + 1.
        let transcript: Option<(String, String, Option<String>, i64)> = tx
            .query_row(
                "SELECT id, segments_json, speaker_hints_json, segment_epoch FROM transcripts
                 WHERE meeting_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| io(&e))?;
        if let Some((tid, segments_json, hints, epoch)) = transcript {
            let segments: Vec<StoredSegment> =
                serde_json::from_str(&segments_json).map_err(|e| io(&e))?;
            let (segments, segment_map) = trim_segments(segments, range);
            let json = serde_json::to_string(&segments).map_err(|e| io(&e))?;
            let (hints, turn_map) = match hints {
                Some(h) => {
                    let (text, map) = trim_hints_mapped(&h, range);
                    (Some(text), map)
                }
                None => (None, TurnMap::new()),
            };
            merges = Some(MergeRemap {
                old_epoch: u32::try_from(epoch.max(0)).unwrap_or(u32::MAX),
                segments: segment_map,
                turns: turn_map,
            });
            tx.execute(
                "DELETE FROM transcript_deltas WHERE transcript_id = ?1",
                params![tid],
            )
            .map_err(|e| io(&e))?;
            tx.execute(
                "UPDATE transcripts SET segments_json = ?1, speaker_hints_json = ?2,
                     content_revision = content_revision + 1,
                     segment_epoch = segment_epoch + 1, updated_at = ?3
                 WHERE id = ?4",
                params![json, hints, now, tid],
            )
            .map_err(|e| io(&e))?;
        }

        // Metadaten: AEC-Verweis, Zusammenfuehrungen und der Vermerk der ersetzten alten
        // Dateien, in derselben Transaktion.
        if trimmed_metadata(&mut root, w.aec_kept, merges.as_ref(), &superseded) {
            tx.execute(
                "UPDATE meetings SET metadata_json = ?1 WHERE id = ?2",
                params![root.to_string(), id],
            )
            .map_err(|e| io(&e))?;
        }

        // Alle Fassungen (auch die geloeschten, falls eine zurueckgeholt wird).
        let variants: Vec<(String, String, Option<String>, Option<String>)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, segments_json, speaker_hints_json, meta_json
                     FROM transcript_variants WHERE meeting_id = ?1",
                )
                .map_err(|e| io(&e))?;
            let rows = stmt
                .query_map(params![id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(|e| io(&e))?;
            rows.collect::<rusqlite::Result<_>>().map_err(|e| io(&e))?
        };
        for (vid, segments_json, hints, meta) in variants {
            let segments: Vec<StoredSegment> =
                serde_json::from_str(&segments_json).map_err(|e| io(&e))?;
            let (segments, map) = trim_segments(segments, range);
            let json = serde_json::to_string(&segments).map_err(|e| io(&e))?;
            let hints = hints.map(|h| trim_hints(&h, range));
            let meta = meta.map(|m| remap_flagged(&m, &map));
            tx.execute(
                "UPDATE transcript_variants SET segments_json = ?1, speaker_hints_json = ?2,
                     meta_json = ?3 WHERE id = ?4",
                params![json, hints, meta, vid],
            )
            .map_err(|e| io(&e))?;
        }

        // Notizen (eine lebende Datei je Besprechung, Revision + 1).
        let notes: Option<String> = tx
            .query_row(
                "SELECT blocks_json FROM meeting_notes WHERE meeting_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| io(&e))?;
        if let Some(blocks_json) = notes {
            let blocks: Vec<NoteBlock> = serde_json::from_str(&blocks_json).map_err(|e| io(&e))?;
            let json = serde_json::to_string(&trim_notes(blocks, range)).map_err(|e| io(&e))?;
            tx.execute(
                "UPDATE meeting_notes SET blocks_json = ?1, revision = revision + 1,
                     updated_at = ?2 WHERE meeting_id = ?3",
                params![json, now, id],
            )
            .map_err(|e| io(&e))?;
        }

        // Teile (Anzeige).
        let parts: Vec<MeetingPart> = {
            let mut stmt = tx
                .prepare(
                    "SELECT part_index, title, source_path, offset_ms, duration_ms
                     FROM meeting_parts WHERE meeting_id = ?1 ORDER BY part_index",
                )
                .map_err(|e| io(&e))?;
            let rows = stmt
                .query_map(params![id], |r| {
                    Ok(MeetingPart {
                        part_index: r.get::<_, i64>(0)?.max(0) as u32,
                        title: r.get(1)?,
                        source_path: r.get(2)?,
                        offset_ms: r.get::<_, i64>(3)?.max(0) as u64,
                        duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
                    })
                })
                .map_err(|e| io(&e))?;
            rows.collect::<rusqlite::Result<_>>().map_err(|e| io(&e))?
        };
        if !parts.is_empty() {
            tx.execute(
                "DELETE FROM meeting_parts WHERE meeting_id = ?1",
                params![id],
            )
            .map_err(|e| io(&e))?;
            for p in trim_parts(parts, range) {
                tx.execute(
                    "INSERT INTO meeting_parts (meeting_id, part_index, title, source_path,
                         offset_ms, duration_ms, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id,
                        i64::from(p.part_index),
                        p.title,
                        p.source_path,
                        p.offset_ms as i64,
                        p.duration_ms as i64,
                        now
                    ],
                )
                .map_err(|e| io(&e))?;
            }
        }

        // Folien: Auftreten verschieben; ohne Auftreten im Bereich weich loeschen.
        let slides: Vec<(String, String)> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id, occurrences_json FROM meeting_slides
                     WHERE meeting_id = ?1 AND deleted_at IS NULL",
                )
                .map_err(|e| io(&e))?;
            let rows = stmt
                .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| io(&e))?;
            rows.collect::<rusqlite::Result<_>>().map_err(|e| io(&e))?
        };
        for (slide_id, json) in slides {
            let list: Vec<SlideOccurrence> = serde_json::from_str(&json).unwrap_or_default();
            let kept = trim_occurrences(list, range);
            if kept.is_empty() {
                tx.execute(
                    "UPDATE meeting_slides SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                    params![now, slide_id],
                )
                .map_err(|e| io(&e))?;
                continue;
            }
            let json = serde_json::to_string(&kept).map_err(|e| io(&e))?;
            tx.execute(
                "UPDATE meeting_slides SET occurrences_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![json, now, slide_id],
            )
            .map_err(|e| io(&e))?;
        }

        // Such-Index der Besprechung: neu aufbauen lassen.
        tx.execute(
            "DELETE FROM meeting_chunks WHERE meeting_id = ?1",
            params![id],
        )
        .map_err(|e| io(&e))?;
        tx.execute(
            "DELETE FROM meeting_index_state WHERE meeting_id = ?1",
            params![id],
        )
        .map_err(|e| io(&e))?;

        tx.commit().map_err(|e| io(&e))?;
        Ok(())
    }

    /// Vorab (vor dem Schneiden der WAVs): dieselben Pruefungen wie in der Transaktion.
    fn trim_precheck(&self, id: &str) -> Result<(), String> {
        let conn = self.get_connection().map_err(|e| io(&e))?;
        let metadata: Option<Option<String>> = conn
            .query_row(
                "SELECT metadata_json FROM meetings WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| io(&e))?;
        let Some(metadata) = metadata else {
            return Err("meeting_not_found".into());
        };
        trim_state(&conn, id, metadata).map(|_| ())
    }
}

/// `metadata_json` einer Besprechung als Objekt, wenn der Zuschnitt erlaubt ist:
/// - kein Eintrag (NULL): leeres Objekt; leer, kaputt oder kein Objekt, oder ein
///   `trim_superseded`, das keine Liste ist: `metadata_unreadable` (sonst gingen Vermerke
///   und Zusammenfuehrungen verloren);
/// - eine angehaltene oder offene Neu-Transkription (Marke `partial_rerun` in
///   irgendeiner Phase, oder eine Arbeitskopie): `rerun_pending`. Die Marke verweist
///   auf Zeiten der Aufnahme, die der Zuschnitt verschiebt.
fn trim_state(conn: &Connection, id: &str, metadata: Option<String>) -> Result<Value, String> {
    let root = match metadata.as_deref() {
        None => Value::Object(serde_json::Map::new()),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => v,
            _ => return Err("metadata_unreadable".into()),
        },
    };
    if !folder::superseded_readable(&root) {
        return Err("metadata_unreadable".into());
    }
    let marker_open = root
        .get(super::rerun_state::KEY)
        .is_some_and(|m| !m.is_null());
    if marker_open || super::variants::rerun_pending(conn, id).map_err(|e| io(&e.code()))? {
        return Err("rerun_pending".into());
    }
    Ok(root)
}

// ---------------------------------------------------------------------------
// Ablauf
// ---------------------------------------------------------------------------

/// Ergebnis von [`trim_meeting`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TrimResult {
    /// Neue Dauer der Aufnahme in ms.
    pub duration_ms: u64,
}

/// Vorsilbe der Ordner mit zugeschnittenen Aufnahmen (`trim-<ulid>/`).
const TRIM_DIR_PREFIX: &str = "trim-";

/// Feste Dateinamen der neuen Spuren im Ordner `trim-<ULID>`: Mikrofon, Systemton. Nie
/// aus den alten Pfaden abgeleitet (siehe Modulkopf).
const NEW_TRACK_NAMES: [&str; 2] = ["mic.wav", "system.wav"];

/// Eine Spur zum Zuschneiden: ihr Pfad, ihre Laenge und (nur Mikrofon, wenn
/// vorhanden) die entechote Kopie daneben.
struct TrackPlan {
    path: PathBuf,
    len_ms: u64,
    aec: Option<PathBuf>,
}

/// Was in den neuen Ordner geschrieben wurde.
struct Written {
    /// Name der neuen Datei je Spur (Mikrofon, Systemton); `None`: die Spur gab es nicht.
    names: [Option<&'static str>; 2],
    /// `mic_aec.wav` wurde mitgeschnitten.
    aec_kept: bool,
    /// Laenge der laengsten neuen Spur in ms.
    len_ms: u64,
}

/// Schneidet jede Spur und die `mic_aec.wav` in den neuen Ordner (feste Namen, `create_new`
/// ueber das Handle). Bei einem Fehler raeumt der Aufrufer ueber dasselbe Handle auf.
fn write_tracks(
    out: &mut folder::NewDir,
    plans: &[Option<TrackPlan>],
    range: TimeRange,
) -> Result<Written, String> {
    let mut written = Written {
        names: [None, None],
        aec_kept: false,
        len_ms: 0,
    };
    for (i, (plan, name)) in plans.iter().zip(NEW_TRACK_NAMES).enumerate() {
        let Some(plan) = plan else {
            continue;
        };
        let cut = cut_wav(&plan.path, out, name, range)?;
        written.len_ms = written.len_ms.max(cut.len_ms());
        written.names[i] = Some(name);
        if let Some(aec) = &plan.aec {
            match cut_wav(aec, out, super::MIC_AEC_FILE, range) {
                Ok(_) => written.aec_kept = true,
                Err(e) => {
                    // Keine halbe Kopie neben der neuen mic.wav liegen lassen (die
                    // Suche nach `derived_audio_paths` fande sie ueber den Namen).
                    out.remove_created(super::MIC_AEC_FILE);
                    warn!("meetings: mic_aec.wav not cut ({e}); the copy is dropped");
                }
            }
        }
    }
    Ok(written)
}

/// Schneidet die Aufnahme der Besprechung auf `range` zu (B, siehe Modulkopf).
/// Prueft, schreibt die neuen WAVs in einen frischen Ordner
/// `<Besprechungsordner>/trim-<ulid>/` (feste Namen `mic.wav`/`system.wav`,
/// `mic_aec.wav` mit demselben Bereich geschnitten, damit `final_pass` sie wieder
/// neben der `mic.wav` findet), dann EINE Transaktion. Die alten Dateien bleiben
/// unangetastet liegen und stehen als `trim_superseded` in den Metadaten. Den Auftrag
/// (Sperre gegen andere Verarbeitungen) haelt der Aufrufer und weist ihn mit `lock` nach.
///
/// Fehlercodes: `meeting_not_found`, `meeting_busy`, `no_audio`, `audio_missing`,
/// `invalid_range`, `nothing_to_trim`, `trim_changed`, `metadata_unreadable`,
/// `rerun_pending`, `wav_unreadable`, `wav_write_failed`, `store_failed`.
pub fn trim_meeting(
    store: &MeetingStore,
    meeting_id: &str,
    range: TimeRange,
    _lock: &JobGuard,
) -> Result<TrimResult, String> {
    let range = range.validate()?;
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| io(&e))?
        .filter(|m| m.deleted_at.is_none())
        .ok_or_else(|| "meeting_not_found".to_string())?;
    if matches!(
        meeting.status.as_str(),
        "recording" | "processing" | "queued"
    ) {
        return Err("meeting_busy".into());
    }
    // Kein Schneiden auf Verdacht: dieselben Pruefungen wie in der Transaktion.
    store.trim_precheck(meeting_id)?;
    let tracks: Vec<Option<String>> = vec![
        meeting.mic_audio_path.clone(),
        meeting.system_audio_path.clone(),
    ];
    if tracks.iter().all(Option::is_none) {
        return Err("no_audio".into());
    }
    let mut plans: Vec<Option<TrackPlan>> = Vec::with_capacity(2);
    for (path, fallback) in tracks.iter().zip(NEW_TRACK_NAMES) {
        let Some(path) = path else {
            plans.push(None);
            continue;
        };
        let path = PathBuf::from(path);
        if !path.exists() {
            return Err("audio_missing".into());
        }
        let len_ms = wav_len_ms(&path)?;
        let aec = (fallback == "mic.wav")
            .then(|| {
                super::derived_audio_paths(&path.to_string_lossy())
                    .into_iter()
                    .map(PathBuf::from)
                    .find(|p| p.is_file())
            })
            .flatten();
        plans.push(Some(TrackPlan { path, len_ms, aec }));
    }
    let longest = plans.iter().flatten().map(|p| p.len_ms).max().unwrap_or(0);
    if range.start_ms >= longest {
        return Err("invalid_range".into());
    }
    let range = TimeRange {
        start_ms: range.start_ms,
        end_ms: range.end_ms.min(longest),
    };
    if range.start_ms == 0 && range.end_ms >= longest {
        return Err("nothing_to_trim".into());
    }
    let old_aec = plans
        .first()
        .and_then(Option::as_ref)
        .and_then(|plan| plan.aec.as_deref())
        .map(|p| p.to_string_lossy().to_string());

    // Der Ordner kommt aus Datenwurzel plus ID, nie aus den gespeicherten Pfaden, und
    // muss ein echtes Verzeichnis sein. Den neuen Ordner legt das Handle an, die Dateien
    // darin entstehen ueber dessen Handle, und bei einem Fehler raeumt dasselbe Handle
    // auf: ein ausgetauschter Vorfahre oder Ordner aendert daran nichts.
    let home = folder::open_folder(store, meeting_id).map_err(|why| {
        warn!("meetings: trim: folder of {meeting_id} not usable ({why:?})");
        "audio_missing".to_string()
    })?;
    let trim_id = folder::TrimId::parse(&Ulid::new().to_string())
        .ok_or_else(|| "store_failed: trim_id".to_string())?;
    let dir_name = trim_id.dir_name();
    let new_dir = home.path.join(&dir_name);
    let mut out = folder::NewDir::create(&home.dir, &dir_name)?;
    let written = match write_tracks(&mut out, &plans, range) {
        Ok(written) => written,
        Err(e) => {
            out.discard(&home.dir);
            return Err(e);
        }
    };

    let path_of = |name: Option<&str>| name.map(|n| new_dir.join(n).to_string_lossy().to_string());
    let (new_mic, new_system) = (path_of(written.names[0]), path_of(written.names[1]));
    let applied = store.apply_trim(&TrimWrite {
        meeting_id,
        trim_id: trim_id.as_str(),
        old_mic: meeting.mic_audio_path.as_deref(),
        old_system: meeting.system_audio_path.as_deref(),
        old_aec: old_aec.as_deref(),
        new_mic: new_mic.as_deref(),
        new_system: new_system.as_deref(),
        duration_ms: written.len_ms,
        range,
        aec_kept: written.aec_kept,
    });
    match applied {
        Ok(()) => {
            info!(
                "meetings: {meeting_id} trimmed to {}..{} ms ({} ms left, old files kept as backup)",
                range.start_ms, range.end_ms, written.len_ms
            );
            Ok(TrimResult {
                duration_ms: written.len_ms,
            })
        }
        Err(e) => {
            out.discard(&home.dir);
            Err(e)
        }
    }
}

/// Zuschneiden in der App: Auftrag (blockt andere Verarbeitungen der Besprechung),
/// Such-Index, Ereignisse fuer die Oberflaeche.
pub fn trim_now(
    app: &tauri::AppHandle,
    store: &Arc<MeetingStore>,
    meeting_id: &str,
    range: TimeRange,
) -> Result<TrimResult, String> {
    use super::job::{self, JobPhase};
    use tauri_specta::Event;

    if job::global().is_running(meeting_id) {
        return Err("meeting_busy".into());
    }
    let guard = job::global()
        .try_start(meeting_id, job::app_emit(app))
        .map_err(|_| "meeting_busy".to_string())?;
    guard.handle().begin_phase(JobPhase::Prepare, 0);
    let trimmed = trim_meeting(store, meeting_id, range, &guard);
    drop(guard);
    let result = trimmed?;
    super::search::indexer::submit(
        app,
        super::search::indexer::IndexJob::Meeting(meeting_id.to_string()),
    );
    let _ = (super::recorder::MeetingEvent::Reset {
        meeting_id: meeting_id.to_string(),
    })
    .emit(app);
    match store.get_meeting(meeting_id) {
        Ok(Some(m)) => {
            let _ = (super::recorder::MeetingEvent::State {
                meeting_id: meeting_id.to_string(),
                status: m.status,
                paused: false,
            })
            .emit(app);
        }
        Ok(None) => {}
        Err(e) => warn!("meetings: state after trim not read ({meeting_id}): {e}"),
    }
    Ok(result)
}
