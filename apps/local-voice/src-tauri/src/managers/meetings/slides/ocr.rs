//! D2: Text auf Folien lesen (Windows-OCR) und Folien nach ihrem Text zusammenfuehren.
//!
//! Stufe 1 der Textgewinnung (Messung und Begruendung: `koordination/bild-video/
//! spike/spike-bericht.md`, Abschnitt "OCR lokal"): `Windows.Media.Ocr`, 47-70 ms
//! je Bild, 0 MB, keine Lizenzfrage. Schwaechen sind bekannt (Tabellenzellen,
//! "§" -> "5", Zahlen): der Text taugt fuer Dubletten, Suche und die Kennzeichnung
//! "ohne Text", Zahlen werden erst durch die Bildanalyse (D3) belastbar.
//!
//! Der Baustein hat zwei Teile:
//!
//! 1. **[`OcrBackend`]**: ein Bild lesen. Auf Windows [`WindowsOcr`]; auf anderen
//!    Plattformen gibt es keine Texterkennung ([`default_backend`] meldet
//!    [`OcrError::Unavailable`], der Lauf macht ohne Text weiter).
//! 2. **Rein**: [`words`], [`jaccard`], [`kind_of`], [`group_with_text`] - der Text-
//!    Abgleich, den D1 in `group_with` offengelassen hat.
//!
//! # Regeln (aus dem Spike)
//!
//! - Weniger als [`MIN_TEXT_WORDS`] Woerter: Art [`KIND_NO_TEXT`] (`ohne_text`), die
//!   Folie wird nicht verworfen, sondern von der Oberflaeche eingeklappt.
//! - Wort-Jaccard mindestens [`SlideDetectConfig::text_jaccard`] (0,5): dieselbe
//!   Folie, auch wenn der Hash weit weg ist (Webcam, Ueberblendung). Texte unter
//!   der Wortgrenze nehmen nie am Textabgleich teil, nur der Hash gilt fuer sie.
//! - Der Hash hat Vorrang: wer innerhalb der Hash-Schwelle liegt, kommt zur
//!   naechsten Folie, der Text entscheidet erst danach.
//!
//! # Fehlerfaelle und Absicherung
//!
//! | Fall | Verhalten | Absicherung (Test) |
//! |---|---|---|
//! | Keine OCR-Sprache installiert / andere Plattform | `default_backend` meldet `ocr_unavailable`; der Lauf erkennt Folien ohne Text, Hash-Dubletten bleiben | `without_a_backend_the_run_is_the_d1_run`, `a_missing_language_is_unavailable` |
//! | de-DE fehlt, en-US da | die erste verfuegbare Sprache der Wunschliste (de, en), sonst die Benutzersprachen | `the_language_choice_follows_the_wish_list_then_the_user_profile` |
//! | Bild fehlt / nicht lesbar / zu gross | `OcrError::Image`, die Folie bleibt ohne Text und ohne Art (die naechste Wiederholung versucht es erneut), der Lauf geht weiter | `a_failing_slide_does_not_stop_the_run` |
//! | OCR liefert nichts / Unsinn | weniger als 3 Woerter = `ohne_text`; Jaccard von Rauschen liegt weit unter 0,5 | `short_or_empty_text_is_ohne_text`, `noise_does_not_merge_slides` |
//! | Sehr langer Text (Datenfolie) | auf [`MAX_TEXT_CHARS`] begrenzt (an einer Zeilengrenze) | `very_long_text_is_capped_on_a_line_boundary` |
//! | Pause / Stopp mitten im Lesen | Kontrollpunkt je Bild; nichts ist bis dahin geschrieben, halbfertige Kandidatenbilder werden geraeumt | `a_stop_while_reading_leaves_no_candidates_and_no_rows` |
//! | Wiederholung | gespeicherter Text wird wiederverwendet, kein erneutes Lesen; Folien ohne Art werden nachgeholt | `a_rerun_reads_nothing_twice_and_fills_gaps` |
//! | Realfall: 35 Abschnitte | 16 Folien statt 24 Hash-Gruppen | `the_real_case_collapses_to_at_most_sixteen_slides` |
//!
//! Speicherbedarf: ein Bild je Lesevorgang (Vollbild als `SoftwareBitmap`, bei
//! 1080p rund 8 MB, kurz); bei vollem RAM scheitert die Windows-API mit einem Fehler,
//! der als `OcrError::Engine` an dieser einen Folie haengen bleibt (fail-open).
//! Es gibt keinen Kindprozess und kein Modell, also nichts fuer `process_guard`.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use super::{hamming, SlideDetectConfig, SlideGroup, SlideSegment};

/// Art einer Folie mit lesbarem Text.
pub const KIND_TEXT: &str = "text";
/// Art einer Folie ohne (nennenswerten) Text: Foto, Sprecheraufnahme, Grafik.
pub const KIND_NO_TEXT: &str = "ohne_text";
/// Weniger Woerter als diese sind kein Folientext (Spike: "< 3 Woerter").
pub const MIN_TEXT_WORDS: usize = 3;
/// Mehr Zeichen Folientext werden abgeschnitten (an einer Zeilengrenze).
pub const MAX_TEXT_CHARS: usize = 20_000;
/// Kennung der Windows-Engine in `meeting_slides.ocr_engine`.
pub const ENGINE_WINDOWS: &str = "windows-ocr";
/// Wunschliste der Sprachen (Kennung-Anfang, in dieser Reihenfolge).
pub const PREFERRED_LANGUAGES: &[&str] = &["de", "en"];

// ---------------------------------------------------------------------------
// Fehler und Schnittstelle
// ---------------------------------------------------------------------------

/// Woran das Lesen eines Bildes scheitert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OcrError {
    /// Es gibt keine Texterkennung (andere Plattform, keine OCR-Sprache installiert).
    Unavailable,
    /// Das Bild fehlt, ist nicht lesbar oder zu gross fuer die Engine.
    Image(String),
    /// Die Engine ist gescheitert.
    Engine(String),
}

impl OcrError {
    pub fn code(&self) -> &'static str {
        match self {
            OcrError::Unavailable => "ocr_unavailable",
            OcrError::Image(_) => "ocr_image_failed",
            OcrError::Engine(_) => "ocr_failed",
        }
    }
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::Image(d) | OcrError::Engine(d) => write!(f, "{}: {d}", self.code()),
            OcrError::Unavailable => f.write_str(self.code()),
        }
    }
}

impl std::error::Error for OcrError {}

/// Was die Engine in einem Bild gelesen hat (schon bereinigt, siehe [`clean_text`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrText {
    pub text: String,
    /// Kennung der Engine (`ocr_engine` der Folie).
    pub engine: &'static str,
    /// Sprachkennung, in der gelesen wurde (`de-DE`).
    pub language: String,
}

/// Ein Texterkenner. Muss von mehreren Threads genutzt werden koennen, liest aber
/// immer ein Bild nach dem anderen.
pub trait OcrBackend: Send + Sync {
    /// Kennung fuer `ocr_engine` (`windows-ocr`).
    fn id(&self) -> &'static str;
    /// Sprache, die die Engine gewaehlt hat (`de-DE`).
    fn language(&self) -> String;
    /// Liest ein Bild (PNG/JPG/...). Gibt auch fuer ein Bild ohne Text `Ok` mit
    /// leerem Text zurueck.
    fn recognize(&self, image: &Path) -> Result<OcrText, OcrError>;
}

/// Die Texterkennung dieser Plattform, mit Sprachen nach Verfuegbarkeit: die erste
/// verfuegbare der `prefer`-Liste (Kennung-Anfang, `de` trifft `de-DE`), sonst die
/// Benutzersprachen. `Err(Unavailable)` auf anderen Plattformen und ohne OCR-Sprache.
pub fn default_backend(prefer: &[&str]) -> Result<Box<dyn OcrBackend>, OcrError> {
    #[cfg(windows)]
    {
        windows_impl::WindowsOcr::new(prefer).map(|b| Box::new(b) as Box<dyn OcrBackend>)
    }
    #[cfg(not(windows))]
    {
        let _ = prefer;
        Err(OcrError::Unavailable)
    }
}

/// Wahl der Sprache: die erste Kennung der Wunschliste, die ein installiertes
/// Paket trifft (ohne Beachtung der Gross-/Kleinschreibung, nur der Anfang), in der
/// Reihenfolge der Wunschliste; `None`: keine getroffen (dann gelten die
/// Benutzersprachen).
pub fn choose_language<'a>(installed: &'a [String], prefer: &[&str]) -> Option<&'a str> {
    prefer.iter().find_map(|want| {
        let want = want.to_lowercase();
        installed
            .iter()
            .find(|tag| {
                let tag = tag.to_lowercase();
                tag == want || tag.starts_with(&format!("{want}-"))
            })
            .map(String::as_str)
    })
}

// ---------------------------------------------------------------------------
// Text (rein)
// ---------------------------------------------------------------------------

/// Bereinigt rohen OCR-Text: einheitliche Zeilenenden, Zeilen gekuerzt, leere
/// Zeilen weg, auf [`MAX_TEXT_CHARS`] begrenzt (an einer Zeilengrenze).
pub fn clean_text(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    for line in raw.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            continue;
        }
        let add = line.chars().count() + usize::from(!out.is_empty());
        if chars + add > MAX_TEXT_CHARS {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        chars += add;
    }
    out
}

/// Die Woerter eines Textes fuer Zaehlung und Abgleich: klein geschrieben, nur
/// Buchstaben und Ziffern, mindestens zwei Zeichen (einzelne Zeichen sind fast
/// immer Erkennungsrauschen: "5" fuer "§", Aufzaehlungspunkte).
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect()
}

/// Wie viele Woerter ein Text hat (siehe [`words`]).
pub fn word_count(text: &str) -> usize {
    words(text).len()
}

/// Wort-Jaccard zweier Texte: |A und B| / |A oder B| ueber die Menge der Woerter.
/// 0, wenn einer keine Woerter hat.
pub fn jaccard(a: &str, b: &str) -> f32 {
    let set = |t: &str| words(t).into_iter().collect::<BTreeSet<String>>();
    let (a, b) = (set(a), set(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(&b).count();
    let union = a.len() + b.len() - inter;
    inter as f32 / union as f32
}

/// Die Art einer Folie nach ihrem Text: [`KIND_TEXT`] ab [`MIN_TEXT_WORDS`] Woertern.
pub fn kind_of(text: &str) -> &'static str {
    if word_count(text) >= MIN_TEXT_WORDS {
        KIND_TEXT
    } else {
        KIND_NO_TEXT
    }
}

// ---------------------------------------------------------------------------
// Zusammenfuehren nach Hash und Text (rein)
// ---------------------------------------------------------------------------

/// Abstand fuer "dieselbe Folie" ueber Hash und Text: Hash innerhalb der Schwelle
/// gewinnt (Abstand 0..=Schwelle); sonst zaehlt der Text (Abstand ab 1000, je
/// aehnlicher desto kleiner). `None`: verschiedene Folien.
fn slide_distance(
    a: (u64, Option<&str>),
    b: (u64, Option<&str>),
    cfg: &SlideDetectConfig,
) -> Option<u32> {
    let d = hamming(a.0, b.0);
    if d <= cfg.hash_threshold {
        return Some(d);
    }
    let (ta, tb) = (a.1?, b.1?);
    if word_count(ta) < MIN_TEXT_WORDS || word_count(tb) < MIN_TEXT_WORDS {
        return None;
    }
    let j = jaccard(ta, tb);
    (j >= cfg.text_jaccard).then(|| 1_000 + ((1.0 - j) * 1_000.0) as u32)
}

/// Fasst Abschnitte zusammen, die nach Hash (Hamming <= Schwelle) ODER nach Text
/// (Wort-Jaccard >= `cfg.text_jaccard`) dieselbe Folie sind. `texts[i]` ist der
/// Text des Abschnitts `i` (`None`: unbekannt oder ohne Text, dann zaehlt nur der
/// Hash); beide Listen sind gleich lang, sonst gelten fehlende Texte als `None`.
/// Ergebnis wie [`super::group_with`]: nach erstem Auftreten geordnet, Schwarzbilder
/// aussen vor.
pub fn group_with_text(
    segments: &[SlideSegment],
    texts: &[Option<String>],
    cfg: &SlideDetectConfig,
) -> Vec<SlideGroup> {
    // Die Abschnitte beginnen alle an verschiedenen Abtastungen: der Beginn nennt den Index.
    let index_of: HashMap<u64, usize> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| (s.start_ms, i))
        .collect();
    let text_of = |s: &SlideSegment| -> Option<&str> {
        index_of
            .get(&s.start_ms)
            .and_then(|&i| texts.get(i))
            .and_then(|t| t.as_deref())
    };
    super::group_with(segments, &|a, b| {
        slide_distance((a.hash, text_of(a)), (b.hash, text_of(b)), cfg)
    })
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[allow(unused_imports)] // oeffentlich fuer Aufrufer, die die Windows-Engine gezielt wollen
pub use windows_impl::WindowsOcr;

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};

    fn engine_err(e: windows::core::Error) -> OcrError {
        OcrError::Engine(format!("{e}"))
    }

    fn image_err(e: windows::core::Error) -> OcrError {
        OcrError::Image(format!("{e}"))
    }

    pub(super) fn available_language_tags() -> Result<Vec<String>, OcrError> {
        let langs = OcrEngine::AvailableRecognizerLanguages().map_err(engine_err)?;
        let mut tags = Vec::new();
        for lang in &langs {
            tags.push(lang.LanguageTag().map_err(engine_err)?.to_string());
        }
        Ok(tags)
    }

    /// Texterkennung ueber `Windows.Media.Ocr` (Sprachpakete des Systems).
    pub struct WindowsOcr {
        engine: OcrEngine,
        language: String,
    }

    impl WindowsOcr {
        /// Waehlt die Sprache nach [`choose_language`]; ohne Treffer die
        /// Benutzersprachen; ohne beides `Unavailable`.
        pub fn new(prefer: &[&str]) -> Result<Self, OcrError> {
            let installed = available_language_tags().unwrap_or_default();
            let engine = match choose_language(&installed, prefer) {
                Some(tag) => {
                    let lang =
                        windows::Globalization::Language::CreateLanguage(&HSTRING::from(tag))
                            .map_err(engine_err)?;
                    OcrEngine::TryCreateFromLanguage(&lang).ok()
                }
                None => None,
            }
            .or_else(|| OcrEngine::TryCreateFromUserProfileLanguages().ok())
            .ok_or(OcrError::Unavailable)?;
            let language = engine
                .RecognizerLanguage()
                .and_then(|l| l.LanguageTag())
                .map(|t| t.to_string())
                .unwrap_or_default();
            Ok(Self { engine, language })
        }
    }

    impl OcrBackend for WindowsOcr {
        fn id(&self) -> &'static str {
            ENGINE_WINDOWS
        }

        fn language(&self) -> String {
            self.language.clone()
        }

        fn recognize(&self, image: &Path) -> Result<OcrText, OcrError> {
            if !image.is_file() {
                return Err(OcrError::Image("Bilddatei fehlt".to_string()));
            }
            // `GetFileFromPathAsync` verlangt einen absoluten Pfad ohne `\\?\`.
            let abs = std::path::absolute(image).map_err(|e| OcrError::Image(e.to_string()))?;
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(abs.as_os_str()))
                .and_then(|op| op.get())
                .map_err(image_err)?;
            let stream = file
                .OpenAsync(FileAccessMode::Read)
                .and_then(|op| op.get())
                .map_err(image_err)?;
            let decoder = BitmapDecoder::CreateAsync(&stream)
                .and_then(|op| op.get())
                .map_err(image_err)?;
            let limit = OcrEngine::MaxImageDimension().map_err(engine_err)?;
            let (w, h) = (
                decoder.PixelWidth().map_err(image_err)?,
                decoder.PixelHeight().map_err(image_err)?,
            );
            if w > limit || h > limit {
                return Err(OcrError::Image(format!(
                    "Bild {w}x{h} ueberschreitet die Grenze {limit}"
                )));
            }
            let bitmap = decoder
                .GetSoftwareBitmapAsync()
                .and_then(|op| op.get())
                .map_err(image_err)?;
            let result = self
                .engine
                .RecognizeAsync(&bitmap)
                .and_then(|op| op.get())
                .map_err(engine_err)?;
            let mut raw = String::new();
            for line in &result.Lines().map_err(engine_err)? {
                raw.push_str(&line.Text().map_err(engine_err)?.to_string());
                raw.push('\n');
            }
            Ok(OcrText {
                text: clean_text(&raw),
                engine: ENGINE_WINDOWS,
                language: self.language.clone(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SlideDetectConfig {
        SlideDetectConfig::default()
    }

    // -- Text -------------------------------------------------------------------

    #[test]
    fn words_are_lowercase_alphanumeric_and_at_least_two_characters() {
        assert_eq!(
            words("Größe/Maße: 70 € — Bußgeld § 3, Q3 2026."),
            vec!["größe", "maße", "70", "bußgeld", "q3", "2026"]
        );
        assert!(words("§ 3 • - 5").is_empty(), "Einzelzeichen sind Rauschen");
        assert_eq!(word_count("Ein Zwei"), 2);
        assert!(words("").is_empty());
    }

    #[test]
    fn jaccard_counts_distinct_words_symmetrically() {
        assert_eq!(jaccard("a1 b2 c3", "a1 b2 c3"), 1.0);
        assert_eq!(jaccard("aa bb cc dd", "aa bb xx yy"), 2.0 / 6.0);
        assert_eq!(
            jaccard("Aa bb", "aa BB"),
            1.0,
            "Gross-/Kleinschreibung gilt nicht"
        );
        assert_eq!(
            jaccard("aa aa aa bb", "aa bb"),
            1.0,
            "Mengen, keine Haeufigkeit"
        );
        assert_eq!(jaccard("", "aa bb"), 0.0);
        assert_eq!(jaccard("", ""), 0.0);
        assert_eq!(jaccard("x", "y"), 0.0);
        let (a, b) = ("Umsatz Region Nord Sued Ost", "Umsatz Region West");
        assert_eq!(jaccard(a, b), jaccard(b, a));
    }

    #[test]
    fn short_or_empty_text_is_ohne_text() {
        assert_eq!(kind_of(""), KIND_NO_TEXT);
        assert_eq!(kind_of("Abb. 3"), KIND_NO_TEXT, "ein Wort");
        assert_eq!(kind_of("Quartalsbericht Q3"), KIND_NO_TEXT, "zwei Woerter");
        assert_eq!(
            kind_of("Agenda Ergebnisse Ausblick"),
            KIND_TEXT,
            "genau drei"
        );
        assert_eq!(kind_of("§ 3 5 - •"), KIND_NO_TEXT, "nur Zeichen");
    }

    #[test]
    fn clean_text_normalises_lines() {
        assert_eq!(
            clean_text("  Titel \r\n\r\n   zweite   Zeile  \n \n"),
            "Titel\nzweite Zeile"
        );
        assert_eq!(clean_text(""), "");
        assert_eq!(clean_text("\n\n"), "");
    }

    #[test]
    fn very_long_text_is_capped_on_a_line_boundary() {
        let line = "wort ".repeat(19) + "ende"; // 99 Zeichen
        let raw = std::iter::repeat(line.as_str())
            .take(1_000)
            .collect::<Vec<_>>()
            .join("\n");
        let cleaned = clean_text(&raw);
        assert!(cleaned.chars().count() <= MAX_TEXT_CHARS);
        assert!(cleaned.chars().count() > MAX_TEXT_CHARS - 200);
        assert!(cleaned.ends_with("ende"), "keine halbe Zeile");
    }

    #[test]
    fn noise_does_not_merge_slides() {
        let slide = "Umsatz nach Region Nord Sued Ost West Uebersee";
        for noise in ["l I | .", "ii ll oo", "xq zv kj"] {
            assert!(jaccard(slide, noise) < 0.1, "{noise}");
        }
    }

    // -- Sprache ----------------------------------------------------------------

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_language_choice_follows_the_wish_list_then_the_user_profile() {
        let installed = tags(&["en-US", "de-DE", "fr-FR"]);
        assert_eq!(choose_language(&installed, &["de", "en"]), Some("de-DE"));
        assert_eq!(
            choose_language(&tags(&["en-GB", "fr-FR"]), &["de", "en"]),
            Some("en-GB"),
            "de fehlt: en"
        );
        assert_eq!(
            choose_language(&tags(&["fr-FR"]), &["de", "en"]),
            None,
            "weder de noch en: Benutzersprache"
        );
        assert_eq!(choose_language(&[], &["de"]), None);
        assert_eq!(choose_language(&installed, &[]), None);
        assert_eq!(
            choose_language(&tags(&["DE-at"]), &["de"]),
            Some("DE-at"),
            "Schreibweise egal"
        );
        assert_eq!(
            choose_language(&tags(&["den-XX"]), &["de"]),
            None,
            "nur ganze Sprachkennung, nicht ein Wortanfang"
        );
    }

    #[test]
    fn a_missing_language_is_unavailable() {
        assert_eq!(OcrError::Unavailable.code(), "ocr_unavailable");
        assert_eq!(
            OcrError::Image("x".into()).to_string(),
            "ocr_image_failed: x"
        );
        assert_eq!(OcrError::Engine("y".into()).code(), "ocr_failed");
        #[cfg(not(windows))]
        assert!(matches!(
            default_backend(PREFERRED_LANGUAGES),
            Err(OcrError::Unavailable)
        ));
    }

    // -- Zusammenfuehren ----------------------------------------------------------

    fn seg(hash: u64, start_ms: u64) -> SlideSegment {
        SlideSegment {
            start_ms,
            end_ms: start_ms + 5_000,
            rep_ms: start_ms + 4_000,
            hash,
            black: false,
        }
    }

    const FAR_A: u64 = 0xA5A5_A5A5_0F0F_F0F0;
    const FAR_B: u64 = 0x1234_5678_9ABC_DEF0;
    const FAR_C: u64 = 0xF0E1_D2C3_B4A5_9687;

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn equal_text_with_far_hashes_is_one_slide() {
        let segs = [seg(FAR_A, 0), seg(FAR_B, 10_000), seg(FAR_C, 20_000)];
        let texts = [
            some("Agenda Ergebnisse Kundenzufriedenheit Massnahmen"),
            some("Ganz andere Folie mit anderem Inhalt hier"),
            some("Agenda Ergebnisse Kundenzufriedenheit Massnahmen Risiken"),
        ];
        let groups = group_with_text(&segs, &texts, &cfg());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].segments, vec![0, 2]);
        assert_eq!(
            groups[0].occurrences.len(),
            2,
            "zwei Vorkommen derselben Folie"
        );
        assert_eq!(groups[1].segments, vec![1]);
    }

    #[test]
    fn the_jaccard_threshold_is_the_one_of_the_config() {
        let segs = [seg(FAR_A, 0), seg(FAR_B, 10_000)];
        // 4 gemeinsame von 8 verschiedenen Woertern = genau 0,5.
        let texts = [some("aa bb cc dd ee ff"), some("aa bb cc dd gg hh")];
        assert_eq!(
            jaccard(texts[0].as_deref().unwrap(), texts[1].as_deref().unwrap()),
            0.5
        );
        assert_eq!(
            group_with_text(&segs, &texts, &cfg()).len(),
            1,
            "0,5 genuegt"
        );
        let strict = SlideDetectConfig {
            text_jaccard: 0.51,
            ..cfg()
        };
        assert_eq!(group_with_text(&segs, &texts, &strict).len(), 2);
        let loose = SlideDetectConfig {
            text_jaccard: 0.3,
            ..cfg()
        };
        assert_eq!(group_with_text(&segs, &texts, &loose).len(), 1);
    }

    #[test]
    fn texts_below_the_word_limit_never_merge_by_text() {
        let segs = [seg(FAR_A, 0), seg(FAR_B, 10_000), seg(FAR_C, 20_000)];
        let texts = [some("Abb. 3"), some("Abb. 3"), None];
        assert_eq!(
            group_with_text(&segs, &texts, &cfg()).len(),
            3,
            "ohne Text trennt der Hash"
        );
        // Fehlende Texte: wie group_by_hash.
        assert_eq!(group_with_text(&segs, &[], &cfg()).len(), 3);
    }

    #[test]
    fn a_close_hash_wins_over_text_and_merges_without_text() {
        let near = FAR_A ^ 0b111;
        let segs = [seg(FAR_A, 0), seg(near, 10_000)];
        let texts = [None, None];
        assert_eq!(group_with_text(&segs, &texts, &cfg()).len(), 1);
        // Naechster Hash vor naechstem Text: der dritte Abschnitt kommt zur Folie mit dem Hash.
        let segs = [seg(FAR_A, 0), seg(FAR_B, 10_000), seg(FAR_B ^ 0b11, 20_000)];
        let texts = [
            some("aa bb cc dd ee"),
            some("ff gg hh ii jj"),
            some("aa bb cc dd ee"),
        ];
        let groups = group_with_text(&segs, &texts, &cfg());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[1].segments, vec![1, 2], "Hash gewinnt gegen Text");
    }

    #[test]
    fn black_segments_stay_out_and_the_order_is_the_first_appearance() {
        let mut black = seg(0, 10_000);
        black.black = true;
        let segs = [seg(FAR_A, 0), black, seg(FAR_B, 20_000), seg(FAR_A, 30_000)];
        let texts = [
            some("aa bb cc"),
            some("aa bb cc"),
            some("xx yy zz"),
            some("aa bb cc"),
        ];
        let groups = group_with_text(&segs, &texts, &cfg());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].segments, vec![0, 3]);
        assert_eq!(groups[1].first, 2);
    }

    // -- Windows-OCR gegen eine eingecheckte Folie ---------------------------------

    /// Anteil der Wahrheits-Woerter (Menge mit Haeufigkeit), die im Text vorkommen.
    fn recall(truth: &str, got: &str) -> f32 {
        let mut pool: std::collections::HashMap<String, i32> = Default::default();
        for w in words(got) {
            *pool.entry(w).or_default() += 1;
        }
        let want = words(truth);
        let hit = want
            .iter()
            .filter(|w| {
                pool.get(*w).is_some_and(|n| *n > 0) && {
                    *pool.get_mut(*w).unwrap() -= 1;
                    true
                }
            })
            .count();
        hit as f32 / want.len().max(1) as f32
    }

    #[test]
    fn recall_counts_words_with_multiplicity() {
        assert_eq!(recall("aa bb cc dd", "aa bb"), 0.5);
        assert_eq!(recall("aa aa", "aa"), 0.5);
        assert_eq!(recall("aa", "AA zz"), 1.0);
        assert_eq!(recall("", "aa"), 0.0);
    }

    /// Akzeptanz D2: die PNG-Folie (per PIL erzeugt, `fixtures/make_d2_fixtures.py`) mit
    /// "Größe/Maße", "Bußgeld: 70 €" wird erkannt, Recall mindestens 0,9 der Woerter.
    #[cfg(windows)]
    #[test]
    fn windows_ocr_reads_the_fixture_slide_with_umlauts_and_euro() {
        let backend = match default_backend(PREFERRED_LANGUAGES) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("keine Windows-OCR-Sprache ({e}) - Test uebersprungen");
                return;
            }
        };
        let image = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/managers/meetings/slides/fixtures/ocr_slide.png");
        let started = std::time::Instant::now();
        let got = backend.recognize(&image).unwrap();
        eprintln!(
            "OCR ({}) in {:?}:
{}",
            got.language,
            started.elapsed(),
            got.text
        );
        assert_eq!(got.engine, "windows-ocr");
        let truth = include_str!("fixtures/ocr_slide.words.txt");
        let r = recall(truth, &got.text);
        assert!(
            r >= 0.9,
            "Recall {r} unter 0,9 (Sprache {}): {}",
            got.language,
            got.text
        );
        assert_eq!(kind_of(&got.text), KIND_TEXT);
        if got.language.to_lowercase().starts_with("de") {
            for must in ["Größe", "Maße", "Bußgeld", "70"] {
                assert!(got.text.contains(must), "{must} fehlt in {:?}", got.text);
            }
        }
    }

    /// Fehler je Bild sind Werte, keine Abbrueche: fehlende, leere und keine Bilddatei.
    #[cfg(windows)]
    #[test]
    fn unreadable_images_are_errors_not_panics() {
        let Ok(backend) = default_backend(PREFERRED_LANGUAGES) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            backend.recognize(&dir.path().join("weg.png")),
            Err(OcrError::Image(_))
        ));
        let junk = dir.path().join("kein-bild.png");
        std::fs::write(&junk, b"das ist kein bild").unwrap();
        assert!(matches!(backend.recognize(&junk), Err(OcrError::Image(_))));
        let empty = dir.path().join("leer.png");
        std::fs::write(&empty, b"").unwrap();
        assert!(backend.recognize(&empty).is_err());
    }

    // -- Realfall -----------------------------------------------------------------

    #[derive(serde::Deserialize)]
    struct CaseSegment {
        start_ms: u64,
        end_ms: u64,
        rep_ms: u64,
        hash: String,
        black: bool,
        kind: String,
        slide: Option<String>,
        text: String,
    }

    /// Spike-Realtest C: 35 Abschnitte (2 Schwarzbilder, 9 Rueckspruenge mit gleichem
    /// Hash, 24 Hash-Gruppen), davon 12 echte Textfolien (echte Windows-OCR-Texte des
    /// Spikes), 8 Varianten mit anderem Hash, 2 stark verstuemmelte Ueberblendstufen,
    /// 2 Bild-/Kamerafolien ohne Text. Hashes und Zeitleiste sind synthetisch.
    #[test]
    fn the_real_case_collapses_to_at_most_sixteen_slides() {
        let raw = include_str!("fixtures/real_case_35.json");
        let case: Vec<CaseSegment> = serde_json::from_str(raw).unwrap();
        assert_eq!(case.len(), 35, "35 Repraesentanten");
        let segs: Vec<SlideSegment> = case
            .iter()
            .map(|c| SlideSegment {
                start_ms: c.start_ms,
                end_ms: c.end_ms,
                rep_ms: c.rep_ms,
                hash: u64::from_str_radix(&c.hash, 16).unwrap(),
                black: c.black,
            })
            .collect();
        let by_hash = super::super::group_by_hash(&segs, &cfg());
        assert_eq!(by_hash.len(), 24, "Hash allein: 24 Gruppen wie im Spike");

        // Wie im Lauf: der Text je Hash-Gruppe kommt vom ersten Abschnitt, Texte unter
        // der Wortgrenze zaehlen als "kein Text".
        let mut texts: Vec<Option<String>> = vec![None; segs.len()];
        for g in &by_hash {
            let text = case[g.first].text.clone();
            let kept = (kind_of(&text) == KIND_TEXT).then_some(text);
            for &i in &g.segments {
                texts[i] = kept.clone();
            }
        }
        let groups = group_with_text(&segs, &texts, &cfg());
        assert!(
            groups.len() <= 16,
            "{} Folien, erwartet hoechstens 16",
            groups.len()
        );
        assert_eq!(groups.len(), 16, "12 echte + 2 Dubletten + 2 ohne Text");

        let no_text = groups
            .iter()
            .filter(|g| kind_of(&case[g.first].text) == KIND_NO_TEXT)
            .count();
        assert_eq!(
            no_text, 2,
            "Foto und Sprecheraufnahme bleiben als ohne_text"
        );
        // Jede der 12 echten Folien ist EINE Gruppe: alle Abschnitte mit ihrer Kennung
        // (echte Folie, Variante, Rueckspruch) liegen zusammen.
        let mut seen = std::collections::HashMap::new();
        for g in &groups {
            for &i in &g.segments {
                if let Some(id) = &case[i].slide {
                    if case[i].kind != "residual" {
                        let prev = seen.insert(id.clone(), g.first);
                        assert!(
                            prev.is_none() || prev == Some(g.first),
                            "Folie {id} zerfaellt in zwei Gruppen"
                        );
                    }
                }
            }
        }
        assert_eq!(seen.len(), 12);
        assert!(
            groups
                .iter()
                .all(|g| !g.segments.iter().any(|&i| segs[i].black)),
            "Schwarzbilder sind keine Folien"
        );
    }
}
