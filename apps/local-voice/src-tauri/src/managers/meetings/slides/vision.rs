//! D3 (Goal issues-abschluss #70, M7 = #69): Bildanalyse von Folien mit Gemma 4 E4B
//! und seinem Bild-Projektor.
//!
//! Stufe 2 der Textgewinnung (Messung und Begruendung: `koordination/bild-video/
//! spike/spike-bericht.md`, Abschnitt "Bildanalyse"): die Windows-OCR (D2) bleibt
//! Stufe 1 (Dubletten, Suche, "ohne Text"), Gemma liest den Text danach nochmal,
//! diesmal mit korrekten Zahlen und Tabellen (60/60 gegen 47/60), und beschreibt die
//! Folie in einem Satz. Beides nur, wenn der Nutzer die Bildanalyse eingeschaltet hat
//! UND sie moeglich ist (GPU, Projektor, Speicher: `managers::llm::vision`).
//!
//! # Regeln aus dem Spike
//!
//! - **Kein JSON-Schema-Zwang.** Mit `response_format: json_schema` lief E4B auf echten
//!   Bildern bis `max_tokens` in eine Leerzeichen-Schleife. Die Antworten sind einfacher
//!   Text: [`read_slide`] verlangt den Folientext Zeile fuer Zeile, [`describe_slide`] zwei
//!   Zeilen (Zeile 1 ein Klassenwort, Zeile 2 ein Satz). Der Parser ([`parse_description`],
//!   [`clean_read`]) wertet selbst aus.
//! - **Verwerfen statt speichern**: eine Antwort, deren erste Zeile kein bekanntes
//!   Klassenwort ist, oder die nur aus Leerzeichen besteht, wird nicht gespeichert. Der
//!   OCR-Text der Folie bleibt, wie er ist.
//! - **Beschreibung ist ein Hinweis**: Ablesen von Positionen ist die Schwachstelle
//!   (Gantt: "Februar" statt "Januar"); D5 kennzeichnet sie im Protokoll als `{Bild: ...}`.
//!   Der Folientext (OCR, bei Bildanalyse von Gemma) ist der belastbare Teil.
//! - **Nur Folien mit Text werden neu gelesen**: findet die Windows-OCR weniger als drei
//!   Woerter (Foto, Sprecheraufnahme), ersetzt Gemma sie nicht (es wuerde eher erfinden);
//!   die Folie bekommt nur die Beschreibung.
//!
//! # Fehlerfaelle und Absicherung
//!
//! | Fall | Verhalten | Absicherung (Test) |
//! |---|---|---|
//! | Bildanalyse aus (Standard) | nichts gefragt, der Lauf ist der von D2 | `without_a_vision_backend_the_slides_keep_their_ocr_text` |
//! | Keine GPU / kein Projektor / zu wenig Grafikspeicher | der Befehl gibt kein Backend mit (`decide_vision`), die Windows-OCR bleibt | `every_missing_precondition_falls_back_with_a_reason` (`managers::llm::vision`) |
//! | Server startet nicht (RAM-Gate, Absturz, Projektor fehlt) | EINE gescheiterte Anfrage beendet die Bildanalyse dieses Laufs (keine 15 Neustartversuche), OCR-Text bleibt | `an_unavailable_server_ends_the_analysis_and_keeps_the_ocr_text` |
//! | Antwort leer, nur Leerzeichen oder unbekannte Klasse | verworfen, die Folie behaelt ihren OCR-Text, die naechste Folie wird trotzdem gefragt | `blank_and_unknown_answers_are_dropped_and_the_run_goes_on`, `a_blank_answer_is_empty_not_text` |
//! | Gemma-Text kuerzer als drei Woerter | OCR-Text bleibt (kein Ersatz durch "Kein Text") | `a_too_short_read_never_replaces_the_ocr_text`, `a_too_short_gemma_text_keeps_the_ocr_text_in_the_run` |
//! | Abgeschnittene Antwort (`max_tokens`) | die letzte, vermutlich halbe Zeile entfaellt; Wiederholungsschleifen werden auf zwei gleiche Zeilen gekuerzt | `a_truncated_read_loses_its_last_line_and_loops_are_cut` |
//! | Wiederholung des Laufs | eine Folie mit Gemma-Text und Beschreibung wird nicht noch einmal gefragt | `a_rerun_asks_nothing_twice` |
//! | Stopp / Pause | Kontrollpunkt je Folie (eine laufende Anfrage dauert ~2 s); nichts Halbes geschrieben | (Kontrollpunkt wie D2, `a_stop_while_reading_leaves_no_candidates_and_no_rows`) |
//! | Ende des Laufs (auch Stopp, Fehler) | `release()`: der Server mit Projektor wird beendet, der naechste Chat startet wieder ohne | `released()`-Pruefungen in `vision_text_replaces_ocr_and_every_slide_gets_a_description` und `an_unavailable_server_ends_the_analysis_and_keeps_the_ocr_text`; Neustart ohne Projektor: `a_vision_request_restarts_a_plain_server_once_and_then_serves_everyone` (`llm::server`) |
//! | Ohne Texterkennung (macOS) | keine Bildanalyse (sie ersetzt OCR-Text, setzt ihn nicht erstmals) | `without_ocr_the_vision_is_never_asked` |
//!
//! Speicherbedarf: der Server mit Projektor belegt ~4,7 GB Grafikspeicher und ~5 GB RAM
//! (Spike). Ein Bild wird einzeln gelesen (JPEG ~100 kB, als base64 im Request). Bei
//! vollem RAM verweigert das Start-Gate des Servers den Start (`Arbeitsspeicher`): die
//! Folge ist `VisionError::Unavailable` und der Rueckfall auf OCR, nie ein Absturz. Der
//! Server laeuft im Job-Objekt von `process_guard` (RAM-/CPU-Deckel).
//!
//! Datenschutz (D9): weder Bildinhalt noch Antworttexte gelangen ins Log, nur Codes.

use std::path::Path;

use super::ocr;

/// Kennung in `ocr_engine` und `description_model`.
pub const ENGINE_GEMMA: &str = "gemma-4-e4b";
/// `max_tokens` fuer das Lesen des Folientextes (Tabellen und Code brauchen Platz).
pub const READ_MAX_TOKENS: u32 = 1024;
/// `max_tokens` fuer die Beschreibung (Spike: <= 200).
pub const DESCRIBE_MAX_TOKENS: u32 = 200;
/// Laengste gespeicherte Beschreibung (Zeichen).
pub const MAX_DESCRIPTION_CHARS: usize = 600;
/// Gleiche Zeilen hintereinander werden auf so viele gekuerzt (Wiederholungsschleife).
const MAX_REPEATED_LINES: usize = 2;

/// Die Klassenwoerter der ersten Antwortzeile von [`describe_slide`]. `sprecher_raum`
/// (statt "kamera"): das Wort "kamera" wurde mit abgebildeten Kameras verwechselt (Spike).
pub const CLASSES: [&str; 4] = ["textfolie", "bildfolie", "sprecher_raum", "leer"];

/// Die Frage nach dem Folientext. "KEINTEXT" statt einer Erklaerung, damit Kommentare
/// nicht als Folientext gespeichert werden.
pub const READ_PROMPT: &str = "Gib den gesamten sichtbaren Text dieser Folie wortgetreu wieder, \
Zeile für Zeile, in der Originalsprache, ohne Kommentar und ohne Formatierung. Bei Tabellen \
gib jede Tabellenzeile als eine Zeile wieder, die Zellen durch ' | ' getrennt, und übernimm \
alle Zahlen genau wie angezeigt. Ist kein Text zu sehen, antworte nur mit dem Wort KEINTEXT.";

/// Die Frage nach Klasse und Beschreibung: zwei Zeilen, kein JSON.
pub const DESCRIBE_PROMPT: &str = "Das Bild ist ein Standbild aus dem Video eines Vortrags. \
Antworte in genau zwei Zeilen. Zeile 1: genau eines der Wörter textfolie, bildfolie, \
sprecher_raum, leer (sprecher_raum = Raum oder Sprecher gefilmt; bildfolie = Folie oder Foto \
ohne nennenswerten Text; leer = schwarzes oder leeres Bild). Zeile 2: ein Satz auf Deutsch, \
was zu sehen ist. Erfinde nichts.";

// ---------------------------------------------------------------------------
// Schnittstelle
// ---------------------------------------------------------------------------

/// Woran eine Bildanfrage scheitert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisionError {
    /// Der Server mit Projektor ist nicht zu haben (RAM-Gate, Absturz, Projektor oder
    /// Modell fehlt). Der Lauf fragt danach nichts mehr.
    Unavailable(String),
    /// Diese eine Anfrage ist gescheitert (Bild unlesbar, Verbindung, Serverfehler).
    Request(String),
    /// Die Antwort war leer oder bestand nur aus Leerzeichen.
    Empty,
    /// Die erste Zeile der Antwort ist kein bekanntes Klassenwort.
    BadClass,
}

impl VisionError {
    /// Kurzer Code fuer Log und Zaehler (nie Inhalt).
    pub fn code(&self) -> &'static str {
        match self {
            VisionError::Unavailable(_) => "vision_unavailable",
            VisionError::Request(_) => "vision_request",
            VisionError::Empty => "vision_empty",
            VisionError::BadClass => "vision_bad_class",
        }
    }
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VisionError::Unavailable(why) | VisionError::Request(why) => {
                write!(f, "{}: {why}", self.code())
            }
            other => f.write_str(other.code()),
        }
    }
}

impl std::error::Error for VisionError {}

/// Antwort einer Bildanfrage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisionReply {
    pub text: String,
    /// Der Server hat bei `max_tokens` abgebrochen.
    pub truncated: bool,
}

/// Ein Bildleser. Blockierend (der Folienauftrag laeuft auf einem Arbeitsthread) und
/// aus mehreren Threads nutzbar, fragt aber immer ein Bild nach dem anderen.
pub trait VisionBackend: Send + Sync {
    /// Kennung fuer `ocr_engine` und `description_model`.
    fn id(&self) -> &'static str {
        ENGINE_GEMMA
    }

    /// Stellt eine Frage zu einem Bild (JPEG/PNG). Startet den Server bei Bedarf.
    fn ask(&self, image: &Path, prompt: &str, max_tokens: u32) -> Result<VisionReply, VisionError>;

    /// Ende des Auftrags: der Server mit Projektor darf beendet werden. Wird bei jedem
    /// Ende des Laufs gerufen (auch Stopp und Fehler), auch wenn nie gefragt wurde.
    fn release(&self) {}
}

/// Der echte Leser: das lokale Gemma 4 E4B ueber den gebuendelten `llama-server`, der
/// dafuer MIT Projektor gestartet und danach wieder beendet wird.
pub struct LocalVision {
    model: String,
}

impl LocalVision {
    pub fn new() -> Self {
        Self {
            model: crate::managers::llm::vision::VISION_MODEL_ID.to_string(),
        }
    }
}

impl Default for LocalVision {
    fn default() -> Self {
        Self::new()
    }
}

/// Fehlertext von `llm_client::post_image_prompt` -> Fehlerart.
fn map_request_error(text: String) -> VisionError {
    VisionError::Request(text)
}

impl VisionBackend for LocalVision {
    fn ask(&self, image: &Path, prompt: &str, max_tokens: u32) -> Result<VisionReply, VisionError> {
        // Der Folienauftrag laeuft auf einem Blocking-Thread (`spawn_blocking`): dort ist
        // `block_on` erlaubt.
        tauri::async_runtime::block_on(async {
            let base = crate::managers::llm::ensure_local_vision(&self.model)
                .await
                .map_err(VisionError::Unavailable)?;
            let reply =
                crate::llm_client::post_image_prompt(&base, &self.model, image, prompt, max_tokens)
                    .await
                    .map_err(map_request_error)?;
            Ok(VisionReply {
                text: reply.content,
                truncated: reply.truncated,
            })
        })
    }

    fn release(&self) {
        tauri::async_runtime::block_on(crate::managers::llm::release_vision());
    }
}

// ---------------------------------------------------------------------------
// Auswertung (rein)
// ---------------------------------------------------------------------------

/// Klasse und Beschreibung einer Folie laut [`describe_slide`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlideVision {
    /// Eines von [`CLASSES`].
    pub class: &'static str,
    /// Ein Satz (hoechstens [`MAX_DESCRIPTION_CHARS`] Zeichen).
    pub description: String,
}

/// Entfernt Markdown- und Anfuehrungszeichen, Aufzaehlungspunkte und Satzzeichen um
/// ein Wort herum ("**textfolie**." -> "textfolie").
fn strip_decor(s: &str) -> &str {
    s.trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '*' | '_'
                    | '`'
                    | '"'
                    | '\''
                    | '„'
                    | '“'
                    | '”'
                    | '«'
                    | '»'
                    | '.'
                    | ','
                    | ';'
                    | ':'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '-'
                    | '•'
            )
    })
}

/// Das Klassenwort der ersten Antwortzeile, oder `None`. Toleriert Beschriftungen
/// ("Zeile 1: textfolie", "Klasse: textfolie"), Gross-/Kleinschreibung, Markdown und
/// Satzzeichen; "Sprecher-Raum" und "Sprecher Raum" gelten als `sprecher_raum`.
fn class_of(line: &str) -> Option<&'static str> {
    let mut text = line.trim().to_lowercase();
    for label in ["zeile 1", "zeile1", "klasse", "art"] {
        if let Some(rest) = text.strip_prefix(label) {
            if rest.trim_start().starts_with(':') {
                text = rest.trim_start()[1..].to_string();
                break;
            }
        }
    }
    let word = strip_decor(&text).replace([' ', '-'], "_");
    CLASSES.into_iter().find(|c| *c == word)
}

/// Streicht eine Beschriftung ("Zeile 2:", "Beschreibung:") vor dem Satz.
fn strip_label(line: &str) -> &str {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    for label in ["zeile 2", "zeile2", "beschreibung", "satz"] {
        if lower.starts_with(label) {
            let rest = &trimmed[label.len()..];
            if let Some(after) = rest.trim_start().strip_prefix(':') {
                return after.trim();
            }
        }
    }
    trimmed
}

fn cap_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    // An einer Wortgrenze schneiden, wenn eine nahe genug liegt.
    match cut.rfind(' ') {
        Some(i) if i > max / 2 => cut[..i].trim_end().to_string(),
        _ => cut,
    }
}

/// Wertet die Antwort auf [`DESCRIBE_PROMPT`] aus: Zeile 1 ein bekanntes Klassenwort,
/// danach ein Satz. Leer oder nur Leerzeichen -> [`VisionError::Empty`]; erste Zeile kein
/// Klassenwort -> [`VisionError::BadClass`] (verwerfen, nicht raten); Klasse ohne Satz ->
/// `Empty` (es gibt nichts zu speichern).
pub fn parse_description(raw: &str) -> Result<SlideVision, VisionError> {
    let mut lines = raw.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().ok_or(VisionError::Empty)?;
    let class = class_of(first).ok_or(VisionError::BadClass)?;
    let sentence: Vec<&str> = lines.map(strip_label).filter(|l| !l.is_empty()).collect();
    let joined = sentence.join(" ");
    let description = cap_chars(
        joined
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim_matches(|c: char| matches!(c, '"' | '„' | '“' | '”' | '*')),
        MAX_DESCRIPTION_CHARS,
    );
    if description.is_empty() {
        return Err(VisionError::Empty);
    }
    Ok(SlideVision { class, description })
}

/// Bereinigt die Antwort auf [`READ_PROMPT`]: Code-Zaun weg, Zeilen gekuerzt, leere
/// Zeilen weg, Wiederholungsschleifen auf [`MAX_REPEATED_LINES`] gleiche Zeilen gekuerzt,
/// bei abgeschnittener Antwort die letzte (vermutlich halbe) Zeile entfernt, auf
/// [`ocr::MAX_TEXT_CHARS`] begrenzt. Leer, nur Leerzeichen oder `KEINTEXT` ->
/// [`VisionError::Empty`].
pub fn clean_read(raw: &str, truncated: bool) -> Result<String, VisionError> {
    let mut lines: Vec<String> = raw
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty() && !l.starts_with("```"))
        .collect();
    if truncated && lines.len() > 1 {
        lines.pop();
    }
    let mut kept: Vec<String> = Vec::with_capacity(lines.len());
    let mut run = 0usize;
    for line in lines {
        if kept.last() == Some(&line) {
            run += 1;
            if run >= MAX_REPEATED_LINES {
                continue;
            }
        } else {
            run = 0;
        }
        kept.push(line);
    }
    let text = ocr::clean_text(&kept.join("\n"));
    if text.is_empty() || strip_decor(&text).eq_ignore_ascii_case("keintext") {
        return Err(VisionError::Empty);
    }
    Ok(text)
}

/// Darf Gemmas Text den OCR-Text ersetzen? Nur mit mindestens [`ocr::MIN_TEXT_WORDS`]
/// Woertern: eine Antwort wie "Kein Text sichtbar." ersetzt nie echten OCR-Text.
pub fn replaces_ocr(gemma: &str) -> bool {
    ocr::word_count(gemma) >= ocr::MIN_TEXT_WORDS
}

// ---------------------------------------------------------------------------
// Fragen
// ---------------------------------------------------------------------------

/// Liest den Text einer Folie (Zahlen und Tabellen inklusive). Bereinigt, nie leer.
pub fn read_slide(backend: &dyn VisionBackend, image: &Path) -> Result<String, VisionError> {
    let reply = backend.ask(image, READ_PROMPT, READ_MAX_TOKENS)?;
    clean_read(&reply.text, reply.truncated)
}

/// Klasse und ein Satz Beschreibung einer Folie. Eine Antwort ohne bekannte Klasse in der
/// ersten Zeile wird verworfen ([`VisionError::BadClass`]).
pub fn describe_slide(
    backend: &dyn VisionBackend,
    image: &Path,
) -> Result<SlideVision, VisionError> {
    let reply = backend.ask(image, DESCRIBE_PROMPT, DESCRIBE_MAX_TOKENS)?;
    parse_description(&reply.text)
}

/// Was die Bildanalyse einer Folie ergab. Jedes Feld `None`: nicht gefragt, verworfen
/// oder gescheitert; die Folie behaelt dann, was sie hat.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlideAnalysis {
    /// Gemmas Text, der den OCR-Text ersetzen darf ([`replaces_ocr`]).
    pub text: Option<String>,
    pub description: Option<String>,
    /// Gescheiterte Anfragen dieser Folie (ohne `Unavailable`).
    pub failed: u32,
    /// Der Server ist nicht zu haben: der Lauf soll nichts mehr fragen.
    pub unavailable: Option<String>,
}

/// Fragt zu EINER Folie: bei `read_text` zuerst den Text, bei `describe` danach die
/// Beschreibung. Ein Fehler einer Anfrage verwirft nur diese Antwort; `Unavailable` bricht ab.
pub fn analyze_slide(
    backend: &dyn VisionBackend,
    image: &Path,
    read_text: bool,
    describe: bool,
) -> SlideAnalysis {
    let mut out = SlideAnalysis::default();
    if read_text {
        match read_slide(backend, image) {
            Ok(text) if replaces_ocr(&text) => out.text = Some(text),
            Ok(_) => log::info!("slides: Gemma-Text zu kurz, OCR-Text bleibt"),
            Err(VisionError::Unavailable(why)) => {
                out.unavailable = Some(why);
                return out;
            }
            Err(e) => {
                log::warn!("slides: Text der Folie nicht gelesen ({})", e.code());
                out.failed += 1;
            }
        }
    }
    if !describe {
        return out;
    }
    match describe_slide(backend, image) {
        Ok(v) => out.description = Some(v.description),
        Err(VisionError::Unavailable(why)) => out.unavailable = Some(why),
        Err(e) => {
            log::warn!("slides: Folie nicht beschrieben ({})", e.code());
            out.failed += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::job::JobHandle;
    use crate::managers::meetings::slides::ocr::{OcrBackend, OcrError, OcrText};
    use crate::managers::meetings::slides::run::{run, SlideOutcome, SlideRun, SlideSummary};
    use crate::managers::meetings::slides::test_support::{
        ffmpeg_available, serial, shared_slide_video, SlideVideo,
    };
    use crate::managers::meetings::slides::{SlideDetectConfig, SLIDES_DIR};
    use crate::managers::meetings::store::{MeetingSource, MeetingStatus, MeetingStore};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    // ---- Parser ------------------------------------------------------------

    #[test]
    fn a_clean_two_line_answer_is_parsed() {
        let v = parse_description("textfolie\nEine Tabelle mit Umsätzen je Region.").unwrap();
        assert_eq!(v.class, "textfolie");
        assert_eq!(v.description, "Eine Tabelle mit Umsätzen je Region.");
    }

    /// Das echte Modell haelt sich nicht immer an den Wortlaut: Beschriftungen, Markdown,
    /// Satzzeichen, Gross-/Kleinschreibung, Leerzeilen und ein Satz ueber mehrere Zeilen.
    #[test]
    fn labels_markdown_and_blank_lines_are_tolerated() {
        let v = parse_description("Zeile 1: **Bildfolie**\n\nZeile 2: „Ein Foto vom Prüfstand.“")
            .unwrap();
        assert_eq!(v.class, "bildfolie");
        assert_eq!(v.description, "Ein Foto vom Prüfstand.");
        let v = parse_description("  Sprecher-Raum.  \nEine Person steht am Pult\nund spricht.")
            .unwrap();
        assert_eq!(v.class, "sprecher_raum");
        assert_eq!(v.description, "Eine Person steht am Pult und spricht.");
        let v = parse_description("Klasse: LEER\nBeschreibung: Schwarzes Bild.").unwrap();
        assert_eq!(
            (v.class, v.description.as_str()),
            ("leer", "Schwarzes Bild.")
        );
    }

    /// Spike: die Antwort VERWERFEN, wenn die erste Zeile keine bekannte Klasse ist.
    #[test]
    fn an_unknown_first_line_is_discarded() {
        for raw in [
            "Das Bild zeigt eine Folie.\nBalkendiagramm.",
            "kamera\nEin Raum.", // das alte Klassenwort
            "{\"art\": \"textfolie\"}",
            "Hier ist die Antwort:\ntextfolie\nSatz.",
            "textfolie Satz in derselben Zeile.",
        ] {
            assert_eq!(parse_description(raw), Err(VisionError::BadClass), "{raw}");
        }
    }

    /// Der Fehlerfall des Spikes: eine Leerzeichen-Schleife bis `max_tokens`.
    #[test]
    fn a_blank_answer_is_empty_not_text() {
        for raw in ["", " ", "   \n \t \n   ", &" ".repeat(4000), "\n\n\n"] {
            assert_eq!(parse_description(raw), Err(VisionError::Empty));
            assert_eq!(clean_read(raw, false), Err(VisionError::Empty));
            assert_eq!(clean_read(raw, true), Err(VisionError::Empty));
        }
        // Nur eine Klasse, kein Satz: nichts zu speichern.
        assert_eq!(parse_description("textfolie"), Err(VisionError::Empty));
        assert_eq!(
            parse_description("textfolie\n   \n"),
            Err(VisionError::Empty)
        );
    }

    #[test]
    fn a_long_description_is_capped_on_a_word_boundary() {
        let long = format!("textfolie\n{}", "Wort ".repeat(400));
        let v = parse_description(&long).unwrap();
        assert!(v.description.chars().count() <= MAX_DESCRIPTION_CHARS);
        assert!(v.description.ends_with("Wort"), "{}", v.description);
    }

    #[test]
    fn the_read_text_is_cleaned_line_by_line() {
        let raw = "```\nUmsatz  je   Region\n\nSüd | 6,8 Mio. €\n  Ost | 3,1 Mio. €  \n```";
        assert_eq!(
            clean_read(raw, false).unwrap(),
            "Umsatz je Region\nSüd | 6,8 Mio. €\nOst | 3,1 Mio. €"
        );
        assert_eq!(clean_read("KEINTEXT", false), Err(VisionError::Empty));
        assert_eq!(clean_read("**keintext**.", false), Err(VisionError::Empty));
    }

    #[test]
    fn a_truncated_read_loses_its_last_line_and_loops_are_cut() {
        let cut = clean_read("Zeile eins mit Text\nZeile zwei mit Text\nZeile dr", true).unwrap();
        assert_eq!(cut, "Zeile eins mit Text\nZeile zwei mit Text");
        // Eine einzelne Zeile bleibt (sonst gaebe es nichts).
        assert_eq!(
            clean_read("Nur eine Zeile", true).unwrap(),
            "Nur eine Zeile"
        );
        // Wiederholungsschleife: nie mehr als zwei gleiche Zeilen hintereinander.
        let looped = format!(
            "Titel der Folie\n{}Ende der Folie",
            "Wiederholung\n".repeat(50)
        );
        assert_eq!(
            clean_read(&looped, false).unwrap(),
            "Titel der Folie\nWiederholung\nWiederholung\nEnde der Folie"
        );
    }

    #[test]
    fn a_too_short_read_never_replaces_the_ocr_text() {
        assert!(replaces_ocr("Umsatz Region Süd 13,1"));
        assert!(!replaces_ocr("Kein Text."));
        assert!(!replaces_ocr("13,1"));
        assert!(!replaces_ocr(""));
    }

    // ---- Fragen mit einem Test-Leser --------------------------------------

    /// Antwortet je nach Frage; zaehlt, was gefragt wurde.
    struct FakeVision {
        read: Mutex<Box<dyn FnMut(usize) -> Result<VisionReply, VisionError> + Send>>,
        describe: Mutex<Box<dyn FnMut(usize) -> Result<VisionReply, VisionError> + Send>>,
        reads: AtomicUsize,
        describes: AtomicUsize,
        released: AtomicUsize,
        images: Mutex<Vec<PathBuf>>,
    }

    fn reply(text: &str) -> Result<VisionReply, VisionError> {
        Ok(VisionReply {
            text: text.to_string(),
            truncated: false,
        })
    }

    impl FakeVision {
        fn new(
            read: impl FnMut(usize) -> Result<VisionReply, VisionError> + Send + 'static,
            describe: impl FnMut(usize) -> Result<VisionReply, VisionError> + Send + 'static,
        ) -> Self {
            Self {
                read: Mutex::new(Box::new(read)),
                describe: Mutex::new(Box::new(describe)),
                reads: AtomicUsize::new(0),
                describes: AtomicUsize::new(0),
                released: AtomicUsize::new(0),
                images: Mutex::new(Vec::new()),
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::Acquire)
        }

        fn describes(&self) -> usize {
            self.describes.load(Ordering::Acquire)
        }

        fn released(&self) -> usize {
            self.released.load(Ordering::Acquire)
        }
    }

    impl VisionBackend for FakeVision {
        fn ask(
            &self,
            image: &Path,
            prompt: &str,
            max_tokens: u32,
        ) -> Result<VisionReply, VisionError> {
            assert!(
                image.is_file(),
                "das Bild muss beim Fragen da sein: {image:?}"
            );
            self.images.lock().unwrap().push(image.to_path_buf());
            if prompt == READ_PROMPT {
                assert_eq!(max_tokens, READ_MAX_TOKENS);
                let n = self.reads.fetch_add(1, Ordering::AcqRel);
                (self.read.lock().unwrap())(n)
            } else {
                assert_eq!(prompt, DESCRIBE_PROMPT);
                assert!(max_tokens <= 200, "Beschreibung: hoechstens 200 Token");
                let n = self.describes.fetch_add(1, Ordering::AcqRel);
                (self.describe.lock().unwrap())(n)
            }
        }

        fn release(&self) {
            self.released.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn some_image() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        std::fs::write(&path, b"x").unwrap();
        (dir, path)
    }

    #[test]
    fn read_and_describe_use_the_prompts_and_the_parsers() {
        let (_d, img) = some_image();
        let fake = FakeVision::new(
            |_| reply("Tabelle\nSüd | 13,1"),
            |_| reply("textfolie\nEine Tabelle."),
        );
        assert_eq!(read_slide(&fake, &img).unwrap(), "Tabelle\nSüd | 13,1");
        let v = describe_slide(&fake, &img).unwrap();
        assert_eq!(
            (v.class, v.description.as_str()),
            ("textfolie", "Eine Tabelle.")
        );
    }

    #[test]
    fn analyzing_a_slide_collects_text_and_description() {
        let (_d, img) = some_image();
        let fake = FakeVision::new(
            |_| reply("Umsatz Region Süd 13,1"),
            |_| reply("textfolie\nEine Umsatztabelle."),
        );
        let a = analyze_slide(&fake, &img, true, true);
        assert_eq!(a.text.as_deref(), Some("Umsatz Region Süd 13,1"));
        assert_eq!(a.description.as_deref(), Some("Eine Umsatztabelle."));
        assert_eq!((a.failed, a.unavailable), (0, None));
        // Ohne `read_text` nur die Beschreibung.
        let b = analyze_slide(&fake, &img, false, true);
        assert_eq!((b.text, b.failed), (None, 0));
        assert_eq!(fake.reads(), 1);
        assert_eq!(fake.describes(), 2);
    }

    #[test]
    fn a_failed_read_still_gets_its_description_and_unavailable_stops_at_once() {
        let (_d, img) = some_image();
        let fake = FakeVision::new(
            |_| Err(VisionError::Request("Verbindung".into())),
            |_| reply("bildfolie\nEin Foto."),
        );
        let a = analyze_slide(&fake, &img, true, true);
        assert_eq!((a.text, a.failed), (None, 1));
        assert_eq!(a.description.as_deref(), Some("Ein Foto."));

        let down = FakeVision::new(
            |_| Err(VisionError::Unavailable("Arbeitsspeicher".into())),
            |_| reply("bildfolie\nEin Foto."),
        );
        let a = analyze_slide(&down, &img, true, true);
        assert_eq!(a.unavailable.as_deref(), Some("Arbeitsspeicher"));
        assert_eq!(
            down.describes(),
            0,
            "nach Unavailable wird nichts mehr gefragt"
        );
    }

    // ---- Der Lauf ------------------------------------------------------------

    /// Texterkennung mit Drehbuch (je Hash-Gruppe ein Aufruf).
    struct ScriptedOcr {
        script: Vec<&'static str>,
        calls: AtomicUsize,
    }

    impl OcrBackend for ScriptedOcr {
        fn id(&self) -> &'static str {
            "fake-ocr"
        }

        fn language(&self) -> String {
            "de-DE".to_string()
        }

        fn recognize(&self, _image: &Path) -> Result<OcrText, OcrError> {
            let n = self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(OcrText {
                text: self.script.get(n).copied().unwrap_or("").to_string(),
                engine: "fake-ocr",
                language: "de-DE".to_string(),
            })
        }
    }

    /// Das 30-s-Video hat drei Folien: Folie 1 und 3 mit Text, Folie 2 wie ein Foto.
    const OCR_SCRIPT: [&str; 3] = [
        "Umsatz Region Sued 131 Prozent",
        "Foto",
        "Zeitplan Einfuehrung Pilotphase Rollout",
    ];

    /// Gemma-Texte der beiden Textfolien: verschiedene Folien haben verschiedenen Text (sonst
    /// fuehrte der Textabgleich einer Wiederholung sie zusammen).
    const GEMMA_1: &str = "Umsatz Region Süd\nUmsatz | 13,1 Prozent";
    const GEMMA_3: &str = "Zeitplan Einführung\nPilotphase | Rollout";

    struct Fx {
        _serial: std::sync::MutexGuard<'static, ()>,
        dir: tempfile::TempDir,
        store: MeetingStore,
        meeting: String,
        video: SlideVideo,
    }

    impl Fx {
        fn meeting_dir(&self) -> PathBuf {
            self.dir.path().join("meeting")
        }
    }

    fn fixture() -> Option<Fx> {
        if !ffmpeg_available() {
            eprintln!("ffmpeg fehlt - Test uebersprungen");
            return None;
        }
        let serial = serial();
        let dir = tempfile::tempdir().unwrap();
        let store = MeetingStore::open_at(&dir.path().join("m.db")).unwrap();
        let meeting = store
            .create_meeting("Vortrag", MeetingSource::Import, None)
            .unwrap()
            .id;
        store.set_status(&meeting, MeetingStatus::Ready).unwrap();
        Some(Fx {
            _serial: serial,
            dir,
            store,
            meeting,
            video: shared_slide_video(),
        })
    }

    fn run_slides(
        f: &Fx,
        ocr: Option<&dyn OcrBackend>,
        vision: Option<&dyn VisionBackend>,
    ) -> SlideSummary {
        let cfg = SlideDetectConfig::default();
        let dir = f.meeting_dir();
        let handle = JobHandle::new(&f.meeting, Arc::new(|_| {}));
        let ctx = SlideRun {
            store: &f.store,
            meeting_id: &f.meeting,
            video: &f.video.path,
            meeting_dir: &dir,
            cfg: &cfg,
            ocr,
            vision,
            pid_out: None,
        };
        run(&handle, &ctx).unwrap()
    }

    fn ocr() -> ScriptedOcr {
        ScriptedOcr {
            script: OCR_SCRIPT.to_vec(),
            calls: AtomicUsize::new(0),
        }
    }

    /// Standard: ohne Bildanalyse bleibt der OCR-Text, keine Beschreibung.
    #[test]
    fn without_a_vision_backend_the_slides_keep_their_ocr_text() {
        let Some(f) = fixture() else { return };
        let summary = run_slides(&f, Some(&ocr()), None);
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(summary.vision_model, None);
        assert_eq!((summary.vision_slides, summary.vision_failed), (0, 0));
        for slide in f.store.slides_list(&f.meeting).unwrap() {
            assert_eq!(slide.ocr_engine.as_deref(), Some("fake-ocr"));
            assert_eq!((slide.description, slide.description_model), (None, None));
        }
    }

    /// Akzeptanz: Bildanalyse an -> Gemmas Text ersetzt den OCR-Text (Zahlen!) einer Textfolie,
    /// jede Folie bekommt ihre Beschreibung, die Foto-Folie behaelt ihren OCR-Text.
    #[test]
    fn vision_text_replaces_ocr_and_every_slide_gets_a_description() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(
            |n| match n {
                0 => reply("Umsatz Region Süd\nUmsatz | 13,1 Prozent"),
                _ => reply("Zeitplan Einführung\nPilotphase | Rollout"),
            },
            |n| reply(&format!("textfolie\nBeschreibung {n}.")),
        );
        let summary = run_slides(&f, Some(&ocr()), Some(&fake));
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(summary.vision_model, Some(ENGINE_GEMMA));
        assert_eq!(summary.vision_slides, 3);
        assert_eq!(summary.vision_failed, 0);
        assert_eq!(
            fake.reads(),
            2,
            "nur die beiden Textfolien werden neu gelesen"
        );
        assert_eq!(fake.describes(), 3, "jede Folie wird beschrieben");

        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides.len(), 3);
        assert_eq!(
            slides[0].ocr_text.as_deref(),
            Some("Umsatz Region Süd\nUmsatz | 13,1 Prozent")
        );
        assert_eq!(slides[0].ocr_engine.as_deref(), Some(ENGINE_GEMMA));
        assert_eq!(slides[0].kind.as_deref(), Some("text"));
        assert_eq!(slides[0].description.as_deref(), Some("Beschreibung 0."));
        assert_eq!(slides[0].description_model.as_deref(), Some(ENGINE_GEMMA));
        // Die Foto-Folie: OCR-Text und Art bleiben, nur die Beschreibung kommt dazu.
        assert_eq!(slides[1].ocr_text.as_deref(), Some("Foto"));
        assert_eq!(slides[1].ocr_engine.as_deref(), Some("fake-ocr"));
        assert_eq!(slides[1].kind.as_deref(), Some("ohne_text"));
        assert_eq!(slides[1].description_model.as_deref(), Some(ENGINE_GEMMA));
        assert_eq!(slides[2].ocr_engine.as_deref(), Some(ENGINE_GEMMA));
        assert_eq!(slides[2].description.as_deref(), Some("Beschreibung 2."));
        // Kein Kandidatenordner und keine halben Bilder bleiben liegen.
        assert!(!f.meeting_dir().join(SLIDES_DIR).join(".cand").exists());
        assert_eq!(fake.released(), 1);
    }

    /// Fallback: Antworten ohne Wert (nur Leerzeichen, unbekannte Klasse) werden verworfen, der
    /// OCR-Text bleibt, und der Lauf fragt die naechsten Folien trotzdem.
    #[test]
    fn blank_and_unknown_answers_are_dropped_and_the_run_goes_on() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(
            |_| reply("      \n   "),
            |_| reply("Das ist eine Folie.\nMit Text."),
        );
        let summary = run_slides(&f, Some(&ocr()), Some(&fake));
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(fake.reads(), 2);
        assert_eq!(fake.describes(), 3, "alle Folien werden gefragt");
        assert_eq!(summary.vision_slides, 0);
        assert_eq!(
            summary.vision_failed, 5,
            "2 Texte + 3 Beschreibungen verworfen"
        );
        for slide in f.store.slides_list(&f.meeting).unwrap() {
            assert_eq!(
                slide.ocr_engine.as_deref(),
                Some("fake-ocr"),
                "OCR-Text bleibt"
            );
            assert_eq!(slide.description, None);
        }
        assert_eq!(fake.released(), 1);
    }

    /// Fallback: ist der Server nicht zu haben (RAM-Gate, Projektor weg), gibt es EINE Anfrage, nicht
    /// eine je Folie; der OCR-Text bleibt, der Lauf endet normal.
    #[test]
    fn an_unavailable_server_ends_the_analysis_and_keeps_the_ocr_text() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(
            |_| {
                Err(VisionError::Unavailable(
                    "Nicht genug freier Arbeitsspeicher".into(),
                ))
            },
            |_| reply("textfolie\nSatz."),
        );
        let summary = run_slides(&f, Some(&ocr()), Some(&fake));
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!(fake.reads() + fake.describes(), 1, "genau eine Anfrage");
        assert_eq!(summary.vision_slides, 0);
        assert_eq!(
            summary.vision_unavailable.as_deref(),
            Some("Nicht genug freier Arbeitsspeicher")
        );
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides.len(), 3, "die Folien sind trotzdem da");
        assert_eq!(slides[0].ocr_text.as_deref(), Some(OCR_SCRIPT[0]));
        assert_eq!(slides[0].ocr_engine.as_deref(), Some("fake-ocr"));
        assert_eq!(fake.released(), 1, "auch dann wird freigegeben");
    }

    /// Zu kurzer Gemma-Text ersetzt den OCR-Text nie.
    #[test]
    fn a_too_short_gemma_text_keeps_the_ocr_text_in_the_run() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(|_| reply("Kein Text."), |_| reply("textfolie\nSatz."));
        let summary = run_slides(&f, Some(&ocr()), Some(&fake));
        assert_eq!(summary.vision_slides, 3, "beschrieben sind sie trotzdem");
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides[0].ocr_text.as_deref(), Some(OCR_SCRIPT[0]));
        assert_eq!(slides[0].ocr_engine.as_deref(), Some("fake-ocr"));
        assert_eq!(slides[0].description.as_deref(), Some("Satz."));
    }

    /// Wiederholung: eine Folie mit Gemma-Text UND Beschreibung wird nicht erneut gefragt.
    #[test]
    fn a_rerun_asks_nothing_twice() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(
            |n| reply(if n == 0 { GEMMA_1 } else { GEMMA_3 }),
            |_| reply("textfolie\nEine Folie."),
        );
        run_slides(&f, Some(&ocr()), Some(&fake));
        let (reads, describes) = (fake.reads(), fake.describes());
        assert_eq!((reads, describes), (2, 3));
        let again = run_slides(&f, Some(&ocr()), Some(&fake));
        assert_eq!(again.outcome, SlideOutcome::Done);
        assert_eq!(
            (fake.reads(), fake.describes()),
            (reads, describes),
            "nichts wird zweimal gefragt"
        );
        assert_eq!(fake.released(), 2, "jeder Lauf gibt frei");
        assert_eq!(f.store.slides_list(&f.meeting).unwrap().len(), 3);
    }

    /// Eine Folie mit Gemma-Text, aber ohne Beschreibung (die letzte scheiterte): nur die
    /// Beschreibung wird nachgeholt, der Text nicht noch einmal gelesen.
    #[test]
    fn a_rerun_fills_only_the_missing_description() {
        let Some(f) = fixture() else { return };
        let first = FakeVision::new(
            |n| reply(if n == 0 { GEMMA_1 } else { GEMMA_3 }),
            |_| Err(VisionError::Request("Verbindung".into())),
        );
        let summary = run_slides(&f, Some(&ocr()), Some(&first));
        assert_eq!(summary.vision_failed, 3);
        let second = FakeVision::new(
            |_| reply("nicht gefragt"),
            |_| reply("textfolie\nNachgeholt."),
        );
        run_slides(&f, Some(&ocr()), Some(&second));
        assert_eq!(second.reads(), 0, "der Gemma-Text steht schon");
        assert_eq!(second.describes(), 3);
        let slides = f.store.slides_list(&f.meeting).unwrap();
        assert_eq!(slides[0].description.as_deref(), Some("Nachgeholt."));
        assert_eq!(slides[0].ocr_engine.as_deref(), Some(ENGINE_GEMMA));
    }

    /// Ohne Texterkennung (macOS, keine OCR-Sprache) ersetzt die Bildanalyse nichts und laeuft nicht.
    #[test]
    fn without_ocr_the_vision_is_never_asked() {
        let Some(f) = fixture() else { return };
        let fake = FakeVision::new(
            |_| reply("Text mit vielen Woertern"),
            |_| reply("textfolie\nSatz."),
        );
        let summary = run_slides(&f, None, Some(&fake));
        assert_eq!(summary.outcome, SlideOutcome::Done);
        assert_eq!((fake.reads(), fake.describes()), (0, 0));
        assert_eq!(summary.vision_model, None);
        for slide in f.store.slides_list(&f.meeting).unwrap() {
            assert_eq!((slide.ocr_text, slide.description), (None, None));
        }
    }

    // ---- Echter Lauf (nicht Teil der Suite) ---------------------------------------

    /// Ein Leser gegen einen laufenden Server (`post_image_prompt`), ohne App-Zustand.
    struct DirectVision {
        base: String,
    }

    impl VisionBackend for DirectVision {
        fn ask(
            &self,
            image: &Path,
            prompt: &str,
            max_tokens: u32,
        ) -> Result<VisionReply, VisionError> {
            let base = self.base.clone();
            let (image, prompt) = (image.to_path_buf(), prompt.to_string());
            let reply = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(crate::llm_client::post_image_prompt(
                        &base, "gemma", &image, &prompt, max_tokens,
                    ))
            })
            .join()
            .expect("Anfrage-Thread")
            .map_err(VisionError::Request)?;
            Ok(VisionReply {
                text: reply.content,
                truncated: reply.truncated,
            })
        }
    }

    /// ECHTER LAUF, nicht Teil der Suite (`cargo test ... -- --ignored real_gemma`): startet
    /// den gebuendelten `llama-server` mit Gemma 4 E4B und Projektor (RAM-Start-Gate, Job-Objekt,
    /// Beenden ueber die PID) und liest eine Tabellenfolie. Erwartung laut Spike: der Wert
    /// "13,1" kommt korrekt zurueck (Windows-OCR verliert ihn), die Beschreibung hat eine
    /// bekannte Klasse. Braucht GPU, ~5 GB Grafikspeicher, ~20 s Modellstart.
    ///
    /// Umgebung: `LVA_D3_SERVER` (llama-server.exe), `LVA_D3_MODEL` (gemma-4-E4B-it-Q4_K_M.gguf),
    /// `LVA_D3_MMPROJ` (gemma-4-E4B-it-mmproj-F16.gguf), `LVA_D3_SLIDE` (Bild der Tabellenfolie
    /// mit dem Wert 13,1, z. B. `spike/frames/s04.png`). Nicht in dieser Arbeit ausgefuehrt.
    #[test]
    #[ignore = "echter Lauf: startet llama-server mit Gemma 4 E4B und Projektor (GPU, ~5 GB)"]
    fn real_gemma_reads_the_table_slide_with_its_numbers() {
        let var =
            |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} nicht gesetzt"));
        let (server, model, mmproj, slide) = (
            PathBuf::from(var("LVA_D3_SERVER")),
            PathBuf::from(var("LVA_D3_MODEL")),
            PathBuf::from(var("LVA_D3_MMPROJ")),
            PathBuf::from(var("LVA_D3_SLIDE")),
        );
        let free_mb = crate::process_guard::check_ram_for_start(10 * 1024).expect("RAM-Start-Gate");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let opts = crate::managers::llm::StartOptions {
            model_id: "llm-gemma4-e4b-q4".into(),
            model_path: model.clone(),
            backend: "cuda".into(),
            context_tokens: 8192,
            gpu_layers: 99,
            embedding: None,
            mmproj: Some(mmproj),
        };
        let args = crate::managers::llm::server::server_args(&opts, port, 8);
        let mut cmd = std::process::Command::new(&server);
        cmd.arg("-m").arg(&model).args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd.spawn().expect("llama-server startet");
        let _guard = crate::process_guard::ProcessGuard::attach(
            &child,
            Some(crate::process_guard::memory_limit_mb(free_mb)),
            crate::process_guard::CPU_CAP_PERCENT,
        );
        let health = format!("http://127.0.0.1:{port}/health");
        let started = std::time::Instant::now();
        let ready = loop {
            if child.try_wait().unwrap().is_some() || started.elapsed().as_secs() > 180 {
                break false;
            }
            let ok = std::process::Command::new("curl")
                .args(["-s", "-f", "-o", "NUL", &health])
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                break true;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        let outcome = std::panic::catch_unwind(|| {
            assert!(ready, "llama-server wurde nicht bereit");
            let vision = DirectVision {
                base: format!("http://127.0.0.1:{port}/v1"),
            };
            let text = read_slide(&vision, &slide).expect("Text gelesen");
            assert!(text.contains("13,1"), "Tabellenwert fehlt: {text}");
            let described = describe_slide(&vision, &slide).expect("beschrieben");
            assert!(CLASSES.contains(&described.class));
        });
        // Beenden ausschliesslich ueber die eigene PID (Prozessbaum), nie ueber den Namen.
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .output();
        let _ = child.wait();
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }
}
