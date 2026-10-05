//! Sprachmodelle aus fremden Ordnern: GGUF-Ordner (LM Studio, eigene Ablage)
//! und der Modellspeicher von Ollama. Die App laedt sie von dort, statt sie
//! ein zweites Mal herunterzuladen.
//!
//! Was der Spike vom 05.10.2026 gezeigt hat, bestimmt die Regeln hier:
//! - Ollama legt Modelle als GGUF-Blobs ohne Endung ab; welcher Blob das
//!   Sprachmodell ist, steht im Manifest (`application/vnd.ollama.image.model`).
//! - Manche dieser Blobs sind Ollamas eigenes Format: der Bildteil steckt IM
//!   Sprachmodell (`<arch>.vision.block_count`). `llama-server` bricht dann nach
//!   0,4 s mit "wrong array length" oder "wrong number of tensors" ab
//!   (qwen3.5:9b, gemma4:26b). Das ist am Kopf erkennbar -- solche Modelle
//!   bleiben sichtbar, aber gesperrt, mit dem Hinweis auf den Ollama-Anbieter.
//! - Alles andere entscheidet erst ein Ladeversuch ([`probe`]); sein Ergebnis
//!   wird je Datei gemerkt.
//!
//! Fremde Dateien werden NIE geloescht -- weder hier noch ueber die Modellkarte.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;

use crate::managers::gguf_meta::{self, GgufMetadata};

/// Praefix der Kennungen fremder Modelle. Katalogkennungen beginnen mit
/// `llm-`, eine Verwechslung ist damit ausgeschlossen.
pub const EXTERNAL_PREFIX: &str = "ext-";

/// Kleinere Dateien sind keine Sprachmodelle, die fuer ein Protokoll taugen
/// (Projektoren, Embeddings, Entwuerfe).
const MIN_MODEL_BYTES: u64 = 200 * 1024 * 1024;

/// Wie tief ein Ordner durchsucht wird. LM Studio legt
/// `<anbieter>/<modell>/<datei>.gguf` ab; mehr als fuenf Ebenen braucht keiner.
const MAX_DEPTH: u8 = 5;

/// Der Ladeversuch gilt als bestanden, wenn so lange nach dem Start kein
/// Fehler kam: beide bekannten Pruefungen (Kopfwerte, Tensoranzahl) laufen
/// vor den Gewichten und scheiterten im Spike nach 0,4-0,6 s.
const PROBE_GRACE: Duration = Duration::from_secs(5);

/// Architekturen, die keine Chat-Modelle sind.
const NON_CHAT_ARCHS: [&str; 6] = [
    "clip",
    "bert",
    "nomic-bert",
    "nomic-bert-moe",
    "jina-bert-v2",
    "t5encoder",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSource {
    /// Ein Ordner mit `.gguf`-Dateien.
    Folder,
    /// Der Modellspeicher von Ollama (`manifests/` + `blobs/`).
    Ollama,
}

/// Kann der `llama-server` der App die Datei laden?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum Compat {
    /// Noch nicht geprueft -- aktivierbar; der erste Start zeigt es.
    Unchecked,
    /// Ladeversuch bestanden.
    Ok,
    /// Laedt nicht; der Grund steht dabei.
    Incompatible(String),
}

/// Was ein Modell als "dasselbe" ausweist, egal wer es abgelegt hat:
/// Bauart, Groesse und Schichtzahl. Gemma 4 12B traegt bei Unsloth, Ollama
/// und LM Studio jeweils `gemma4 / 12B / 48`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Type)]
pub struct ModelIdent {
    pub arch: String,
    pub size_label: String,
    pub layers: u64,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct ExternalModel {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub source: ExternalSource,
    pub compat: Compat,
    #[serde(skip)]
    pub ident: Option<ModelIdent>,
}

/// Kennung aus dem Pfad: stabil ueber Neustarts (sie steht in den
/// Einstellungen des aktiven Modells), anders als `DefaultHasher`.
pub fn id_for_path(path: &Path) -> String {
    let digest = Sha256::digest(path.to_string_lossy().to_lowercase().as_bytes());
    let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{EXTERNAL_PREFIX}{hex}")
}

pub fn is_external_id(id: &str) -> bool {
    id.starts_with(EXTERNAL_PREFIX)
}

/// Ist `dir` ein Ollama-Modellspeicher?
pub fn is_ollama_store(dir: &Path) -> bool {
    dir.join("manifests").is_dir() && dir.join("blobs").is_dir()
}

/// Wo Ollama hier seine Modelle ablegt: `OLLAMA_MODELS`, sonst
/// `~/.ollama/models`. `None`, wenn dort kein Speicher liegt.
pub fn detect_ollama_store() -> Option<PathBuf> {
    let from_env = std::env::var_os("OLLAMA_MODELS").map(PathBuf::from);
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(|h| PathBuf::from(h).join(".ollama").join("models"));
    from_env
        .into_iter()
        .chain(home)
        .find(|p| is_ollama_store(p))
}

/// Typische GGUF-Ablage von LM Studio, falls vorhanden.
pub fn detect_lmstudio_dir() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
    let dir = PathBuf::from(home).join(".lmstudio").join("models");
    dir.is_dir().then_some(dir)
}

// ---------------------------------------------------------------------------
// Kopf lesen und einordnen
// ---------------------------------------------------------------------------

const GENERAL_KEYS: [&str; 4] = [
    "general.architecture",
    "general.name",
    "general.size_label",
    "general.type",
];

/// Was der Kopf ueber eine Datei sagt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HeadInfo {
    pub name: Option<String>,
    pub ident: Option<ModelIdent>,
    /// `Some(grund)`: das ist kein Chat-Modell -- ausblenden.
    pub skip: Option<&'static str>,
    /// `Some(grund)`: Chat-Modell, das `llama-server` sicher nicht laedt.
    pub incompatible: Option<String>,
}

fn read_head(path: &Path) -> Option<HeadInfo> {
    let general = gguf_meta::read_file_header(path, &GENERAL_KEYS)?;
    let arch = general.get_str("general.architecture")?.to_string();
    let keys = [
        format!("{arch}.block_count"),
        format!("{arch}.vision.block_count"),
        format!("{arch}.pooling_type"),
    ];
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let arch_meta = gguf_meta::read_file_header(path, &key_refs)?;
    Some(classify(&general, &arch_meta, &arch))
}

pub(crate) fn classify(general: &GgufMetadata, arch_meta: &GgufMetadata, arch: &str) -> HeadInfo {
    let skip =
        if general.get_str("general.type") == Some("mmproj") || NON_CHAT_ARCHS.contains(&arch) {
            Some("projector_or_encoder")
        } else if arch_meta.get_u64(&format!("{arch}.pooling_type")).is_some() {
            Some("embedding")
        } else {
            None
        };
    let incompatible = arch_meta
        .get_u64(&format!("{arch}.vision.block_count"))
        .map(|_| "ollama_merged_vision".to_string());
    let layers = arch_meta.get_u64(&format!("{arch}.block_count"));
    let size_label = general.get_str("general.size_label").map(str::to_string);
    let ident = match (size_label, layers) {
        (Some(size_label), Some(layers)) => Some(ModelIdent {
            arch: arch.to_string(),
            size_label,
            layers,
        }),
        _ => None,
    };
    HeadInfo {
        name: general.get_str("general.name").map(str::to_string),
        ident,
        skip,
        incompatible,
    }
}

// ---------------------------------------------------------------------------
// Scannen
// ---------------------------------------------------------------------------

/// Ein Fund vor dem Einordnen: Datei und der Name, unter dem der Nutzer sie
/// kennt (Ollama-Tag oder Dateiname).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub label: String,
    pub source: ExternalSource,
}

/// `.gguf`-Dateien eines Ordners. Projektoren (`mmproj-*`) und die
/// Folgeteile geteilter Modelle (`-00002-of-00003`) zaehlen nicht --
/// `llama-server` laedt ein geteiltes Modell ueber den ersten Teil.
pub(crate) fn scan_folder(dir: &Path) -> Vec<Candidate> {
    fn walk(dir: &Path, depth: u8, out: &mut Vec<Candidate>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, out);
                }
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            if !lower.ends_with(".gguf") || lower.starts_with("mmproj") {
                continue;
            }
            if let Some(part) = split_part(&lower) {
                if part != 1 {
                    continue;
                }
            }
            if entry.metadata().map(|m| m.len()).unwrap_or(0) < MIN_MODEL_BYTES {
                continue;
            }
            let label = name[..name.len() - ".gguf".len()].to_string();
            out.push(Candidate {
                path,
                label,
                source: ExternalSource::Folder,
            });
        }
    }
    let mut out = Vec::new();
    walk(dir, MAX_DEPTH, &mut out);
    out
}

/// Teilnummer aus `name-00002-of-00003.gguf`.
fn split_part(lower_name: &str) -> Option<u32> {
    let stem = lower_name.strip_suffix(".gguf")?;
    let (head, total) = stem.rsplit_once("-of-")?;
    if total.len() != 5 || !total.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (_, part) = head.rsplit_once('-')?;
    if part.len() != 5 {
        return None;
    }
    part.parse().ok()
}

#[derive(Deserialize)]
struct OllamaManifest {
    layers: Vec<OllamaLayer>,
}

#[derive(Deserialize)]
struct OllamaLayer {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
}

/// Modelle des Ollama-Speichers: je Manifest der Blob des Sprachmodells.
/// Mehrere Tags auf denselben Blob (`qwen3:latest`, `qwen3:8b`) werden ein
/// Eintrag mit beiden Namen. Cloud-Modelle haben keinen Blob und fallen weg.
pub(crate) fn scan_ollama(store: &Path) -> Vec<Candidate> {
    let manifests = store.join("manifests");
    let mut by_blob: Vec<(PathBuf, Vec<String>)> = Vec::new();
    for (file, name) in manifest_files(&manifests) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<OllamaManifest>(&text) else {
            continue;
        };
        let Some(layer) = manifest
            .layers
            .iter()
            .find(|l| l.media_type == "application/vnd.ollama.image.model")
        else {
            continue;
        };
        let blob = store.join("blobs").join(layer.digest.replace(':', "-"));
        if blob.metadata().map(|m| m.len()).unwrap_or(0) < MIN_MODEL_BYTES {
            continue;
        }
        match by_blob.iter_mut().find(|(b, _)| *b == blob) {
            Some((_, names)) => names.push(name),
            None => by_blob.push((blob, vec![name])),
        }
    }
    by_blob
        .into_iter()
        .map(|(path, mut names)| {
            names.sort();
            Candidate {
                path,
                label: names.join(", "),
                source: ExternalSource::Ollama,
            }
        })
        .collect()
}

/// `manifests/<host>/<namespace>/<modell>/<tag>` mit dem Namen, den
/// `ollama list` zeigt: `modell:tag` fuer die offizielle Bibliothek, sonst
/// `namespace/modell:tag`.
fn manifest_files(manifests: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let dirs = |p: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    };
    for host in dirs(manifests).into_iter().filter(|p| p.is_dir()) {
        let official = host.file_name().is_some_and(|n| n == "registry.ollama.ai");
        for ns in dirs(&host).into_iter().filter(|p| p.is_dir()) {
            let ns_name = ns
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            for model in dirs(&ns).into_iter().filter(|p| p.is_dir()) {
                let model_name = model
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                for tag in dirs(&model).into_iter().filter(|p| p.is_file()) {
                    let tag_name = tag
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let name = if official && ns_name == "library" {
                        format!("{model_name}:{tag_name}")
                    } else {
                        format!("{ns_name}/{model_name}:{tag_name}")
                    };
                    out.push((tag, name));
                }
            }
        }
    }
    out
}

/// Alle Ordner einlesen, einordnen und mit dem gemerkten Pruefergebnis
/// versehen. Doppelte Funde (derselbe Ordner zweimal eingetragen) zaehlen
/// einmal.
pub fn scan(dirs: &[String], probes: &ProbeCache) -> Vec<ExternalModel> {
    let mut candidates = Vec::new();
    for dir in dirs {
        let dir = PathBuf::from(dir.trim());
        if dir.as_os_str().is_empty() || !dir.is_dir() {
            continue;
        }
        if is_ollama_store(&dir) {
            candidates.extend(scan_ollama(&dir));
        } else {
            candidates.extend(scan_folder(&dir));
        }
    }
    let mut out: Vec<ExternalModel> = Vec::new();
    for c in candidates {
        let id = id_for_path(&c.path);
        if out.iter().any(|m| m.id == id) {
            continue;
        }
        let Some(head) = read_head(&c.path) else {
            continue;
        };
        if head.skip.is_some() {
            continue;
        }
        let size_bytes = c.path.metadata().map(|m| m.len()).unwrap_or(0);
        let compat = match head.incompatible {
            Some(reason) => Compat::Incompatible(reason),
            None => probes.get(&c.path).unwrap_or(Compat::Unchecked),
        };
        out.push(ExternalModel {
            id,
            name: c.label,
            path: c.path.to_string_lossy().into_owned(),
            size_bytes,
            source: c.source,
            compat,
            ident: head.ident,
        });
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Kennung eines Katalogmodells, das nach Bauart, Groesse und Schichtzahl
/// dasselbe ist wie ein gepruefter fremder Fund -- dessen App-Kopie ist dann
/// entbehrlich. Nur `Compat::Ok` zaehlt: eine Kopie zu loeschen, deren Ersatz
/// nicht laedt (qwen3.5:9b aus Ollama), haette das Modell gekostet.
pub fn ident_of_file(path: &Path) -> Option<ModelIdent> {
    read_head(path)?.ident
}

// ---------------------------------------------------------------------------
// Ladeversuch
// ---------------------------------------------------------------------------

/// Gemerkte Ladeversuche, je Datei mit Groesse und Aenderungszeit -- eine
/// neu heruntergeladene Fassung wird neu geprueft.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProbeCache {
    entries: HashMap<String, Compat>,
}

fn cache_key(path: &Path) -> Option<String> {
    let meta = path.metadata().ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    Some(format!(
        "{}|{}|{}",
        path.to_string_lossy().to_lowercase(),
        meta.len(),
        mtime
    ))
}

impl ProbeCache {
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) {
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(file, text);
        }
    }

    pub fn get(&self, path: &Path) -> Option<Compat> {
        self.entries.get(&cache_key(path)?).cloned()
    }

    pub fn set(&mut self, path: &Path, compat: Compat) {
        if let Some(key) = cache_key(path) {
            self.entries.insert(key, compat);
        }
    }
}

/// Einordnung einer Ausgabezeile von `llama-server` beim Laden.
#[derive(Debug, PartialEq)]
pub(crate) enum ProbeLine {
    Failed(String),
    Loaded,
    Other,
}

pub(crate) fn classify_line(line: &str) -> ProbeLine {
    if let Some(i) = line.find("error loading model") {
        let reason = line[i..]
            .trim_start_matches("error loading model")
            .trim_start_matches(':')
            .trim();
        return ProbeLine::Failed(reason.to_string());
    }
    if line.contains("failed to load model") || line.contains("unknown model architecture") {
        return ProbeLine::Failed(line.trim().to_string());
    }
    if line.contains("model loaded") {
        return ProbeLine::Loaded;
    }
    ProbeLine::Other
}

/// Laedt `model` probeweise -- nur auf der CPU, ohne Umpacken und ohne
/// Aufwaermen, und hoechstens [`PROBE_GRACE`] lang: die Pruefungen, an denen
/// fremde Dateien scheitern, laufen vor den Gewichten. So belegt der Versuch
/// weder Grafikspeicher noch viel Arbeitsspeicher. Blockiert -- auf einem
/// Blocking-Thread aufrufen.
pub fn probe(binary: &Path, model: &Path) -> Compat {
    let port = match std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()) {
        Ok(addr) => addr.port(),
        Err(_) => return Compat::Unchecked,
    };
    let mut cmd = std::process::Command::new(binary);
    cmd.arg("-m")
        .arg(model)
        .args(["--host", "127.0.0.1", "--port", &port.to_string()])
        .args([
            "-ngl",
            "0",
            "-c",
            "512",
            "--parallel",
            "1",
            "--no-warmup",
            "--no-repack",
            "-fit",
            "off",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());
    if let Some(dir) = binary.parent() {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
        cmd.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Ladeversuch nicht startbar: {e}");
            return Compat::Unchecked;
        }
    };
    let stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel::<ProbeLine>();
    let reader = std::thread::spawn(move || {
        let Some(err) = stderr else { return };
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            let class = classify_line(&line);
            if class != ProbeLine::Other && tx.send(class).is_err() {
                return;
            }
        }
    });
    let started = Instant::now();
    let verdict = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ProbeLine::Failed(reason)) => break Compat::Incompatible(reason),
            Ok(ProbeLine::Loaded) => break Compat::Ok,
            Ok(ProbeLine::Other) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Ausgabe zu, ohne Urteil: der Prozess ist weg. Ein Ende
                // vor der Schonfrist ohne Fehlerzeile ist trotzdem ein Fehler.
                break if started.elapsed() < PROBE_GRACE {
                    Compat::Incompatible("server_exited".to_string())
                } else {
                    Compat::Ok
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Ok(Some(_)) = child.try_wait() {
            // Restliche Zeilen abwarten -- die Fehlerzeile kommt oft erst
            // mit dem Prozessende an.
            continue;
        }
        if started.elapsed() >= PROBE_GRACE {
            break Compat::Ok;
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    drop(rx);
    let _ = reader.join();
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::gguf_meta::GgufValue;

    fn meta(pairs: &[(&str, GgufValue)]) -> GgufMetadata {
        GgufMetadata {
            kv: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    fn s(v: &str) -> GgufValue {
        GgufValue::String(v.to_string())
    }

    /// Gegen echte Ordner und die echte Laufzeit (nur von Hand):
    /// `LV_MODEL_DIRS="D:\ollama\models;C:\...\.lmstudio\models"`
    /// `LV_LLAMA_SERVER=...\llama-server.exe`
    /// `cargo test --lib real_model_dirs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_model_dirs_scan_and_probe() {
        let dirs: Vec<String> = std::env::var("LV_MODEL_DIRS")
            .expect("LV_MODEL_DIRS")
            .split(';')
            .map(str::to_string)
            .collect();
        let server = PathBuf::from(std::env::var("LV_LLAMA_SERVER").expect("LV_LLAMA_SERVER"));
        let started = Instant::now();
        let models = scan(&dirs, &ProbeCache::default());
        println!("Scan: {} Modelle in {:?}", models.len(), started.elapsed());
        for m in &models {
            let t = Instant::now();
            let compat = match &m.compat {
                Compat::Incompatible(_) => m.compat.clone(),
                _ => probe(&server, Path::new(&m.path)),
            };
            println!(
                "{:<55} {:>6} MB  {:?}  Kopf={:?}  Pruefung {:?} in {:?}",
                m.name,
                m.size_bytes >> 20,
                m.source,
                m.ident,
                compat,
                t.elapsed()
            );
        }
        assert!(!models.is_empty());
    }

    /// Die Koepfe aus dem Spike vom 05.10.2026.
    #[test]
    fn ollama_merged_vision_is_incompatible_plain_text_model_is_not() {
        let general = meta(&[
            ("general.architecture", s("qwen35")),
            ("general.type", s("model")),
        ]);
        let merged = meta(&[
            ("qwen35.block_count", GgufValue::U32(32)),
            ("qwen35.vision.block_count", GgufValue::U32(27)),
        ]);
        let head = classify(&general, &merged, "qwen35");
        assert_eq!(head.incompatible.as_deref(), Some("ollama_merged_vision"));
        assert!(head.skip.is_none());
        // Ohne size_label keine Kennung -- also auch kein Duplikat-Vorschlag.
        assert!(head.ident.is_none());

        let general = meta(&[
            ("general.architecture", s("qwen35")),
            ("general.name", s("Qwen3.8 27B 0814")),
            ("general.size_label", s("27B")),
        ]);
        let plain = meta(&[("qwen35.block_count", GgufValue::U32(65))]);
        let head = classify(&general, &plain, "qwen35");
        assert!(head.incompatible.is_none());
        assert_eq!(
            head.ident,
            Some(ModelIdent {
                arch: "qwen35".into(),
                size_label: "27B".into(),
                layers: 65
            })
        );
    }

    #[test]
    fn projectors_and_embeddings_are_skipped() {
        let mmproj = meta(&[
            ("general.architecture", s("clip")),
            ("general.type", s("mmproj")),
        ]);
        assert!(classify(&mmproj, &meta(&[]), "clip").skip.is_some());
        let bert = meta(&[("general.architecture", s("bert"))]);
        assert!(classify(&bert, &meta(&[]), "bert").skip.is_some());
        let pooled = meta(&[("general.architecture", s("qwen3"))]);
        let arch = meta(&[("qwen3.pooling_type", GgufValue::U32(1))]);
        assert_eq!(classify(&pooled, &arch, "qwen3").skip, Some("embedding"));
    }

    #[test]
    fn same_model_from_different_publishers_has_the_same_ident() {
        let unsloth = meta(&[
            ("general.architecture", s("gemma4")),
            ("general.name", s("Gemma-4-12B-It")),
            ("general.size_label", s("12B")),
        ]);
        let ollama = meta(&[
            ("general.architecture", s("gemma4")),
            ("general.size_label", s("12B")),
        ]);
        let layers = meta(&[("gemma4.block_count", GgufValue::U32(48))]);
        assert_eq!(
            classify(&unsloth, &layers, "gemma4").ident,
            classify(&ollama, &layers, "gemma4").ident
        );
    }

    #[test]
    fn split_parts_are_recognised() {
        assert_eq!(split_part("model-00001-of-00003.gguf"), Some(1));
        assert_eq!(split_part("model-00002-of-00003.gguf"), Some(2));
        assert_eq!(split_part("qwen3-14b-q4_k_m.gguf"), None);
        assert_eq!(split_part("gpt-oss-20b-mxfp4.gguf"), None);
    }

    /// Die Fehlerzeilen aus dem Spike, wortgleich.
    #[test]
    fn load_errors_are_recognised() {
        let line = "0.00.382.543 E llama_model_load: error loading model: error loading model hyperparameters: key qwen35.rope.dimension_sections has wrong array length; expected 4, got 3";
        match classify_line(line) {
            ProbeLine::Failed(reason) => assert!(reason.contains("wrong array length"), "{reason}"),
            other => panic!("{other:?}"),
        }
        let line = "0.00.620.004 E llama_model_load: error loading model: done_getting_tensors: wrong number of tensors; expected 1014, got 658";
        assert!(
            matches!(classify_line(line), ProbeLine::Failed(r) if r.contains("wrong number of tensors"))
        );
        assert_eq!(
            classify_line("0.04.542.234 I srv  llama_server: model loaded"),
            ProbeLine::Loaded
        );
        assert_eq!(
            classify_line("W model has unused tensor blk.64.attn_norm.weight"),
            ProbeLine::Other
        );
    }

    #[test]
    fn ids_are_stable_and_prefixed() {
        let a = id_for_path(Path::new(r"D:\ollama\models\blobs\sha256-abc"));
        assert_eq!(
            a,
            id_for_path(Path::new(r"D:\OLLAMA\models\blobs\sha256-abc"))
        );
        assert!(is_external_id(&a));
        assert_eq!(a.len(), EXTERNAL_PREFIX.len() + 12);
        assert_ne!(
            a,
            id_for_path(Path::new(r"D:\ollama\models\blobs\sha256-abd"))
        );
    }

    fn write_manifest(store: &Path, rel: &[&str], digest: &str) {
        let dir = rel[..rel.len() - 1]
            .iter()
            .fold(store.join("manifests"), |p, s| p.join(s));
        std::fs::create_dir_all(&dir).unwrap();
        let json = format!(
            r#"{{"layers":[{{"mediaType":"application/vnd.ollama.image.license","digest":"sha256:lic"}},{{"mediaType":"application/vnd.ollama.image.model","digest":"{digest}"}}]}}"#
        );
        std::fs::write(dir.join(rel[rel.len() - 1]), json).unwrap();
    }

    fn sparse_file(path: &Path, len: u64) {
        let f = std::fs::File::create(path).unwrap();
        f.set_len(len).unwrap();
    }

    #[test]
    fn ollama_tags_on_one_blob_become_one_entry_and_cloud_models_vanish() {
        let store = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store.path().join("blobs")).unwrap();
        sparse_file(
            &store.path().join("blobs").join("sha256-aaa"),
            MIN_MODEL_BYTES + 1,
        );
        write_manifest(
            store.path(),
            &["registry.ollama.ai", "library", "qwen3", "8b"],
            "sha256:aaa",
        );
        write_manifest(
            store.path(),
            &["registry.ollama.ai", "library", "qwen3", "latest"],
            "sha256:aaa",
        );
        write_manifest(
            store.path(),
            &["registry.ollama.ai", "richardyoung", "coder", "q4"],
            "sha256:missing",
        );
        // Cloud-Modell: Manifest ohne Modell-Layer.
        let cloud = store
            .path()
            .join("manifests/registry.ollama.ai/library/kimi/cloud");
        std::fs::create_dir_all(cloud.parent().unwrap()).unwrap();
        std::fs::write(&cloud, r#"{"layers":[]}"#).unwrap();

        assert!(is_ollama_store(store.path()));
        let found = scan_ollama(store.path());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].label, "qwen3:8b, qwen3:latest");
        assert_eq!(found[0].source, ExternalSource::Ollama);
    }

    #[test]
    fn folder_scan_skips_projectors_small_files_and_later_split_parts() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("lmstudio-community").join("gemma");
        std::fs::create_dir_all(&nested).unwrap();
        sparse_file(
            &nested.join("gemma-4-12B-it-QAT-Q4_0.gguf"),
            MIN_MODEL_BYTES + 1,
        );
        sparse_file(&nested.join("mmproj-gemma-4-12B.gguf"), MIN_MODEL_BYTES + 1);
        sparse_file(&nested.join("tiny.gguf"), 1024);
        sparse_file(&nested.join("big-00001-of-00002.gguf"), MIN_MODEL_BYTES + 1);
        sparse_file(&nested.join("big-00002-of-00002.gguf"), MIN_MODEL_BYTES + 1);
        std::fs::write(nested.join("readme.txt"), b"x").unwrap();
        let labels: Vec<String> = scan_folder(dir.path())
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(
            labels,
            vec!["big-00001-of-00002", "gemma-4-12B-it-QAT-Q4_0"]
        );
    }

    #[test]
    fn probe_cache_round_trips_and_forgets_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("m.gguf");
        std::fs::write(&model, b"abc").unwrap();
        let mut cache = ProbeCache::default();
        cache.set(&model, Compat::Ok);
        let file = dir.path().join("probe.json");
        cache.save(&file);
        let loaded = ProbeCache::load(&file);
        assert_eq!(loaded.get(&model), Some(Compat::Ok));
        std::fs::write(&model, b"abcd").unwrap();
        assert_eq!(loaded.get(&model), None, "andere Groesse = neue Datei");
    }
}
