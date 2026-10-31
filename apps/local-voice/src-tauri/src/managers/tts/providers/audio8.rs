//! Audio8-TTS-Preview-0.6B als wählbarer Anbieter (Hülle).
//!
//! Entscheidung des Owners (31.10.2026): Audio8 kommt als ZUSÄTZLICHE, wählbare
//! Engine neben Fish, Piper und Phonon, weil die 0.6B-Gewichte laut Modellkarte
//! unter Apache 2.0 stehen und damit ohne Lizenzgebühr kommerziell nutzbar sind
//! (Fish-Gewichte und Phonon-40M sind nichtkommerziell). Qualität bleibt das
//! Hauptkriterium; Standard bleibt `fish`.
//!
//! Stand dieser Fassung: Deskriptor, Lizenzangaben, Modellordner-Prüfung und
//! Opt-in sind da. Die ONNX-Inferenz (`ort`) fehlt noch, bis die Ein- und
//! Ausgaben des Modells belegt sind (Tokenizer, Tensoren, Abtastrate,
//! Streaming); bis dahin meldet `synthesize` ehrlich `Unavailable`.
//!
//! Lizenz: nur die 0.6B-Variante ist Apache 2.0. Die 0.1B-Variante hat laut
//! Recherche eine eigene Lizenz mit Umsatzgrenze und ist NICHT gemeint. Die
//! NOTICE des Upstream-Modells ist noch nicht gelesen → `verified = false`.
//! Trainingsdaten sind nicht offengelegt. Kein Auto-Download.

use std::path::PathBuf;

use futures_util::future::BoxFuture;

use super::contract::{
    AudioClip, AudioEncoding, CancelToken, Device, LicenseInfo, ProviderCaps, ProviderDescriptor,
    ProviderHealth, ProviderTier, StreamingMode, SynthesisOptions, TtsError, TtsProvider,
    UseSubject, VoiceDescriptor,
};

pub const AUDIO8_ID: &str = "audio8";
/// Kennung der unterstützten Gewichte (geht in Cache-Schlüssel und Lizenz-Fingerabdruck ein).
pub const AUDIO8_MODEL: &str = "Audio8-TTS-Preview-0.6B";
/// Unterordner im Datenordner der App.
const MODEL_SUBDIR: &str = "tts/audio8";

/// Sprachen laut Modellkarte (11); die Hülle bietet nur die an, die die App kennt.
const LANGS: [&str; 11] = [
    "zh", "en", "nl", "fr", "de", "it", "ja", "ko", "pl", "es", "yue",
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Audio8Config {
    /// Ordner mit den ONNX-Dateien; `None` = `<Datenordner>/tts/audio8`.
    pub model_dir: Option<PathBuf>,
}

pub struct Audio8Provider {
    model_dir: Option<PathBuf>,
}

impl Audio8Provider {
    pub fn new(app_data: Option<PathBuf>, cfg: Audio8Config) -> Self {
        let model_dir = cfg
            .model_dir
            .or_else(|| app_data.map(|d| d.join(MODEL_SUBDIR)));
        Self { model_dir }
    }

    fn license() -> LicenseInfo {
        LicenseInfo {
            code: "Apache-2.0".to_string(),
            weights: "Apache-2.0 (nur 0.6B; 0.1B hat eine eigene Lizenz)".to_string(),
            non_commercial: false,
            verified: false,
            source_url: "https://huggingface.co/Audio8/Audio8-TTS-Preview-0.6b".to_string(),
            verified_on: None,
            note: "Nach Suchergebnissen Apache 2.0; Modellkarte, LICENSE und NOTICE sind noch \
                   nicht selbst gelesen. Trainingsdaten nicht offengelegt. Vor Auslieferung \
                   NOTICE-Pflichten übernehmen und keine fremden Stimmen klonen."
                .to_string(),
        }
    }

    fn model_present(&self) -> Result<PathBuf, String> {
        let dir = self
            .model_dir
            .clone()
            .ok_or_else(|| "kein Datenordner für das Audio8-Modell bekannt".to_string())?;
        let has_onnx = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "onnx"))
            })
            .unwrap_or(false);
        if has_onnx {
            Ok(dir)
        } else {
            Err(format!(
                "Audio8-Modell nicht gefunden (erwartet: ONNX-Dateien in {})",
                dir.display()
            ))
        }
    }
}

impl TtsProvider for Audio8Provider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: AUDIO8_ID.to_string(),
            label: "Audio8 (Apache 2.0)".to_string(),
            tier: ProviderTier::Experimental,
            opt_in: true,
            caps: ProviderCaps {
                languages: LANGS.map(String::from).to_vec(),
                streaming: StreamingMode::Sentence,
                formats: vec![AudioEncoding::Wav],
                // Abtastrate ist noch nicht belegt; mit der Inferenz eintragen.
                sample_rates: vec![],
                cloning: false,
                devices: vec![Device::Cpu],
                platforms: ["windows-x64", "linux-x64", "macos-aarch64", "macos-x64"]
                    .map(String::from)
                    .to_vec(),
                style_tags: false,
                voice_switching: false,
            },
            license: Self::license(),
        }
    }

    fn health_check(&self) -> BoxFuture<'_, ProviderHealth> {
        Box::pin(async move {
            let mut h = match self.model_present() {
                Err(reason) => ProviderHealth::unavailable(
                    reason,
                    Some(
                        "Modellordner manuell befüllen; die App lädt Audio8 nicht selbst herunter."
                            .to_string(),
                    ),
                ),
                Ok(_) => ProviderHealth::unavailable(
                    "Audio8-Inferenz ist in dieser Fassung noch nicht angeschlossen",
                    Some(
                        "Folgt, sobald die ONNX-Schnittstelle des Modells belegt ist.".to_string(),
                    ),
                ),
            };
            h.device = Some(Device::Cpu);
            h
        })
    }

    fn list_voices<'a>(
        &'a self,
        _language: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn synthesize<'a>(
        &'a self,
        _text: &'a str,
        _opts: &'a SynthesisOptions,
        _cancel: &'a CancelToken,
    ) -> BoxFuture<'a, Result<AudioClip, TtsError>> {
        Box::pin(async move {
            self.model_present().map_err(TtsError::unavailable)?;
            Err(TtsError::unavailable(
                "Audio8-Inferenz ist in dieser Fassung noch nicht angeschlossen",
            ))
        })
    }

    fn cache_tag(&self, voice: Option<&str>) -> String {
        format!("{AUDIO8_ID}:{AUDIO8_MODEL}:{}", voice.unwrap_or(""))
    }

    fn use_subject(&self) -> UseSubject {
        UseSubject {
            model: AUDIO8_MODEL.to_string(),
            base_url: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deskriptor_ist_opt_in_und_kommerziell_frei_aber_ungeprueft() {
        let p = Audio8Provider::new(None, Audio8Config::default());
        let d = p.descriptor();
        assert_eq!(d.id, "audio8");
        assert!(d.opt_in);
        assert!(!d.license.non_commercial);
        assert!(!d.license.verified);
        assert!(d.caps.languages.iter().any(|l| l == "de"));
    }

    #[tokio::test]
    async fn ohne_modell_ist_der_anbieter_nicht_verfuegbar() {
        let dir = std::env::temp_dir().join("lva-audio8-test-leer");
        let p = Audio8Provider::new(
            None,
            Audio8Config {
                model_dir: Some(dir),
            },
        );
        let h = p.health_check().await;
        assert!(!h.available);
        assert!(h.reason.unwrap().contains("nicht gefunden"));
        let cancel = CancelToken::new("t");
        let err = p
            .synthesize("Hallo", &SynthesisOptions::default(), &cancel)
            .await
            .unwrap_err();
        assert_eq!(err.kind, super::super::contract::TtsErrorKind::Unavailable);
    }
}
