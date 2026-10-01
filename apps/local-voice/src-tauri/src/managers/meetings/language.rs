//! G5 (Goal Issues-Abschluss #70): die Sprache einer Besprechung erkennen und danach
//! das passende Modell waehlen. Alles Reine steht hier (ohne Tauri, Modell, Speicher);
//! die Anbindung an die Pipeline (`language_run.rs`) und die Kommandos
//! (`commands/meeting_language.rs`) sind duenne Huellen darum.
//!
//! Erkennung in drei Stufen, die staerkste zuerst:
//! 1. **probe**: eine Probe aus dem Audio (bis zu drei kurze Fenster) laeuft durch das
//!    geladene Modell; liefert es eine eigene Spracherkennung (`Transcript.language`,
//!    transcribe-cpp: Whisper, Qwen3-ASR, Parakeet v3 melden `lang_detect`), zaehlt sie.
//! 2. **text**: der erkannte Text der Probe oder das fertige Transkript, bestimmt mit
//!    `whatlang` (reines Rust, ohne Modelldatei). Rueckfall fuer Modelle ohne eigene
//!    Erkennung (alle ONNX-Engines von transcribe-rs) und Gegenprobe nach dem Lauf.
//! 3. **user**: Korrektur ueber den Chip im Kopf oder die Wahl beim Neu-Transkribieren.
//!    Die Nutzerwahl hat immer Vorrang und wird nie von einer Erkennung ueberschrieben.
//!
//! Nichts hier rät: ein Text unter [`MIN_LETTERS`] Buchstaben oder mit zu geringer
//! Sicherheit ergibt `None`, nicht eine geratene Sprache.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Herkunft der gespeicherten Sprache (Schluessel `source` im Metadatenobjekt).
pub const SOURCE_PROBE: &str = "probe";
pub const SOURCE_TEXT: &str = "text";
pub const SOURCE_USER: &str = "user";
/// Die Einstellung `meeting_language` war eine feste Sprache (kein `auto`).
pub const SOURCE_SETTING: &str = "setting";

/// Schluessel in `meetings.metadata_json`.
pub const METADATA_KEY: &str = "language";

/// Mindestzahl Buchstaben, bevor ein Text eine Sprache zugesprochen bekommt.
pub const MIN_LETTERS: usize = 40;
/// Mindestsicherheit (0..1) von `whatlang` fuer die Stufe `text`.
pub const MIN_TEXT_CONFIDENCE: f64 = 0.5;
/// Ab dieser Sicherheit darf der Text eine FESTE Einstellung (nicht die
/// Nutzerwahl im Dialog) korrigieren.
pub const STRONG_TEXT_CONFIDENCE: f64 = 0.85;
/// Ein Modell mit mindestens so vielen Sprachen gilt als mehrsprachig.
pub const MULTILINGUAL_MIN_LANGUAGES: usize = 10;

/// (Code, englischer Name fuer Prompts, deutscher Name fuer Anzeigen im Protokoll).
/// Die Reihenfolge ist die der Auswahlliste in der Oberflaeche.
pub const LANGUAGES: &[(&str, &str, &str)] = &[
    ("de", "German", "Deutsch"),
    ("en", "English", "Englisch"),
    ("fr", "French", "Französisch"),
    ("es", "Spanish", "Spanisch"),
    ("it", "Italian", "Italienisch"),
    ("pt", "Portuguese", "Portugiesisch"),
    ("nl", "Dutch", "Niederländisch"),
    ("pl", "Polish", "Polnisch"),
    ("cs", "Czech", "Tschechisch"),
    ("sk", "Slovak", "Slowakisch"),
    ("sl", "Slovenian", "Slowenisch"),
    ("hr", "Croatian", "Kroatisch"),
    ("hu", "Hungarian", "Ungarisch"),
    ("ro", "Romanian", "Rumänisch"),
    ("bg", "Bulgarian", "Bulgarisch"),
    ("el", "Greek", "Griechisch"),
    ("da", "Danish", "Dänisch"),
    ("sv", "Swedish", "Schwedisch"),
    ("no", "Norwegian", "Norwegisch"),
    ("fi", "Finnish", "Finnisch"),
    ("et", "Estonian", "Estnisch"),
    ("lv", "Latvian", "Lettisch"),
    ("lt", "Lithuanian", "Litauisch"),
    ("mt", "Maltese", "Maltesisch"),
    ("uk", "Ukrainian", "Ukrainisch"),
    ("ru", "Russian", "Russisch"),
    ("tr", "Turkish", "Türkisch"),
    ("ar", "Arabic", "Arabisch"),
    ("he", "Hebrew", "Hebräisch"),
    ("fa", "Persian", "Persisch"),
    ("hi", "Hindi", "Hindi"),
    ("zh", "Chinese", "Chinesisch"),
    ("ja", "Japanese", "Japanisch"),
    ("ko", "Korean", "Koreanisch"),
    ("vi", "Vietnamese", "Vietnamesisch"),
    ("th", "Thai", "Thailändisch"),
    ("id", "Indonesian", "Indonesisch"),
];

/// Der Basiscode einer Sprachangabe: `en-US` -> `en`, `EN` -> `en`, `zh-Hans` -> `zh`.
/// `None` fuer Leeres, `auto` und alles, was kein Sprachcode ist.
pub fn normalize_code(raw: &str) -> Option<String> {
    let base = raw.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    let valid = (2..=3).contains(&base.len()) && base.chars().all(|c| c.is_ascii_lowercase());
    (valid && base != "auto").then_some(base)
}

/// Englischer Name fuer Prompts ("German"); unbekannt: der Code selbst.
pub fn name_en(code: &str) -> String {
    let code = normalize_code(code).unwrap_or_else(|| code.to_string());
    LANGUAGES
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, en, _)| (*en).to_string())
        .unwrap_or(code)
}

/// Deutscher Name fuer Anzeigen im Protokoll ("Deutsch"); unbekannt: der Code selbst.
pub fn name_de(code: &str) -> String {
    let code = normalize_code(code).unwrap_or_else(|| code.to_string());
    LANGUAGES
        .iter()
        .find(|(c, _, _)| *c == code)
        .map(|(_, _, de)| (*de).to_string())
        .unwrap_or(code)
}

// ---------------------------------------------------------------------------
// Stufe 2: Sprache eines Textes
// ---------------------------------------------------------------------------

/// Eine Spracherkennung aus Text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextDetection {
    pub code: String,
    /// 0..1, wie `whatlang` sie angibt.
    pub confidence: f64,
}

/// ISO 639-3 (`whatlang`) -> ISO 639-1, soweit die App die Sprache kennt.
fn iso1(code3: &str) -> Option<&'static str> {
    Some(match code3 {
        "deu" => "de",
        "eng" => "en",
        "fra" => "fr",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "nld" => "nl",
        "pol" => "pl",
        "ces" => "cs",
        "slk" => "sk",
        "slv" => "sl",
        "hrv" => "hr",
        "hun" => "hu",
        "ron" => "ro",
        "bul" => "bg",
        "ell" => "el",
        "dan" => "da",
        "swe" => "sv",
        "nob" => "no",
        "fin" => "fi",
        "est" => "et",
        "lav" => "lv",
        "lit" => "lt",
        "ukr" => "uk",
        "rus" => "ru",
        "tur" => "tr",
        "ara" => "ar",
        "heb" => "he",
        "pes" => "fa",
        "hin" => "hi",
        "cmn" => "zh",
        "jpn" => "ja",
        "kor" => "ko",
        "vie" => "vi",
        "tha" => "th",
        "ind" => "id",
        _ => return None,
    })
}

fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// Die Sprache eines Textes, oder `None` (zu kurz, zu unsicher, unbekannte Sprache).
pub fn detect_text(text: &str) -> Option<TextDetection> {
    if letter_count(text) < MIN_LETTERS {
        return None;
    }
    let info = whatlang::detect(text)?;
    if info.confidence() < MIN_TEXT_CONFIDENCE {
        return None;
    }
    let code = iso1(info.lang().code())?;
    Some(TextDetection {
        code: code.to_string(),
        confidence: info.confidence(),
    })
}

// ---------------------------------------------------------------------------
// Stufe 1: Probe aus dem Audio
// ---------------------------------------------------------------------------

/// Laenge eines Probefensters.
pub const PROBE_WINDOW_MS: usize = 15_000;
/// So viele Fenster hoechstens (je eines pro Drittel der Aufnahme).
pub const PROBE_WINDOWS: usize = 3;
/// Unter diesem Effektivwert (von 32768) gilt ein Fenster als Stille.
const SILENCE_RMS: f64 = 120.0;

fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// Bis zu [`PROBE_WINDOWS`] Probefenster (16 kHz, mono, f32): je Drittel der Aufnahme
/// das lauteste Fenster, Stille bleibt weg. Eine kuerzere Aufnahme ist ein einziges
/// Fenster. Keine Allokation ausser den Fenstern selbst; liest nur.
pub fn sample_windows(samples: &[i16]) -> Vec<Vec<f32>> {
    let window = PROBE_WINDOW_MS * 16;
    let step = 16_000; // 1 s
    if samples.is_empty() {
        return Vec::new();
    }
    let to_f32 = |s: &[i16]| s.iter().map(|v| f32::from(*v) / 32768.0).collect::<Vec<f32>>();
    if samples.len() <= window {
        return if rms(samples) >= SILENCE_RMS {
            vec![to_f32(samples)]
        } else {
            Vec::new()
        };
    }
    let thirds = PROBE_WINDOWS;
    let mut out = Vec::new();
    for third in 0..thirds {
        let from = samples.len() * third / thirds;
        let to = samples.len() * (third + 1) / thirds;
        let mut best: Option<(usize, f64)> = None;
        let mut at = from;
        while at + window <= to.max(from + window).min(samples.len()) {
            let energy = rms(&samples[at..at + window]);
            if best.is_none_or(|(_, e)| energy > e) {
                best = Some((at, energy));
            }
            at += step * 5;
        }
        if let Some((start, energy)) = best {
            if energy >= SILENCE_RMS {
                out.push(to_f32(&samples[start..start + window]));
            }
        }
    }
    out
}

/// Das Ergebnis der Probe: je Fenster die eigene Erkennung des Modells (falls es eine
/// hat) und alle Texte zusammen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProbeResult {
    pub native: Vec<Option<String>>,
    pub text: String,
}

/// Was die Probe ergibt: die haeufigste eigene Erkennung des Modells; ohne eine die
/// Sprache des Probentextes. `confidence` ist bei `probe` der Anteil der Fenster, die
/// zustimmten.
pub fn decide_probe(result: &ProbeResult) -> Option<(String, &'static str, f64)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut total = 0usize;
    for code in result.native.iter().flatten() {
        let Some(code) = normalize_code(code) else {
            continue;
        };
        total += 1;
        match counts.iter_mut().find(|(c, _)| *c == code) {
            Some((_, n)) => *n += 1,
            None => counts.push((code, 1)),
        }
    }
    if let Some((code, n)) = counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))) {
        return Some((code, SOURCE_PROBE, n as f64 / total.max(1) as f64));
    }
    detect_text(&result.text).map(|d| (d.code, SOURCE_TEXT, d.confidence))
}

// ---------------------------------------------------------------------------
// Modellwahl
// ---------------------------------------------------------------------------

/// Ein Modell des Katalogs, wie die Wahl es sieht.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelCandidate {
    pub id: String,
    pub name: String,
    /// Sprachen laut Katalog; leer = unbekannt (die Wahl nimmt dann an, es passt).
    pub languages: Vec<String>,
    pub downloaded: bool,
    pub size_mb: u64,
    /// 0..100 aus dem Katalog.
    pub accuracy: u32,
}

impl ModelCandidate {
    fn multilingual(&self) -> bool {
        self.languages.len() >= MULTILINGUAL_MIN_LANGUAGES
    }

    /// `Some(true/false)`; `None`, wenn der Katalog keine Sprachen kennt.
    pub fn covers(&self, code: &str) -> Option<bool> {
        let want = normalize_code(code)?;
        if self.languages.is_empty() {
            return None;
        }
        Some(
            self.languages
                .iter()
                .any(|l| normalize_code(l).as_deref() == Some(want.as_str())),
        )
    }
}

/// Warum die Wahl so ausfiel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelReason {
    /// Der Nutzer hat das Modell gewaehlt (Einstellung oder Dialog): bleibt.
    UserChoice,
    /// Das bisherige Modell deckt die Sprache ab (oder der Katalog sagt nichts): bleibt.
    Covers,
    /// Ein installiertes mehrsprachiges Modell ersetzt das bisherige.
    Switched,
    /// Kein installiertes Modell deckt die Sprache ab: das bisherige bleibt, ein
    /// Vorschlag aus dem Katalog steht dabei.
    NoneInstalled,
    /// Keine Sprache bekannt: nichts zu waehlen.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    pub id: String,
    pub reason: ModelReason,
    /// Deckt das GEWAEHLTE Modell die Sprache ab? `None`: unbekannt.
    pub covers: Option<bool>,
    /// Bessere Wahl, wenn das gewaehlte Modell die Sprache nicht abdeckt: das beste
    /// Katalogmodell fuer die Sprache (installiert oder nicht).
    pub suggestion: Option<String>,
}

/// Die beste Wahl unter den Modellen, die `code` abdecken: mehrsprachige zuerst, dann
/// die genauere, dann die kleinere.
fn best_for<'a>(
    catalog: &'a [ModelCandidate],
    code: &str,
    installed_only: bool,
) -> Option<&'a ModelCandidate> {
    catalog
        .iter()
        .filter(|m| m.covers(code) == Some(true) && (!installed_only || m.downloaded))
        .max_by(|a, b| {
            a.multilingual()
                .cmp(&b.multilingual())
                .then(a.accuracy.cmp(&b.accuracy))
                .then(b.size_mb.cmp(&a.size_mb))
                .then(b.id.cmp(&a.id))
        })
}

/// Welches Modell rechnet eine Aufnahme dieser Sprache?
///
/// - Keine Sprache: das bisherige bleibt (`Unknown`).
/// - `user_explicit`: die Nutzerwahl bleibt IMMER (`UserChoice`); deckt das Modell die
///   Sprache nicht ab, steht der Vorschlag dabei.
/// - Deckt das bisherige Modell die Sprache ab (oder ist unbekannt, was es kann): es
///   bleibt (`Covers`). Das ist der Regelfall Deutsch und spart den Modellwechsel.
/// - Sonst das beste INSTALLIERTE Modell, das sie abdeckt (`Switched`); gibt es keins,
///   bleibt das bisherige und der Vorschlag nennt das beste Katalogmodell
///   (`NoneInstalled`).
pub fn choose_model(
    language: Option<&str>,
    current: &ModelCandidate,
    user_explicit: bool,
    catalog: &[ModelCandidate],
) -> ModelChoice {
    let Some(code) = language.and_then(normalize_code) else {
        return ModelChoice {
            id: current.id.clone(),
            reason: ModelReason::Unknown,
            covers: None,
            suggestion: None,
        };
    };
    let covers = current.covers(&code);
    let better = || best_for(catalog, &code, false).map(|m| m.id.clone());
    if user_explicit {
        return ModelChoice {
            id: current.id.clone(),
            reason: ModelReason::UserChoice,
            covers,
            suggestion: (covers == Some(false)).then(better).flatten(),
        };
    }
    if covers != Some(false) {
        return ModelChoice {
            id: current.id.clone(),
            reason: ModelReason::Covers,
            covers,
            suggestion: None,
        };
    }
    match best_for(catalog, &code, true) {
        Some(model) => ModelChoice {
            id: model.id.clone(),
            reason: ModelReason::Switched,
            covers: Some(true),
            suggestion: None,
        },
        None => ModelChoice {
            id: current.id.clone(),
            reason: ModelReason::NoneInstalled,
            covers: Some(false),
            suggestion: better(),
        },
    }
}

// ---------------------------------------------------------------------------
// Nach dem Lauf: die gueltige Sprache
// ---------------------------------------------------------------------------

/// Was ueber die Sprache einer Besprechung gespeichert wird (`metadata_json.language`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredLanguage {
    pub code: String,
    /// `probe`, `text`, `user`, `setting`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Die feste Einstellung/Wahl, gegen die der Text widersprach (`code` ist dann die
    /// Sprache des Textes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forced: Option<String>,
    /// Der Text widersprach der gewaehlten Sprache, die Wahl blieb.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mismatch: Option<String>,
    /// Modell, das transkribiert hat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

/// Die gueltige Sprache nach einem Lauf: die gewaehlte (`planned`, mit Herkunft) gegen
/// die Gegenprobe am fertigen Transkript (`text`).
///
/// - Nutzerwahl (`user`): bleibt; ein widersprechender Text steht als `mismatch`.
/// - feste Einstellung (`setting`) und ein starker, widersprechender Text: gilt der Text
///   (`source` = `text`, `forced` = die Einstellung); ein Modell mit festem Sprachcode
///   hat sonst falsch transkribiert, ohne dass es jemand merkt.
/// - Probe: bleibt; ein widersprechender Text steht als `mismatch`.
/// - Keine Wahl: der Text, wenn er sicher genug ist; sonst keine Sprache (`None`).
pub fn settle(
    planned: Option<(String, &'static str, f64)>,
    text: Option<TextDetection>,
    model_id: Option<String>,
) -> Option<StoredLanguage> {
    match (planned, text) {
        (None, None) => None,
        (None, Some(t)) => Some(StoredLanguage {
            code: t.code,
            source: SOURCE_TEXT.to_string(),
            confidence: Some(t.confidence),
            forced: None,
            mismatch: None,
            model_id,
        }),
        (Some((code, source, confidence)), text) => {
            let contradicts = text.as_ref().filter(|t| t.code != code);
            if source == SOURCE_SETTING {
                if let Some(t) = contradicts.filter(|t| t.confidence >= STRONG_TEXT_CONFIDENCE) {
                    return Some(StoredLanguage {
                        code: t.code.clone(),
                        source: SOURCE_TEXT.to_string(),
                        confidence: Some(t.confidence),
                        forced: Some(code),
                        mismatch: None,
                        model_id,
                    });
                }
            }
            Some(StoredLanguage {
                code,
                source: source.to_string(),
                confidence: Some(confidence),
                forced: None,
                mismatch: contradicts
                    .filter(|t| t.confidence >= STRONG_TEXT_CONFIDENCE)
                    .map(|t| t.code.clone()),
                model_id,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Entscheidung eines Laufs
// ---------------------------------------------------------------------------

/// Muss vor der Transkription eine Probe laufen? Nur, wenn nichts die Sprache festlegt:
/// weder die Wahl im Dialog (ein Code) noch die Einstellung `meeting_language` (ein Code).
/// `auto` im Dialog heisst ausdruecklich "erkennen", auch bei fester Einstellung.
pub fn needs_probe(user_language: Option<&str>, setting_language: &str) -> bool {
    match user_language.map(str::trim).filter(|s| !s.is_empty()) {
        Some(user) => normalize_code(user).is_none(),
        None => normalize_code(setting_language).is_none(),
    }
}

/// Was die Entscheidung eines Laufs braucht.
pub struct RunInput<'a> {
    /// Sprache aus dem Dialog der Neu-Transkription (`auto`: erkennen).
    pub user_language: Option<&'a str>,
    /// Einstellung `meeting_language`.
    pub setting_language: &'a str,
    /// Das Modell, das ohne Spracherkennung liefe (Nutzerwahl, Standard oder Rueckfall).
    pub current: &'a ModelCandidate,
    /// `current` ist eine Nutzerwahl (Einstellung `meeting_model` oder Dialog).
    pub user_model: bool,
    pub catalog: &'a [ModelCandidate],
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunDecision {
    /// Die Sprache des Laufs mit Herkunft und Sicherheit; `None`: unbekannt.
    pub planned: Option<(String, &'static str, f64)>,
    pub choice: ModelChoice,
    /// Die Sprache, die dem Modell fuer diesen Lauf vorgegeben wird: nur, wenn sie bekannt
    /// ist und das Modell sie nicht ausschliesst.
    pub force: Option<String>,
}

/// Sprache und Modell eines Laufs. `probe`: das Ergebnis der Probe, falls eine lief.
/// Rangfolge der Sprache: Wahl im Dialog, feste Einstellung, Probe. Das Modell: Nutzerwahl,
/// sonst `choose_model`.
pub fn decide_run(input: &RunInput<'_>, probe: Option<&ProbeResult>) -> RunDecision {
    let user = input.user_language.map(str::trim).filter(|s| !s.is_empty());
    let fixed = match user {
        Some(user) => normalize_code(user).map(|code| (code, SOURCE_USER)),
        None => normalize_code(input.setting_language).map(|code| (code, SOURCE_SETTING)),
    };
    let planned = match fixed {
        Some((code, source)) => Some((code, source, 1.0)),
        None => probe.and_then(decide_probe),
    };
    let choice = choose_model(
        planned.as_ref().map(|p| p.0.as_str()),
        input.current,
        input.user_model,
        input.catalog,
    );
    let force = planned
        .as_ref()
        .filter(|_| choice.covers != Some(false))
        .map(|p| p.0.clone());
    RunDecision {
        planned,
        choice,
        force,
    }
}

// ---------------------------------------------------------------------------
// Anzeige im Kopf
// ---------------------------------------------------------------------------

/// Ein Modellvorschlag fuer die Sprache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ModelSuggestion {
    pub model_id: String,
    pub name: String,
    /// Schon installiert (sonst muss es erst unter Modelle geladen werden).
    pub downloaded: bool,
}

/// Was der Chip im Kopf der Besprechung braucht.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct LanguageInfo {
    /// Die Sprache der Besprechung (Code); `None`: unbekannt.
    pub code: Option<String>,
    /// `probe`, `text`, `user`, `setting`; `None`: nichts gespeichert (Altbestand).
    pub source: Option<String>,
    pub confidence: Option<f64>,
    /// Die feste Einstellung, gegen die der Text widersprach.
    pub forced: Option<String>,
    /// Der Text widersprach der gewaehlten Sprache.
    pub mismatch: Option<String>,
    /// Das Modell, das transkribiert hat, und ob es die Sprache abdeckt.
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub model_covers: Option<bool>,
    /// Bessere Wahl, wenn das Modell die Sprache nicht abdeckt.
    pub suggestion: Option<ModelSuggestion>,
}

/// Der Chip aus den gespeicherten Angaben: `stored` (Metadaten), die Spalte
/// `meetings.language`, das Modell der Transkription und der Katalog.
pub fn build_info(
    stored: Option<&StoredLanguage>,
    meeting_language: Option<&str>,
    model_id: Option<&str>,
    catalog: &[ModelCandidate],
) -> LanguageInfo {
    let code = stored
        .map(|s| s.code.clone())
        .or_else(|| meeting_language.and_then(normalize_code));
    let model = model_id.and_then(|id| catalog.iter().find(|m| m.id == id));
    let model_covers = code
        .as_deref()
        .and_then(|c| model.and_then(|m| m.covers(c)));
    let suggestion = match (code.as_deref(), model, model_covers) {
        (Some(code), Some(current), Some(false)) => {
            let choice = choose_model(Some(code), current, false, catalog);
            let named = |id: &str, downloaded: bool| {
                catalog.iter().find(|m| m.id == id).map(|m| ModelSuggestion {
                    model_id: m.id.clone(),
                    name: m.name.clone(),
                    downloaded,
                })
            };
            match choice.reason {
                ModelReason::Switched => named(&choice.id, true),
                ModelReason::NoneInstalled => choice.suggestion.as_deref().and_then(|id| named(id, false)),
                _ => None,
            }
        }
        _ => None,
    };
    LanguageInfo {
        code,
        source: stored.map(|s| s.source.clone()),
        confidence: stored.and_then(|s| s.confidence),
        forced: stored.and_then(|s| s.forced.clone()),
        mismatch: stored.and_then(|s| s.mismatch.clone()),
        model_id: model_id.map(str::to_string),
        model_name: model.map(|m| m.name.clone()),
        model_covers,
        suggestion,
    }
}

#[cfg(test)]
mod tests;
