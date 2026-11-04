//! Tauri-Commands für Zusammenführungs-Vorschläge aus Stimmabdrücken
//! (Konzept Stimmprofile, Schritt 2a). Die Logik ohne Tauri steht in
//! [`voice_matches`] und [`dismiss`] (testbar mit Store und Attrappe).
//!
//! Datenschutz: Die Stimmabdrücke bleiben im Arbeitsspeicher (nur dieser
//! Besprechung). Gespeichert wird nur, welche Paare der Nutzer verworfen hat.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::{json, Value};
use specta::Type;
use tauri::{AppHandle, Emitter, State};

use crate::managers::meetings::speakers;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::meetings::voiceprint::{self, similar, Embedder};

/// Ein geladenes Modell: Abdruck eines Ausschnitts.
pub type EmbedFn = Box<dyn FnMut(&[f32]) -> Result<Vec<f32>, String>>;
/// Lädt das Modell erst bei Bedarf.
pub type LoadFn<'a> = &'a mut dyn FnMut() -> Result<EmbedFn, String>;

/// Verworfene Vorschläge in `metadata_json` (je Segment-Epoche).
pub const DISMISSED_KEY: &str = "speaker_voice_dismissed";
const MAX_DISMISSED: usize = 100;

/// Ein Vorschlag: Sprecher `a` und `b` derselben Spur klingen gleich.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct VoiceMatch {
    pub channel: u8,
    pub a: u32,
    pub b: u32,
    /// Ähnlichkeit in Prozent (Kosinus × 100, gerundet).
    pub score: u32,
}

/// Antwort für die Sprechertabelle. `state`: `ready`, `off` (Einstellung),
/// `model_missing`, `low_memory`, `no_audio`.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct VoiceMatches {
    pub state: String,
    pub matches: Vec<VoiceMatch>,
}

impl VoiceMatches {
    fn empty(state: &str) -> Self {
        Self {
            state: state.into(),
            matches: Vec::new(),
        }
    }
}

/// Stand für die Einstellungszeile.
#[derive(Clone, Debug, PartialEq, Serialize, Type)]
pub struct VoiceprintStatus {
    pub enabled: bool,
    pub model_ready: bool,
    pub downloading: bool,
    /// Größe des Modells in MB (gerundet).
    pub size_mb: u32,
}

/// Abdrücke je Ausschnitt `(Besprechung, Kanal, Start, Ende)`, nur für die
/// zuletzt angefragte Besprechung. Nach einem Zusammenführen ändern sich nur
/// die Ausschnitte des betroffenen Sprechers; der Rest kommt von hier.
type ClipKey = (u8, u64, u64);
struct ClipCache {
    meeting: String,
    clips: HashMap<ClipKey, Vec<f32>>,
}

fn cache() -> &'static Mutex<ClipCache> {
    static CACHE: OnceLock<Mutex<ClipCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(ClipCache {
            meeting: String::new(),
            clips: HashMap::new(),
        })
    })
}

/// Sperre auch nach einer Panik in einem anderen Lauf (der Inhalt ist dann
/// höchstens unvollständig, nie falsch: Einträge werden nur ganz eingefügt).
fn lock_cache() -> std::sync::MutexGuard<'static, ClipCache> {
    cache().lock().unwrap_or_else(|e| e.into_inner())
}

/// Vergisst alle Abdrücke (Einstellung aus).
pub fn forget_all() {
    let mut c = lock_cache();
    c.meeting.clear();
    c.clips.clear();
}

/// Vergisst die Abdrücke von `meeting_id` (Besprechung gelöscht).
pub fn forget_meeting(meeting_id: &str) {
    let mut c = lock_cache();
    if c.meeting == meeting_id {
        c.meeting.clear();
        c.clips.clear();
    }
}

fn dismissed_pairs(metadata: Option<&Value>, epoch: u32) -> Vec<(u8, u32, u32)> {
    metadata
        .and_then(|m| m.get(DISMISSED_KEY))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|d| d.get("epoch").and_then(Value::as_u64) == Some(u64::from(epoch)))
                .filter_map(|d| {
                    let n = |k: &str| d.get(k).and_then(Value::as_u64);
                    Some((n("channel")? as u8, n("a")? as u32, n("b")? as u32))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Vorschläge einer Besprechung. `embedder` lädt das Modell erst, wenn ein
/// Ausschnitt fehlt (in Tests eine Attrappe).
pub fn voice_matches(
    store: &MeetingStore,
    meeting_id: &str,
    embedder: LoadFn<'_>,
) -> Result<VoiceMatches, String> {
    let meeting = store
        .get_meeting(meeting_id)
        .map_err(|e| e.to_string())?
        .filter(|m| m.deleted_at.is_none())
        .ok_or_else(|| "meeting_not_found".to_string())?;
    let metadata = store.metadata_json(meeting_id).map_err(|e| e.to_string())?;
    let epoch = store.segment_epoch(meeting_id).map_err(|e| e.to_string())?;
    let hints = store.speaker_hints(meeting_id).map_err(|e| e.to_string())?;
    let Some(hints) = hints.and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return Ok(VoiceMatches::empty("no_audio"));
    };
    let turns = speakers::turns_from_hints(&hints);
    let tracks = speakers::diarize_tracks(&meeting, metadata.as_ref());

    // Benannte Sprecher: zwei verschiedene Namen schlagen wir nicht vor, zwei
    // gleiche zeigt schon der Hinweis bei gleichem Namen.
    let named: Vec<(u8, u32)> = store
        .speaker_rows(meeting_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| {
            r.display_name
                .as_deref()
                .is_some_and(|n| !n.trim().is_empty())
        })
        .map(|r| (r.channel, r.speaker_index))
        .collect();
    let dismissed = dismissed_pairs(metadata.as_ref(), epoch);

    let mut model: Option<EmbedFn> = None;
    // Zwischenspeicher nur kurz sperren: Lesen und Modell laufen ohne Sperre,
    // damit Ausschalten (`forget_all`) nie auf einen Lauf wartet.
    let mut clips: HashMap<ClipKey, Vec<f32>> = {
        let cache = lock_cache();
        if cache.meeting == meeting_id {
            cache.clips.clone()
        } else {
            HashMap::new()
        }
    };
    let known = clips.len();
    let mut voices = Vec::new();
    let mut any_audio = false;
    for (channel, list) in &turns {
        let distinct: BTreeMap<u32, ()> = list
            .iter()
            .filter(|t| t.speaker != 0)
            .map(|t| (t.speaker, ()))
            .collect();
        if distinct.len() < 2 {
            continue;
        }
        let Some(track) = tracks.iter().find(|t| t.channel == *channel) else {
            continue;
        };
        if !track.path.is_file() {
            continue;
        }
        any_audio = true;
        // Audio erst lesen, wenn ein Ausschnitt nicht im Zwischenspeicher ist.
        let mut pcm: Option<Vec<f32>> = None;
        let clips = &mut clips;
        let mut embed = |clip: similar::Clip| -> Result<Option<Vec<f32>>, String> {
            let key = (*channel, clip.start_ms, clip.end_ms);
            if let Some(v) = clips.get(&key) {
                return Ok(Some(v.clone()));
            }
            if pcm.is_none() {
                pcm = Some(speakers::read_channel_pcm(&track.path)?);
            }
            let Some(part) = voiceprint::clip_pcm(pcm.as_deref().unwrap_or_default(), clip) else {
                return Ok(None);
            };
            if model.is_none() {
                model = Some(embedder()?);
            }
            let Some(m) = model.as_mut() else {
                return Ok(None);
            };
            let v = m(part)?;
            clips.insert(key, v.clone());
            Ok(Some(v))
        };
        voices.extend(voiceprint::voices_of_channel(*channel, list, &mut embed)?);
    }
    if clips.len() != known {
        let mut cache = lock_cache();
        if cache.meeting != meeting_id {
            cache.meeting = meeting_id.to_string();
            cache.clips.clear();
        }
        cache.clips.extend(clips);
    }
    if !any_audio {
        return Ok(VoiceMatches::empty("no_audio"));
    }
    let skip = |c: u8, a: u32, b: u32| {
        dismissed.contains(&(c, a, b)) || (named.contains(&(c, a)) && named.contains(&(c, b)))
    };
    let matches = similar::pairs(&voices, similar::DEFAULT_THRESHOLD, skip)
        .into_iter()
        .map(|p| VoiceMatch {
            channel: p.channel,
            a: p.a,
            b: p.b,
            score: (p.score * 100.0).round().clamp(0.0, 100.0) as u32,
        })
        .collect();
    Ok(VoiceMatches {
        state: "ready".into(),
        matches,
    })
}

/// „Nein, verschiedene Personen“: das Paar erscheint in dieser Fassung des
/// Transkripts nicht mehr. Eine neue Sprechertrennung beginnt neu.
pub fn dismiss(
    store: &MeetingStore,
    meeting_id: &str,
    channel: u8,
    a: u32,
    b: u32,
) -> Result<(), String> {
    if a == 0 || b == 0 || a == b {
        return Err("speaker_invalid".into());
    }
    let (a, b) = (a.min(b), a.max(b));
    let epoch = store.segment_epoch(meeting_id).map_err(|e| e.to_string())?;
    let metadata = store.metadata_json(meeting_id).map_err(|e| e.to_string())?;
    let mut list: Vec<Value> = metadata
        .as_ref()
        .and_then(|m| m.get(DISMISSED_KEY))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.get("epoch").and_then(Value::as_u64) == Some(u64::from(epoch)))
        .collect();
    if dismissed_pairs(Some(&json!({ DISMISSED_KEY: list.clone() })), epoch)
        .contains(&(channel, a, b))
    {
        return Ok(());
    }
    list.push(json!({ "channel": channel, "a": a, "b": b, "epoch": epoch }));
    if list.len() > MAX_DISMISSED {
        list.drain(..list.len() - MAX_DISMISSED);
    }
    store
        .set_metadata_key(meeting_id, DISMISSED_KEY, Value::Array(list))
        .map_err(|e| e.to_string())
}

fn models_root(app: &AppHandle) -> PathBuf {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("models"))
        .unwrap_or_default()
}

fn model_file(app: &AppHandle) -> Result<PathBuf, String> {
    voiceprint::model_path(&models_root(app)).ok_or_else(|| "model_missing".to_string())
}

static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[tauri::command]
#[specta::specta]
pub async fn meeting_voice_matches(
    app: AppHandle,
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
) -> Result<VoiceMatches, String> {
    if !crate::settings::get_settings(&app).meeting_voice_suggestions {
        return Ok(VoiceMatches::empty("off"));
    }
    let path = model_file(&app)?;
    if !path.is_file() {
        return Ok(VoiceMatches::empty("model_missing"));
    }
    if crate::process_guard::check_ram_for_start(voiceprint::RAM_NEED_MB).is_err() {
        return Ok(VoiceMatches::empty("low_memory"));
    }
    let store = Arc::clone(&store);
    tauri::async_runtime::spawn_blocking(move || {
        let mut load = || -> Result<EmbedFn, String> {
            let mut model = Embedder::load(&path)?;
            Ok(Box::new(move |pcm: &[f32]| model.embed(pcm)))
        };
        voice_matches(&store, &meeting_id, &mut load)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub fn meeting_voice_match_dismiss(
    store: State<'_, Arc<MeetingStore>>,
    meeting_id: String,
    channel: u8,
    a: u32,
    b: u32,
) -> Result<(), String> {
    dismiss(&store, &meeting_id, channel, a, b)
}

#[tauri::command]
#[specta::specta]
pub fn meeting_voiceprint_status(app: AppHandle) -> Result<VoiceprintStatus, String> {
    let entry = voiceprint::catalog_entry().ok_or_else(|| "model_missing".to_string())?;
    let size: u64 = entry.files.iter().map(|f| f.size_bytes).sum();
    Ok(VoiceprintStatus {
        enabled: crate::settings::get_settings(&app).meeting_voice_suggestions,
        model_ready: model_file(&app)?.is_file(),
        downloading: DOWNLOADING.load(Ordering::Relaxed),
        size_mb: (size.div_ceil(1_000_000)).min(u64::from(u32::MAX)) as u32,
    })
}

/// Lädt das Modell (30 MB) -- nur auf Knopfdruck. Fortschritt über die
/// üblichen Ereignisse `model-download-progress` und `model-download-complete`.
#[tauri::command]
#[specta::specta]
pub async fn meeting_voiceprint_download(app: AppHandle) -> Result<(), String> {
    use crate::managers::model::download::{HttpDownloadEvent, HttpDownloadOutcome};
    use crate::managers::model::ModelManager;
    let entry = voiceprint::catalog_entry().ok_or_else(|| "model_missing".to_string())?;
    let file = entry
        .files
        .first()
        .ok_or_else(|| "model_missing".to_string())?;
    let dest = model_file(&app)?;
    if dest.is_file() {
        return Ok(());
    }
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return Err("download_running".into());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            DOWNLOADING.store(false, Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    let dir = dest.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let partial = PathBuf::from(format!("{}.partial", dest.display()));
    let handle = app.clone();
    let emit = move |event: HttpDownloadEvent<'_>| {
        if let HttpDownloadEvent::Progress(p) = event {
            let _ = handle.emit("model-download-progress", p);
        }
    };
    let cancel = hf_hub::api::tokio::CancellationToken::new();
    let outcome = ModelManager::download_http_resumable_with_events(
        voiceprint::MODEL_ID,
        &file.url,
        &partial,
        Some(file.size_bytes),
        file.sha256.as_deref(),
        &cancel,
        &emit,
    )
    .await
    .map_err(|e| e.to_string())?;
    if matches!(outcome, HttpDownloadOutcome::Completed) {
        std::fs::rename(&partial, &dest).map_err(|e| e.to_string())?;
        let _ = app.emit("model-download-complete", voiceprint::MODEL_ID);
    }
    Ok(())
}

/// Einstellung `meeting_voice_suggestions`. Ausschalten vergisst die Abdrücke sofort.
#[tauri::command]
#[specta::specta]
pub fn change_meeting_voice_suggestions_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.meeting_voice_suggestions = enabled;
    crate::settings::write_settings(&app, settings);
    if !enabled {
        forget_all();
    }
    Ok(())
}

#[cfg(test)]
#[path = "meeting_voiceprint_tests.rs"]
mod tests;
