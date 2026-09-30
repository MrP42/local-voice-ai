//! Untertitel eines YouTube-Videos ueber das selbst installierte `yt-dlp` (A3).
//!
//! Nur mit Schalter „privat“ und gefundenem Programm (`fetch::Access`); hier steht
//! die Mechanik:
//! - `list_tracks`: `--list-subs` (kein Download), die Tabelle wird gelesen;
//!   manuelle Spuren vor automatischen, bei automatischen nur die Originalsprache
//!   (sonst waeren es ~150 Uebersetzungen; Form der Liste siehe `parse_list_subs`).
//! - `fetch_track`: `--skip-download --write-subs|--write-auto-subs --sub-format vtt`
//!   in einen Temp-Ordner, die VTT-Datei wird bereinigt und mit dem Parser des
//!   Datei-Imports (`meetings::subtitle`) gelesen.
//!
//! YouTube-VTT ist kein sauberes VTT: automatische Untertitel wiederholen in jedem
//! Cue die letzte Zeile des vorigen (Rolltext), enthalten Wort-Zeitmarken
//! (`<00:00:01.040><c> wort</c>`), Entities und Zeilen nur aus Leerzeichen, die den
//! Parser aus dem Tritt braechten. `normalize_vtt` raeumt davor auf.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::fetch::{TempDir, ToolRun};
use super::run::{self, RunEnd, RunOpts};
use super::YoutubeError;
use crate::managers::meetings::store::StoredSegment;
use crate::managers::meetings::subtitle::parse_subtitles;

#[cfg(test)]
mod tests;

/// Zeitlimit fuer `--list-subs` (ein Seitenabruf) und fuer das Laden einer Spur.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(90);
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Groesste gelesene Tabelle (Byte).
const MAX_LIST_BYTES: usize = 256 * 1024;
/// Groesste gelesene Untertiteldatei (Byte): eine Stunde Text sind < 1 MB.
const MAX_VTT_BYTES: u64 = 8 * 1024 * 1024;
/// Hoechstzahl angezeigter Spuren.
const MAX_TRACKS: usize = 40;

/// Eine verfuegbare Untertitelspur.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SubtitleTrack {
    /// Sprachcode wie von yt-dlp genannt (`de`, `en`, `de-orig`).
    pub language: String,
    pub name: String,
    /// Automatisch erzeugt (Spracherkennung von YouTube), nicht hochgeladen.
    pub auto: bool,
    /// Die Spur, die ohne Nachfrage genommen wird.
    pub recommended: bool,
}

/// Ein Sprachcode darf nur aus diesen Zeichen bestehen (er geht als Wert von
/// `--sub-langs`, das ein regulaerer Ausdruck ist).
pub fn valid_language(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 35
        && !code.starts_with('-')
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn list_args(video_id: &str) -> Vec<String> {
    let mut a = base_args();
    a.extend(["--skip-download".into(), "--list-subs".into()]);
    a.extend(["--".into(), watch_url(video_id)]);
    a
}

pub fn fetch_args(video_id: &str, language: &str, auto: bool) -> Vec<String> {
    let mut a = base_args();
    a.extend([
        "--skip-download".into(),
        if auto {
            "--write-auto-subs"
        } else {
            "--write-subs"
        }
        .into(),
        "--sub-langs".into(),
        language.to_string(),
        "--sub-format".into(),
        "vtt".into(),
        "--restrict-filenames".into(),
        "--no-mtime".into(),
        "-o".into(),
        "%(id)s.%(ext)s".into(),
    ]);
    a.extend(["--".into(), watch_url(video_id)]);
    a
}

/// Gemeinsame Argumente jedes Laufs: keine Konfigdateien, nie eine Playlist.
pub fn base_args() -> Vec<String> {
    vec![
        "--ignore-config".into(),
        "--no-playlist".into(),
        "--no-warnings".into(),
    ]
}

/// Die Adresse baut dieses Modul aus der geprueften Video-ID (nie aus Nutzertext).
pub fn watch_url(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={video_id}")
}

// ---------------------------------------------------------------------------
// Tabelle von --list-subs
// ---------------------------------------------------------------------------

/// Liest die Tabelle. Bekannte Form:
/// ```text
/// [info] Available automatic captions for ID:
/// Language Name    Formats
/// de-orig  German (Original) vtt, ttml
/// [info] Available subtitles for ID:
/// Language Name    Formats
/// en       English vtt
/// ```
/// Unbekannte Zeilen werden uebergangen, nie als Spur gewertet.
pub fn parse_list_subs(output: &str) -> Vec<SubtitleTrack> {
    #[derive(PartialEq, Clone, Copy)]
    enum Section {
        None,
        Auto,
        Manual,
    }
    let code = Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{0,34}$").expect("static regex");
    let mut section = Section::None;
    let (mut manual, mut auto) = (Vec::new(), Vec::new());
    for raw in output.lines() {
        let line = raw.trim_end();
        let lower = line.to_ascii_lowercase();
        if lower.contains("available automatic captions") {
            section = Section::Auto;
            continue;
        }
        if lower.contains("available subtitles") {
            section = Section::Manual;
            continue;
        }
        if lower.contains("has no automatic captions") || lower.contains("has no subtitles") {
            section = Section::None;
            continue;
        }
        if section == Section::None || line.starts_with('[') || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else { continue };
        if first == "Language" || !code.is_match(first) {
            continue;
        }
        // Der Rest: Name, dann die Formate (letzte Spalte, kommagetrennt).
        let rest: Vec<&str> = parts.collect();
        let name_words: Vec<&str> = rest
            .iter()
            .copied()
            .take_while(|w| !w.ends_with(',') && !is_format(w))
            .collect();
        let name = name_words.join(" ");
        let track = SubtitleTrack {
            language: first.to_string(),
            name: if name.is_empty() {
                first.to_string()
            } else {
                name
            },
            auto: section == Section::Auto,
            recommended: false,
        };
        match section {
            Section::Auto => auto.push(track),
            _ => manual.push(track),
        }
    }
    // Automatisch: nur die Originalsprache. Aeltere yt-dlp nennen sie `xx-orig`;
    // heutige zeigen sie als `xx` ohne Namen und haengen ~150 Uebersetzungen an
    // (`fr-en`, Name „French from English“). Uebersetzungen bleiben nur, wenn es
    // sonst nichts gibt.
    if auto.iter().any(|t| t.language.ends_with("-orig")) {
        auto.retain(|t| t.language.ends_with("-orig"));
    }
    let translation = |t: &SubtitleTrack| t.name.contains(" from ");
    if auto.iter().any(|t| !translation(t)) {
        auto.retain(|t| !translation(t));
    }
    let mut tracks: Vec<SubtitleTrack> = manual.into_iter().chain(auto).collect();
    tracks.truncate(MAX_TRACKS);
    if let Some(first) = tracks.first_mut() {
        first.recommended = true;
    }
    tracks
}

fn is_format(word: &str) -> bool {
    matches!(
        word,
        "vtt" | "ttml" | "srv1" | "srv2" | "srv3" | "json3" | "srt" | "ass" | "lrc"
    )
}

// ---------------------------------------------------------------------------
// VTT aufraeumen
// ---------------------------------------------------------------------------

fn timecode() -> Regex {
    Regex::new(
        r"^(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})\s*-->\s*(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})",
    )
    .expect("static regex")
}

fn stamp(h: Option<&str>, m: &str, s: &str, ms: &str) -> String {
    let h: u32 = h.and_then(|h| h.parse().ok()).unwrap_or(0);
    format!("{h:02}:{m}:{s}.{ms}")
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Macht aus YouTube-VTT ein Text, den `parse_subtitles` richtig liest.
/// `auto`: Rolltext entfernen (eine Zeile, die schon im vorigen Cue stand, faellt weg).
pub fn normalize_vtt(raw: &str, auto: bool) -> String {
    let time = timecode();
    let tags = Regex::new(r"<[^>]*>").expect("static regex");
    struct Cue {
        start: String,
        end: String,
        lines: Vec<String>,
    }
    let mut cues: Vec<Cue> = Vec::new();
    let mut open = false;
    let all: Vec<&str> = raw.lines().map(|l| l.trim_end_matches('\r')).collect();
    for (at, line) in all.iter().copied().enumerate() {
        if let Some(c) = time.captures(line.trim()) {
            cues.push(Cue {
                start: stamp(c.get(1).map(|m| m.as_str()), &c[2], &c[3], &c[4]),
                end: stamp(c.get(5).map(|m| m.as_str()), &c[6], &c[7], &c[8]),
                lines: Vec::new(),
            });
            open = true;
            continue;
        }
        if line.is_empty() {
            // Ein Cue endet an einer Leerzeile, nach der ein neuer Cue (oder das
            // Ende) kommt. Steht dahinter noch Text, war es nur eine Luecke im
            // Cue (manche Dateien verlieren die Zeile aus einem Leerzeichen).
            let next = all[at + 1..].iter().find(|l| !l.trim().is_empty());
            if next.is_none_or(|l| time.is_match(l.trim())) {
                open = false;
            }
            continue;
        }
        if !open {
            continue;
        }
        let cleaned = decode_entities(&tags.replace_all(line, ""));
        let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
        if !cleaned.is_empty() {
            if let Some(cue) = cues.last_mut() {
                cue.lines.push(cleaned);
            }
        }
    }
    let mut out = String::from("WEBVTT\n\n");
    let mut previous: Vec<String> = Vec::new();
    for cue in cues {
        let lines: Vec<String> = if auto {
            cue.lines
                .iter()
                .filter(|l| !previous.contains(l))
                .cloned()
                .collect()
        } else {
            cue.lines.clone()
        };
        previous = cue.lines;
        if lines.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            cue.start,
            cue.end,
            lines.join("\n")
        ));
    }
    out
}

/// YouTube-VTT -> Segmente. Fehler: keine Zeile Text (Datei leer oder kein VTT).
pub fn parse_youtube_vtt(raw: &str, auto: bool) -> Result<Vec<StoredSegment>, String> {
    let normalized = normalize_vtt(raw, auto);
    let mut segments = parse_subtitles(&normalized)?;
    segments.retain(|s| !s.text.trim().is_empty());
    for (i, s) in segments.iter_mut().enumerate() {
        s.segment_index = u32::try_from(i).unwrap_or(u32::MAX);
    }
    if segments.is_empty() {
        return Err("Die Untertitel enthalten keinen Text.".to_string());
    }
    Ok(segments)
}

// ---------------------------------------------------------------------------
// Laufen lassen
// ---------------------------------------------------------------------------

/// Die verfuegbaren Spuren (ein Seitenabruf ueber yt-dlp, kein Download).
pub fn list_tracks(
    tool: &ToolRun<'_>,
    video_id: &str,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SubtitleTrack>, YoutubeError> {
    let tmp = TempDir::create(tool.temp_base)?;
    let end = run::run(
        tool.exe,
        &list_args(video_id),
        &RunOpts {
            timeout: LIST_TIMEOUT,
            max_stdout: MAX_LIST_BYTES,
            cancel,
            cwd: Some(tmp.path()),
        },
    )
    .map_err(YoutubeError::ToolStart)?;
    match end {
        RunEnd::Finished {
            code: 0, stdout, ..
        } => Ok(parse_list_subs(&String::from_utf8_lossy(&stdout))),
        other => Err(super::fetch::classify(other)),
    }
}

/// Laedt eine Spur und liest sie. Der Temp-Ordner verschwindet in jedem Fall.
pub fn fetch_track(
    tool: &ToolRun<'_>,
    video_id: &str,
    track: &SubtitleTrack,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<StoredSegment>, YoutubeError> {
    if !valid_language(&track.language) {
        return Err(YoutubeError::BadResponse("Sprachcode".to_string()));
    }
    let tmp = TempDir::create(tool.temp_base)?;
    let end = run::run(
        tool.exe,
        &fetch_args(video_id, &track.language, track.auto),
        &RunOpts {
            timeout: FETCH_TIMEOUT,
            max_stdout: 4096,
            cancel,
            cwd: Some(tmp.path()),
        },
    )
    .map_err(YoutubeError::ToolStart)?;
    match end {
        RunEnd::Finished { code: 0, .. } => {}
        other => return Err(super::fetch::classify(other)),
    }
    let file = find_vtt(tmp.path()).ok_or(YoutubeError::NoSubtitles)?;
    let len = std::fs::metadata(&file)
        .map_err(|e| YoutubeError::Store(e.to_string()))?
        .len();
    if len > MAX_VTT_BYTES {
        return Err(YoutubeError::BadResponse(
            "Untertiteldatei zu gross".to_string(),
        ));
    }
    let raw = std::fs::read_to_string(&file).map_err(|e| YoutubeError::Store(e.to_string()))?;
    parse_youtube_vtt(&raw, track.auto).map_err(YoutubeError::BadResponse)
}

fn find_vtt(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("vtt"))
        })
        .max_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
}
