//! D3 (Goal issues-abschluss #70, M7 = #69): Bildanalyse von Folien mit dem lokalen
//! Sprachmodell -- die Entscheidung, WANN sie laufen darf.
//!
//! Gemma 4 E4B kann mit seinem Bild-Projektor (`mmproj`, 990 MB, optionaler
//! Katalog-Download) Zahlen und Tabellen von Folien fehlerfrei lesen, wo Windows-OCR
//! sie verliert (Spike: 60/60 gegen 47/60 Zahlen). Der Projektor kostet ~0,8 GB
//! Grafikspeicher, solange der Server mit `--mmproj` laeuft, und die Bildanalyse ist
//! mit Grafikkarte 2 s je Folie, auf der CPU 98 s (unbrauchbar). Deshalb:
//!
//! - **Standard aus**, nur auf Wunsch (Einstellung "Bildanalyse fuer Folien").
//! - **Nur mit GPU**: ein CPU-Backend oder eine nicht messbare dedizierte Karte
//!   bietet sie gar nicht an; unter [`VISION_MIN_FREE_VRAM_MB`] freiem Grafikspeicher
//!   ebenfalls nicht.
//! - **Nur fuer Folienauftraege**: der Server startet dafuer mit Projektor
//!   ([`super::ensure_local_vision`]) und wird danach beendet
//!   ([`super::release_vision`]); der naechste Chat startet wieder ohne.
//!
//! Wer die Bildanalyse will und sie nicht bekommt (kein Projektor, keine GPU, zu wenig
//! Speicher), faellt auf die Windows-OCR zurueck: die Folien sind da, nur ohne
//! Gemma-Text und ohne Beschreibung ([`VisionPlan::Unavailable`] nennt den Grund).
//!
//! Kein Modul hier darf `settings::get_settings(&AppHandle)` rufen: der Schalter
//! kommt als Parameter vom Aufrufer (der Befehlsschicht).

use super::context::free_dedicated_vram_mb;

/// Das Modell, zu dem der Projektor gehoert (Katalog-Kennung). Die Bildanalyse laeuft
/// immer mit diesem, auch wenn der Chat ein anderes Modell nutzt.
pub const VISION_MODEL_ID: &str = "llm-gemma4-e4b-q4";
/// Der Projektor (Katalogzweck `llm-projector`).
pub const VISION_PROJECTOR_ID: &str = "llm-gemma4-e4b-mmproj";
/// So viel Grafikspeicher (MiB) muss frei sein: 4,7 GB gemessen (Modell 3,9 GB +
/// Projektor 0,8 GB) plus Reserve fuer Bild-Token und Kontext.
pub const VISION_MIN_FREE_VRAM_MB: u64 = 6 * 1024;

/// Fehlercode (Praefix `<code>: <Text>`): Projektor nicht geladen.
pub const CODE_NO_PROJECTOR: &str = "no_projector";
/// Gruende, warum die Bildanalyse trotz Schalter nicht laeuft (`VisionPlan::code`).
pub const CODE_NO_MODEL: &str = "vision_no_model";
pub const CODE_NO_PROJECTOR_FILE: &str = "vision_no_projector";
pub const CODE_NO_GPU: &str = "vision_no_gpu";
pub const CODE_LOW_VRAM: &str = "vision_low_vram";

/// Was fuer einen Folienauftrag gilt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisionPlan {
    /// Der Nutzer will keine Bildanalyse.
    Off,
    /// Alles da: der Auftrag startet den Server mit Projektor.
    Ready,
    /// Gewuenscht, aber nicht moeglich; es bleibt bei der Windows-OCR.
    Unavailable(&'static str),
}

impl VisionPlan {
    /// Kurzer Code fuer Oberflaeche und Log: `off`, `ready` oder der Grund.
    pub fn code(self) -> &'static str {
        match self {
            VisionPlan::Off => "off",
            VisionPlan::Ready => "ready",
            VisionPlan::Unavailable(code) => code,
        }
    }
}

/// Die Entscheidung, rein: Schalter, Dateien, Backend und freier Grafikspeicher rein,
/// Plan raus. `backend` ist die Kennung der gewaehlten Laufzeit (`cpu`, `vulkan`, `cuda`,
/// `metal`); `None` = unbekannt (keine Laufzeit) zaehlt wie CPU. `free_vram_mb` ist der
/// freie Speicher der knappsten dedizierten Karte; `None` = nicht messbar (iGPU,
/// macOS): dann wird nichts geraten, die Bildanalyse bleibt aus.
pub fn decide_vision(
    enabled: bool,
    model_present: bool,
    projector_present: bool,
    backend: Option<&str>,
    free_vram_mb: Option<u64>,
) -> VisionPlan {
    if !enabled {
        return VisionPlan::Off;
    }
    // Zuerst die Grafikkarte: ohne sie hat auch der Download des Projektors keinen Zweck,
    // die Oberflaeche soll ihn dann gar nicht anbieten.
    if backend.is_none_or(|b| b == "cpu") {
        return VisionPlan::Unavailable(CODE_NO_GPU);
    }
    match free_vram_mb {
        None => return VisionPlan::Unavailable(CODE_NO_GPU),
        Some(free) if free < VISION_MIN_FREE_VRAM_MB => {
            return VisionPlan::Unavailable(CODE_LOW_VRAM)
        }
        Some(_) => {}
    }
    if !model_present {
        return VisionPlan::Unavailable(CODE_NO_MODEL);
    }
    if !projector_present {
        return VisionPlan::Unavailable(CODE_NO_PROJECTOR_FILE);
    }
    VisionPlan::Ready
}

/// Zustand fuer die Oberflaeche (Einstellung "Bildanalyse fuer Folien").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionFacts {
    pub model_present: bool,
    pub projector_present: bool,
    pub downloading: bool,
    /// Gewaehlte Laufzeit (`cpu`, `vulkan`, ...); `None`, wenn keine installiert ist.
    pub backend: Option<String>,
    pub free_vram_mb: Option<u64>,
}

impl VisionFacts {
    /// Plan, WENN der Nutzer die Bildanalyse einschaltet (fuer die Frage "anbieten?").
    pub fn plan_if_enabled(&self) -> VisionPlan {
        decide_vision(
            true,
            self.model_present,
            self.projector_present,
            self.backend.as_deref(),
            self.free_vram_mb,
        )
    }
}

/// Misst, was die Entscheidung braucht. Blockierend im GPU-Teil (DXGI, Millisekunden),
/// darum auf einem Blocking-Thread. Fehler beim Messen fuehren zu "nicht messbar",
/// nie zu einem Abbruch.
pub async fn gather_facts() -> VisionFacts {
    let (runtime, backend) = match super::RUNTIME.get() {
        Some(runtime) => {
            let backend = runtime.resolve_runtime().await.ok().map(|(_, b, _)| b);
            (Some(runtime.clone()), backend)
        }
        None => (None, None),
    };
    let free_vram_mb = tokio::task::spawn_blocking(|| {
        free_dedicated_vram_mb(&super::resources::system_memory().gpus)
    })
    .await
    .ok()
    .flatten();
    let present = |path: Option<std::path::PathBuf>| path.is_some_and(|p| p.is_file());
    VisionFacts {
        model_present: present(runtime.as_ref().and_then(|r| r.model_path(VISION_MODEL_ID))),
        projector_present: present(
            runtime
                .as_ref()
                .and_then(|r| r.projector_path(VISION_PROJECTOR_ID)),
        ),
        downloading: runtime
            .as_ref()
            .is_some_and(|r| r.is_downloading_id(VISION_PROJECTOR_ID)),
        backend,
        free_vram_mb,
    }
}

/// Plan fuer einen Folienauftrag jetzt: `enabled` ist die Einstellung des Nutzers.
pub async fn vision_plan(enabled: bool) -> VisionPlan {
    if !enabled {
        return VisionPlan::Off;
    }
    let facts = gather_facts().await;
    decide_vision(
        true,
        facts.model_present,
        facts.projector_present,
        facts.backend.as_deref(),
        facts.free_vram_mb,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024;

    /// Standard aus: der Schalter entscheidet vor allem anderen, auch wenn alles da ist.
    #[test]
    fn off_stays_off_whatever_the_machine_has() {
        assert_eq!(
            decide_vision(false, true, true, Some("cuda"), Some(20 * GB)),
            VisionPlan::Off
        );
        assert_eq!(
            decide_vision(false, false, false, None, None),
            VisionPlan::Off
        );
    }

    /// Mit Schalter, Modell, Projektor, GPU und genug Speicher: los.
    #[test]
    fn everything_present_on_a_gpu_is_ready() {
        for backend in ["cuda", "vulkan", "metal"] {
            assert_eq!(
                decide_vision(true, true, true, Some(backend), Some(8 * GB)),
                VisionPlan::Ready,
                "{backend}"
            );
        }
        // Genau an der Grenze reicht es.
        assert_eq!(
            decide_vision(
                true,
                true,
                true,
                Some("cuda"),
                Some(VISION_MIN_FREE_VRAM_MB)
            ),
            VisionPlan::Ready
        );
    }

    /// Fallback: kein Projektor / kein Modell / keine GPU / zu wenig Speicher = OCR bleibt.
    #[test]
    fn every_missing_precondition_falls_back_with_a_reason() {
        let some = Some("cuda");
        assert_eq!(
            decide_vision(true, true, false, some, Some(20 * GB)),
            VisionPlan::Unavailable(CODE_NO_PROJECTOR_FILE)
        );
        assert_eq!(
            decide_vision(true, false, true, some, Some(20 * GB)),
            VisionPlan::Unavailable(CODE_NO_MODEL)
        );
        assert_eq!(
            decide_vision(true, true, true, Some("cpu"), Some(20 * GB)),
            VisionPlan::Unavailable(CODE_NO_GPU)
        );
        assert_eq!(
            decide_vision(true, false, false, Some("cpu"), None),
            VisionPlan::Unavailable(CODE_NO_GPU),
            "ohne GPU nennt die Oberflaeche nie einen Download als Grund"
        );
        assert_eq!(
            decide_vision(true, true, true, None, Some(20 * GB)),
            VisionPlan::Unavailable(CODE_NO_GPU),
            "keine Laufzeit gilt wie CPU"
        );
        assert_eq!(
            decide_vision(true, true, true, some, None),
            VisionPlan::Unavailable(CODE_NO_GPU),
            "nicht messbar (iGPU, macOS): nichts raten"
        );
        assert_eq!(
            decide_vision(true, true, true, some, Some(VISION_MIN_FREE_VRAM_MB - 1)),
            VisionPlan::Unavailable(CODE_LOW_VRAM)
        );
    }

    #[test]
    fn plan_codes_are_stable_for_the_ui() {
        assert_eq!(VisionPlan::Off.code(), "off");
        assert_eq!(VisionPlan::Ready.code(), "ready");
        assert_eq!(VisionPlan::Unavailable(CODE_NO_GPU).code(), "vision_no_gpu");
        let facts = VisionFacts {
            model_present: true,
            projector_present: false,
            downloading: false,
            backend: Some("cuda".into()),
            free_vram_mb: Some(20 * GB),
        };
        assert_eq!(facts.plan_if_enabled().code(), "vision_no_projector");
    }

    /// Katalog: der Projektor ist ein eigener Zweck mit Groesse, Pruefsumme, Lizenz und
    /// erscheint nie in der Liste der Sprachmodelle.
    #[test]
    fn the_projector_is_a_catalog_entry_with_checksum_and_license() {
        use crate::catalog::{tts_entries, Purpose};
        let entries = tts_entries(Purpose::LlmProjector);
        let entry = entries
            .iter()
            .find(|e| e.id == VISION_PROJECTOR_ID)
            .expect("Projektor im Katalog");
        let file = &entry.files[0];
        assert_eq!(file.size_bytes, 990_372_672);
        assert_eq!(
            file.sha256.as_deref(),
            Some("ddf46c21d7078e95338cfc22306b19b276a29a5ad089023449dd54d4b6170a51")
        );
        assert!(file.url.ends_with("/mmproj-F16.gguf"), "{}", file.url);
        assert_eq!(entry.license.as_deref(), Some("Apache-2.0"));
        assert!(entry.license_url.is_some());
        assert!(tts_entries(Purpose::LlmModel)
            .iter()
            .all(|e| e.id != VISION_PROJECTOR_ID));
        assert!(
            tts_entries(Purpose::LlmModel)
                .iter()
                .any(|e| e.id == VISION_MODEL_ID),
            "das Modell des Projektors steht im Katalog"
        );
    }
}
