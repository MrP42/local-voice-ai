//! Die Attrappe der headless Sandbox-Instanz (`--agent-bridge-serve`, nur mit `LVA_MEETINGS_DIR`):
//! dieselben Handler wie in der App, aber ohne Fenster, Warteschlange, Sprach-Engine und
//! Mikrofon. Damit belegt `scripts/mcp_smoke.py --write` den ganzen Weg Agent -> MCP-Proxy ->
//! Pipe -> Recht -> Handler -> Datenbank gegen die echte EXE, ohne die App des Nutzers zu
//! beruehren.
//!
//! - **Einreihen**: schreibt Besprechung und Warteschlangenzeile (Status `queued`); es gibt keinen
//!   Arbeiter, die Datei wird nie gelesen.
//! - **Seiten und Audio**: Ordner unter `<Sandbox>/agent-pages`; die „Stimme“ ist ein kurzer
//!   Testton (440 Hz, eine Sekunde), geschrieben ueber eine Temp-Datei. Es startet keine Engine.
//! - **Aufnahme**: gibt es hier nicht (`recording()` ist `None`): nichts kann aufnehmen.
//! - **YouTube**: der echte Weg; mit `LVA_YOUTUBE_OEMBED_URL` zeigt der oEmbed-Abruf auf einen
//!   lokalen Teststand (nur in der Sandbox wirksam, siehe `oembed::oembed_base`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::{Host, PageRef, QueuedImport, Rendered};
use crate::managers::meetings::store::MeetingStore;
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::youtube::source::AddOptions;

/// Abtastrate und Laenge des Testtons.
const SAMPLE_RATE: u32 = 16_000;
const TONE_SECONDS: u32 = 1;

pub struct SandboxHost {
    store: Arc<MeetingStore>,
    dir: PathBuf,
    counter: AtomicU64,
}

impl SandboxHost {
    pub fn new(store: Arc<MeetingStore>, dir: PathBuf) -> Self {
        Self {
            store,
            dir,
            counter: AtomicU64::new(0),
        }
    }

    /// Der Ordner der Seiten dieser Sandbox: `<Ordner der Besprechungen>/agent-pages`.
    pub fn pages_dir_for(meetings_dir: &Path) -> PathBuf {
        meetings_dir.join("agent-pages")
    }

    fn page_dir(&self, id: &str) -> Result<PathBuf, String> {
        let valid = id
            .strip_prefix("page_")
            .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric()));
        if !valid {
            return Err("Ungültige Seiten-Kennung".to_string());
        }
        Ok(self.dir.join(id))
    }
}

/// Schreibt den Testton ueber eine Temp-Datei: nie eine halbe Datei unter dem Zielnamen.
pub fn write_test_tone(target: &Path) -> Result<u64, String> {
    let part = target.with_extension("wav.part");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let result = (|| -> Result<(), String> {
        let mut w = hound::WavWriter::create(&part, spec).map_err(|e| e.to_string())?;
        for i in 0..SAMPLE_RATE * TONE_SECONDS {
            let t = i as f32 / SAMPLE_RATE as f32;
            let v = (t * 440.0 * std::f32::consts::TAU).sin() * 0.3 * f32::from(i16::MAX);
            w.write_sample(v as i16).map_err(|e| e.to_string())?;
        }
        w.finalize().map_err(|e| e.to_string())?;
        std::fs::rename(&part, target).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => std::fs::metadata(target)
            .map(|m| m.len())
            .map_err(|e| e.to_string()),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

impl Host for SandboxHost {
    fn enqueue_import(&self, title: &str, source_path: &str) -> Result<QueuedImport, String> {
        let m = self
            .store
            .queue_enqueue(title, source_path, None)
            .map_err(|e| e.to_string())?;
        Ok(QueuedImport {
            meeting_id: m.id,
            title: m.title,
            status: m.status,
        })
    }

    fn create_page(&self, title: &str, text: &str) -> Result<PageRef, String> {
        let id = format!(
            "page_{}{}",
            chrono::Utc::now().timestamp_millis(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        );
        let dir = self.page_dir(&id)?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let state = json!({ "title": title, "text": text, "tab": "original" }).to_string();
        std::fs::write(dir.join("state.json"), state).map_err(|e| e.to_string())?;
        Ok(PageRef {
            id,
            title: title.to_string(),
        })
    }

    fn page_text(&self, page_id: &str) -> Result<String, String> {
        let raw = std::fs::read_to_string(self.page_dir(page_id)?.join("state.json"))
            .map_err(|e| e.to_string())?;
        Ok(serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default())
    }

    fn render_audio(
        &self,
        page_id: &str,
        _text: &str,
        file_name: &str,
    ) -> Result<Rendered, String> {
        let dir = self.page_dir(page_id)?;
        let mut target = dir.join(format!("{file_name}.wav"));
        let mut n = 2;
        while target.exists() {
            target = dir.join(format!("{file_name} ({n}).wav"));
            n += 1;
        }
        let bytes = write_test_tone(&target)?;
        Ok(Rendered {
            file_name: target
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or(file_name)
                .to_string(),
            path: target.to_string_lossy().into_owned(),
            bytes,
            format: "wav".to_string(),
        })
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        None
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions::production()
    }
}
