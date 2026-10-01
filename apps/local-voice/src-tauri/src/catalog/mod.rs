//! The bundled, offline model catalog.
//!
//! `catalog.json` is generated at build time by `scripts/gen_catalog.py` from the
//! `handy-computer` Hugging Face org (card `transcribe_cpp` capabilities +
//! benchmarks, a GGUF header probe for name/params, and local curation for the
//! recommended set). It is compiled into the binary so Handy ships a complete
//! model list with zero network access.
//!
//! Each entry is normalised into a [`ModelDescriptor`] — the same source-agnostic
//! shape every other producer (HF discovery, on-disk scans, the legacy table)
//! yields — so the catalog is "just another producer". Its explicit `capabilities`
//! map becomes a [`CapabilityProbe`] with confident `Some(..)` values; the runtime
//! `GgufHeaderProber` is the same shape with `None` where a header omits a key,
//! which is why the two are interchangeable (the catalog is a baked probe).

use std::collections::HashMap;

use once_cell::sync::Lazy;
use serde::Deserialize;

use crate::managers::model::{
    default_quant_file, EngineType, ModelDescriptor, ModelSource, QuantFile,
};
use crate::managers::model_capabilities::{CapabilityProbe, Compatibility};

#[derive(Deserialize)]
struct CatalogRoot {
    /// Base URLs tried in order when the Hugging Face download fails. The full
    /// file URL is `{mirror}/{repo_id}/{revision}/{filename}` — the same three
    /// values that form the HF resolve URL, so a mirror is a plain static host.
    #[serde(default)]
    mirrors: Vec<String>,
    models: Vec<CatalogModel>,
}

/// What a catalog entry is for. Defaults to [`Purpose::Asr`] when the field is
/// absent from `catalog.json` — every one of the 68 pre-existing entries omits
/// it and stays exactly as it was. `TtsVoice`/`TtsRuntime` entries are Piper
/// downloads (runtime binary, voice files): [`CATALOG`] below filters to `Asr`
/// only, so they never reach [`ModelDescriptor`] (an ASR/HF-shaped producer);
/// `managers::tts::models` reads them separately via [`tts_entries`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    #[default]
    Asr,
    TtsVoice,
    TtsRuntime,
    /// `llama-server`-Paket je Plattform und Backend (`managers::llm`).
    LlmRuntime,
    /// GGUF-Sprachmodell fuer den lokalen Server.
    LlmModel,
    /// M4-P4b: GGUF-Embedding-Modell fuer den zweiten Server (Besprechungs-
    /// suche). Getrennt von `LlmModel`, damit es nie als Chat-Modell erscheint.
    LlmEmbedding,
    /// M3-P3a: Sprechertrennungs-Modell (Kategorie "Sprechertrennung",
    /// `managers::meetings::diarize`). Nie im ASR-Katalog: es transkribiert nicht.
    Diarization,
}

/// One model as written in `catalog.json`. Only the fields the descriptor needs
/// are declared; serde ignores the rest (slug, family, license, …).
#[derive(Deserialize)]
struct CatalogModel {
    /// HF repo id for an `asr` entry, e.g. `handy-computer/whisper-small-gguf`.
    /// For `tts-*` entries this is just a stable identifier — their files carry
    /// an explicit [`QuantFile::url`] instead of being HF-resolved from it.
    id: String,
    /// Commit sha the catalog's sizes/hashes were generated from. Both HF
    /// acquisition and mirror keys use it, so downloaded bytes provably match
    /// the hashes regardless of source. Cache *lookup* additionally falls back
    /// to `main` (see `hf_cached_path`) so downloads that predate pinning keep
    /// resolving. Unused by `tts-*` entries.
    revision: Option<String>,
    name: String,
    description: String,
    architecture: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    capabilities: CatalogCaps,
    speed_score: Option<f32>,
    accuracy_score: Option<f32>,
    files: Vec<QuantFile>,
    default_quant: Option<String>,
    recommended_rank: Option<u32>,
    /// Part of the small curated onboarding set (badged "Recommended"). Distinct
    /// from `recommended_rank`, which only orders the full list.
    #[serde(default)]
    recommended: bool,
    /// See [`Purpose`].
    #[serde(default)]
    purpose: Purpose,
    /// Kuratierte Merkmale ("schnell", "zusammenfassung", "mehrsprachig") --
    /// fuer Sprachmodelle, damit die Oberflaeche sagen kann, wofuer eines
    /// taugt, ohne Zahlen zu erfinden.
    #[serde(default)]
    tags: Vec<String>,
    /// Lizenz laut Quelle (SPDX-Kennung, SPDX-Ausdruck oder Name) -- Issue #7.
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    license_url: Option<String>,
    /// Herkunfts-/Mischlizenzhinweis (z. B. aus einer Forschungslizenz feinabgestimmt).
    #[serde(default)]
    license_note: Option<String>,
    /// Ausdruecklich nur nicht-kommerziell nutzbar (zusaetzlich zur Erkennung
    /// am Lizenznamen, etwa `cc-by-nc-4.0`).
    #[serde(default)]
    non_commercial: bool,
    /// Grund, warum dieser Eintrag auf seiner Plattform nicht startet (z. B.
    /// `incomplete_archive`); die Oberflaeche bietet ihn dann nicht zum
    /// Download an (Issue #29).
    #[serde(default)]
    unsupported_reason: Option<String>,
}

/// Nicht-kommerzielle Lizenz? Ausdruecklich markiert oder `-NC-` im Namen
/// (`cc-by-nc-4.0`, `CC-BY-NC-SA-4.0`). Gleiche Regel wie `scripts/lib/notices-core.mjs`.
pub fn license_is_non_commercial(license: Option<&str>, explicit: bool) -> bool {
    explicit
        || license.is_some_and(|l| {
            l.to_ascii_lowercase()
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|part| part == "nc")
        })
}

#[derive(Deserialize, Default)]
struct CatalogCaps {
    #[serde(default)]
    streaming: bool,
    #[serde(default)]
    translate: bool,
    #[serde(default)]
    lang_detect: bool,
    // `timestamps` (a string enum) is present in the catalog but has no
    // `CapabilityProbe` field yet — wire it through when the probe gains one.
}

impl From<&CatalogModel> for ModelDescriptor {
    fn from(m: &CatalogModel) -> Self {
        // The default download file. Its name is folded into the id so a catalog
        // entry collides (dedups) with the very same file later discovered in
        // the HF cache — both compute `"{repo_id}/{filename}"`.
        let default_filename = default_quant_file(&m.files, m.default_quant.as_deref())
            .map(|f| f.filename.clone())
            .unwrap_or_default();

        ModelDescriptor {
            id: format!("{}/{}", m.id, default_filename),
            source: ModelSource::HuggingFace {
                repo_id: m.id.clone(),
                // Acquire at the pin: `resolve/<sha>` is immutable (CDN-friendly)
                // and guarantees the bytes match the catalog's hashes. `main`
                // only remains as a lookup fallback for pre-pinning caches.
                revision: m.revision.clone().unwrap_or_else(|| "main".to_string()),
            },
            name: m.name.clone(),
            description: m.description.clone(),
            engine_type: EngineType::TranscribeCpp,
            caps: CapabilityProbe {
                verdict: Compatibility::Compatible, // curated org models we ship support for
                display_name: None,
                architecture: m.architecture.clone(),
                variant: None,
                languages: Some(m.languages.clone()),
                supports_streaming: Some(m.capabilities.streaming),
                supports_translation: Some(m.capabilities.translate),
                supports_language_detect: Some(m.capabilities.lang_detect),
            },
            files: m.files.clone(),
            default_quant: m.default_quant.clone(),
            // catalog scores are 0–100; ModelInfo / the UI bars use 0.0–1.0.
            speed_score: m.speed_score.unwrap_or(0.0) / 100.0,
            accuracy_score: m.accuracy_score.unwrap_or(0.0) / 100.0,
            recommended_rank: m.recommended_rank,
            recommended: m.recommended,
            license: m.license.clone(),
            license_url: m.license_url.clone(),
            license_non_commercial: license_is_non_commercial(
                m.license.as_deref(),
                m.non_commercial,
            ),
        }
    }
}

/// The raw parsed catalog. Kept alive (not consumed) so mirror metadata that
/// deliberately stays out of [`ModelDescriptor`] can be looked up separately.
static ROOT: Lazy<CatalogRoot> = Lazy::new(|| {
    serde_json::from_str(include_str!("catalog.json"))
        .expect("bundled catalog.json is valid JSON matching the catalog schema")
});

/// The bundled catalog, parsed once and normalised into descriptors.
///
/// Filtered to `purpose == Asr` — the one place that happens. TTS entries
/// never reach [`ModelDescriptor`] (an ASR/Hugging-Face-shaped producer);
/// [`tts_entries`] reads them separately.
pub static CATALOG: Lazy<Vec<ModelDescriptor>> = Lazy::new(|| {
    ROOT.models
        .iter()
        .filter(|m| m.purpose == Purpose::Asr)
        .map(ModelDescriptor::from)
        .collect()
});

/// One file belonging to a `tts-runtime`/`tts-voice` catalog entry, with its
/// download URL already resolved. Unlike ASR files (always Hugging-Face
/// resolved from the entry's `id`+`revision`), a TTS file only ever carries an
/// explicit [`QuantFile::url`] — some are GitHub release assets, and even the
/// Hugging-Face-hosted Piper voices live at repo paths that differ from the
/// flat local filename we want, so there is no single reconstruction rule.
/// A file without a `url` is dropped rather than guessed at.
pub struct TtsCatalogFile {
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub sha256: Option<String>,
}

/// One `tts-runtime`/`tts-voice` catalog entry, read directly by
/// `managers::tts::models` (bypassing [`ModelDescriptor`] entirely).
pub struct TtsCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub files: Vec<TtsCatalogFile>,
    pub tags: Vec<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub license_non_commercial: bool,
    /// Siehe `CatalogModel::unsupported_reason`.
    pub unsupported_reason: Option<String>,
}

/// All bundled catalog entries of the given TTS `purpose`, in catalog order.
/// Files lacking a `url` (see [`TtsCatalogFile`]) are silently dropped — a
/// data bug in `catalog.json`, not something to paper over with a guessed URL.
pub fn tts_entries(purpose: Purpose) -> Vec<TtsCatalogEntry> {
    ROOT.models
        .iter()
        .filter(|m| m.purpose == purpose)
        .map(|m| TtsCatalogEntry {
            id: m.id.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            tags: m.tags.clone(),
            license: m.license.clone(),
            license_url: m.license_url.clone(),
            license_non_commercial: license_is_non_commercial(
                m.license.as_deref(),
                m.non_commercial,
            ),
            unsupported_reason: m.unsupported_reason.clone(),
            files: m
                .files
                .iter()
                .filter_map(|f| {
                    Some(TtsCatalogFile {
                        filename: f.filename.clone(),
                        url: f.url.clone()?,
                        size_bytes: f.size_bytes,
                        sha256: f.sha256.clone(),
                    })
                })
                .collect(),
        })
        .collect()
}

/// A mirror copy of a catalog model's default file, with the expected content
/// hash for end-to-end verification. Mirrors are untrusted bit-pipes: the
/// sha256 here (from the catalog compiled into the binary) is the trust anchor,
/// which is why it is mandatory — a file without one is never offered from a
/// mirror at all.
pub struct MirrorFile {
    pub url: String,
    pub sha256: String,
    /// Catalog size — drives progress totals and resume sanity checks.
    pub size_bytes: u64,
}

/// Ordered mirror URLs for a catalog model's file — any listed quant, not just
/// the default — or empty when the model isn't from the catalog / no mirrors
/// are configured. `model_id` is the registry id (`"{repo_id}/{filename}"`).
/// (The mirror may only host default quants; a miss there just 404s and the
/// caller reports it, so listing every quant here costs nothing.)
pub fn mirror_fallbacks(model_id: &str) -> Vec<MirrorFile> {
    let Some((m, file)) = ROOT.models.iter().find_map(|m| {
        m.files
            .iter()
            .find(|f| format!("{}/{}", m.id, f.filename) == model_id)
            .map(|f| (m, f))
    }) else {
        return Vec::new();
    };
    let Some(revision) = m.revision.as_deref() else {
        return Vec::new();
    };
    // No hash means no verification means no mirror: never fetch from an
    // untrusted host without the catalog trust anchor.
    let Some(sha256) = file.sha256.as_deref() else {
        return Vec::new();
    };
    ROOT.mirrors
        .iter()
        .map(|base| MirrorFile {
            url: format!(
                "{}/{}/{}/{}",
                base.trim_end_matches('/'),
                m.id,
                revision,
                file.filename
            ),
            sha256: sha256.to_string(),
            size_bytes: file.size_bytes,
        })
        .collect()
}

/// The catalog descriptor + specific `files[]` entry owning `filename`,
/// matched across every listed quant (not just the default). `repo_id`, when
/// given, must also match — the HF-cache scan uses it to keep a foreign repo
/// that happens to reuse a catalog filename from masquerading as ours.
pub fn file_in_catalog(
    filename: &str,
    repo_id: Option<&str>,
) -> Option<(&'static ModelDescriptor, &'static QuantFile)> {
    let catalog: &'static Vec<ModelDescriptor> = Lazy::force(&CATALOG);
    catalog.iter().find_map(|d| {
        if let Some(repo) = repo_id {
            match &d.source {
                ModelSource::HuggingFace { repo_id: r, .. } if r == repo => {}
                _ => return None,
            }
        }
        d.files
            .iter()
            .find(|f| f.filename == filename)
            .map(|f| (d, f))
    })
}

/// Editorial recommended rank keyed by descriptor id (the same id the model
/// registry uses). Built once from the catalog.
static RANK_BY_ID: Lazy<HashMap<String, u32>> = Lazy::new(|| {
    CATALOG
        .iter()
        .filter_map(|d| d.recommended_rank.map(|r| (d.id.clone(), r)))
        .collect()
});

/// Recommended rank for a model id (lower = higher priority). Returns
/// `u32::MAX` for unranked/unknown ids so they sort last in an ascending sort.
pub fn rank_of(model_id: &str) -> u32 {
    RANK_BY_ID.get(model_id).copied().unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::model_capabilities::KNOWN_ARCHES;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_parses_and_is_nonempty() {
        assert!(!CATALOG.is_empty(), "bundled catalog should contain models");
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|d| d.id.as_str()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "catalog descriptor ids must be unique");
    }

    #[test]
    fn scores_are_normalised_0_to_1() {
        for d in CATALOG.iter() {
            assert!((0.0..=1.0).contains(&d.speed_score), "{} speed", d.id);
            assert!((0.0..=1.0).contains(&d.accuracy_score), "{} acc", d.id);
        }
    }

    #[test]
    fn every_catalog_model_has_mirror_fallbacks_with_hashes() {
        // The mirror fallback is the safety net for HF outages and blocked
        // networks; a catalog entry without one (missing revision, missing
        // sha256, empty mirrors) silently loses that net.
        for d in CATALOG.iter() {
            let mirrors = mirror_fallbacks(&d.id);
            assert!(!mirrors.is_empty(), "{}: no mirror fallbacks", d.id);
            for m in &mirrors {
                assert!(
                    m.sha256.len() == 64,
                    "{}: mirror entry lacks a sha256",
                    d.id
                );
                assert!(m.size_bytes > 0, "{}: mirror entry lacks a size", d.id);
                assert!(m.url.starts_with("https://"), "{}: bad url {}", d.id, m.url);
            }
        }
    }

    /// Requirement: a catalog entry without `purpose` deserializes as `asr`,
    /// and ALL 68 pre-existing entries are still exactly that — none of them
    /// were accidentally reinterpreted as `tts-voice`/`tts-runtime`.
    #[test]
    fn catalog_without_purpose_stays_asr() {
        assert_eq!(
            CATALOG.len(),
            68,
            "the 68 pre-existing entries must still be the only `asr` ones \
             (new tts-* entries must be excluded from CATALOG)"
        );
    }

    #[test]
    fn purpose_defaults_to_asr_when_absent_from_json() {
        let json = r#"{
            "id": "some/repo", "name": "n", "description": "d",
            "architecture": null, "files": [],
            "revision": null, "default_quant": null, "recommended_rank": null
        }"#;
        let m: CatalogModel = serde_json::from_str(json).unwrap();
        assert_eq!(m.purpose, Purpose::Asr);
    }

    /// Die Sprachmodell-Eintraege: je Plattform eine Laufzeit, vier Modelle.
    /// Sie duerfen nie im ASR-Katalog auftauchen, und jede Datei traegt eine
    /// Pruefsumme -- ohne sie waere der Download nicht verifizierbar.
    #[test]
    fn llm_entries_are_complete_and_stay_out_of_the_asr_catalog() {
        let runtimes = tts_entries(Purpose::LlmRuntime);
        assert!(
            runtimes.iter().any(|e| e.id == "llm-runtime-windows-x64-vulkan"),
            "Windows-Vulkan-Laufzeit fehlt"
        );
        assert!(runtimes.iter().any(|e| e.id == "llm-runtime-macos-aarch64"));
        let models = tts_entries(Purpose::LlmModel);
        assert!(models.len() >= 4, "vier Modelle erwartet, {}", models.len());
        for entry in runtimes.iter().chain(models.iter()) {
            assert!(!entry.files.is_empty(), "{}: keine Datei", entry.id);
            for f in &entry.files {
                assert!(f.sha256.is_some(), "{}: {} ohne Pruefsumme", entry.id, f.filename);
                assert!(f.size_bytes > 0, "{}: {} ohne Groesse", entry.id, f.filename);
            }
        }
        assert!(
            models.iter().all(|m| !m.tags.is_empty()),
            "jedes Sprachmodell traegt Merkmale"
        );
        assert!(
            CATALOG.iter().all(|d| !d.id.starts_with("llm-")),
            "Sprachmodell-Eintraege gehoeren nicht in den ASR-Katalog"
        );
    }

    /// M4-P4b: das Embedding-Modell hat einen eigenen Zweck -- es darf weder
    /// als Sprachmodell (Chat) noch im ASR-Katalog auftauchen -- und traegt
    /// Groesse und Pruefsumme laut M4-Spike.
    #[test]
    fn the_embedding_model_has_its_own_purpose_and_a_checksum() {
        let entries = tts_entries(Purpose::LlmEmbedding);
        let bge = entries
            .iter()
            .find(|e| e.id == "emb-bge-m3-q8")
            .expect("emb-bge-m3-q8 im Katalog");
        let file = &bge.files[0];
        assert_eq!(file.size_bytes, 634_553_760);
        assert_eq!(
            file.sha256.as_deref(),
            Some("950f4a8e5e19477a6d3c26d2f162233c20002c601f75e4b002e3239997821167")
        );
        assert!(file.url.starts_with("https://huggingface.co/gpustack/bge-m3-GGUF/"));
        assert!(tts_entries(Purpose::LlmModel)
            .iter()
            .all(|e| e.id != "emb-bge-m3-q8"));
        assert!(CATALOG.iter().all(|d| d.id != "emb-bge-m3-q8"));
    }

    #[test]
    fn purpose_tts_variants_parse_and_are_excluded_from_catalog() {
        let voice_json = r#"{
            "id": "rhasspy/piper-voices", "name": "n", "description": "d",
            "architecture": null, "files": [],
            "revision": null, "default_quant": null, "recommended_rank": null,
            "purpose": "tts-voice"
        }"#;
        let voice: CatalogModel = serde_json::from_str(voice_json).unwrap();
        assert_eq!(voice.purpose, Purpose::TtsVoice);
        assert_ne!(voice.purpose, Purpose::Asr);

        let runtime_json = r#"{
            "id": "piper-runtime-windows-x64", "name": "n", "description": "d",
            "architecture": null, "files": [],
            "revision": null, "default_quant": null, "recommended_rank": null,
            "purpose": "tts-runtime"
        }"#;
        let runtime: CatalogModel = serde_json::from_str(runtime_json).unwrap();
        assert_eq!(runtime.purpose, Purpose::TtsRuntime);
    }

    #[test]
    fn tts_entries_only_returns_files_with_a_url() {
        let runtimes = tts_entries(Purpose::TtsRuntime);
        assert!(
            !runtimes.is_empty(),
            "expected at least one tts-runtime entry"
        );
        for entry in &runtimes {
            for file in &entry.files {
                assert!(
                    file.url.starts_with("https://"),
                    "{}: bad url {}",
                    entry.id,
                    file.url
                );
            }
        }

        let voices = tts_entries(Purpose::TtsVoice);
        assert_eq!(voices.len(), 10, "expected the 10 curated Piper voices");
        for entry in &voices {
            assert_eq!(
                entry.files.len(),
                2,
                "{}: a Piper voice ships an .onnx + .onnx.json",
                entry.id
            );
        }
    }

    #[test]
    fn catalog_architectures_are_known_to_capability_probe() {
        let missing: BTreeSet<&str> = CATALOG
            .iter()
            .filter_map(|d| d.caps.architecture.as_deref())
            .filter(|arch| !KNOWN_ARCHES.contains(arch))
            .collect();

        assert!(
            missing.is_empty(),
            "catalog architecture(s) missing from KNOWN_ARCHES: {:?}",
            missing
        );
    }

    // ── Lizenzangaben (Issue #7) ─────────────────────────────────────────

    #[test]
    fn license_is_non_commercial_erkennt_nc_lizenzen() {
        assert!(license_is_non_commercial(Some("cc-by-nc-4.0"), false));
        assert!(license_is_non_commercial(Some("CC-BY-NC-SA-4.0"), false));
        assert!(license_is_non_commercial(Some("Blizzard-2013-Research-Licence"), true));
        assert!(!license_is_non_commercial(Some("cc-by-4.0"), false));
        assert!(!license_is_non_commercial(Some("Apache-2.0"), false));
        assert!(!license_is_non_commercial(Some("Unclear"), false));
        assert!(!license_is_non_commercial(None, false));
    }

    #[test]
    fn jeder_asr_eintrag_nennt_eine_konkrete_lizenz() {
        for d in CATALOG.iter() {
            let l = d.license.as_deref().unwrap_or("");
            assert!(
                !l.is_empty() && !l.eq_ignore_ascii_case("other"),
                "{}: Lizenz fehlt oder ist nur \"other\" ({l:?})",
                d.id
            );
        }
    }

    #[test]
    fn jeder_nicht_asr_eintrag_nennt_eine_lizenz() {
        for purpose in [
            Purpose::TtsVoice,
            Purpose::TtsRuntime,
            Purpose::LlmRuntime,
            Purpose::LlmModel,
            Purpose::LlmEmbedding,
            Purpose::Diarization,
        ] {
            for e in tts_entries(purpose) {
                assert!(
                    e.license.as_deref().is_some_and(|l| !l.trim().is_empty()),
                    "{}: Lizenz fehlt",
                    e.id
                );
                assert!(e.license_url.is_some(), "{}: Lizenz-Link fehlt", e.id);
            }
        }
    }

    #[test]
    fn canary_1b_ist_als_nicht_kommerziell_markiert_andere_nicht() {
        let canary = CATALOG
            .iter()
            .find(|d| d.id == "handy-computer/canary-1b-gguf/canary-1b-Q5_K_M.gguf")
            .or_else(|| CATALOG.iter().find(|d| d.id.contains("canary-1b-gguf")))
            .expect("Canary 1B steht im Katalog");
        assert!(canary.license_non_commercial);
        assert_eq!(canary.license.as_deref(), Some("cc-by-nc-4.0"));
        let nc: Vec<&str> = CATALOG
            .iter()
            .filter(|d| d.license_non_commercial)
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(nc.len(), 1, "nur Canary 1B ist im ASR-Katalog nicht-kommerziell: {nc:?}");
    }

    #[test]
    fn nicht_kommerzielle_piper_stimmen_sind_markiert() {
        let voices = tts_entries(Purpose::TtsVoice);
        let nc: BTreeSet<&str> = voices
            .iter()
            .filter(|v| v.license_non_commercial)
            .map(|v| v.id.as_str())
            .collect();
        // Lessac (Blizzard-2013-Forschungslizenz) und Ryan (CC-BY-NC-SA-4.0) selbst sowie
        // die davon feinabgestimmten Stimmen (K1): Thorsten, Kerstin, Amy, Alan, Alba.
        assert_eq!(
            nc,
            BTreeSet::from([
                "de_DE-kerstin-low",
                "de_DE-thorsten-high",
                "de_DE-thorsten-medium",
                "en_GB-alan-medium",
                "en_GB-alba-medium",
                "en_US-amy-medium",
                "en_US-lessac-high",
                "en_US-lessac-medium",
                "en_US-ryan-high",
            ])
        );
        // Von Grund auf trainiert (M-AILABS, BSD-3-Clause): frei.
        assert!(!voices.iter().any(|v| v.id == "de_DE-eva_k-x_low" && v.license_non_commercial));
    }

    #[test]
    fn piper_laufzeiten_nennen_das_espeak_ng_copyleft() {
        for e in tts_entries(Purpose::TtsRuntime) {
            let l = e.license.unwrap_or_default();
            assert!(l.contains("GPL-3.0-or-later"), "{}: {l}", e.id);
        }
    }

    #[test]
    fn macos_piper_archive_sind_als_unvollstaendig_markiert_windows_nicht() {
        let rt = tts_entries(Purpose::TtsRuntime);
        let by = |id: &str| rt.iter().find(|e| e.id == id).unwrap();
        assert_eq!(
            by("piper-runtime-macos-x64").unsupported_reason.as_deref(),
            Some("incomplete_archive")
        );
        assert_eq!(
            by("piper-runtime-macos-aarch64").unsupported_reason.as_deref(),
            Some("incomplete_archive")
        );
        assert!(by("piper-runtime-windows-x64").unsupported_reason.is_none());
    }
}
