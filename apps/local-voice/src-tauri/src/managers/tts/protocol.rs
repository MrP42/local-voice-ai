//! Pure Bausteine des Fish-Speech-HTTP-Protokolls: URL, Request-Körper,
//! Text-Vorbereitung und WAV-Plausibilitätsprüfung. Bewusst ohne I/O,
//! damit jede Regel ohne Server testbar ist.

pub fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// Emotions-Tag-Spans (`[…]`) als Byte-Ranges. Ein Tag reicht von `[` bis zum
/// nächsten `]` OHNE Zeilenumbruch dazwischen — eine vergessene schließende
/// Klammer darf nie den Resttext (womöglich über mehrere Zeilen) zum Tag
/// machen. Keine Verschachtelung: das erste `]` schließt, auch wenn davor ein
/// weiteres `[` steht.
pub fn tag_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            let mut closed_at = None;
            while j < bytes.len() {
                match bytes[j] {
                    b']' => {
                        closed_at = Some(j);
                        break;
                    }
                    b'\n' => break,
                    _ => {}
                }
                j += 1;
            }
            if let Some(end) = closed_at {
                spans.push(i..end + 1);
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    spans
}

/// Zahl der Zeichen außerhalb aller Tag-Spans — die Länge, die für den
/// Hörer tatsächlich vorgelesen wird.
fn visible_char_count(s: &str) -> usize {
    let spans = tag_spans(s);
    s.char_indices()
        .filter(|(idx, _)| !spans.iter().any(|span| span.contains(idx)))
        .count()
}

/// Serialisierungsstil je Modellgeneration: S2-Pro versteht `[…]` nativ
/// (`Square`), ein künftiges S1-mini-Ziel bräuchte `(…)` (`Paren`).
/// Nicht produktiv verwendet, bevor S1-mini angebunden ist — daher
/// `#[allow(dead_code)]` auf der Variante.
pub enum TagStyle {
    Square,
    #[allow(dead_code)]
    Paren,
}

/// Übersetzt Emotions-Tags für die Zielgeneration. `Square` lässt den Text
/// unverändert. `Paren` ersetzt `[x]` durch `(y)`, wenn `x` (getrimmt,
/// case-insensitiv) im `known_paren`-Mapping (Square-Name → S1-Name) steht;
/// unbekannte (Freitext-)Tags werden ersatzlos entfernt, doppelte Leerzeichen
/// dabei zusammengefasst.
pub fn serialize_tags(text: &str, style: TagStyle, known_paren: &[(String, String)]) -> String {
    match style {
        TagStyle::Square => text.to_string(),
        TagStyle::Paren => {
            let spans = tag_spans(text);
            let mut result = String::with_capacity(text.len());
            let mut last = 0usize;
            for span in &spans {
                result.push_str(&text[last..span.start]);
                let inner = &text[span.start + 1..span.end - 1];
                let key = inner.trim().to_lowercase();
                if let Some((_, s1_name)) = known_paren
                    .iter()
                    .find(|(square, _)| square.trim().to_lowercase() == key)
                {
                    result.push('(');
                    result.push_str(s1_name);
                    result.push(')');
                }
                last = span.end;
            }
            result.push_str(&text[last..]);
            collapse_double_spaces(&result)
        }
    }
}

/// Fasst Läufe mehrerer Leerzeichen (Artefakt entfernter Freitext-Tags) zu
/// einem einzigen zusammen.
fn collapse_double_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c == ' ' {
            if prev_space {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        out.push(c);
    }
    out
}

pub struct PreparedText {
    pub text: String,
    pub truncated: bool,
}

/// Leer/Whitespace → None (kein Serverstart für nichts). Längenkappung in
/// Zeichen, damit kein UTF-8-Zeichen zerschnitten wird. Fällt die
/// Schnittgrenze in einen Tag-Span, wird stattdessen VOR dem Tag geschnitten
/// (das Tag verschwindet ganz, statt halbiert im Text zu landen).
pub fn prepare_text(raw: &str, max_chars: u32) -> Option<PreparedText> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let max = max_chars as usize;
    let count = trimmed.chars().count();
    if count <= max {
        return Some(PreparedText {
            text: trimmed.to_string(),
            truncated: false,
        });
    }
    let cut = trimmed
        .char_indices()
        .nth(max)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(trimmed.len());
    let spans = tag_spans(trimmed);
    let cut = spans
        .iter()
        .find(|span| cut > span.start && cut < span.end)
        .map(|span| span.start)
        .unwrap_or(cut);
    Some(PreparedText {
        text: trimmed[..cut].to_string(),
        truncated: true,
    })
}

/// Non-Streaming-WAV-Request. Ohne Referenzstimme hält der feste Seed die
/// Zufallsstimme zwischen Aufträgen stabil; mit Stimme wählt `reference_id`
/// die geklonte Stimme und `use_memory_cache` lässt den Server das
/// Referenz-Encoding zwischen Requests wiederverwenden.
pub fn tts_request_body(text: &str, seed: i64, reference_id: Option<&str>) -> serde_json::Value {
    tts_request_body_in_format(text, seed, reference_id, "wav")
}

/// Wie `tts_request_body`, aber mit wählbarem Ausgabeformat — der Fish-Server
/// encodiert wav/mp3/opus direkt (Datei-Export).
pub fn tts_request_body_in_format(
    text: &str,
    seed: i64,
    reference_id: Option<&str>,
    format: &str,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "text": serialize_tags(text, TagStyle::Square, &[]),
        "format": format,
        "seed": seed,
        "streaming": false,
    });
    if let Some(voice) = reference_id {
        body["reference_id"] = serde_json::json!(voice);
        body["use_memory_cache"] = serde_json::json!("on");
    }
    body
}

/// Formatbewusste Plausibilitätsprüfung der Serverantwort: Magic-Bytes plus
/// nennenswerte Nutzlast, damit HTML-Fehlerseiten nie als Audio durchgehen.
pub fn looks_like_audio(bytes: &[u8], format: &str) -> bool {
    if bytes.len() <= 1024 {
        return false;
    }
    match format {
        "wav" => bytes.starts_with(b"RIFF"),
        "mp3" => bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF]),
        "opus" => bytes.starts_with(b"OggS"),
        _ => false,
    }
}

/// RIFF-Magic plus nennenswerte Nutzlast (>1 KiB): filtert HTML-Fehlerseiten
/// und leere Antworten, ohne einen vollen WAV-Parser zu brauchen.
pub fn looks_like_wav(bytes: &[u8]) -> bool {
    bytes.len() > 1024 && bytes.starts_with(b"RIFF")
}

/// Zerlegt Text an Satzenden für die Sprech-Pipeline: Satz 1 wird abgespielt,
/// während Satz 2 schon synthetisiert wird — die gefühlte Latenz ist damit die
/// Synthese des ERSTEN Satzes, nicht des ganzen Textes.
///
/// Ein Schnitt passiert nur an `.!?…` vor Whitespace UND wenn das bisherige
/// Stück mindestens 15 Zeichen hat — das lässt deutsche Abkürzungen
/// („z. B.", „Dr.") zusammen, statt sie als Mini-Sätze vorzulesen.
/// Eine bekannte Stimme: `id` ist der voice_id-Wert fürs Backend, `names`
/// alle Anzeigenamen/Aliase, gegen die Sprecher-Marker im Text abgeglichen
/// werden (z. B. Vorname und vollständiger Anzeigename).
pub struct KnownSpeaker {
    pub id: String,
    pub names: Vec<String>,
}

/// Ein Stück Vorlesetext samt Stimme und optionalem Stil (`<Name:Stil>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerSegment {
    pub voice: Option<String>,
    pub style: Option<String>,
    pub text: String,
}

/// Ein Spitzklammer-Kandidat `<…>` — noch nicht gegen bekannte Sprecher
/// geprüft. `start`/`end` sind Byte-Offsets im untersuchten Text (inklusive
/// der Klammern selbst).
pub struct MarkerCandidate {
    pub start: usize,
    pub end: usize,
    pub name: String,
    pub style: Option<String>,
}

/// Scannt char-basiert (kein Regex-Crate) nach `<`…`>`-Kandidaten ohne `<`,
/// `>` oder Zeilenumbruch dazwischen, mit 1–60 Zeichen Inhalt. Stil wird am
/// ERSTEN `:` abgetrennt; Name und Stil werden getrimmt. Ob der Name zu einer
/// bekannten Stimme gehört, entscheidet erst der Aufrufer — ein Kandidat ist
/// noch keine Zusage (z. B. `<div>` aus eingefügtem HTML bleibt Kandidat,
/// schaltet aber nichts, weil kein bekannter Name matcht).
pub fn scan_marker_candidates(text: &str) -> Vec<MarkerCandidate> {
    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            let mut closed_at = None;
            while j < bytes.len() {
                match bytes[j] {
                    b'>' => {
                        closed_at = Some(j);
                        break;
                    }
                    b'<' | b'\n' => break,
                    _ => {}
                }
                j += 1;
            }
            if let Some(end) = closed_at {
                let inner = &text[i + 1..end];
                let char_len = inner.chars().count();
                if (1..=60).contains(&char_len) {
                    let (name, style) = match inner.split_once(':') {
                        Some((n, s)) => (n.trim().to_string(), Some(s.trim().to_string())),
                        None => (inner.trim().to_string(), None),
                    };
                    candidates.push(MarkerCandidate {
                        start: i,
                        end: end + 1,
                        name,
                        // Leerer Stil nach dem Trimmen (`<Anna: >`) zählt als
                        // "kein Stil" — das Backend ist nachsichtig, Blockieren
                        // ist Editor-Sache.
                        style: style.filter(|s| !s.is_empty()),
                    });
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    candidates
}

/// Sucht unter allen `speakers` einen, dessen Namen `name` matcht.
/// Unicode-Kleinschreibung (`to_lowercase`, NICHT `eq_ignore_ascii_case`),
/// damit z. B. „MÜLLER" „müller" matcht.
fn resolve_speaker<'a>(name: &str, speakers: &'a [KnownSpeaker]) -> Option<&'a KnownSpeaker> {
    let needle = name.to_lowercase();
    speakers
        .iter()
        .find(|speaker| speaker.names.iter().any(|n| n.to_lowercase() == needle))
}

/// Alt-Format: eine Zeile, die mit einem bekannten Namen und einem
/// Doppelpunkt beginnt (`olga: Guten Morgen.`, `Frau Müller: Guten Tag.`),
/// schaltet auf diese Stimme um. „Achtung: nicht vergessen" fängt genauso an,
/// ist aber keine Sprecherzeile — ohne den Abgleich gegen `speakers` würde
/// jeder Doppelpunkt am Zeilenanfang Text verschlucken.
fn alt_speaker_marker<'a>(
    line: &str,
    speakers: &'a [KnownSpeaker],
) -> Option<(&'a KnownSpeaker, String)> {
    let trimmed = line.trim_start();
    let (name, rest) = trimmed.split_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let speaker = resolve_speaker(name, speakers)?;
    Some((speaker, rest.to_string()))
}

/// Verarbeitet ein Text-Stück (eine Zeile oder deren Rest nach einem
/// Alt-Format-Marker): schaltet bei jedem bekannten `<Name>`/`<Name:Stil>`-
/// Kandidaten um und flusht den bisherigen Puffer unter der bisherigen
/// Stimme. Unbekannte Kandidaten werden nicht herausgeschnitten — ihr Text
/// bleibt wörtlich erhalten.
fn process_speaker_chunk(
    chunk: &str,
    speakers: &[KnownSpeaker],
    current_voice: &mut Option<String>,
    current_style: &mut Option<String>,
    buffer: &mut String,
    segments: &mut Vec<SpeakerSegment>,
) {
    let mut last = 0usize;
    for candidate in scan_marker_candidates(chunk) {
        if let Some(speaker) = resolve_speaker(&candidate.name, speakers) {
            buffer.push_str(&chunk[last..candidate.start]);
            flush_speaker_segment(segments, current_voice, current_style, buffer);
            *current_voice = Some(speaker.id.clone());
            *current_style = candidate.style.clone();
            last = candidate.end;
        }
    }
    buffer.push_str(&chunk[last..]);
}

fn flush_speaker_segment(
    segments: &mut Vec<SpeakerSegment>,
    voice: &Option<String>,
    style: &Option<String>,
    buffer: &mut String,
) {
    if !buffer.trim().is_empty() {
        segments.push(SpeakerSegment {
            voice: voice.clone(),
            style: style.clone(),
            text: buffer.trim().to_string(),
        });
    }
    buffer.clear();
}

/// Zerlegt Vorlesetext in Abschnitte je Sprecher — neue Spitzklammer-Syntax
/// (`<Name>`, `<Name:Stil>`, inline oder am Zeilenanfang) und das alte
/// „Name:"-Zeilenformat gemischt. Ein Marker gilt ab seiner Stelle bis zum
/// nächsten; Text vor dem ersten Marker gehört der eingestellten Stimme
/// (`voice: None`). Marker innerhalb eines Tag-Spans (`[…<x>…]`) werden nicht
/// gesondert behandelt.
pub fn split_speaker_segments(text: &str, speakers: &[KnownSpeaker]) -> Vec<SpeakerSegment> {
    let mut segments: Vec<SpeakerSegment> = Vec::new();
    let mut current_voice: Option<String> = None;
    let mut current_style: Option<String> = None;
    let mut buffer = String::new();

    for line in text.lines() {
        match alt_speaker_marker(line, speakers) {
            Some((speaker, rest)) => {
                flush_speaker_segment(&mut segments, &current_voice, &current_style, &mut buffer);
                current_voice = Some(speaker.id.clone());
                current_style = None;
                process_speaker_chunk(
                    &rest,
                    speakers,
                    &mut current_voice,
                    &mut current_style,
                    &mut buffer,
                    &mut segments,
                );
            }
            None => {
                process_speaker_chunk(
                    line,
                    speakers,
                    &mut current_voice,
                    &mut current_style,
                    &mut buffer,
                    &mut segments,
                );
            }
        }
        buffer.push('\n');
    }
    flush_speaker_segment(&mut segments, &current_voice, &current_style, &mut buffer);
    segments
}

/// Tag-bewusst: Satzzeichen INNERHALB eines Tag-Spans (`[…]`) beenden nie
/// einen Satz, damit Freitext-Tags wie `[dead tired, end of a long shift.]`
/// nicht mittendrin zerschnitten werden. Die Mindestchunk-Länge zählt über
/// `visible_char_count`, damit z. B. `[laughing] Hi.` nicht als voller Satz
/// gilt (nur 4 sichtbare Zeichen).
pub fn split_sentences(text: &str) -> Vec<String> {
    const MIN_CHUNK_CHARS: usize = 15;
    let spans = tag_spans(text);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut sentences = Vec::new();
    let mut start = 0usize;
    for (pos, &(byte_idx, c)) in chars.iter().enumerate() {
        let is_end = matches!(c, '.' | '!' | '?' | '…');
        if !is_end || spans.iter().any(|span| span.contains(&byte_idx)) {
            continue;
        }
        let next_byte_idx = chars.get(pos + 1).map(|(b, _)| *b).unwrap_or(text.len());
        let next_is_boundary = chars.get(pos + 1).is_none_or(|(_, n)| n.is_whitespace());
        if !next_is_boundary {
            continue;
        }
        let chunk = text[start..next_byte_idx].trim();
        if visible_char_count(chunk) >= MIN_CHUNK_CHARS {
            sentences.push(chunk.to_string());
            start = next_byte_idx;
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    sentences
}

/// Ein Stueck Vorlesetext: entweder zu sprechen oder Stille (Millisekunden).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechPart {
    Speak(String),
    Silence(u32),
}

/// Dauer eines Pausen-Tags, oder `None` für jedes andere Tag.
///
/// Nur diese vier Namen erzeugen Stille; alles andere (`[whisper]`,
/// Freitext-Stimmungen) bleibt im gesprochenen Text stehen — das versteht das
/// Modell selbst. Vergleich getrimmt und case-insensitiv, damit `[ Pause ]`
/// genauso zählt wie `[pause]`.
fn pause_millis(inner: &str) -> Option<u32> {
    match inner.trim().to_lowercase().as_str() {
        "pause" => Some(500),
        "short pause" => Some(250),
        "long pause" => Some(1000),
        "break" => Some(700),
        _ => None,
    }
}

/// Zerlegt Vorlesetext an Pausen-Tags in Sprech- und Stille-Abschnitte.
///
/// Eine Pause ist Stille definierter Länge — dafür braucht es kein
/// Sprachmodell. Die App erzeugt sie selbst, damit sie in jeder Sprache und
/// mit jeder Engine wirkt; das Modell kennt `[long pause]`/`[break]` gar
/// nicht und läse sie sonst wörtlich vor.
///
/// Die Klammer-Regeln kommen unverändert aus `tag_spans` (kein Zeilenumbruch
/// im Tag, das erste `]` schließt). Leere Sprech-Stücke entstehen nicht:
/// Pausen am Anfang, am Ende oder direkt hintereinander liefern nur Stille.
pub fn split_pauses(text: &str) -> Vec<SpeechPart> {
    let mut parts = Vec::new();
    let mut buffer = String::new();
    let mut last = 0usize;
    for span in tag_spans(text) {
        let inner = &text[span.start + 1..span.end - 1];
        if let Some(ms) = pause_millis(inner) {
            buffer.push_str(&text[last..span.start]);
            flush_speak(&mut buffer, &mut parts);
            parts.push(SpeechPart::Silence(ms));
            last = span.end;
        }
    }
    buffer.push_str(&text[last..]);
    flush_speak(&mut buffer, &mut parts);
    parts
}

fn flush_speak(buffer: &mut String, parts: &mut Vec<SpeechPart>) {
    let trimmed = buffer.trim();
    if !trimmed.is_empty() {
        parts.push(SpeechPart::Speak(trimmed.to_string()));
    }
    buffer.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_always_loopback() {
        assert_eq!(base_url(8080), "http://127.0.0.1:8080");
        assert_eq!(base_url(9000), "http://127.0.0.1:9000");
    }

    #[test]
    fn empty_or_whitespace_text_is_rejected() {
        assert!(prepare_text("", 100).is_none());
        assert!(prepare_text("   \n\t", 100).is_none());
    }

    #[test]
    fn overlong_text_is_truncated_at_a_char_boundary() {
        // 'ä' ist 2 Bytes; die Grenze zählt Zeichen, nicht Bytes.
        let p = prepare_text("ääääää", 4).unwrap();
        assert_eq!(p.text, "ääää");
        assert!(p.truncated);
        let ok = prepare_text("kurz", 100).unwrap();
        assert_eq!(ok.text, "kurz");
        assert!(!ok.truncated);
    }

    #[test]
    fn request_body_pins_wav_and_seed_and_disables_streaming() {
        let b = tts_request_body("Hallo", 42, None);
        assert_eq!(b["text"], "Hallo");
        assert_eq!(b["format"], "wav");
        assert_eq!(b["seed"], 42);
        assert_eq!(b["streaming"], false);
        assert!(
            b.get("reference_id").is_none(),
            "ohne Stimme kein reference_id-Feld"
        );
        assert!(b.get("use_memory_cache").is_none());
    }

    #[test]
    fn request_body_carries_the_voice_and_enables_the_reference_cache() {
        let b = tts_request_body("Hallo", 42, Some("patrick"));
        assert_eq!(b["reference_id"], "patrick");
        assert_eq!(b["use_memory_cache"], "on");
        assert_eq!(b["seed"], 42, "Seed bleibt für deterministisches Sampling");
    }

    #[test]
    fn sentences_split_at_real_boundaries() {
        let text = "Hallo Patrick, schön dich zu hören. Wie geht es dir heute? Alles klar!";
        assert_eq!(
            split_sentences(text),
            vec![
                "Hallo Patrick, schön dich zu hören.".to_string(),
                "Wie geht es dir heute?".to_string(),
                "Alles klar!".to_string(),
            ]
        );
    }

    #[test]
    fn abbreviations_do_not_produce_mini_sentences() {
        let text = "Das ist z. B. ein Satz mit Abkürzung. Und hier kommt noch ein zweiter Satz.";
        let parts = split_sentences(text);
        assert_eq!(parts.len(), 2, "war: {parts:?}");
        assert!(parts[0].contains("z. B."));
    }

    #[test]
    fn single_or_empty_text_stays_whole() {
        assert_eq!(
            split_sentences("Nur ein Satz ohne Ende"),
            vec!["Nur ein Satz ohne Ende"]
        );
        assert!(split_sentences("   ").is_empty());
    }

    #[test]
    fn sprecherzeilen_schalten_die_stimme_um() {
        let voices = ids(&["olga", "patrick"]);
        let text = "Vorspann ohne Sprecher.
olga: Guten Morgen.
Wie geht es dir?
patrick: Danke, gut.";
        let segments = split_speaker_segments(text, &voices);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].voice, None, "Text vor der ersten Markierung");
        assert_eq!(segments[0].text, "Vorspann ohne Sprecher.");
        assert_eq!(segments[1].voice.as_deref(), Some("olga"));
        assert_eq!(
            segments[1].text,
            "Guten Morgen.
Wie geht es dir?",
            "die Folgezeile gehoert noch olga"
        );
        assert_eq!(segments[2].voice.as_deref(), Some("patrick"));
        assert_eq!(segments[2].text, "Danke, gut.");
    }

    #[test]
    fn ein_gewoehnlicher_doppelpunkt_ist_keine_sprecherzeile() {
        let voices = ids(&["olga"]);
        let segments = split_speaker_segments("Achtung: nicht vergessen.", &voices);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice, None);
        assert_eq!(
            segments[0].text, "Achtung: nicht vergessen.",
            "der Text darf nicht angeknabbert werden"
        );
    }

    #[test]
    fn sprechernamen_sind_gross_klein_egal_und_duerfen_leer_ausgehen() {
        let voices = ids(&["Olga"]);
        let segments = split_speaker_segments(
            "OLGA:
Erste Zeile.",
            &voices,
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(
            segments[0].voice.as_deref(),
            Some("Olga"),
            "gemeldet wird die Stimme, wie sie wirklich heisst"
        );
        assert_eq!(segments[0].text, "Erste Zeile.");
    }

    #[test]
    fn ohne_bekannte_stimmen_bleibt_alles_ein_stueck() {
        let segments = split_speaker_segments(
            "olga: Hallo.
patrick: Hi.",
            &[],
        );
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice, None);
    }

    #[test]
    fn export_formats_reach_the_server_and_are_validated_by_magic() {
        let b = tts_request_body_in_format("Hallo", 42, None, "mp3");
        assert_eq!(b["format"], "mp3");
        let mut mp3 = b"ID3".to_vec();
        mp3.extend_from_slice(&[0u8; 2000]);
        assert!(looks_like_audio(&mp3, "mp3"));
        let mut ogg = b"OggS".to_vec();
        ogg.extend_from_slice(&[0u8; 2000]);
        assert!(looks_like_audio(&ogg, "opus"));
        assert!(
            !looks_like_audio(&ogg, "wav"),
            "falsches Magic je Format zählt nicht"
        );
        assert!(
            !looks_like_audio(b"OggS", "opus"),
            "Mini-Antworten sind Fehlerseiten"
        );
    }

    #[test]
    fn wav_check_wants_riff_and_some_payload() {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&[0u8; 2000]);
        assert!(looks_like_wav(&wav));
        assert!(!looks_like_wav(b"RIFF")); // nur Header, kein Audio
        assert!(!looks_like_wav(b"<html>error</html>xxxxxxxxxxxxxxxx"));
    }

    // ---- Teil 1: Tag-Sicherheit ----------------------------------------

    #[test]
    fn tag_span_umfasst_klammer_bis_klammer() {
        let spans = tag_spans("Vorher [whisper] Nachher");
        assert_eq!(spans, vec![7..16]);
        assert_eq!(&"Vorher [whisper] Nachher"[7..16], "[whisper]");
    }

    #[test]
    fn tag_span_erkennt_freitext_mit_satzzeichen() {
        let text = "[dead tired, end of a long shift.] Hallo.";
        let spans = tag_spans(text);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            &text[spans[0].clone()],
            "[dead tired, end of a long shift.]"
        );
    }

    #[test]
    fn ungeschlossene_klammer_endet_am_zeilenende_und_wird_kein_tag() {
        let text = "Text [vergessen\nzweite Zeile ohne Klammer";
        assert!(
            tag_spans(text).is_empty(),
            "eine vergessene schliessende Klammer darf nicht den Resttext verschlucken"
        );
    }

    #[test]
    fn tags_sind_nicht_verschachtelt_erstes_schliesst() {
        let text = "[a [b] c]";
        let spans = tag_spans(text);
        assert_eq!(spans, vec![0..6], "erstes ']' schliesst, Rest bleibt Text");
        assert_eq!(&text[spans[0].clone()], "[a [b]");
    }

    #[test]
    fn visible_char_count_zaehlt_nur_text_ausserhalb_von_tags() {
        assert_eq!(visible_char_count("[laughing] Hi."), 4, "nur ' Hi.' zaehlt");
        assert_eq!(visible_char_count("Ganz normal."), 12);
    }

    #[test]
    fn tag_mit_satzzeichen_bleibt_beim_satz_splitten_ganz() {
        let text = "Sie sagte [dead tired, end of a long shift.] Wir machen weiter morgen frueh.";
        let parts = split_sentences(text);
        assert_eq!(parts.len(), 1, "war: {parts:?}");
        assert!(parts[0].contains("[dead tired, end of a long shift.]"));
    }

    #[test]
    fn satzgrenze_direkt_nach_tag_wird_erkannt() {
        // Das Satzende-Zeichen folgt direkt (ohne Leerzeichen) auf ein
        // schliessendes ']' — das darf die Grenzerkennung nicht verwirren.
        let text = "Ich fluestere jetzt[ganz leise]. Und rede dann wieder normal weiter.";
        let parts = split_sentences(text);
        assert_eq!(parts.len(), 2, "war: {parts:?}");
        assert_eq!(parts[0], "Ich fluestere jetzt[ganz leise].");
        assert_eq!(parts[1], "Und rede dann wieder normal weiter.");
    }

    #[test]
    fn mindestlaenge_zaehlt_ueber_sichtbare_zeichen_nicht_rohlaenge() {
        let text = "[laughing] Hi. Das hier ist jetzt aber ein richtig langer zweiter Satz.";
        let parts = split_sentences(text);
        assert_eq!(
            parts.len(),
            1,
            "'[laughing] Hi.' hat nur 4 sichtbare Zeichen, zaehlt nicht als voller Satz: {parts:?}"
        );
    }

    #[test]
    fn tag_ueber_kappungsgrenze_wird_ganz_entfernt() {
        // "Vorher " = 7 Zeichen, dann folgt das Tag "[laughing]" (10 Zeichen).
        // max_chars=10 landet mitten im Tag -> Tag komplett weg statt halbiert.
        let p = prepare_text("Vorher [laughing] Nachher", 10).unwrap();
        assert_eq!(p.text, "Vorher ");
        assert!(p.truncated);
    }

    #[test]
    fn kappung_ohne_tag_im_weg_bleibt_wie_bisher() {
        // 7 Zeichen treffen die Grenze exakt am Tag-Anfang ('[') — das Tag
        // steht also gar nicht erst zur Debatte, keine Sonderbehandlung noetig.
        let p = prepare_text("Vorher [laughing] Nachher", 7).unwrap();
        assert_eq!(p.text, "Vorher ");
        assert!(p.truncated);
    }

    #[test]
    fn square_serialisierung_ist_unveraendert() {
        let text = "Hallo [whisper] Welt [dead tired, long day]";
        assert_eq!(serialize_tags(text, TagStyle::Square, &[]), text);
    }

    #[test]
    fn paren_mapping_ersetzt_bekannte_tags_und_verwirft_freitext() {
        let mapping = vec![("whisper".to_string(), "soft".to_string())];
        let text = "Hallo [whisper] Welt [dead tired, long day] Ende";
        let out = serialize_tags(text, TagStyle::Paren, &mapping);
        assert_eq!(out, "Hallo (soft) Welt Ende", "war: {out:?}");
    }

    #[test]
    fn paren_mapping_ist_case_insensitiv_und_getrimmt() {
        let mapping = vec![("Whisper".to_string(), "soft".to_string())];
        let text = "[ WHISPER ] Hallo";
        assert_eq!(
            serialize_tags(text, TagStyle::Paren, &mapping),
            "(soft) Hallo"
        );
    }

    #[test]
    fn tts_request_body_ruft_serialize_tags_square_auf() {
        let b = tts_request_body("Hallo [whisper] Welt", 1, None);
        assert_eq!(
            b["text"], "Hallo [whisper] Welt",
            "Square ist heute ein No-op, verankert nur die Aufrufstelle"
        );
    }

    // ---- Pausen: Stille statt Bitte ans Modell --------------------------

    #[test]
    fn text_ohne_tags_bleibt_ein_einziges_sprechstueck() {
        assert_eq!(
            split_pauses("Guten Tag, alles klar."),
            vec![SpeechPart::Speak("Guten Tag, alles klar.".to_string())]
        );
    }

    #[test]
    fn pausen_tag_zerlegt_in_sprechen_stille_sprechen() {
        assert_eq!(
            split_pauses("Guten Tag. [pause] Und weiter."),
            vec![
                SpeechPart::Speak("Guten Tag.".to_string()),
                SpeechPart::Silence(500),
                SpeechPart::Speak("Und weiter.".to_string()),
            ]
        );
    }

    #[test]
    fn alle_vier_pausenlaengen_sind_verankert() {
        for (tag, ms) in [
            ("pause", 500u32),
            ("short pause", 250),
            ("long pause", 1000),
            ("break", 700),
        ] {
            assert_eq!(
                split_pauses(&format!("A [{tag}] B")),
                vec![
                    SpeechPart::Speak("A".to_string()),
                    SpeechPart::Silence(ms),
                    SpeechPart::Speak("B".to_string()),
                ],
                "Tag [{tag}]"
            );
        }
    }

    #[test]
    fn unbekannte_tags_bleiben_im_gesprochenen_text() {
        assert_eq!(
            split_pauses("Ich [whisper] fluestere."),
            vec![SpeechPart::Speak("Ich [whisper] fluestere.".to_string())],
            "andere Tags versteht das Modell selbst"
        );
    }

    #[test]
    fn pausen_tag_ist_gross_klein_und_leerzeichen_egal() {
        assert_eq!(
            split_pauses("A [ SHORT Pause ] B"),
            vec![
                SpeechPart::Speak("A".to_string()),
                SpeechPart::Silence(250),
                SpeechPart::Speak("B".to_string()),
            ],
            "getrimmt und case-insensitiv"
        );
    }

    #[test]
    fn zwei_pausen_hintereinander_ergeben_zwei_stille_stuecke() {
        assert_eq!(
            split_pauses("A [pause] [break] B"),
            vec![
                SpeechPart::Speak("A".to_string()),
                SpeechPart::Silence(500),
                SpeechPart::Silence(700),
                SpeechPart::Speak("B".to_string()),
            ],
            "kein leeres Sprechstueck dazwischen"
        );
    }

    #[test]
    fn pause_am_anfang_oder_ende_erzeugt_kein_leeres_sprechstueck() {
        assert_eq!(
            split_pauses("[long pause] Erst jetzt."),
            vec![
                SpeechPart::Silence(1000),
                SpeechPart::Speak("Erst jetzt.".to_string()),
            ]
        );
        assert_eq!(
            split_pauses("Und Schluss. [pause]"),
            vec![
                SpeechPart::Speak("Und Schluss.".to_string()),
                SpeechPart::Silence(500),
            ]
        );
        assert!(split_pauses("  [pause]  ").len() == 1);
    }

    #[test]
    fn ungeschlossene_klammer_ist_auch_hier_kein_pausen_tag() {
        // Dieselben Klammer-Regeln wie `tag_spans`: ohne schliessende Klammer
        // in derselben Zeile ist es kein Tag.
        let text = "A [pause\nB";
        assert_eq!(
            split_pauses(text),
            vec![SpeechPart::Speak(text.to_string())]
        );
    }

    // ---- Teil 2: Sprecher-Parser ---------------------------------------

    /// Bekannte Stimmen aus nackten Ids — die id ist dann zugleich ihr
    /// einziger Name. Genau so sah der Abgleich vor der Sprecher-Registry
    /// aus; die Tests, die das Verhalten festhalten, brauchen ihn weiter.
    fn ids(ids: &[&str]) -> Vec<KnownSpeaker> {
        ids.iter()
            .map(|id| KnownSpeaker {
                id: (*id).to_string(),
                names: vec![(*id).to_string()],
            })
            .collect()
    }

    fn anna() -> KnownSpeaker {
        KnownSpeaker {
            id: "anna-id".to_string(),
            names: vec!["Anna".to_string()],
        }
    }

    fn frau_mueller() -> KnownSpeaker {
        KnownSpeaker {
            id: "mueller-id".to_string(),
            names: vec!["Frau Müller".to_string()],
        }
    }

    #[test]
    fn spitzklammer_am_zeilenanfang_schaltet_sprecher() {
        let speakers = vec![anna()];
        let segments = split_speaker_segments("<Anna>\nGuten Morgen.", &speakers);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[0].style, None);
        assert_eq!(segments[0].text, "Guten Morgen.");
    }

    #[test]
    fn spitzklammer_inline_schaltet_sprecher_mitten_im_text() {
        let speakers = vec![anna()];
        let text = "Vorspann. <Anna> Ab hier spricht Anna.";
        let segments = split_speaker_segments(text, &speakers);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].voice, None);
        assert_eq!(segments[0].text, "Vorspann.");
        assert_eq!(segments[1].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[1].text, "Ab hier spricht Anna.");
    }

    #[test]
    fn unbekannte_spitzklammer_bleibt_literal_und_schaltet_nichts() {
        let speakers = vec![anna()];
        let text = "<div>Eingefuegtes HTML</div> <Anna> Text.";
        let segments = split_speaker_segments(text, &speakers);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].voice, None);
        assert_eq!(segments[0].text, "<div>Eingefuegtes HTML</div>");
        assert_eq!(segments[1].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[1].text, "Text.");
    }

    #[test]
    fn stil_aus_spitzklammer_wird_geliefert() {
        let speakers = vec![anna()];
        let segments = split_speaker_segments("<Anna:fluesternd> Ganz leise.", &speakers);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[0].style.as_deref(), Some("fluesternd"));
    }

    #[test]
    fn leerer_stil_nach_trim_wird_zu_none() {
        let speakers = vec![anna()];
        let segments = split_speaker_segments("<Anna: > Text.", &speakers);
        assert_eq!(
            segments[0].style, None,
            "Backend ist nachsichtig, blockiert nicht"
        );
    }

    #[test]
    fn umlaute_und_leerzeichen_im_namen_matchen_case_insensitiv() {
        let speakers = vec![frau_mueller()];
        let segments = split_speaker_segments("<FRAU MÜLLER> Guten Tag.", &speakers);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice.as_deref(), Some("mueller-id"));
    }

    #[test]
    fn alt_und_neu_syntax_gemischt() {
        let speakers = vec![anna(), frau_mueller()];
        let text = "Frau Müller: Guten Tag.\n<Anna> Hallo zurück.";
        let segments = split_speaker_segments(text, &speakers);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].voice.as_deref(), Some("mueller-id"));
        assert_eq!(segments[0].style, None);
        assert_eq!(segments[0].text, "Guten Tag.");
        assert_eq!(segments[1].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[1].text, "Hallo zurück.");
    }

    #[test]
    fn sprecherzeile_und_tag_kombiniert() {
        let speakers = vec![anna()];
        let segments = split_speaker_segments("Anna: [whisper] Ganz leise.", &speakers);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].voice.as_deref(), Some("anna-id"));
        assert_eq!(segments[0].text, "[whisper] Ganz leise.");
    }

    #[test]
    fn scan_marker_candidates_liefert_byte_offsets_und_stil() {
        let text = "Hallo <Anna:fluesternd> Welt <div> Ende";
        let candidates = scan_marker_candidates(text);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            &text[candidates[0].start..candidates[0].end],
            "<Anna:fluesternd>"
        );
        assert_eq!(candidates[0].name, "Anna");
        assert_eq!(candidates[0].style.as_deref(), Some("fluesternd"));
        assert_eq!(&text[candidates[1].start..candidates[1].end], "<div>");
        assert_eq!(candidates[1].name, "div");
        assert_eq!(candidates[1].style, None);
    }

    #[test]
    /// Der Bruch, den diese Etappe geschlossen hat: die Pipeline gab dem
    /// Parser nur die nackten voice_ids. Ein Marker mit dem ANZEIGENAMEN
    /// schaltete deshalb nicht — und wurde, weil er kein Marker war, sogar
    /// mit vorgelesen. Beides prueft dieser Test in einem.
    fn anzeigename_schaltet_und_verschwindet_aus_dem_gesprochenen_text() {
        let speakers = vec![frau_mueller()];
        for text in ["Frau Müller: Guten Tag.", "<Frau Müller> Guten Tag."] {
            let segments = split_speaker_segments(text, &speakers);
            assert_eq!(segments.len(), 1, "{text}");
            assert_eq!(segments[0].voice.as_deref(), Some("mueller-id"), "{text}");
            assert_eq!(
                segments[0].text, "Guten Tag.",
                "der Marker darf nicht im gesprochenen Text landen"
            );
        }
    }
}
