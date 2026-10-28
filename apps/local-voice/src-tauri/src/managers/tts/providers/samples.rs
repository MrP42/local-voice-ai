//! Hörproben der Anbieter-Stimmen (Phonon): ein kurzer Satz je Stimme als WAV-Datei,
//! erzeugt im Hintergrund, sobald eine Stimme da ist — nach der Installation eines Modells,
//! nach Import oder Erstellen einer eigenen Stimme und beim ersten Anzeigen der Liste (so
//! bekommen auch schon installierte Stimmen ihre Probe). Reinhören spielt danach nur noch
//! die Datei ab, ohne Wartezeit.
//!
//! * **Wo:** `<Daten>/voice_samples/<anbieter>/<schlüssel>.wav`. Der Schlüssel kommt aus
//!   [`TtsProvider::cache_tag`] (Modell, Quantisierung, Sprache, Stimme, Größe und
//!   Änderungszeit der Stimmdatei) und dem Probetext: wird eine Stimme ersetzt oder ein
//!   anderes Modell gewählt, gilt die alte Probe nicht mehr und eine neue entsteht.
//! * **Wer:** nur lokale Anbieter aus [`SAMPLE_PROVIDERS`]. Ein Cloud-Anbieter würde für
//!   jede Probe Daten senden und womöglich Geld kosten — dort gibt es keine Vorratsproben.
//! * **Wie:** nacheinander (eine Synthese zur Zeit, [`GEN_LOCK`]), jede Datei zuerst unter
//!   einem Hilfsnamen geschrieben und dann umbenannt (nie eine halbe WAV). Die Freigaben
//!   werden vor jeder Stimme neu geprüft; ist der Anbieter gesperrt, endet der Lauf.
//! * **Meldung:** Ereignis `tts-provider-voice-sample` je fertiger Probe.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::contract::{AudioEncoding, SynthesisOptions, TtsError, TtsProvider, VoiceDescriptor};
use super::phonon::fnv1a64;
use super::pipeline::next_request_id;
use crate::managers::tts::TtsManager;

/// Anbieter, für die Hörproben auf Vorrat entstehen (nur lokale).
pub const SAMPLE_PROVIDERS: &[&str] = &["phonon"];
/// Frist je Probe.
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Frist für die Stimmenliste (wie in den Befehlen).
const VOICES_LIMIT: Duration = Duration::from_secs(10);

/// Eine Synthese zur Zeit: Proben laufen neben dem Vorlesen auf der CPU und sollen es nicht
/// mehrfach ausbremsen.
static GEN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Dateien, die gerade erzeugt werden oder in der Schlange stehen (gegen Doppelaufträge,
/// wenn Liste und Dropdown gleichzeitig nachfragen).
static PENDING: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Hörprobe einer Stimme; `wav_path` fehlt, solange sie noch entsteht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProviderVoiceSample {
    pub voice_id: String,
    pub wav_path: Option<String>,
}

/// Ereignis `tts-provider-voice-sample`: eine Probe ist fertig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
pub struct TtsProviderVoiceSample {
    pub provider_id: String,
    pub voice_id: String,
    pub wav_path: String,
}

/// Ein ausstehender Auftrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleJob {
    pub voice_id: String,
    pub language: String,
    pub text: &'static str,
    pub path: PathBuf,
}

/// Der Probetext je Sprache: für alle Stimmen derselbe, sonst vergleicht man Sätze statt Stimmen.
pub fn sample_text(language: &str) -> &'static str {
    match language.split(['-', '_']).next().unwrap_or("") {
        "en" => "Hello, this is how I sound when I read aloud. Do you like my voice?",
        "fr" => "Bonjour, voici comment je sonne quand je lis à voix haute.",
        "es" => "Hola, así sueno cuando leo en voz alta.",
        "pt" => "Olá, é assim que eu soo quando leio em voz alta.",
        _ => "Hallo, so klinge ich beim Vorlesen. Gefällt dir meine Stimme?",
    }
}

/// Ziel der Probe einer Stimme (siehe Moduldoku).
pub fn sample_path(dir: &Path, provider: &dyn TtsProvider, voice_id: &str, text: &str) -> PathBuf {
    let tag = provider.cache_tag(Some(voice_id));
    dir.join(format!("{:016x}.wav", fnv1a64(&format!("{tag}|{text}"))))
}

/// Für jede Stimme: vorhandene Probe oder Auftrag. Rein bis auf das Nachsehen, ob die Datei
/// existiert.
pub fn plan(
    dir: &Path,
    provider: &dyn TtsProvider,
    voices: &[VoiceDescriptor],
) -> (Vec<ProviderVoiceSample>, Vec<SampleJob>) {
    let fallback = provider
        .descriptor()
        .caps
        .languages
        .first()
        .cloned()
        .unwrap_or_else(|| "de".to_string());
    let mut list = Vec::with_capacity(voices.len());
    let mut jobs = Vec::new();
    for voice in voices {
        let language = if voice.language.is_empty() || voice.language == "und" {
            fallback.clone()
        } else {
            voice.language.clone()
        };
        let text = sample_text(&language);
        let path = sample_path(dir, provider, &voice.id, text);
        let ready = path.is_file();
        list.push(ProviderVoiceSample {
            voice_id: voice.id.clone(),
            wav_path: ready.then(|| path.to_string_lossy().into_owned()),
        });
        if !ready {
            jobs.push(SampleJob {
                voice_id: voice.id.clone(),
                language,
                text,
                path,
            });
        }
    }
    (list, jobs)
}

/// Aufträge vormerken; zurück kommen nur die, die nicht schon laufen.
fn claim(jobs: Vec<SampleJob>) -> Vec<SampleJob> {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let set = pending.get_or_insert_with(HashSet::new);
    jobs.into_iter()
        .filter(|job| set.insert(job.path.clone()))
        .collect()
}

fn release(path: &Path) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(set) = pending.as_mut() {
        set.remove(path);
    }
}

/// WAV unter Hilfsnamen schreiben, dann umbenennen.
fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("wav.part");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Die Aufträge nacheinander abarbeiten. `check` prüft vor jeder Stimme die Freigaben;
/// `emit` meldet jede fertige Probe. Ein Fehler bei einer Stimme beendet nur deren Auftrag.
pub async fn run_jobs<F, Fut, C, E>(
    provider_id: &str,
    jobs: Vec<SampleJob>,
    synth: F,
    check: C,
    emit: E,
) where
    F: Fn(String, SynthesisOptions, &'static str) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, TtsError>>,
    C: Fn() -> Result<(), TtsError>,
    E: Fn(TtsProviderVoiceSample),
{
    let mut jobs = jobs.into_iter();
    while let Some(job) = jobs.next() {
        let _turn = GEN_LOCK.lock().await;
        if let Err(e) = check() {
            log::info!("Hörproben für '{provider_id}' angehalten: {}", e.message);
            release(&job.path);
            for rest in jobs.by_ref() {
                release(&rest.path);
            }
            return;
        }
        let opts = SynthesisOptions {
            language: job.language.clone(),
            voice: (job.voice_id != "default").then(|| job.voice_id.clone()),
            format: AudioEncoding::Wav,
            speed: None,
            seed: None,
            timeout: SAMPLE_TIMEOUT,
        };
        match synth(next_request_id("sample"), opts, job.text).await {
            Ok(data) => match write_atomically(&job.path, &data) {
                Ok(()) => emit(TtsProviderVoiceSample {
                    provider_id: provider_id.to_string(),
                    voice_id: job.voice_id.clone(),
                    wav_path: job.path.to_string_lossy().into_owned(),
                }),
                Err(e) => log::warn!("Hörprobe '{}' nicht gespeichert: {e}", job.voice_id),
            },
            Err(e) => log::warn!("Hörprobe '{}' nicht erzeugt: {}", job.voice_id, e.message),
        }
        release(&job.path);
    }
}

/// Ordner der Hörproben eines Anbieters.
pub fn samples_dir(app: &AppHandle, provider_id: &str) -> Option<PathBuf> {
    let base = crate::portable::data_dir()
        .cloned()
        .or_else(|| app.path().app_local_data_dir().ok())?;
    Some(base.join("voice_samples").join(provider_id))
}

/// Stand der Proben eines Anbieters; fehlende werden im Hintergrund erzeugt. Für Anbieter
/// ohne Vorratsproben eine leere Liste.
pub async fn samples_for(
    app: &AppHandle,
    provider_id: &str,
) -> Result<Vec<ProviderVoiceSample>, TtsError> {
    if !SAMPLE_PROVIDERS.contains(&provider_id) {
        return Ok(Vec::new());
    }
    let manager = app.state::<Arc<TtsManager>>().inner().clone();
    let snapshot = manager.provider_snapshot();
    snapshot.checked(provider_id)?;
    let provider = snapshot
        .registry
        .get(provider_id)
        .ok_or_else(|| TtsError::bad_request(format!("Unbekannter Anbieter '{provider_id}'")))?;
    let voices = tokio::time::timeout(VOICES_LIMIT, provider.list_voices(None))
        .await
        .map_err(|_| TtsError::timeout("Die Stimmenliste kam nicht rechtzeitig"))??;
    let dir = samples_dir(app, provider_id)
        .ok_or_else(|| TtsError::unavailable("Kein Datenordner für die Hörproben"))?;
    let (list, jobs) = plan(&dir, provider.as_ref(), &voices);
    let jobs = claim(jobs);
    if !jobs.is_empty() {
        let app = app.clone();
        let id = provider_id.to_string();
        let registry = snapshot.registry.clone();
        tauri::async_runtime::spawn(async move {
            let synth = |request_id: String, opts: SynthesisOptions, text: &'static str| {
                let registry = registry.clone();
                let id = id.clone();
                async move {
                    registry
                        .synthesize(&id, &request_id, text, &opts)
                        .await
                        .map(|clip| clip.data)
                }
            };
            let check = || {
                let manager = app.state::<Arc<TtsManager>>();
                manager.provider_snapshot().checked(&id).map(|_| ())
            };
            let emit = |sample: TtsProviderVoiceSample| {
                let _ = sample.emit(&app);
            };
            run_jobs(&id, jobs, synth, check, emit).await;
        });
    }
    Ok(list)
}

/// Nach Installation oder neuer Stimme: fehlende Proben im Hintergrund erzeugen. Ist der
/// Anbieter (noch) nicht freigegeben, geschieht nichts — die Liste holt es später nach.
pub fn refresh_in_background(app: &AppHandle, provider_id: &'static str) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = samples_for(&app, provider_id).await {
            log::debug!("Keine Hörproben für '{provider_id}': {}", e.message);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::mock::MockProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn voice(id: &str, language: &str) -> VoiceDescriptor {
        VoiceDescriptor {
            id: id.into(),
            label: id.into(),
            language: language.into(),
            gender: None,
            license: None,
        }
    }

    #[test]
    fn probetext_folgt_der_sprache_der_stimme() {
        assert!(sample_text("de").starts_with("Hallo"));
        assert!(sample_text("en_US").starts_with("Hello"));
        assert!(sample_text("xx").starts_with("Hallo"));
    }

    #[test]
    fn vorhandene_proben_werden_gemeldet_fehlende_beauftragt() {
        let dir = tempfile::TempDir::new().unwrap();
        let provider = MockProvider::default();
        let voices = [
            voice("default", "de"),
            voice("Anton", "de"),
            voice("Lessac", "en"),
        ];
        let anton = sample_path(dir.path(), &provider, "Anton", sample_text("de"));
        std::fs::write(&anton, b"RIFF").unwrap();

        let (list, jobs) = plan(dir.path(), &provider, &voices);
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].wav_path.as_deref(), Some(anton.to_str().unwrap()));
        assert!(list[0].wav_path.is_none());
        let ids: Vec<_> = jobs.iter().map(|j| j.voice_id.as_str()).collect();
        assert_eq!(ids, ["default", "Lessac"]);
        assert!(jobs[1].text.starts_with("Hello"));
    }

    #[test]
    fn jede_stimme_hat_ihre_eigene_datei() {
        let dir = Path::new("x");
        let provider = MockProvider::default();
        let a = sample_path(dir, &provider, "Anton", sample_text("de"));
        let b = sample_path(dir, &provider, "Annika", sample_text("de"));
        assert_ne!(a, b);
        assert_eq!(a, sample_path(dir, &provider, "Anton", sample_text("de")));
    }

    #[test]
    fn doppelte_auftraege_werden_nur_einmal_vergeben() {
        let job = SampleJob {
            voice_id: "Doppelt".into(),
            language: "de".into(),
            text: sample_text("de"),
            path: PathBuf::from("doppelt-test.wav"),
        };
        assert_eq!(claim(vec![job.clone()]).len(), 1);
        assert!(claim(vec![job.clone()]).is_empty());
        release(&job.path);
        assert_eq!(claim(vec![job.clone()]).len(), 1);
        release(&job.path);
    }

    #[tokio::test]
    async fn proben_entstehen_nacheinander_und_werden_gemeldet() {
        let dir = tempfile::TempDir::new().unwrap();
        let jobs: Vec<SampleJob> = ["default", "Anton"]
            .iter()
            .map(|id| SampleJob {
                voice_id: id.to_string(),
                language: "de".into(),
                text: sample_text("de"),
                path: dir.path().join(format!("{id}.wav")),
            })
            .collect();
        let voices_seen = Mutex::new(Vec::new());
        let emitted = Mutex::new(Vec::new());
        run_jobs(
            "phonon",
            jobs,
            |_rid, opts: SynthesisOptions, _text| {
                voices_seen.lock().unwrap().push(opts.voice.clone());
                async { Ok(b"RIFFdata".to_vec()) }
            },
            || Ok(()),
            |s| emitted.lock().unwrap().push(s.voice_id),
        )
        .await;
        // Die Standardstimme geht ohne `--voice` an den Anbieter.
        assert_eq!(
            *voices_seen.lock().unwrap(),
            [None, Some("Anton".to_string())]
        );
        assert_eq!(*emitted.lock().unwrap(), ["default", "Anton"]);
        assert_eq!(
            std::fs::read(dir.path().join("Anton.wav")).unwrap(),
            b"RIFFdata"
        );
        assert!(!dir.path().join("Anton.wav.part").exists());
    }

    #[tokio::test]
    async fn gesperrter_anbieter_beendet_den_lauf_ohne_synthese() {
        let dir = tempfile::TempDir::new().unwrap();
        let jobs = vec![SampleJob {
            voice_id: "Anton".into(),
            language: "de".into(),
            text: sample_text("de"),
            path: dir.path().join("gesperrt.wav"),
        }];
        let calls = AtomicUsize::new(0);
        run_jobs(
            "phonon",
            jobs,
            |_rid, _opts, _text| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(Vec::new()) }
            },
            || Err(TtsError::bad_request("gesperrt")),
            |_| panic!("keine Meldung erwartet"),
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!dir.path().join("gesperrt.wav").exists());
    }

    #[tokio::test]
    async fn fehler_einer_stimme_haelt_die_anderen_nicht_auf() {
        let dir = tempfile::TempDir::new().unwrap();
        let jobs: Vec<SampleJob> = ["kaputt", "gut"]
            .iter()
            .map(|id| SampleJob {
                voice_id: id.to_string(),
                language: "de".into(),
                text: sample_text("de"),
                path: dir.path().join(format!("{id}.wav")),
            })
            .collect();
        let emitted = Mutex::new(Vec::new());
        run_jobs(
            "phonon",
            jobs,
            |_rid, opts: SynthesisOptions, _text| async move {
                if opts.voice.as_deref() == Some("kaputt") {
                    Err(TtsError::backend("kaputt"))
                } else {
                    Ok(b"RIFF".to_vec())
                }
            },
            || Ok(()),
            |s| emitted.lock().unwrap().push(s.voice_id),
        )
        .await;
        assert_eq!(*emitted.lock().unwrap(), ["gut"]);
    }
}
