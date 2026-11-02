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

/// Wie viele Abschnitte bei Cloud-/Abo-Modellen gleichzeitig laufen.
pub const PARALLEL_CLOUD: usize = 3;

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
}

/// Kein Zwischenspeicher.
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
}

/// Beobachter der Übersetzung — in der App sendet er Tauri-Ereignisse, im Test
/// sammelt er sie.
pub trait TranslateObserver: Send + Sync {
    fn started(&self, _info: &StartInfo) {}
    fn chunk(&self, _info: &ChunkInfo) {}
    fn progress(&self, _info: &ProgressInfo) {}
}

/// Ohne Beobachter.
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
        let result = translate_block(&piece, |chunk| call(chunk)).await;
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

/// Ergebnis eines Abschnitts: Text und ob er aus dem Cache kam.
async fn section_outcome<F, Fut>(
    section: &Section,
    cache: &dyn SectionCache,
    cancel: &std::sync::atomic::AtomicBool,
    call: &F,
) -> Result<(String, bool), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    use std::sync::atomic::Ordering;
    if section.text.trim().is_empty() {
        return Ok((section.text.clone(), true));
    }
    if let Some(hit) = cache.get(&section.text) {
        return Ok((hit, true));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(CANCELLED.to_string());
    }
    let translated = translate_section(&section.text, call).await?;
    // Sofort ablegen — auch wenn ein früherer Abschnitt noch läuft oder
    // später ein Fehler kommt, geht dieser nicht mehr verloren.
    cache.put(&section.text, &translated);
    Ok((translated, false))
}

/// Ganzer Text, kapitel- und abschnittsweise: fertige Abschnitte kommen aus dem
/// Cache, bis zu `parallel` laufen gleichzeitig, Ereignisse und Ergebnis folgen
/// der Reihenfolge des Textes. `call` ist ein einzelner Modellaufruf (mit
/// Wiederholung bei vorübergehenden Fehlern).
pub async fn translate_document<F, Fut>(
    text: &str,
    block_chars: usize,
    parallel: usize,
    cache: &dyn SectionCache,
    observer: &dyn TranslateObserver,
    cancel: &std::sync::atomic::AtomicBool,
    call: F,
) -> Result<String, String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    use futures_util::StreamExt;
    let (chapters, sections) = split_sections(text, block_chars);
    let total = sections.len();
    let total_chars: usize = sections.iter().map(|s| s.text.chars().count()).sum();
    observer.started(&StartInfo {
        total,
        chapters: chapters.len(),
        total_chars,
    });
    let title_of = |chapter: usize| chapters.get(chapter).and_then(|c| c.title.clone());
    observer.progress(&ProgressInfo {
        done: 0,
        total,
        chapter: 0,
        chapters: chapters.len(),
        chapter_title: title_of(0),
        done_chars: 0,
        total_chars,
    });

    let call = &call;
    let mut stream = futures_util::stream::iter(
        sections
            .iter()
            .map(|section| async move { section_outcome(section, cache, cancel, call).await }),
    )
    .buffered(parallel.max(1));

    let mut out: Vec<String> = Vec::with_capacity(total);
    let mut done_chars = 0usize;
    let mut first_in_chapter: Vec<usize> = vec![0; chapters.len()];
    let mut seen_chapter: Vec<usize> = vec![0; chapters.len()];
    for s in &sections {
        first_in_chapter[s.chapter] += 1;
    }
    while let Some(outcome) = stream.next().await {
        let index = out.len();
        let section = &sections[index];
        let (translated, cached) = match outcome {
            Ok(done) => done,
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => {
                let nr = seen_chapter[section.chapter] + 1;
                let of = first_in_chapter[section.chapter];
                return Err(format!(
                    "Übersetzung bei Kapitel {}, Abschnitt {nr} von {of} abgebrochen: {e}",
                    section.chapter + 1
                ));
            }
        };
        seen_chapter[section.chapter] += 1;
        done_chars += section.text.chars().count();
        observer.chunk(&ChunkInfo {
            index,
            total,
            chapter: section.chapter,
            chapters: chapters.len(),
            chapter_title: title_of(section.chapter),
            text: translated.clone(),
            cached,
        });
        out.push(translated);
        observer.progress(&ProgressInfo {
            done: index + 1,
            total,
            chapter: section.chapter,
            chapters: chapters.len(),
            chapter_title: title_of(section.chapter),
            done_chars,
            total_chars,
        });
    }
    Ok(out.join("\n"))
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

/// Wie viele Abschnitte gleichzeitig laufen: Cloud/Abo mehrere, lokal einer.
pub fn translation_parallelism(settings: &AppSettings) -> usize {
    if active_model_is_local(settings) {
        1
    } else {
        PARALLEL_CLOUD
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
) -> Result<String, String> {
    use std::sync::atomic::Ordering;
    TRANSLATE_CANCEL.store(false, Ordering::SeqCst);
    translate_document(
        text,
        translation_block_chars(settings),
        translation_parallelism(settings),
        cache,
        observer,
        &TRANSLATE_CANCEL,
        |chunk| translate_with_retry(settings, chunk, target_lang, cpu_only),
    )
    .await
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
    fn events_and_result_keep_the_order_even_when_sections_run_in_parallel() {
        let text = book();
        let rec = Recorder::default();
        let cancel = AtomicBool::new(false);
        let calls = AtomicUsize::new(0);
        // Frühe Abschnitte brauchen länger als späte.
        let out = block_on(translate_document(
            &text,
            150,
            3,
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
        .unwrap();
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
        .unwrap();
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
        .unwrap();
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
        .unwrap();
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
