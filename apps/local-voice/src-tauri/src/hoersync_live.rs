//! Hörbuch-Sync Stufe 2: Live-Abgleich (Patrick 03.11.2026).
//!
//! Das Frontend fragt beim Hören Fenster VOR der Abspielstelle an
//! ([`audiosync_window`]): der Bereich wird per ffmpeg dekodiert, mit dem
//! Besprechungsmodell erkannt und über [`hoersync`] gegen den Buchtext gelegt.
//! Gespeichert wird je Buch nur das Ergebnis (`sync-live.json`: Stützpunkte,
//! Einschübe, abgedeckte Zeiten), nie das Transkript. Ein zweites Hören
//! braucht so keine Erkennung mehr.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::pages::write_atomic;
use crate::library::{tracks_of, AudioTrack, LibraryBook, SyncPoint};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::transcription::{TimedSegment, TranscriptionManager};

/// Abtastrate für die Erkennung.
const RATE: u32 = 16_000;
/// Längstes Fenster je Aufruf (die Erkennung hält sonst zu lange die Sperre).
const MAX_WINDOW_MS: f64 = 60_000.0;
/// Kürzer lohnt die Erkennung nicht (und ein leeres Fenster würde endlos wiederholt).
const MIN_WINDOW_MS: f64 = 1_000.0;
/// Suchfenster um den Hinweis (Buchwörter).
const HINT_WINDOW: usize = 4_000;
/// Unter dieser Abdeckung gilt ein Fenster mit Hinweis als verfehlt: dann
/// wird das ganze Buch durchsucht (nach Sprung oder Kürzung im Hörbuch).
const MIN_COVERAGE: f32 = 0.35;
/// Ab dieser Abdeckung gilt ein Treffer ohne Hinweis als verlässlich.
const GLOBAL_COVERAGE: f32 = 0.6;

/// Gehörter Text ohne Gegenstück im Buch (vor Zeichen `at`).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct AudioSyncInsert {
    pub at: u32,
    pub start_ms: f64,
    pub end_ms: f64,
    pub text: String,
}

/// Abgedeckter Zeitraum des Hörbuchs (ms, global über alle Teile).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct AudioSyncSpan {
    pub from_ms: f64,
    pub to_ms: f64,
}

/// Inhalt von `sync-live.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct AudioSyncCache {
    /// Fingerabdruck des Buchtexts; passt er nicht, ist der Cache verworfen.
    pub text_key: String,
    pub points: Vec<SyncPoint>,
    pub inserts: Vec<AudioSyncInsert>,
    pub covered: Vec<AudioSyncSpan>,
}

/// Ergebnis eines Fensters.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct AudioSyncWindow {
    pub points: Vec<SyncPoint>,
    pub inserts: Vec<AudioSyncInsert>,
    /// Anteil der gehörten Wörter mit Treffer (0..1).
    pub coverage: f32,
    /// Hinweis für das nächste Fenster (Buchwort-Index nach dem letzten Treffer).
    pub next_hint: Option<u32>,
    pub from_ms: f64,
    pub to_ms: f64,
}

// ---------------------------------------------------------------------------
// Reine Helfer (ohne App, testbar)
// ---------------------------------------------------------------------------

/// Fingerabdruck des Buchtexts (Länge in UTF-16 + FNV-1a), Schlüssel des Caches.
pub fn text_key(text: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    let mut len = 0u32;
    for u in text.encode_utf16() {
        h ^= u32::from(u);
        h = h.wrapping_mul(0x0100_0193);
        len += 1;
    }
    format!("{len}:{h:x}")
}

/// Teil und Stelle darin für eine globale Zeit. `None`, wenn eine Dauer
/// davor unbekannt ist (dann lässt sich die Stelle nicht finden).
pub fn locate(tracks: &[AudioTrack], global_ms: f64) -> Option<(usize, f64)> {
    let mut start = 0.0;
    for (i, t) in tracks.iter().enumerate() {
        let d = t.duration_ms?;
        if global_ms < start + d || i + 1 == tracks.len() {
            return Some((i, (global_ms - start).max(0.0)));
        }
        start += d;
    }
    None
}

/// Text aus dem Seitenstand (`state.json` der Vorlesen-Seite).
pub fn text_of_state(state: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(state).ok()?;
    v.get("text")?.as_str().map(str::to_owned)
}

/// Erkennungssegmente → gehörte Wörter (globale ms). Wortzeiten des Modells
/// haben Vorrang; sonst wird die Segmentzeit nach Zeichen verteilt.
pub fn heard_of(segments: &[TimedSegment], offset_ms: f64) -> Vec<hoersync::HeardWord> {
    let mut out = Vec::new();
    for seg in segments {
        match &seg.words {
            Some(words) if !words.is_empty() => {
                for w in words {
                    let norm = hoersync::normalize(&w.text);
                    if norm.is_empty() {
                        continue;
                    }
                    out.push(hoersync::HeardWord {
                        text: w.text.trim().to_string(),
                        norm,
                        start_ms: offset_ms + w.start_ms as f64,
                        end_ms: offset_ms + w.end_ms as f64,
                    });
                }
            }
            _ => out.extend(hoersync::heard_words(&[hoersync::Segment {
                text: seg.text.clone(),
                start_ms: offset_ms + seg.start_ms as f64,
                end_ms: offset_ms + seg.end_ms as f64,
            }])),
        }
    }
    out
}

/// Fenster in den Cache übernehmen: alte Punkte und Einschübe im Zeitraum
/// des Fensters ersetzen, Abdeckung zusammenfassen, alles monoton halten.
pub fn merge(cache: &mut AudioSyncCache, w: &AudioSyncWindow) {
    let inside = |ms: f64| ms >= w.from_ms && ms < w.to_ms;
    cache.points.retain(|p| !inside(p.ms));
    cache.points.extend(w.points.iter().cloned());
    cache.points = monotone_chain(std::mem::take(&mut cache.points));
    cache.inserts.retain(|i| !inside(i.start_ms));
    cache.inserts.extend(w.inserts.iter().cloned());
    cache
        .inserts
        .sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    cache.covered.push(AudioSyncSpan {
        from_ms: w.from_ms,
        to_ms: w.to_ms,
    });
    cache
        .covered
        .sort_by(|a, b| a.from_ms.total_cmp(&b.from_ms));
    let mut spans: Vec<AudioSyncSpan> = Vec::new();
    for s in cache.covered.drain(..) {
        match spans.last_mut() {
            Some(l) if s.from_ms <= l.to_ms + 1.0 => l.to_ms = l.to_ms.max(s.to_ms),
            _ => spans.push(s),
        }
    }
    cache.covered = spans;
}

/// Längste in Zeit und Zeichen streng steigende Kette (O(n log n)). Ein
/// einzelner Ausreißer kostet so nur sich selbst, nicht alle Punkte danach.
pub fn monotone_chain(mut pts: Vec<SyncPoint>) -> Vec<SyncPoint> {
    pts.retain(|p| p.ms.is_finite());
    // Gleiche Zeit: absteigend in Zeichen, damit höchstens einer davon zählt.
    pts.sort_by(|a, b| a.ms.total_cmp(&b.ms).then(b.chars.cmp(&a.chars)));
    let n = pts.len();
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let c = pts[i].chars;
        let k = tails.partition_point(|&t| pts[t].chars < c);
        prev[i] = if k > 0 { Some(tails[k - 1]) } else { None };
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut idx = Vec::with_capacity(tails.len());
    let mut cur = tails.last().copied();
    while let Some(i) = cur {
        idx.push(i);
        cur = prev[i];
    }
    idx.reverse();
    idx.into_iter().map(|i| pts[i].clone()).collect()
}

/// Buchwort-Index zur Zeichenstelle (für den Hinweis aus der Schätzung).
pub fn word_at(words: &[hoersync::BookWord], chars: u32) -> usize {
    words.partition_point(|w| w.start < chars)
}

/// Ein Fenster gegen den Buchtext legen (ohne Erkennung, für Tests).
pub fn align_window(
    words: &[hoersync::BookWord],
    heard: &[hoersync::HeardWord],
    hint: Option<u32>,
    from_ms: f64,
    to_ms: f64,
) -> AudioSyncWindow {
    let mut a = hoersync::align(words, heard, hint.map(|h| h as usize), HINT_WINDOW);
    if hint.is_some() && a.coverage < MIN_COVERAGE {
        let global = hoersync::align(words, heard, None, 0);
        // Nur übernehmen, wenn die Fundstelle eindeutig ist: ein schwacher
        // globaler Treffer darf die Kette nicht ans andere Buchende werfen.
        if global.coverage >= GLOBAL_COVERAGE && global.coverage > a.coverage {
            a = global;
        }
    }
    AudioSyncWindow {
        points: a
            .points
            .iter()
            .map(|p| SyncPoint {
                chars: p.chars,
                ms: p.ms,
            })
            .collect(),
        inserts: a
            .inserts
            .iter()
            .map(|i| AudioSyncInsert {
                at: i.at,
                start_ms: i.start_ms,
                end_ms: i.end_ms,
                text: i.text.clone(),
            })
            .collect(),
        coverage: a.coverage,
        next_hint: a.book_span.map(|(_, b)| (b + 1) as u32),
        from_ms,
        to_ms,
    }
}

// ---------------------------------------------------------------------------
// App-Anbindung
// ---------------------------------------------------------------------------

/// Buchwörter je Text zwischenspeichern (der Text ändert sich selten).
static WORDS: Mutex<Option<(String, Arc<Vec<hoersync::BookWord>>)>> = Mutex::new(None);
/// Ein Fenster zur Zeit (die Erkennung ist ohnehin seriell).
static RUNNING: Mutex<()> = Mutex::new(());

fn words_for(text: &str) -> Arc<Vec<hoersync::BookWord>> {
    let key = text_key(text);
    let mut slot = WORDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, w)) = slot.as_ref() {
        if *k == key {
            return w.clone();
        }
    }
    let w = Arc::new(hoersync::book_words(text));
    *slot = Some((key, w.clone()));
    w
}

fn cache_path(dir: &Path) -> PathBuf {
    dir.join("sync-live.json")
}

fn load_cache(dir: &Path, key: &str) -> AudioSyncCache {
    std::fs::read_to_string(cache_path(dir))
        .ok()
        .and_then(|raw| serde_json::from_str::<AudioSyncCache>(&raw).ok())
        .filter(|c| c.text_key == key)
        .unwrap_or_else(|| AudioSyncCache {
            text_key: key.to_string(),
            ..Default::default()
        })
}

struct Ctx {
    dir: PathBuf,
    book: LibraryBook,
    text: String,
}

fn context(app: &AppHandle, book_id: &str) -> Result<Ctx, String> {
    let root = crate::library::sync_root(app)?;
    let dir = crate::library::sync_book_dir(&root, book_id)?;
    let book = crate::library::sync_read_book(&dir).ok_or("Buch nicht gefunden")?;
    let page = book
        .text
        .as_ref()
        .ok_or("Das Buch hat keinen Text")?
        .page_id
        .clone();
    let state = crate::commands::pages::page_state_load(app.clone(), page)?;
    let text = text_of_state(&state).ok_or("Kein Buchtext gespeichert")?;
    Ok(Ctx { dir, book, text })
}

/// Gespeicherter Abgleich des Buchs (leer, wenn der Text sich geändert hat).
#[tauri::command]
#[specta::specta]
pub async fn audiosync_load(app: AppHandle, book_id: String) -> Result<AudioSyncCache, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let ctx = context(&app, &book_id)?;
        Ok(load_cache(&ctx.dir, &text_key(&ctx.text)))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ein Fenster des Hörbuchs erkennen und gegen den Text legen.
///
/// `from_ms` und `len_ms` gelten global über alle Teile; ein Fenster endet
/// am Ende seines Teils. `hint`: erwartete Zeichenstelle (aus dem letzten
/// Fenster oder der Schätzung); ohne Hinweis wird das ganze Buch durchsucht.
#[tauri::command]
#[specta::specta]
pub async fn audiosync_window(
    app: AppHandle,
    book_id: String,
    from_ms: f64,
    len_ms: f64,
    hint_chars: Option<u32>,
) -> Result<AudioSyncWindow, String> {
    tauri::async_runtime::spawn_blocking(move || {
        window_blocking(&app, &book_id, from_ms, len_ms, hint_chars)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn window_blocking(
    app: &AppHandle,
    book_id: &str,
    from_ms: f64,
    len_ms: f64,
    hint_chars: Option<u32>,
) -> Result<AudioSyncWindow, String> {
    let _one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let ctx = context(app, book_id)?;
    let audio = ctx.book.audio.as_ref().ok_or("Das Buch hat kein Hörbuch")?;
    let tracks = tracks_of(audio);
    let (ti, local) =
        locate(&tracks, from_ms.max(0.0)).ok_or("Dauer der Hörbuchteile unbekannt")?;
    let track = &tracks[ti];
    if !crate::library::sync_plain_name(&track.file) {
        return Err("Ungültiger Dateiname".into());
    }
    let len = len_ms.clamp(5_000.0, MAX_WINDOW_MS);
    let end_local = track
        .duration_ms
        .map_or(local + len, |d| (local + len).min(d));
    if end_local - local < MIN_WINDOW_MS {
        return Err("Fenster leer (Ende des Hörbuchteils)".into());
    }
    let to_ms = from_ms + (end_local - local);
    let src = ctx.dir.join("audio").join(&track.file);
    let tmp = TempWav::new(book_id);
    let cancel = AtomicBool::new(false);
    crate::media::decode_media_range_to_wav_cancellable(
        &src,
        &tmp.0,
        RATE,
        Some((local as u64, Some(end_local as u64))),
        &cancel,
    )?;
    let samples = read_wav(&tmp.0)?;
    drop(tmp);

    // Diktat oder Aufnahme hat Vorrang: das Erkennungsmodell nicht wegnehmen.
    if recording(app) {
        return Err("Aufnahme läuft".into());
    }
    let tm = app.state::<Arc<TranscriptionManager>>();
    let settings = crate::settings::get_settings(app);
    tm.initiate_meeting_model_load(&settings);
    let segments = tm.transcribe_segments(samples);
    // Danach wieder das Diktatmodell laden (wie die Nachbearbeitung).
    tm.initiate_model_load_target(&settings.selected_model);
    let segments = segments.map_err(|e| e.to_string())?;
    let heard = heard_of(&segments, from_ms);

    let words = words_for(&ctx.text);
    let hint = hint_chars.map(|c| word_at(&words, c) as u32);
    let mut w = align_window(&words, &heard, hint, from_ms, to_ms);
    // Hinweis als Zeichenstelle zurück (das Frontend kennt keine Wortindizes).
    w.next_hint = w
        .next_hint
        .and_then(|i| words.get(i as usize).map(|bw| bw.start));

    let key = text_key(&ctx.text);
    let _lib = crate::library::sync_lock();
    let mut cache = load_cache(&ctx.dir, &key);
    merge(&mut cache, &w);
    let json = serde_json::to_vec(&cache).map_err(|e| e.to_string())?;
    write_atomic(&cache_path(&ctx.dir), &json)?;
    Ok(w)
}

fn recording(app: &AppHandle) -> bool {
    app.try_state::<Arc<AudioRecordingManager>>()
        .is_some_and(|a| a.is_recording())
}

/// Temporäre WAV-Datei mit eindeutigem Namen, die auf jedem Weg verschwindet.
struct TempWav(PathBuf);

impl TempWav {
    fn new(book_id: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let safe: String = book_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(40)
            .collect();
        Self(std::env::temp_dir().join(format!(
            "lva-hoersync-{safe}-{}-{n}.wav",
            std::process::id()
        )))
    }
}

impl Drop for TempWav {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn read_wav(path: &Path) -> Result<Vec<f32>, String> {
    let mut r = hound::WavReader::open(path).map_err(|e| format!("WAV unlesbar: {e}"))?;
    let spec = r.spec();
    let out: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV unlesbar: {e}"))?
        }
        hound::SampleFormat::Float => r
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV unlesbar: {e}"))?,
    };
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(d: f64) -> AudioTrack {
        AudioTrack {
            file: "a.mp3".into(),
            title: String::new(),
            duration_ms: Some(d),
            transcript_job: None,
            transcript_jobs: vec![],
        }
    }

    #[test]
    fn stelle_ueber_mehrere_teile() {
        let t = [track(1000.0), track(2000.0)];
        assert_eq!(locate(&t, 500.0), Some((0, 500.0)));
        assert_eq!(locate(&t, 1500.0), Some((1, 500.0)));
        assert_eq!(locate(&t, 9000.0), Some((1, 8000.0)));
        let unknown = [
            AudioTrack {
                duration_ms: None,
                ..track(0.0)
            },
            track(10.0),
        ];
        assert_eq!(locate(&unknown, 5.0), None);
    }

    #[test]
    fn fingerabdruck_aus_laenge_und_inhalt() {
        // Länge in UTF-16-Einheiten vorn, Reihenfolge zählt.
        assert!(text_key("ab").starts_with("2:"));
        assert_ne!(text_key("ab"), text_key("ba"));
    }

    #[test]
    fn text_aus_seitenstand() {
        assert_eq!(
            text_of_state(r#"{"text":"Hallo","x":1}"#).as_deref(),
            Some("Hallo")
        );
        assert_eq!(text_of_state("kaputt"), None);
    }

    #[test]
    fn wortzeiten_haben_vorrang() {
        use crate::managers::transcription::WordTime;
        let seg = TimedSegment {
            start_ms: 0,
            end_ms: 2000,
            text: "Ein Turm".into(),
            words: Some(vec![
                WordTime {
                    text: " Ein".into(),
                    start_ms: 100,
                    end_ms: 300,
                },
                WordTime {
                    text: " Turm".into(),
                    start_ms: 1200,
                    end_ms: 1900,
                },
            ]),
        };
        let h = heard_of(&[seg], 10_000.0);
        assert_eq!(h.len(), 2);
        assert_eq!(h[1].start_ms, 11_200.0);
        assert_eq!(h[1].text, "Turm");
    }

    #[test]
    fn zusammenfuehren_ersetzt_das_fenster_und_bleibt_monoton() {
        let mut c = AudioSyncCache::default();
        let w1 = AudioSyncWindow {
            points: vec![
                SyncPoint {
                    chars: 10,
                    ms: 1000.0,
                },
                SyncPoint {
                    chars: 20,
                    ms: 2000.0,
                },
            ],
            from_ms: 0.0,
            to_ms: 3000.0,
            ..Default::default()
        };
        merge(&mut c, &w1);
        // Neues Fenster überlappt und liefert für 2000 ms eine bessere Stelle.
        let w2 = AudioSyncWindow {
            points: vec![
                SyncPoint {
                    chars: 22,
                    ms: 2100.0,
                },
                SyncPoint {
                    chars: 30,
                    ms: 3500.0,
                },
            ],
            from_ms: 2000.0,
            to_ms: 4000.0,
            ..Default::default()
        };
        merge(&mut c, &w2);
        assert_eq!(
            c.points,
            vec![
                SyncPoint {
                    chars: 10,
                    ms: 1000.0
                },
                SyncPoint {
                    chars: 22,
                    ms: 2100.0
                },
                SyncPoint {
                    chars: 30,
                    ms: 3500.0
                },
            ]
        );
        assert_eq!(
            c.covered,
            vec![AudioSyncSpan {
                from_ms: 0.0,
                to_ms: 4000.0
            }]
        );
        // Ein verfehltes Fenster (zeigt zurück) bricht die Ordnung nicht.
        let w3 = AudioSyncWindow {
            points: vec![SyncPoint {
                chars: 5,
                ms: 5000.0,
            }],
            from_ms: 4500.0,
            to_ms: 6000.0,
            ..Default::default()
        };
        merge(&mut c, &w3);
        assert!(c
            .points
            .windows(2)
            .all(|p| p[0].chars < p[1].chars && p[0].ms < p[1].ms));
        assert_eq!(c.covered.len(), 2);
    }

    #[test]
    fn fenster_mit_falschem_hinweis_sucht_im_ganzen_buch() {
        let text = (0..12_000)
            .map(|i| format!("wort{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let words = hoersync::book_words(&text);
        let heard = hoersync::heard_words(&[hoersync::Segment {
            text: (9000..9040)
                .map(|i| format!("wort{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            start_ms: 0.0,
            end_ms: 20_000.0,
        }]);
        // Hinweis bei Wort 0: das Suchfenster reicht nicht bis 9000.
        let w = align_window(&words, &heard, Some(0), 0.0, 20_000.0);
        assert!(w.coverage > 0.9);
        assert_eq!(w.next_hint, Some(9040));
    }

    #[test]
    fn ein_ausreisser_kostet_nur_sich_selbst() {
        let p = |chars: u32, ms: f64| SyncPoint { chars, ms };
        let out = monotone_chain(vec![
            p(10, 1000.0),
            p(9000, 2000.0), // Ausreißer weit vorn im Buch
            p(30, 3000.0),
            p(40, 4000.0),
            p(50, 5000.0),
        ]);
        let chars: Vec<u32> = out.iter().map(|p| p.chars).collect();
        assert_eq!(chars, vec![10, 30, 40, 50]);
    }

}
