//! Textübersetzung für die Audio-Übersetzung (TP3).
//!
//! Bewusst über die vorhandene Post-Process-Provider-Infrastruktur: der
//! Nutzer konfiguriert Provider/Modell/Key an einer Stelle, und „Custom"
//! zeigt per Default auf das lokale Ollama (http://localhost:11434/v1) —
//! damit bleibt auch die Übersetzung vollständig lokal möglich.

use crate::settings::AppSettings;

/// Ein Prompt, der nur die Übersetzung zurückverlangt — keine Erklärungen,
/// keine Anführungszeichen, Ton und Namen bleiben erhalten.
pub fn translation_prompt(target_lang: &str, text: &str) -> String {
    format!(
        "Translate the following text into {target_lang}. Reply with ONLY the \
         translation - no explanations, no quotation marks around it. Keep \
         names and numbers unchanged and match the tone of the original. \
         Preserve the layout of the original exactly: the same line breaks, \
         the same blank lines between paragraphs, one output paragraph for \
         one input paragraph. Never merge paragraphs into a single block of \
         text. Keep the Markdown formatting unchanged: lines starting with \
         #, ## or ### stay headings (translate the heading text, keep the \
         hash marks), and **bold**, *italic*, lists (- item) and quotes \
         (> text) keep their markup. Tokens like ⟦0⟧ stand for speaker names, \
         sound tags and page marks: copy every such token unchanged, exactly \
         once, at the matching place.\n\n{text}"
    )
}

/// Übersetzt über den aktiven Post-Process-Provider.
pub async fn translate(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
) -> Result<String, String> {
    translate_on(settings, text, target_lang, false).await
}

/// Wie [`translate`], aber mit der Wahl, die GPU freizulassen.
///
/// `cpu_only` greift nur bei einem lokalen Ollama: dessen nativer Endpunkt
/// `/api/chat` nimmt `num_gpu: 0` entgegen, der OpenAI-kompatible Pfad kennt
/// keine Geräteauswahl. Bei entfernten Anbietern stellt sich die Frage nicht,
/// bei vLLM gibt es die Möglichkeit nicht — dort läuft alles wie bisher.
///
/// Der Grund: der Fish-Speech-Server belegt rund 17 GB Grafikspeicher. Ein
/// Übersetzungsmodell daneben bringt beide zum Straucheln.
pub async fn translate_on(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
    cpu_only: bool,
) -> Result<String, String> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| {
            "Kein Post-Processing-Provider konfiguriert (Einstellungen → Post Process)".to_string()
        })?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!(
            "Für '{}' ist kein Modell eingetragen (Einstellungen → Nachbearbeitung → Modell). Ganz lokal geht es mit dem Anbieter 'Ollama (lokal)' oder 'vLLM (lokal)'.",
            provider.label
        ));
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let prompt = translation_prompt(target_lang, text);
    // Übersetzen ist Umformen, kein Nachdenken: Reasoning aus, wo steuerbar
    // (gleiche Provider-Sonderfälle wie beim Post-Processing).
    let (reasoning_effort, reasoning) = match provider.id.as_str() {
        "custom" => (Some("none".to_string()), None),
        "openrouter" => (
            None,
            Some(crate::llm_client::ReasoningConfig {
                effort: Some("none".to_string()),
                exclude: Some(true),
            }),
        ),
        _ => (None, None),
    };
    // Auf der CPU nur ueber den nativen Weg — und wenn der nicht antwortet,
    // lieber uebersetzen als scheitern: dann eben auf der GPU.
    let native = cpu_only
        .then(|| crate::llm_client::ollama_native_url(&provider.base_url))
        .flatten();
    if let Some(url) = native {
        match crate::llm_client::send_ollama_native(
            crate::managers::usage::Purpose::Translation,
            &provider,
            &url,
            &model,
            prompt.clone(),
            true,
        )
        .await
        {
            Ok(Some(content)) => {
                let trimmed = content.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
                log::warn!("Ollama (CPU) lieferte leere Uebersetzung — versuche den ueblichen Weg");
            }
            Ok(None) => log::warn!("Ollama (CPU) ohne Inhalt — versuche den ueblichen Weg"),
            Err(e) => {
                log::warn!("Ollama (CPU) nicht erreichbar ({e}) — versuche den ueblichen Weg")
            }
        }
    }

    let outcome = match crate::llm_client::send_chat_completion_checked(
        crate::managers::usage::Purpose::Translation,
        &provider,
        api_key,
        &model,
        prompt,
        None,
        None,
        reasoning_effort,
        reasoning,
    )
    .await
    {
        // Abgeschnittene Antwort (Ausgabegrenze erreicht): nie als fertige
        // Übersetzung durchgehen lassen, der Aufrufer teilt den Text.
        Ok(reply) if reply.truncated => Err(TRUNCATED.to_string()),
        Ok(reply) => match reply.content {
            Some(content) => {
                let trimmed = content.trim().to_string();
                if trimmed.is_empty() {
                    Err("Übersetzung kam leer zurück".into())
                } else {
                    Ok(trimmed)
                }
            }
            None => Err("Übersetzungsantwort ohne Inhalt".into()),
        },
        Err(e) => Err(format!("Übersetzung fehlgeschlagen: {e}")),
    };
    // Ein lokales Ollama-Modell sofort wieder entladen — der Speicher gehört
    // dem Fish-Speech-Server. Der kompatible Pfad kennt kein keep_alive,
    // deshalb der Nachschuss; bei entfernten Anbietern läuft er ins Leere
    // (ollama_native_url liefert dort None). Auch nach einem FEHLER: gerade
    // ein abgebrochener Lauf lässt das Modell sonst geladen stehen.
    crate::llm_client::ollama_unload(&provider.base_url, &model).await;
    outcome
}

// ---- Blockweise Übersetzung mit Fortschritt und geschützten Marken ----------

/// Zielgröße eines Übersetzungsblocks bei einem LOKALEN Modell (Zeichen). Ganze
/// Zeilen, nie mitten in einer Sprecherzeile getrennt. Kleine Modelle verlieren
/// bei langen Texten den Faden, ausserdem laufen sie ohne Mengenbegrenzung.
pub const BLOCK_CHARS_LOCAL: usize = 2500;
/// Bei Cloud-, API- und Abo-Modellen (Kontext ab 200 000 Token) gehen größere
/// Abschnitte in einer Anfrage: etwa vier bis fünf Buchseiten. Riesenblöcke
/// (die Antwort ist so lang wie der Text) laufen in Ausgabegrenzen und
/// Zeitüberschreitungen; kleinere Abschnitte lassen sich außerdem fortsetzen,
/// wiederholen und parallel übersetzen.
pub const BLOCK_CHARS_CLOUD: usize = 12_000;

/// Fehlercode, wenn das Modell die Antwort abgeschnitten hat.
pub const TRUNCATED: &str = "truncated";

/// Wie viele Arbeiter bei Cloud-/Abo-Modellen gleichzeitig übersetzen: jeder
/// einen zusammenhängenden Bereich des Buchs, Abschnitt für Abschnitt.
pub const TRANSLATE_WORKERS_CLOUD: usize = 4;

/// Laeuft das aktive Sprachmodell auf diesem Rechner (App-Modell, Ollama, vLLM,
/// jede Adresse auf localhost)? Alles andere ist ein Cloud-, API- oder Abo-Modell.
pub fn active_model_is_local(settings: &AppSettings) -> bool {
    let Some(provider) = settings.active_post_process_provider() else {
        return false;
    };
    if crate::managers::llm::is_local(provider) || matches!(provider.id.as_str(), "ollama" | "vllm")
    {
        return true;
    }
    let url = provider.base_url.to_ascii_lowercase();
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(&url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(
        host,
        "localhost" | "127.0.0.1" | "[::1]" | "::1" | "0.0.0.0"
    )
}

/// Blockgroesse fuer die Uebersetzung: lokal klein, sonst eine Anfrage.
pub fn translation_block_chars(settings: &AppSettings) -> usize {
    if active_model_is_local(settings) {
        BLOCK_CHARS_LOCAL
    } else {
        BLOCK_CHARS_CLOUD
    }
}

/// Vorübergehende Fehler, bei denen ein zweiter Versuch Sinn hat: Modell
/// ausgelastet, Mengenbegrenzung, kurzer Ausfall.
pub fn is_transient_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    [
        "at capacity",
        "overloaded",
        "rate limit",
        "rate_limit",
        "429",
        "503",
        "try again",
        "temporarily",
    ]
    .iter()
    .any(|m| e.contains(m))
}

/// Abbruch der laufenden Übersetzung (`tts_transform_cancel`).
pub static TRANSLATE_CANCEL: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Fehlercode, wenn der Nutzer abgebrochen hat.
pub const CANCELLED: &str = "transform_cancelled";

/// Text in Blöcke ganzer Zeilen bis etwa `max` Zeichen. Leerzeilen bleiben
/// erhalten; zusammengesetzt mit `"\n"` ergibt sich wieder der Ausgangstext.
pub fn split_blocks(text: &str, max: usize) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut size = 0usize;
    for line in text.split('\n') {
        let len = line.chars().count() + 1;
        if !current.is_empty() && size + len > max {
            blocks.push(current.join("\n"));
            current.clear();
            size = 0;
        }
        current.push(line);
        size += len;
    }
    blocks.push(current.join("\n"));
    blocks
}

/// Länge (in Bytes) einer Seitenmarke `{{S. n}}` am Anfang von `text`.
fn page_mark_len(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("{{S. ")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=7).contains(&digits) || !rest[digits..].starts_with("}}") {
        return None;
    }
    Some("{{S. ".len() + digits + 2)
}

/// Sprechermarken `<Name>`/`<Name:Stil>` und Tags `[…]` (1–60 Zeichen, ohne
/// Zeilenumbruch) — dieselbe Regel wie die Sprechererkennung beim Vorlesen.
fn marker_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < bytes.len() {
        let (start, c) = bytes[i];
        if c == '{' {
            // Seitenmarke `{{S. 12}}`.
            if let Some(len) = page_mark_len(&text[start..]) {
                spans.push((start, start + len));
                // Die Marke besteht nur aus ASCII: Byte-Länge = Zeichenzahl.
                i += len;
                continue;
            }
        }
        let close = match c {
            '<' => '>',
            '[' => ']',
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i + 1;
        let mut found = None;
        while j < bytes.len() && j - i <= 61 {
            let d = bytes[j].1;
            if d == close {
                found = Some(j);
                break;
            }
            if d == '\n' || d == c {
                break;
            }
            j += 1;
        }
        match found {
            Some(end) if end > i + 1 => {
                let end_byte = bytes[end].0 + close.len_utf8();
                spans.push((start, end_byte));
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    spans
}

/// Text mit Platzhaltern `⟦n⟧` statt der Marken und die Marken selbst.
pub fn mask_markers(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut tokens = Vec::new();
    let mut last = 0;
    for (s, e) in marker_spans(text) {
        out.push_str(&text[last..s]);
        out.push_str(&format!("⟦{}⟧", tokens.len()));
        tokens.push(text[s..e].to_string());
        last = e;
    }
    out.push_str(&text[last..]);
    (out, tokens)
}

/// Platzhalter zurück in Marken. `None`, wenn das Modell einen Platzhalter
/// verloren, verdoppelt oder erfunden hat — dann stimmt die Zuordnung nicht.
pub fn unmask_markers(text: &str, tokens: &[String]) -> Option<String> {
    let mut out = text.to_string();
    for (n, token) in tokens.iter().enumerate() {
        let placeholder = format!("⟦{n}⟧");
        if out.matches(&placeholder).count() != 1 {
            return None;
        }
        out = out.replacen(&placeholder, token, 1);
    }
    (!out.contains('⟦')).then_some(out)
}

/// Text in Marken und Zwischenstücke zerlegen (Rückfall, wenn das Modell die
/// Platzhalter nicht hält: dann wird nur zwischen den Marken übersetzt).
fn split_at_markers(text: &str) -> Vec<(bool, String)> {
    let mut parts = Vec::new();
    let mut last = 0;
    for (s, e) in marker_spans(text) {
        if s > last {
            parts.push((false, text[last..s].to_string()));
        }
        parts.push((true, text[s..e].to_string()));
        last = e;
    }
    if last < text.len() {
        parts.push((false, text[last..].to_string()));
    }
    parts
}

/// Führende und folgende Leerzeichen eines Stücks beim Übersetzen erhalten.
fn keep_padding(original: &str, translated: &str) -> String {
    let lead = &original[..original.len() - original.trim_start().len()];
    let trail = &original[original.trim_end().len()..];
    format!("{lead}{}{trail}", translated.trim())
}

/// Einen Block übersetzen: mit Platzhaltern, bei Verlust einmal neu, danach
/// Stück für Stück zwischen den Marken. `translate` ist der Modellaufruf.
pub async fn translate_block<F, Fut>(block: &str, mut translate: F) -> Result<String, String>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    if block.trim().is_empty() {
        return Ok(block.to_string());
    }
    let (masked, tokens) = mask_markers(block);
    if tokens.is_empty() {
        return Ok(keep_padding(block, &translate(masked).await?));
    }
    for _ in 0..2 {
        let reply = translate(masked.clone()).await?;
        if let Some(text) = unmask_markers(&reply, &tokens) {
            return Ok(keep_padding(block, &text));
        }
        log::warn!("Übersetzung: Platzhalter verloren, neuer Versuch");
    }
    log::warn!("Übersetzung: Platzhalter erneut verloren, übersetze zwischen den Marken");
    let mut out = String::new();
    for (is_marker, part) in split_at_markers(block) {
        if is_marker || !part.chars().any(char::is_alphabetic) {
            out.push_str(&part);
        } else {
            out.push_str(&keep_padding(
                &part,
                &translate(part.trim().to_string()).await?,
            ));
        }
    }
    Ok(out)
}

// ---- Kapitelweise, fortsetzbare, streamende Übersetzung ----------------------

/// Höchstens so oft wird ein abgeschnittener Abschnitt halbiert.
const MAX_SPLITS: u8 = 2;

/// Ab dieser Eingabelänge (Zeichen) prüft die Plausibilitätsprobe die Antwort.
const PLAUSIBLE_MIN_INPUT: usize = 400;

/// Eine Antwort unter diesem Anteil der Eingabelänge gilt als abgeschnitten.
const PLAUSIBLE_MIN_RATIO: f64 = 0.4;

/// Ein Kapitel des Ausgangstexts: Titel der Überschriftszeile (ohne `#`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    pub title: Option<String>,
}

/// Ein Abschnitt: Teil eines Kapitels, in einer Anfrage übersetzt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// 0-basierter Kapitelindex.
    pub chapter: usize,
    pub text: String,
}

/// Überschriftszeile `#`, `##` oder `###` gefolgt von einem Leerzeichen:
/// der Titel, sonst `None`.
fn heading_title(line: &str) -> Option<&str> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=3).contains(&hashes) || line.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    Some(line[hashes..].trim())
}

/// Text in Kapitel (an Markdown-Überschriften) und diese in Abschnitte bis etwa
/// `max` Zeichen zerlegen. Alle Abschnitte mit `"\n"` verbunden ergeben genau
/// den Ausgangstext. Text vor der ersten Überschrift bildet ein Kapitel ohne
/// Titel.
pub fn split_sections(text: &str, max: usize) -> (Vec<Chapter>, Vec<Section>) {
    let mut chapters: Vec<Chapter> = Vec::new();
    let mut bodies: Vec<Vec<&str>> = Vec::new();
    for line in text.split('\n') {
        if let Some(title) = heading_title(line) {
            chapters.push(Chapter {
                title: Some(title.to_string()),
            });
            bodies.push(vec![line]);
        } else if let Some(body) = bodies.last_mut() {
            body.push(line);
        } else {
            chapters.push(Chapter { title: None });
            bodies.push(vec![line]);
        }
    }
    let sections = bodies
        .iter()
        .enumerate()
        .flat_map(|(chapter, lines)| {
            split_blocks(&lines.join("\n"), max)
                .into_iter()
                .map(move |text| Section { chapter, text })
        })
        .collect();
    (chapters, sections)
}

/// Ablage fertiger Abschnitte (fortsetzbare Übersetzung).
pub trait SectionCache: Send + Sync {
    fn get(&self, section: &str) -> Option<String>;
    fn put(&self, section: &str, translated: &str);
    /// Eintrag verwerfen (die Schlussprüfung hat ihn beanstandet).
    fn remove(&self, _section: &str) {}
}

/// Kein Zwischenspeicher.
#[allow(dead_code)]
pub struct NoCache;

impl SectionCache for NoCache {
    fn get(&self, _: &str) -> Option<String> {
        None
    }
    fn put(&self, _: &str, _: &str) {}
}

/// Abschnitts-Cache auf Platte: eine Datei je Abschnitt, benannt nach dem
/// Streuwert aus Abschnittstext und Zielsprache.
pub struct DiskSectionCache {
    dir: Option<std::path::PathBuf>,
    target_lang: String,
}

impl DiskSectionCache {
    pub fn new(dir: Option<std::path::PathBuf>, target_lang: &str) -> Self {
        Self {
            dir,
            target_lang: target_lang.to_string(),
        }
    }

    fn path(&self, section: &str) -> Option<std::path::PathBuf> {
        use sha2::{Digest, Sha256};
        let dir = self.dir.as_ref()?;
        let mut h = Sha256::new();
        // Fassung: ändert sich Prompt oder Maskierung grundlegend, hochzählen.
        h.update(b"v3-abschnitte\0");
        h.update(self.target_lang.as_bytes());
        h.update(b"\0");
        h.update(section.as_bytes());
        let digest = h.finalize();
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        Some(dir.join(format!("{name}.txt")))
    }
}

impl SectionCache for DiskSectionCache {
    fn get(&self, section: &str) -> Option<String> {
        std::fs::read_to_string(self.path(section)?)
            .ok()
            .filter(|s| !s.is_empty())
    }

    fn remove(&self, section: &str) {
        if let Some(path) = self.path(section) {
            let _ = std::fs::remove_file(path);
        }
    }

    fn put(&self, section: &str, translated: &str) {
        let Some(path) = self.path(section) else {
            return;
        };
        if let Some(dir) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("Abschnitts-Cache nicht verfuegbar: {e}");
                return;
            }
        }
        // Erst in eine Nebendatei, dann umbenennen: ein Abbruch mitten im
        // Schreiben hinterlässt keinen halben Abschnitt.
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, translated).is_ok() {
            if let Err(e) = std::fs::rename(&tmp, &path) {
                log::warn!("Abschnitt nicht abgelegt: {e}");
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
}

/// Start der Übersetzung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartInfo {
    pub total: usize,
    pub chapters: usize,
    pub total_chars: usize,
}

/// Ein fertiger Abschnitt, in Index-Reihenfolge gemeldet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    pub index: usize,
    pub total: usize,
    pub chapter: usize,
    pub chapters: usize,
    pub chapter_title: Option<String>,
    pub text: String,
    pub cached: bool,
}

/// Fortschritt (Abschnitte und Zeichen des Originals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressInfo {
    pub done: usize,
    pub total: usize,
    pub chapter: usize,
    pub chapters: usize,
    pub chapter_title: Option<String>,
    pub done_chars: usize,
    pub total_chars: usize,
    /// Je Arbeiter: fertige und alle Abschnitte seines Bereichs.
    pub workers: Vec<(usize, usize)>,
}

/// Beobachter der Übersetzung — in der App sendet er Tauri-Ereignisse, im Test
/// sammelt er sie.
pub trait TranslateObserver: Send + Sync {
    fn started(&self, _info: &StartInfo) {}
    fn chunk(&self, _info: &ChunkInfo) {}
    fn progress(&self, _info: &ProgressInfo) {}
}

/// Ohne Beobachter.
#[allow(dead_code)]
pub struct NoObserver;

impl TranslateObserver for NoObserver {}

/// Antwort deutlich kürzer als die Eingabe? Sprachen mit Schriftzeichen (CJK)
/// sind von Natur aus kürzer — dort greift die Probe nicht.
fn looks_cut_off(input: &str, output: &str) -> bool {
    let in_len = input.chars().count();
    if in_len <= PLAUSIBLE_MIN_INPUT {
        return false;
    }
    let out_len = output.chars().count();
    let wide = output
        .chars()
        .filter(|&c| matches!(c as u32, 0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF))
        .count();
    if wide * 10 > out_len * 3 {
        return false;
    }
    (out_len as f64) < in_len as f64 * PLAUSIBLE_MIN_RATIO
}

/// Zeilenweise in zwei etwa gleich große Hälften teilen; mit `"\n"` verbunden
/// ergeben sie wieder das Original. `None`, wenn nur eine Zeile da ist.
fn split_in_halves(text: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() < 2 {
        return None;
    }
    let half = text.chars().count() / 2;
    let mut size = 0usize;
    let mut cut = lines.len() - 1;
    for (i, line) in lines.iter().enumerate() {
        size += line.chars().count() + 1;
        if size >= half {
            cut = (i + 1).min(lines.len() - 1);
            break;
        }
    }
    Some((lines[..cut].join("\n"), lines[cut..].join("\n")))
}

/// Einen Abschnitt übersetzen. Kommt die Antwort abgeschnitten zurück, wird er
/// (höchstens [`MAX_SPLITS`]-mal) halbiert und jede Hälfte einzeln übersetzt.
async fn translate_section<F, Fut>(section: &str, call: &F) -> Result<String, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    use std::collections::VecDeque;
    let mut pending: VecDeque<(String, u8)> = VecDeque::from([(section.to_string(), 0)]);
    let mut done: Vec<String> = Vec::new();
    while let Some((piece, depth)) = pending.pop_front() {
        let can_split = depth < MAX_SPLITS && split_in_halves(&piece).is_some();
        let result = translate_block(&piece, call).await;
        let cut_off = match &result {
            Err(e) if e == TRUNCATED => true,
            Ok(out) if looks_cut_off(&piece, out) => {
                if !can_split {
                    // Letzte Stufe: eine knappe Übersetzung ist möglich (viel
                    // Aufzählung, kurze Zielsprache) — nicht deshalb scheitern.
                    log::warn!("Übersetzung ungewöhnlich kurz, übernehme sie trotzdem");
                }
                can_split
            }
            _ => false,
        };
        if !cut_off {
            done.push(result?);
            continue;
        }
        let Some((left, right)) = split_in_halves(&piece).filter(|_| can_split) else {
            return Err(TRUNCATED.to_string());
        };
        log::warn!("Übersetzung abgeschnitten, teile den Abschnitt in zwei Hälften");
        pending.push_front((right, depth + 1));
        pending.push_front((left, depth + 1));
    }
    Ok(done.join("\n"))
}

/// Seitenmarken (`{{S. n}}`, in Reihenfolge) und übrige Sprecher-/Tag-Marken
/// (sortiert) eines Textes — Grundlage der Schlussprüfung.
fn marker_signature(text: &str) -> (Vec<String>, Vec<String>) {
    let mut pages = Vec::new();
    let mut others = Vec::new();
    for (s, e) in marker_spans(text) {
        if text[s..e].starts_with("{{S. ") {
            pages.push(text[s..e].to_string());
        } else {
            others.push(text[s..e].to_string());
        }
    }
    others.sort();
    (pages, others)
}

/// Anzahl der Markdown-Überschriftszeilen (`#`–`###`).
fn heading_lines(text: &str) -> usize {
    text.split('\n')
        .filter(|l| heading_title(l).is_some())
        .count()
}

/// Schlussprüfung eines übersetzten Abschnitts: gleiche Seitenmarken (Anzahl und
/// Werte), gleiche Sprecher-/Tag-Marken, gleich viele Überschriftszeilen, nicht
/// leer, Länge plausibel. `Some(Grund)`, wenn etwas nicht stimmt.
pub fn verify_section(original: &str, translated: &str) -> Option<String> {
    if original.trim().is_empty() {
        return None;
    }
    if translated.trim().is_empty() {
        return Some("Abschnitt ist leer".to_string());
    }
    let (pages_a, others_a) = marker_signature(original);
    let (pages_b, others_b) = marker_signature(translated);
    if pages_a != pages_b {
        return Some(format!(
            "Seitenmarken weichen ab ({} statt {})",
            pages_b.len(),
            pages_a.len()
        ));
    }
    if others_a != others_b {
        return Some("Sprecher-/Tag-Marken weichen ab".to_string());
    }
    let (ha, hb) = (heading_lines(original), heading_lines(translated));
    if ha != hb {
        return Some(format!("Überschriftszeilen weichen ab ({hb} statt {ha})"));
    }
    if looks_cut_off(original, translated) {
        return Some("Übersetzung auffällig kurz".to_string());
    }
    None
}

/// Fertiger Abschnitt: Text, aus dem Cache?, Warnung (Prüfung zweimal gescheitert).
struct SectionDone {
    text: String,
    cached: bool,
    warning: Option<String>,
}

/// Ergebnis eines Abschnitts. Gespeichert wird nur, was die Schlussprüfung
/// besteht; sonst einmal neu übersetzt, danach mit Warnung übernommen.
async fn section_outcome<F, Fut>(
    section: &Section,
    cache: &dyn SectionCache,
    cancel: &std::sync::atomic::AtomicBool,
    call: &F,
) -> Result<SectionDone, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    use std::sync::atomic::Ordering;
    let done = |text: String, cached: bool, warning: Option<String>| SectionDone {
        text,
        cached,
        warning,
    };
    if section.text.trim().is_empty() {
        return Ok(done(section.text.clone(), true, None));
    }
    if let Some(hit) = cache.get(&section.text) {
        if verify_section(&section.text, &hit).is_none() {
            return Ok(done(hit, true, None));
        }
        cache.remove(&section.text);
    }
    let mut last: Option<(String, String)> = None;
    for _ in 0..2 {
        if cancel.load(Ordering::SeqCst) {
            return Err(CANCELLED.to_string());
        }
        let translated = translate_section(&section.text, call).await?;
        match verify_section(&section.text, &translated) {
            None => {
                // Sofort ablegen — auch wenn ein anderer Arbeiter noch läuft
                // oder später ein Fehler kommt, geht dieser nicht verloren.
                cache.put(&section.text, &translated);
                return Ok(done(translated, false, None));
            }
            Some(problem) => {
                log::warn!("Übersetzung: Schlussprüfung beanstandet einen Abschnitt ({problem})");
                last = Some((translated, problem));
            }
        }
    }
    let (translated, problem) = last.expect("zwei Versuche");
    Ok(done(translated, false, Some(problem)))
}

/// Die Bereiche der Arbeiter: zusammenhängend, nach Zeichenmenge ausgewogen,
/// nur an Kapitelgrenzen — bei weniger Kapiteln als Arbeitern an
/// Abschnittsgrenzen. Höchstens so viele Bereiche wie Abschnitte.
#[allow(clippy::single_range_in_vec_init)]
pub fn partition_workers(sections: &[Section], workers: usize) -> Vec<std::ops::Range<usize>> {
    let len = sections.len();
    let n = workers.max(1).min(len.max(1));
    if n == 1 {
        return vec![0..len];
    }
    let mut prefix = vec![0usize; len + 1];
    for (i, s) in sections.iter().enumerate() {
        prefix[i + 1] = prefix[i] + s.text.chars().count();
    }
    let total = prefix[len];
    let chapter_cuts: Vec<usize> = (1..len)
        .filter(|&i| sections[i].chapter != sections[i - 1].chapter)
        .collect();
    let cands: Vec<usize> = if chapter_cuts.len() + 1 >= n {
        chapter_cuts
    } else {
        (1..len).collect()
    };
    let mut cuts = Vec::with_capacity(n - 1);
    let mut from = 0usize;
    for k in 1..n {
        let target = total * k / n;
        let last_allowed = cands.len() - (n - k);
        let best = (from..=last_allowed)
            .min_by_key(|&j| prefix[cands[j]].abs_diff(target))
            .unwrap_or(from);
        cuts.push(cands[best]);
        from = best + 1;
    }
    let mut ranges = Vec::with_capacity(n);
    let mut start = 0;
    for c in cuts {
        ranges.push(start..c);
        start = c;
    }
    ranges.push(start..len);
    ranges
}

/// Ergebnis einer Übersetzung: der Text und Warnungen der Schlussprüfung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub text: String,
    pub warnings: Vec<String>,
}

struct Shared {
    results: Vec<Option<(String, bool)>>,
    next_emit: usize,
    done: usize,
    done_chars: usize,
    worker_done: Vec<usize>,
    errors: Vec<(usize, String)>,
    warnings: Vec<(usize, String)>,
    out: Vec<String>,
}

/// Ganzer Text, kapitel- und abschnittsweise: fertige Abschnitte kommen aus dem
/// Cache; `workers` Arbeiter übersetzen je einen zusammenhängenden Bereich,
/// Abschnitt für Abschnitt. Der Fortschritt kommt sofort bei jedem fertigen
/// Abschnitt, die Abschnitts-Ereignisse und das Ergebnis folgen der Reihenfolge
/// des Textes (spätere fertige Abschnitte warten auf die Lücke davor). `call`
/// ist ein einzelner Modellaufruf (mit Wiederholung bei vorübergehenden Fehlern).
pub async fn translate_document<F, Fut>(
    text: &str,
    block_chars: usize,
    workers: usize,
    cache: &dyn SectionCache,
    observer: &dyn TranslateObserver,
    cancel: &std::sync::atomic::AtomicBool,
    call: F,
) -> Result<Outcome, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;
    let (chapters, sections) = split_sections(text, block_chars);
    let total = sections.len();
    let total_chars: usize = sections.iter().map(|s| s.text.chars().count()).sum();
    let ranges = partition_workers(&sections, workers);
    let worker_totals: Vec<usize> = ranges.iter().map(|r| r.len()).collect();
    let title_of = |chapter: usize| chapters.get(chapter).and_then(|c| c.title.clone());
    let worker_view = |done: &[usize]| -> Vec<(usize, usize)> {
        done.iter()
            .copied()
            .zip(worker_totals.iter().copied())
            .collect()
    };
    observer.started(&StartInfo {
        total,
        chapters: chapters.len(),
        total_chars,
    });
    observer.progress(&ProgressInfo {
        done: 0,
        total,
        chapter: 0,
        chapters: chapters.len(),
        chapter_title: title_of(0),
        done_chars: 0,
        total_chars,
        workers: worker_view(&vec![0; ranges.len()]),
    });

    let shared = Mutex::new(Shared {
        results: vec![None; total],
        next_emit: 0,
        done: 0,
        done_chars: 0,
        worker_done: vec![0; ranges.len()],
        errors: Vec::new(),
        warnings: Vec::new(),
        out: Vec::with_capacity(total),
    });
    let abort = AtomicBool::new(false);
    let call = &call;
    let run_worker = |w: usize, range: std::ops::Range<usize>| {
        let (shared, abort, sections, chapters_len) = (&shared, &abort, &sections, chapters.len());
        let (title_of, worker_view) = (&title_of, &worker_view);
        async move {
            for index in range {
                if abort.load(Ordering::SeqCst) {
                    return;
                }
                let section = &sections[index];
                let outcome = section_outcome(section, cache, cancel, call).await;
                let mut st = shared.lock().unwrap();
                let done = match outcome {
                    Ok(done) => done,
                    Err(e) => {
                        st.errors.push((index, e));
                        abort.store(true, Ordering::SeqCst);
                        return;
                    }
                };
                if let Some(problem) = done.warning {
                    st.warnings.push((index, problem));
                }
                st.results[index] = Some((done.text, done.cached));
                st.done += 1;
                st.done_chars += section.text.chars().count();
                st.worker_done[w] += 1;
                observer.progress(&ProgressInfo {
                    done: st.done,
                    total,
                    chapter: section.chapter,
                    chapters: chapters_len,
                    chapter_title: title_of(section.chapter),
                    done_chars: st.done_chars,
                    total_chars,
                    workers: worker_view(&st.worker_done),
                });
                while st.next_emit < total {
                    let i = st.next_emit;
                    let Some((text, cached)) = st.results[i].take() else {
                        break;
                    };
                    observer.chunk(&ChunkInfo {
                        index: i,
                        total,
                        chapter: sections[i].chapter,
                        chapters: chapters_len,
                        chapter_title: title_of(sections[i].chapter),
                        text: text.clone(),
                        cached,
                    });
                    st.out.push(text);
                    st.next_emit += 1;
                }
            }
        }
    };
    futures_util::future::join_all(
        ranges
            .iter()
            .cloned()
            .enumerate()
            .map(|(w, range)| run_worker(w, range)),
    )
    .await;

    let st = shared.into_inner().unwrap();
    // Position im Kapitel für Fehler und Warnungen.
    let place = |index: usize| {
        let chapter = sections[index].chapter;
        let nr = sections[..=index]
            .iter()
            .filter(|s| s.chapter == chapter)
            .count();
        let of = sections.iter().filter(|s| s.chapter == chapter).count();
        (chapter + 1, nr, of)
    };
    if let Some((index, e)) = st.errors.iter().min_by_key(|(i, _)| *i) {
        if st.errors.iter().any(|(_, e)| e == CANCELLED) {
            return Err(CANCELLED.to_string());
        }
        let (c, nr, of) = place(*index);
        return Err(format!(
            "Übersetzung bei Kapitel {c}, Abschnitt {nr} von {of} abgebrochen: {e}"
        ));
    }
    let mut warnings = st.warnings;
    warnings.sort_by_key(|(i, _)| *i);
    let warnings = warnings
        .into_iter()
        .map(|(i, problem)| {
            let (c, nr, of) = place(i);
            format!("Kapitel {c}, Abschnitt {nr} von {of}: {problem}")
        })
        .collect();
    Ok(Outcome {
        text: st.out.join("\n"),
        warnings,
    })
}

/// Ausgelastetes Modell oder Mengenbegrenzung: zweimal mit Pause neu.
async fn translate_with_retry(
    settings: &AppSettings,
    chunk: String,
    target_lang: &str,
    cpu_only: bool,
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    let mut attempt = 0;
    loop {
        match translate_on(settings, &chunk, target_lang, cpu_only).await {
            Err(e) if attempt < 2 && is_transient_error(&e) => {
                log::warn!("Übersetzung: vorübergehender Fehler ({e}), neuer Versuch");
                let pause = if attempt == 0 { 3 } else { 8 };
                for _ in 0..pause * 4 {
                    if TRANSLATE_CANCEL.load(Ordering::SeqCst) {
                        return Err(CANCELLED.to_string());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Wie viele Arbeiter gleichzeitig übersetzen: Cloud/Abo mehrere, lokal einer.
pub fn translation_workers(settings: &AppSettings) -> usize {
    if active_model_is_local(settings) {
        1
    } else {
        TRANSLATE_WORKERS_CLOUD
    }
}

/// Ganzer Text über den aktiven Provider: kapitelweise, mit Abschnitts-Cache,
/// Beobachter und Abbruch (`TRANSLATE_CANCEL`).
pub async fn translate_blocks(
    settings: &AppSettings,
    text: &str,
    target_lang: &str,
    cpu_only: bool,
    cache: &dyn SectionCache,
    observer: &dyn TranslateObserver,
) -> Result<Outcome, String> {
    use std::sync::atomic::Ordering;
    TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
    translate_document(
        text,
        translation_block_chars(settings),
        translation_workers(settings),
        cache,
        observer,
        &TRANSLATE_CANCEL,
        |chunk| translate_with_retry(settings, chunk, target_lang, cpu_only),
    )
    .await
}

// ---- Auftragszustand für die Oberfläche --------------------------------------

/// Fortschritt eines Arbeiters.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct WorkerProgress {
    pub done: u32,
    pub total: u32,
}

/// Der laufende (oder zuletzt beendete) Übersetzungsauftrag. Er lebt im
/// Backend: schließt die Oberfläche, läuft er weiter, und sie holt sich den
/// Stand mit `tts_translate_job` wieder.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TranslateJob {
    pub lang: String,
    /// Schlüssel des Originaltexts (wie der Dateiname im Ganztext-Cache).
    pub text_key: String,
    pub done: u32,
    pub total: u32,
    pub chapters: u32,
    pub workers: Vec<WorkerProgress>,
    /// Bisher zusammenhängend fertiger Text (vom Anfang an lückenlos).
    pub partial: String,
    pub error: Option<String>,
    pub running: bool,
    /// Beanstandungen der Schlussprüfung (Kapitel/Abschnitt), die trotz
    /// zweitem Versuch blieben.
    pub warnings: Vec<String>,
}

/// Hält den [`TranslateJob`] aktuell; als Beobachter an die Übersetzung gehängt.
pub struct JobTracker {
    inner: std::sync::Mutex<TranslateJob>,
}

impl JobTracker {
    pub fn new(lang: &str, text_key: &str) -> Self {
        Self {
            inner: std::sync::Mutex::new(TranslateJob {
                lang: lang.to_string(),
                text_key: text_key.to_string(),
                running: true,
                ..TranslateJob::default()
            }),
        }
    }

    pub fn snapshot(&self) -> TranslateJob {
        self.inner.lock().unwrap().clone()
    }

    /// Auftrag beenden: Fehler oder Warnungen festhalten.
    pub fn finish(&self, result: &Result<Outcome, String>) {
        let mut job = self.inner.lock().unwrap();
        job.running = false;
        match result {
            Ok(outcome) => job.warnings = outcome.warnings.clone(),
            Err(e) => job.error = Some(e.clone()),
        }
    }
}

impl TranslateObserver for JobTracker {
    fn started(&self, info: &StartInfo) {
        let mut job = self.inner.lock().unwrap();
        job.total = info.total as u32;
        job.chapters = info.chapters as u32;
    }

    fn chunk(&self, info: &ChunkInfo) {
        let mut job = self.inner.lock().unwrap();
        if info.index > 0 {
            job.partial.push('\n');
        }
        job.partial.push_str(&info.text);
    }

    fn progress(&self, info: &ProgressInfo) {
        let mut job = self.inner.lock().unwrap();
        job.done = info.done as u32;
        job.total = info.total as u32;
        job.workers = info
            .workers
            .iter()
            .map(|&(done, total)| WorkerProgress {
                done: done as u32,
                total: total as u32,
            })
            .collect();
    }
}

#[cfg(test)]
mod block_tests {
    use super::*;

    fn settings_with(provider: &str, base_url: &str) -> AppSettings {
        let mut s = crate::settings::get_default_settings();
        if let Some(p) = s
            .post_process_providers
            .iter_mut()
            .find(|p| p.id == provider)
        {
            p.base_url = base_url.to_string();
        }
        s.post_process_provider_id = provider.to_string();
        s
    }

    #[test]
    fn cloud_models_get_one_request_and_local_ones_small_blocks() {
        let cloud = settings_with("openai", "https://api.openai.com/v1");
        assert!(!active_model_is_local(&cloud));
        assert_eq!(translation_block_chars(&cloud), BLOCK_CHARS_CLOUD);
        // Vier Seiten Text (~12 000 Zeichen): bei Cloud EIN Block, lokal mehrere.
        let text = "Ein Satz im Text. ".repeat(700);
        assert_eq!(
            split_blocks(&text, translation_block_chars(&cloud)).len(),
            1
        );
        let local = settings_with("ollama", "http://localhost:11434/v1");
        assert!(active_model_is_local(&local));
        assert_eq!(translation_block_chars(&local), BLOCK_CHARS_LOCAL);
    }

    #[test]
    fn a_remote_model_behind_a_custom_address_is_not_local() {
        let set_url = |s: &mut AppSettings, url: &str| {
            if let Some(p) = s
                .post_process_providers
                .iter_mut()
                .find(|p| p.id == "custom")
            {
                p.base_url = url.to_string();
            }
        };
        let mut s = crate::settings::get_default_settings();
        s.post_process_provider_id = "custom".into();
        set_url(&mut s, "https://llm.example.com/v1");
        assert!(!active_model_is_local(&s));
        set_url(&mut s, "http://127.0.0.1:8000/v1");
        assert!(active_model_is_local(&s));
    }

    #[test]
    fn busy_or_rate_limited_models_are_retried_other_errors_are_not() {
        assert!(is_transient_error(
            "cli_failed: {\"message\":\"Selected model is at capacity. Please try again\"}"
        ));
        assert!(is_transient_error(
            "Übersetzung fehlgeschlagen: 429 Too Many Requests"
        ));
        assert!(!is_transient_error(
            "cli_model_not_in_plan: Dieses Modell nimmt der Server nicht an"
        ));
        assert!(!is_transient_error("Kein Modell eingetragen"));
    }

    #[test]
    fn blocks_keep_whole_lines_and_rejoin_to_the_original() {
        let text = "<A> eins zwei\n\n<B> drei vier\n<A> fünf";
        let blocks = split_blocks(text, 16);
        assert!(blocks.len() > 1, "{blocks:?}");
        assert_eq!(blocks.join("\n"), text);
        assert!(blocks.iter().all(|b| !b.starts_with(' ')));
    }

    #[test]
    fn markers_and_tags_become_placeholders_and_come_back() {
        let text = "<Erzählerin> Hallo [long pause] Welt <Erzähler:ruhig> Ende";
        let (masked, tokens) = mask_markers(text);
        assert_eq!(masked, "⟦0⟧ Hallo ⟦1⟧ Welt ⟦2⟧ Ende");
        assert_eq!(tokens, ["<Erzählerin>", "[long pause]", "<Erzähler:ruhig>"]);
        assert_eq!(
            unmask_markers("⟦0⟧ Hello ⟦1⟧ world ⟦2⟧ end", &tokens).as_deref(),
            Some("<Erzählerin> Hello [long pause] world <Erzähler:ruhig> end")
        );
    }

    #[test]
    fn a_lost_or_doubled_placeholder_is_detected() {
        let tokens = vec!["<A>".to_string(), "<B>".to_string()];
        assert_eq!(unmask_markers("⟦0⟧ Hello", &tokens), None);
        assert_eq!(unmask_markers("⟦0⟧ ⟦0⟧ ⟦1⟧", &tokens), None);
        assert_eq!(unmask_markers("⟦0⟧ ⟦1⟧ ⟦2⟧", &tokens), None, "erfunden");
    }

    #[test]
    fn no_marker_in_plain_angle_text() {
        assert!(mask_markers("<>").1.is_empty());
        assert!(mask_markers("<zeile\nweiter>").1.is_empty());
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn speakers_are_never_translated() {
        let out = block_on(translate_block(
            "<Erzählerin> Guten Tag.",
            |s: String| async move {
                // Ein Modell, das brav die Platzhalter haelt.
                Ok(s.replace("Guten Tag.", "Good day."))
            },
        ))
        .unwrap();
        assert_eq!(out, "<Erzählerin> Good day.");
    }

    #[test]
    fn when_the_model_drops_placeholders_only_the_text_between_markers_is_translated() {
        let calls = std::cell::RefCell::new(Vec::new());
        let out = block_on(translate_block(
            "<Erzählerin> Guten Tag. <Erzähler> Hallo.",
            |s: String| {
                calls.borrow_mut().push(s.clone());
                async move {
                    Ok(if s.contains('⟦') {
                        "<Female narrator> Good day. <Male narrator> Hello.".to_string()
                    } else {
                        s.replace("Guten Tag.", "Good day.")
                            .replace("Hallo.", "Hello.")
                    })
                }
            },
        ))
        .unwrap();
        assert_eq!(out, "<Erzählerin> Good day. <Erzähler> Hello.");
        assert_eq!(calls.borrow().len(), 4, "2 Versuche + 2 Stücke");
    }
}

#[cfg(test)]
mod section_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[derive(Default)]
    struct Recorder {
        started: Mutex<Vec<StartInfo>>,
        chunks: Mutex<Vec<ChunkInfo>>,
        progress: Mutex<Vec<ProgressInfo>>,
    }

    impl TranslateObserver for Recorder {
        fn started(&self, info: &StartInfo) {
            self.started.lock().unwrap().push(info.clone());
        }
        fn chunk(&self, info: &ChunkInfo) {
            self.chunks.lock().unwrap().push(info.clone());
        }
        fn progress(&self, info: &ProgressInfo) {
            self.progress.lock().unwrap().push(info.clone());
        }
    }

    #[derive(Default)]
    struct MemCache(Mutex<HashMap<String, String>>);

    impl SectionCache for MemCache {
        fn get(&self, section: &str) -> Option<String> {
            self.0.lock().unwrap().get(section).cloned()
        }
        fn put(&self, section: &str, translated: &str) {
            self.0
                .lock()
                .unwrap()
                .insert(section.to_string(), translated.to_string());
        }
    }

    /// Drei Kapitel plus Vorspann, jedes mit mehreren Zeilen.
    fn book() -> String {
        let mut t = String::from("Vorwort ohne Überschrift.\n");
        for c in 1..=3 {
            t.push_str(&format!("\n## Kapitel {c}\n\n"));
            for z in 0..6 {
                t.push_str(&format!(
                    "Zeile {z} in Kapitel {c}. {{{{S. {}}}}} Ende der Zeile.\n\n",
                    c * 10 + z
                ));
            }
        }
        t.trim_end().to_string()
    }

    fn upper(s: String) -> std::future::Ready<Result<String, String>> {
        std::future::ready(Ok(s.to_uppercase()))
    }

    #[test]
    fn sections_rejoin_to_the_original_and_follow_the_chapters() {
        let text = book();
        for max in [60, 200, 1_000, 100_000] {
            let (chapters, sections) = split_sections(&text, max);
            let joined: Vec<&str> = sections.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(joined.join("\n"), text, "max {max}");
            assert_eq!(chapters.len(), 4, "Vorspann + 3 Kapitel");
            assert_eq!(chapters[0].title, None);
            assert_eq!(chapters[2].title.as_deref(), Some("Kapitel 2"));
            // Kapitelindizes wachsen und jeder Abschnitt liegt im richtigen Kapitel.
            assert!(sections.windows(2).all(|w| w[0].chapter <= w[1].chapter));
            if max >= 1_000 {
                assert_eq!(sections.len(), 4);
            }
        }
        let (_, sections) = split_sections(&text, 200);
        let third = sections.iter().find(|s| s.chapter == 3).unwrap();
        assert!(third.text.contains("Kapitel 3"));
    }

    #[test]
    fn headings_start_chapters_only_for_one_to_three_hashes() {
        let (chapters, sections) =
            split_sections("# Eins\ntext\n#### Vier\n#Kein\n### Drei\nx", 10_000);
        assert_eq!(
            chapters
                .iter()
                .map(|c| c.title.as_deref())
                .collect::<Vec<_>>(),
            [Some("Eins"), Some("Drei")]
        );
        assert_eq!(sections.len(), 2);
        let (c, s) = split_sections("", 100);
        assert_eq!((c.len(), s.len()), (1, 1));
    }

    #[test]
    fn page_marks_and_headings_survive_masking() {
        let text = "## Kapitel 3: Der Sturm\n\nEr kam {{S. 41}} spät. {{S. 42}} Dann ging er.";
        let (masked, tokens) = mask_markers(text);
        assert_eq!(tokens, ["{{S. 41}}", "{{S. 42}}"]);
        assert_eq!(
            masked,
            "## Kapitel 3: Der Sturm\n\nEr kam ⟦0⟧ spät. ⟦1⟧ Dann ging er."
        );
        let back = unmask_markers(
            "## Chapter 3: The Storm\n\nHe came ⟦0⟧ late. ⟦1⟧ Then he left.",
            &tokens,
        );
        assert_eq!(
            back.as_deref(),
            Some("## Chapter 3: The Storm\n\nHe came {{S. 41}} late. {{S. 42}} Then he left.")
        );
        // Kein Treffer ohne Zahl oder mit fremdem Inhalt.
        assert!(mask_markers("{{S. }} {{S. x}} {{andere}} {a}").1.is_empty());
        let out = block_on(translate_block(text, |s: String| async move {
            Ok(s.replace("Er kam", "He came").replace("spät", "late"))
        }))
        .unwrap();
        assert!(
            out.contains("{{S. 41}}")
                && out.contains("{{S. 42}}")
                && out.starts_with("## Kapitel 3")
        );
    }

    #[test]
    fn the_prompt_asks_to_keep_markdown_and_page_marks() {
        let p = translation_prompt("English", "x");
        assert!(p.contains("stay headings"));
        assert!(p.contains("page marks"));
    }

    #[test]
    fn events_and_result_keep_the_order_even_when_sections_run_on_four_workers() {
        let text = book();
        let rec = Recorder::default();
        let cancel = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        // Frühe Abschnitte brauchen länger als späte.
        let out = block_on(translate_document(
            &text,
            150,
            4,
            &NoCache,
            &rec,
            &cancel,
            |s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    let wait = 20usize.saturating_sub(s.len() % 17);
                    for _ in 0..wait {
                        tokio::task::yield_now().await;
                    }
                    Ok(s.to_uppercase())
                }
            },
        ))
        .unwrap()
        .text;
        assert_eq!(out, text.to_uppercase());
        let chunks = rec.chunks.lock().unwrap();
        let start = rec.started.lock().unwrap();
        assert_eq!(start.len(), 1);
        assert_eq!(start[0].total, chunks.len());
        assert_eq!(start[0].chapters, 4);
        assert_eq!(
            start[0].total_chars,
            text.chars().count() - (chunks.len() - 1)
        );
        assert!(chunks.len() > 8);
        assert!(chunks
            .iter()
            .enumerate()
            .all(|(i, c)| c.index == i && c.total == chunks.len()));
        assert!(chunks.iter().all(|c| !c.cached));
        let joined: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(joined.join("\n"), out);
        assert_eq!(chunks[0].chapter_title, None);
        assert_eq!(chunks.last().unwrap().chapter, 3);
        assert_eq!(
            chunks.last().unwrap().chapter_title.as_deref(),
            Some("Kapitel 3")
        );
        let progress = rec.progress.lock().unwrap();
        assert_eq!(progress.len(), chunks.len() + 1);
        let last = progress.last().unwrap();
        assert_eq!((last.done, last.total), (chunks.len(), chunks.len()));
        assert_eq!(last.done_chars, last.total_chars);
        assert_eq!(last.workers.len(), 4);
        assert!(last.workers.iter().all(|&(d, t)| d == t && t > 0));
        assert_eq!(
            last.workers.iter().map(|w| w.1).sum::<usize>(),
            chunks.len()
        );
        assert!(progress
            .windows(2)
            .all(|w| w[0].done_chars <= w[1].done_chars));
    }

    #[test]
    fn a_failure_reports_chapter_and_section_and_the_next_run_resumes() {
        let text = book();
        let cache = MemCache::default();
        let cancel = AtomicBool::new(false);
        let rec = Recorder::default();
        // Das Modell scheitert an Kapitel 2 (Index 2 = zweites Kapitel).
        let first = block_on(translate_document(
            &text,
            150,
            1,
            &cache,
            &rec,
            &cancel,
            |s: String| async move {
                if s.contains("Kapitel 2") && s.contains("Zeile 2") {
                    Err("kaputt".to_string())
                } else {
                    Ok(s.to_uppercase())
                }
            },
        ));
        let err = first.unwrap_err();
        assert!(
            err.starts_with("Übersetzung bei Kapitel 3, Abschnitt "),
            "{err}"
        );
        assert!(err.ends_with("abgebrochen: kaputt"), "{err}");
        let done_first = rec.chunks.lock().unwrap().len();
        assert!(done_first > 0);

        // Zweiter Lauf: bereits fertige Abschnitte kommen aus dem Cache.
        let rec2 = Recorder::default();
        let calls = AtomicUsize::new(0);
        let out = block_on(translate_document(
            &text,
            150,
            1,
            &cache,
            &rec2,
            &cancel,
            |s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                upper(s)
            },
        ))
        .unwrap()
        .text;
        assert_eq!(out, text.to_uppercase());
        let chunks = rec2.chunks.lock().unwrap();
        assert!(chunks[..done_first].iter().all(|c| c.cached));
        assert!(chunks[done_first..].iter().all(|c| !c.cached));
        assert_eq!(calls.load(Ordering::SeqCst), chunks.len() - done_first);
    }

    #[test]
    fn a_truncated_section_is_halved_and_translated_in_parts() {
        // Ein Kapitel aus 12 Zeilen; das "Modell" schneidet alles über 300 Zeichen ab.
        let body: Vec<String> = (0..12)
            .map(|i| format!("Zeile {i}: {}", "wort ".repeat(12)))
            .collect();
        let text = format!("## Eins\n{}", body.join("\n"));
        assert!(text.chars().count() > 600);
        let calls = AtomicUsize::new(0);
        let out = block_on(translate_document(
            &text,
            100_000,
            1,
            &NoCache,
            &NoObserver,
            &AtomicBool::new(false),
            |s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if s.chars().count() > 450 {
                        Err(TRUNCATED.to_string())
                    } else {
                        Ok(s.to_uppercase())
                    }
                }
            },
        ))
        .unwrap()
        .text;
        assert_eq!(out, text.to_uppercase());
        assert_eq!(calls.load(Ordering::SeqCst), 3, "ganz + zwei Hälften");
    }

    #[test]
    fn a_section_that_stays_truncated_fails_after_two_splits() {
        let text = (0..40)
            .map(|i| format!("Zeile {i} mit etwas Text"))
            .collect::<Vec<_>>()
            .join("\n");
        let calls = AtomicUsize::new(0);
        let err = block_on(translate_document(
            &text,
            100_000,
            1,
            &NoCache,
            &NoObserver,
            &AtomicBool::new(false),
            |_s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err::<String, String>(TRUNCATED.to_string()))
            },
        ))
        .unwrap_err();
        assert!(
            err.contains("Kapitel 1") && err.ends_with("abgebrochen: truncated"),
            "{err}"
        );
        // 1 + 2 + 4 Aufrufe: der erste Fehler pro Ast der letzten Stufe stoppt.
        assert!(calls.load(Ordering::SeqCst) <= 7);
    }

    #[test]
    fn a_suspiciously_short_answer_counts_as_truncated() {
        let lines: Vec<String> = (0..10)
            .map(|i| format!("Zeile {i}: {}", "wort ".repeat(14)))
            .collect();
        let text = lines.join("\n");
        let calls = AtomicUsize::new(0);
        let out = block_on(translate_document(
            &text,
            100_000,
            1,
            &NoCache,
            &NoObserver,
            &AtomicBool::new(false),
            |s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    // Lange Eingaben: nur der Anfang kommt zurück (10 %).
                    if s.chars().count() > 450 {
                        Ok(s.chars().take(s.chars().count() / 10).collect())
                    } else {
                        Ok(s.to_uppercase())
                    }
                }
            },
        ))
        .unwrap()
        .text;
        assert_eq!(out, text.to_uppercase());
        assert!(calls.load(Ordering::SeqCst) >= 3);
        // Kurze Eingaben und CJK-Antworten lösen die Probe nicht aus.
        assert!(!looks_cut_off("kurz", ""));
        assert!(!looks_cut_off(&"wort ".repeat(200), &"字".repeat(200)));
        assert!(looks_cut_off(&"wort ".repeat(200), "wort wort"));
    }

    #[test]
    fn cancel_stops_between_sections() {
        let text = book();
        let cancel = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        let err = block_on(translate_document(
            &text,
            150,
            1,
            &NoCache,
            &NoObserver,
            &cancel,
            |s: String| {
                if calls.fetch_add(1, Ordering::SeqCst) == 1 {
                    cancel.store(true, Ordering::SeqCst);
                }
                upper(s)
            },
        ))
        .unwrap_err();
        assert_eq!(err, CANCELLED);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    fn sections_of(chapters: &[usize]) -> Vec<Section> {
        chapters
            .iter()
            .map(|&chapter| Section {
                chapter,
                text: "x".repeat(100),
            })
            .collect()
    }

    #[test]
    fn workers_get_contiguous_balanced_ranges_cut_at_chapter_borders() {
        // Acht Kapitel à zwei Abschnitte, vier Arbeiter: je zwei Kapitel.
        let secs = sections_of(&[0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7]);
        let r = partition_workers(&secs, 4);
        assert_eq!(r, vec![0..4, 4..8, 8..12, 12..16]);
        // Ungleiche Kapitel: Schnitte nur an Kapitelgrenzen.
        let secs = sections_of(&[0, 0, 0, 0, 0, 0, 1, 2, 3, 3, 3, 3]);
        let r = partition_workers(&secs, 4);
        assert_eq!(r.len(), 4);
        for w in r.windows(2) {
            assert_eq!(w[0].end, w[1].start);
            assert_ne!(secs[w[1].start].chapter, secs[w[1].start - 1].chapter);
        }
        assert_eq!((r[0].start, r.last().unwrap().end), (0, 12));
    }

    #[test]
    fn with_fewer_chapters_than_workers_it_cuts_at_section_borders() {
        let secs = sections_of(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 1]);
        let r = partition_workers(&secs, 4);
        assert_eq!(r.len(), 4);
        assert!(r.iter().all(|x| !x.is_empty()));
        assert_eq!(r.iter().map(|x| x.len()).sum::<usize>(), 10);
        // Weniger Abschnitte als Arbeiter, lokal ein Arbeiter.
        assert_eq!(partition_workers(&sections_of(&[0, 0]), 4).len(), 2);
        assert_eq!(partition_workers(&secs, 1), vec![0..10]);
        assert_eq!(partition_workers(&[], 4), vec![0..0]);
    }

    #[test]
    fn verification_compares_marks_speakers_headings_and_length() {
        let orig = "## Kapitel 1\n\n<Mara> Hallo [lacht] {{S. 4}} Welt. {{S. 5}} Ende.";
        let ok = "## Chapter 1\n\n<Mara> Hello [lacht] {{S. 4}} world. {{S. 5}} End.";
        assert_eq!(verify_section(orig, ok), None);
        let p = verify_section(orig, &ok.replace("{{S. 5}} ", "")).unwrap();
        assert!(p.contains("Seitenmarken"), "{p}");
        let p = verify_section(orig, &ok.replace("{{S. 5}}", "{{S. 6}}")).unwrap();
        assert!(p.contains("Seitenmarken"), "{p}");
        let p = verify_section(orig, &ok.replace("<Mara>", "<Mary>")).unwrap();
        assert!(p.contains("Sprecher"), "{p}");
        let p = verify_section(orig, &ok.replace("## ", "")).unwrap();
        assert!(p.contains("Überschrift"), "{p}");
        assert!(verify_section(orig, "  ").unwrap().contains("leer"));
        assert_eq!(verify_section("", ""), None);
        let long = "wort ".repeat(200);
        assert!(verify_section(&long, "wort").unwrap().contains("kurz"));
    }

    #[test]
    fn a_failed_check_translates_the_section_again_and_then_only_warns() {
        let text = "## Eins\n\nHallo {{S. 1}} Welt.\n## Zwei\n\nNoch ein Absatz.";
        // Das Modell verliert beim ersten Versuch die Überschriftszeichen.
        let first_try = AtomicUsize::new(0);
        let cache = MemCache::default();
        let out = block_on(translate_document(
            text,
            30,
            1,
            &cache,
            &NoObserver,
            &AtomicBool::new(false),
            |s: String| {
                let n = first_try.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(if n == 0 {
                    s.to_uppercase().replace("## ", "")
                } else {
                    s.to_uppercase()
                }))
            },
        ))
        .unwrap();
        assert_eq!(out.text, text.to_uppercase());
        assert!(out.warnings.is_empty());
        assert_eq!(cache.0.lock().unwrap().len(), 2);

        // Dauerhaft beanstandet: Ergebnis kommt trotzdem, mit Warnung, nicht im Cache.
        let cache = MemCache::default();
        let calls = AtomicUsize::new(0);
        let out = block_on(translate_document(
            text,
            30,
            1,
            &cache,
            &NoObserver,
            &AtomicBool::new(false),
            |s: String| {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(s.replace("## ", "").replace("Hallo", "Hi")))
            },
        ))
        .unwrap();
        assert!(!out.text.is_empty());
        assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
        assert!(out.warnings[0].starts_with("Kapitel 1, Abschnitt 1 von 1: "));
        assert!(out.warnings[1].starts_with("Kapitel 2, Abschnitt 1 von 1: "));
        assert!(cache.0.lock().unwrap().is_empty());
        assert_eq!(calls.load(Ordering::SeqCst), 4, "je Abschnitt zweimal");
    }

    #[test]
    fn a_bad_cache_entry_is_discarded_and_retranslated() {
        let text = "## Eins\n\nHallo {{S. 1}} Welt.";
        let cache = MemCache::default();
        cache.put(text, "kaputt ohne alles");
        let out = block_on(translate_document(
            text,
            1000,
            1,
            &cache,
            &NoObserver,
            &AtomicBool::new(false),
            upper,
        ))
        .unwrap();
        assert_eq!(out.text, text.to_uppercase());
        assert_eq!(
            cache.get(text).as_deref(),
            Some(text.to_uppercase().as_str())
        );
    }

    #[test]
    fn progress_is_immediate_while_chunks_wait_for_the_gap() {
        // Zwei Arbeiter; der erste hängt lange an Abschnitt 0, der zweite ist schnell fertig.
        let text = "## A\n\nerster\n## B\n\nzweiter\n## C\n\ndritter\n## D\n\nvierter";
        let rec = Recorder::default();
        let out = block_on(translate_document(
            text,
            1000,
            2,
            &NoCache,
            &rec,
            &AtomicBool::new(false),
            |s: String| async move {
                if s.contains("erster") {
                    for _ in 0..50 {
                        tokio::task::yield_now().await;
                    }
                }
                Ok(s.to_uppercase())
            },
        ))
        .unwrap();
        assert_eq!(out.text, text.to_uppercase());
        let chunks = rec.chunks.lock().unwrap();
        assert!(chunks.iter().enumerate().all(|(i, c)| c.index == i));
        let progress = rec.progress.lock().unwrap();
        // Fortschritt zählt auch Abschnitte hinter der Lücke: irgendwann steht
        // done auf 2 (Arbeiter 2 fertig), bevor der erste Abschnitt gemeldet ist.
        assert!(progress
            .iter()
            .any(|p| p.done == 2 && p.workers[1].0 == 2 && p.workers[0].0 == 0));
        assert_eq!(chunks.len(), 4);
    }

    #[test]
    fn the_job_tracker_follows_the_run() {
        let tracker = JobTracker::new("German", "abc");
        assert!(tracker.snapshot().running);
        let text = "## A\n\neins\n## B\n\nzwei";
        let res = block_on(translate_document(
            text,
            1000,
            2,
            &NoCache,
            &tracker,
            &AtomicBool::new(false),
            upper,
        ));
        tracker.finish(&res);
        let job = tracker.snapshot();
        assert!(!job.running && job.error.is_none());
        assert_eq!((job.done, job.total, job.chapters), (2, 2, 2));
        assert_eq!(job.partial, text.to_uppercase());
        assert_eq!(job.workers.len(), 2);
        assert_eq!(
            (job.lang.as_str(), job.text_key.as_str()),
            ("German", "abc")
        );

        let failing = JobTracker::new("German", "abc");
        let res = block_on(translate_document(
            text,
            1000,
            1,
            &NoCache,
            &failing,
            &AtomicBool::new(false),
            |_s: String| std::future::ready(Err::<String, String>("kaputt".into())),
        ));
        failing.finish(&res);
        let job = failing.snapshot();
        assert!(!job.running);
        assert!(job.error.unwrap().contains("Kapitel 1"));
    }

    #[test]
    fn the_disk_cache_is_keyed_by_text_and_language() {
        let dir = tempfile::tempdir().unwrap();
        let de = DiskSectionCache::new(Some(dir.path().join("chunks")), "German");
        let en = DiskSectionCache::new(Some(dir.path().join("chunks")), "English");
        assert_eq!(de.get("Hallo"), None);
        de.put("Hallo", "Hello");
        assert_eq!(de.get("Hallo").as_deref(), Some("Hello"));
        assert_eq!(en.get("Hallo"), None);
        assert_eq!(de.get("Hallo."), None);
        assert_eq!(DiskSectionCache::new(None, "German").get("Hallo"), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Der native Endpunkt ist der einzige, der die GPU abwaehlen laesst.
    #[test]
    fn der_native_ollama_endpunkt_wird_richtig_abgeleitet() {
        use crate::llm_client::ollama_native_url;
        assert_eq!(
            ollama_native_url("http://localhost:11434/v1").as_deref(),
            Some("http://localhost:11434/api/chat")
        );
        assert_eq!(
            ollama_native_url("http://127.0.0.1:11434/v1/").as_deref(),
            Some("http://127.0.0.1:11434/api/chat")
        );
        // Entfernte Anbieter: die Frage stellt sich nicht.
        assert_eq!(ollama_native_url("https://api.openai.com/v1"), None);
        assert_eq!(ollama_native_url("https://api.groq.com/openai/v1"), None);
    }

    #[test]
    fn prompt_names_the_target_language_and_forbids_chatter() {
        let p = translation_prompt("German", "Hello world");
        assert!(p.contains("into German"));
        assert!(p.contains("ONLY the"));
        assert!(p.ends_with("Hello world"));
    }

    /// Absätze sind der Grund, warum ein vorgelesener Text lesbar bleibt.
    /// Ohne diese Ansage lieferten Modelle eine einzige Textwand zurück.
    #[test]
    fn prompt_verlangt_die_absaetze_des_originals() {
        let p = translation_prompt("English", "Erster Absatz.\n\nZweiter Absatz.");
        assert!(p.contains("blank lines between paragraphs"));
        assert!(p.contains("Never merge paragraphs"));
        // Der Text selbst geht mit seinen Umbrüchen hinein, nicht geglättet.
        assert!(p.ends_with("Erster Absatz.\n\nZweiter Absatz."));
    }

    /// Mock eines OpenAI-kompatiblen /chat/completions-Endpunkts.
    async fn spawn_llm_mock(reply: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 65536];
                    let mut read = 0usize;
                    loop {
                        let n = sock.read(&mut buf[read..]).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        read += n;
                        let text = String::from_utf8_lossy(&buf[..read]).to_lowercase();
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let content_length = text
                                .lines()
                                .find_map(|l| l.strip_prefix("content-length: "))
                                .and_then(|v| v.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            if read >= header_end + 4 + content_length {
                                let body = format!(
                                    r#"{{"choices":[{{"message":{{"role":"assistant","content":"{reply}"}}}}]}}"#
                                );
                                let head = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                                    body.len()
                                );
                                let _ = sock.write_all(head.as_bytes()).await;
                                let _ = sock.write_all(body.as_bytes()).await;
                                let _ = sock.shutdown().await;
                                break;
                            }
                        }
                    }
                });
            }
        });
        port
    }

    fn settings_with_mock_provider(port: u16) -> crate::settings::AppSettings {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        if let Some(custom) = settings.post_process_provider_mut("custom") {
            custom.base_url = format!("http://127.0.0.1:{port}/v1");
        }
        settings
            .post_process_models
            .insert("custom".into(), "test-model".into());
        settings
    }

    #[tokio::test]
    async fn translate_returns_the_models_reply() {
        let port = spawn_llm_mock("Hallo Welt").await;
        let settings = settings_with_mock_provider(port);
        let out = translate(&settings, "Hello world", "German").await.unwrap();
        assert_eq!(out, "Hallo Welt");
    }

    #[tokio::test]
    async fn translate_without_configured_model_fails_with_guidance() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        settings
            .post_process_models
            .insert("custom".into(), "".into());
        let err = translate(&settings, "Hello", "German").await.unwrap_err();
        assert!(err.contains("kein Modell eingetragen"), "war: {err}");
        assert!(
            err.contains("Einstellungen → Nachbearbeitung"),
            "Fehlermeldung nennt den Ort zum Nachtragen: {err}"
        );
        assert!(
            err.contains("Ollama"),
            "Fehlermeldung nennt den lokalen Weg: {err}"
        );
        // Der Anbieter steht mit seiner Beschriftung da, nicht mit der internen
        // Id — 'Custom' findet der Nutzer in der Liste, 'custom' nicht.
        assert!(err.contains("'Custom'"), "war: {err}");
    }
}
