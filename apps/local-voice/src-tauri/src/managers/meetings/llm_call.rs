//! Generische LLM-Mechanik fuer Besprechungsdokumente (Protokoll und
//! KI-Notizen, M1/P1b).
//!
//! Aus `minutes.rs` herausgeloest, damit beide Dokumentarten dieselbe
//! Mechanik benutzen: Anbieter aufloesen, JSON mit Struktur-Retry abfragen,
//! einen Block der map-Stufe mit eigenem Retry-Budget abfragen, die
//! deterministischen Kopf-Fakten bilden. Das Verhalten ist gegenueber dem
//! Stand in `minutes.rs` unveraendert (Texte, Versuchszahlen, Reihenfolge).
//!
//! Kein Modul hier darf `settings::get_settings(&AppHandle)` rufen: die
//! Einstellungen kommen als Parameter (sonst startet die Test-Exe nicht,
//! STATUS_ENTRYPOINT_NOT_FOUND).
//!
//! Datenschutz (D9): Logzeilen nennen nur Laengen, Zaehler und Codes. Wo ein
//! Parser-Fehler den Wert im Wortlaut nennen koennte (serde_json bei
//! Typfehlern), gibt es die Option `redact_parse_errors`.

use std::future::Future;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::stats::{speaking_shares, SpeakerShare};
use super::store::{Meeting, StoredSegment};
use crate::managers::usage::Purpose;
use crate::settings::{AppSettings, PostProcessProvider};

/// Versuche je Block der map-Stufe (eigenes Budget, unabhaengig vom
/// Struktur-Retry innerhalb eines Versuchs).
pub const CHUNK_ATTEMPTS: usize = 2;
/// Versuche je Abfrage: ein Retry mit dem Fehlertext repariert Struktur-
/// Fehler meist; mehr waere nur Wartezeit.
const JSON_ATTEMPTS: usize = 2;

// -- Formatierung ---------------------------------------------------------

pub fn mm_ss(ms: u64) -> String {
    let total_seconds = ms / 1_000;
    format!("{:02}:{:02}", total_seconds / 60, total_seconds % 60)
}

pub fn duration_label(ms: u64) -> String {
    let total_seconds = ms / 1_000;
    let hours = total_seconds / 3_600;
    if hours > 0 {
        format!(
            "{}:{:02}:{:02}",
            hours,
            (total_seconds % 3_600) / 60,
            total_seconds % 60
        )
    } else {
        mm_ss(ms)
    }
}

// -- Kopf-Fakten ------------------------------------------------------------

/// Die deterministisch berechneten Kopfdaten einer Besprechung (Titel, Datum,
/// Dauer, Redeanteile). Zahlen daraus gehen dem Modell als Fakten mit und
/// werden nie von ihm neu berechnet.
#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct MeetingHead {
    pub title: String,
    pub date_iso: String,
    pub duration_ms: u64,
    pub shares: Vec<SpeakerShare>,
    /// Nur ein Kanal im Transkript: Redeanteile sind dann keine Information,
    /// sondern Rauschen — Tabelle und Validator lassen sie weg.
    pub single_speaker: bool,
    /// Der einzige Kanal ist eine Mischaufnahme (MixedCapture, Kanal 2): ein
    /// Import kann vier Personen enthalten, die alle auf denselben Kanal
    /// laufen. „Ein Kanal" heißt hier ausdrücklich NICHT „ein Sprecher" —
    /// die Unterscheidung steuert die Prompt-Formulierung.
    pub mixed_channel: bool,
}

/// Segmente nach Startzeit sortieren. `get_segments` liefert sie in
/// `segment_index`-Reihenfolge, die bei Live-Meetings Mikrofon- und
/// System-Blöcke verschränkt; für das Prompt-Rendering ist die gemeinsame
/// Zeitachse (beide Kanäle starten beim Meeting-Start) die bessere Ordnung.
pub fn sorted_segments(segments: &[StoredSegment]) -> Vec<StoredSegment> {
    let mut sorted = segments.to_vec();
    sorted.sort_by_key(|segment| segment.start_ms);
    sorted
}

pub fn head_facts_block(head: &MeetingHead) -> String {
    let mut block = format!(
        "# Meeting facts (computed, treat as given — restate them, never recompute)\n\
         Title: {}\nDate: {}\nDuration: {}\n",
        head.title,
        head.date_iso,
        duration_label(head.duration_ms),
    );
    if head.mixed_channel {
        // Eine Mischaufnahme kann beliebig viele Personen enthalten. Dem
        // Modell hier „ein Sprecher" als Fakt zu geben, würde ein Meeting mit
        // vier Personen zum Monolog machen — genau die Halluzination, die der
        // System-Prompt verbietet.
        block.push_str(
            "Speakers: the transcript is a single mixed recording channel; the \
             number of speakers is unknown and speaking shares are not \
             available. Attribute statements only where the transcript itself \
             makes the speaker clear.\n",
        );
    } else if head.single_speaker || head.shares.is_empty() {
        block.push_str("Speakers: a single recorded speaker (no speaking shares).\n");
    } else {
        block.push_str("Speaking shares:\n");
        for share in &head.shares {
            block.push_str(&format!(
                "- {} (channel {}): {:.1} % of the speech time\n",
                share.label, share.channel, share.percent
            ));
        }
    }
    block
}

/// Kopfdaten aus den Store-Fakten. `date_iso` bevorzugt den Start der
/// Aufnahme und fällt auf das Anlagedatum zurück (Importe haben kein
/// `started_at`).
pub fn build_head(meeting: &Meeting, segments: &[StoredSegment]) -> MeetingHead {
    let shares = speaking_shares(segments);
    let duration_ms = meeting.duration_ms.unwrap_or_else(|| {
        segments
            .iter()
            .map(|segment| segment.end_ms)
            .max()
            .unwrap_or(0)
    });
    let timestamp = meeting.started_at.unwrap_or(meeting.created_at);
    let date_iso = chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    // Importe landen vollständig auf Kanal 2 (MixedCapture) — dort steht ein
    // Kanal für unbekannt viele Sprecher, nicht für einen.
    let mixed_channel = segments.iter().any(|segment| segment.channel == 2);

    MeetingHead {
        title: meeting.title.clone(),
        date_iso,
        duration_ms,
        single_speaker: shares.len() <= 1,
        mixed_channel,
        shares,
    }
}

// -- Antwort-Aufbereitung ---------------------------------------------------

/// Modelle liefern das JSON gelegentlich in einem Codefence; das kostet einen
/// Retry, den ein Dreizeiler spart.
pub fn strip_code_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = match rest.find('\n') {
        Some(newline) => &rest[newline + 1..],
        None => rest,
    };
    body.trim_end().trim_end_matches("```").trim()
}

/// Ein Parser-Fehler ohne Wortlaut: nur Klasse, Zeile und Spalte. serde_json
/// nennt bei Typfehlern den gefundenen Wert (`invalid type: string "…"`), und
/// der stammt aus Notizen oder Transkript.
pub fn describe_json_error(error: &serde_json::Error) -> String {
    format!(
        "{:?}-Fehler in Zeile {}, Spalte {}",
        error.classify(),
        error.line(),
        error.column()
    )
}

// -- Anbieter ---------------------------------------------------------------

pub const CODE_NO_PROVIDER: &str = "no_provider";
pub const CODE_NO_MODEL: &str = "no_model";

/// Anbieter, Modell und Schluessel — das, was ein Aufruf braucht.
pub type ResolvedProvider = (PostProcessProvider, String, String);

/// Warum kein Anbieter aufgeloest werden konnte: `code` fuer Maschinen
/// (`no_provider` | `no_model`), `message` fuer Menschen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: String,
}

pub fn resolve_provider_coded(settings: &AppSettings) -> Result<ResolvedProvider, ProviderError> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or_else(|| ProviderError {
            code: CODE_NO_PROVIDER,
            message: "Kein LLM-Provider konfiguriert (Einstellungen → Nachbearbeitung)".to_string(),
        })?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        // Der Hinweis nannte frueher den Umweg ueber 'Custom'; seit es eigene
        // Eintraege fuer Ollama und vLLM gibt, ist der falsch.
        return Err(ProviderError {
            code: CODE_NO_MODEL,
            message: format!(
                "Für '{}' ist kein Modell eingetragen (Einstellungen → Nachbearbeitung → Modell). Ganz lokal geht es mit dem Anbieter 'Ollama (lokal)' oder 'vLLM (lokal)'.",
                provider.label
            ),
        });
    }
    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    Ok((provider, model, api_key))
}

pub fn resolve_provider(settings: &AppSettings) -> Result<ResolvedProvider, String> {
    resolve_provider_coded(settings).map_err(|e| e.message)
}

// -- Retry-Entscheidung -----------------------------------------------------

/// Kennzeichen der RAM-Start-Sperre (`process_guard::check_ram_for_start`).
/// Ein Test haelt es an der echten Meldung fest.
pub const MEMORY_ERROR_MARKER: &str = "Zu wenig freier Arbeitsspeicher";

pub fn is_memory_error(err: &str) -> bool {
    err.contains(MEMORY_ERROR_MARKER)
}

/// Lohnt ein zweiter Versuch? Nie bei einem RAM-Fehler (das Modell passt
/// gerade nicht, ein Retry startet es erneut und verschlimmert die Lage) und
/// nie, wenn der freie RAM unter der Reserve liegt (der Speicherwaechter hat
/// den Server womoeglich mitten im Lauf beendet). `free_mb == 0` heisst
/// "nicht messbar" und blockiert nie, wie `check_ram_for_start`.
pub fn should_retry(err: &str, free_mb: u64) -> bool {
    if is_memory_error(err) {
        return false;
    }
    if free_mb != 0 && free_mb < crate::process_guard::RAM_RESERVE_MB {
        return false;
    }
    true
}

// -- Abfragen ---------------------------------------------------------------

pub struct AskOptions<'a> {
    pub purpose: Purpose,
    /// Praefix der Fehlermeldungen und Logzeilen ("Protokoll", "KI-Notizen").
    pub noun: &'a str,
    /// Parser-Fehler nur klassifiziert ausgeben statt im Wortlaut (siehe
    /// `describe_json_error`). Der Retry-Prompt an das Modell behaelt den
    /// vollen Text: er geht nicht ins Log.
    pub redact_parse_errors: bool,
}

/// Gueltiges JSON, aber fachlich unbrauchbar: einmal mit klarem Hinweis
/// nachfragen. Wird nur im ersten Versuch geprueft.
pub struct SemanticRetry {
    /// Kurzer Grund fuer den Log; kein Nutzertext.
    pub reason: String,
    /// Wird an den Prompt angehaengt.
    pub hint: String,
}

/// Eine strukturierte Abfrage mit einem Struktur-Retry: ungueltiges JSON (und
/// ein `semantic_check`-Treffer im ersten Versuch) wird einmal mit
/// Hinweistext wiederholt. Transportfehler kommen sofort zurueck — dafuer ist
/// `retry_chunk` zustaendig. Die Closures sind `Sync`, damit das Future
/// `Send` bleibt (Tauri-Commands verlangen es).
pub async fn ask_json<T: DeserializeOwned>(
    settings: &AppSettings,
    opts: &AskOptions<'_>,
    system_prompt: &str,
    schema_for: &(dyn Fn(bool) -> serde_json::Value + Sync),
    user_prompt: &str,
    semantic_check: &(dyn Fn(&T) -> Option<SemanticRetry> + Sync),
) -> Result<T, String> {
    let (provider, model, api_key) = resolve_provider(settings)?;
    let schema = schema_for(crate::managers::llm::is_local(&provider));

    let mut prompt = user_prompt.to_string();
    let mut last_error = String::new();
    for attempt in 0..JSON_ATTEMPTS {
        let response = crate::llm_client::send_chat_completion_with_schema(
            opts.purpose,
            &provider,
            api_key.clone(),
            &model,
            prompt.clone(),
            Some(system_prompt.to_string()),
            Some(schema.clone()),
            None,
            None,
        )
        .await
        .map_err(|e| format!("{}-Erzeugung fehlgeschlagen: {e}", opts.noun))?
        .ok_or_else(|| format!("{}-Antwort ohne Inhalt", opts.noun))?;

        match serde_json::from_str::<T>(strip_code_fence(&response)) {
            Ok(value) => {
                let retry = if attempt == 0 {
                    semantic_check(&value)
                } else {
                    None
                };
                match retry {
                    // Gueltiges JSON, aber fachlich leer (Protokoll: leere
                    // Zusammenfassung — das Modell hat die Regel "leere
                    // Listen nicht auffuellen" auf Pflichtfelder uebertragen).
                    Some(retry) => {
                        last_error = retry.reason;
                        log::warn!(
                            "{}-Antwort fachlich unbrauchbar (Versuch 1): {} — wiederhole mit Hinweis",
                            opts.noun,
                            last_error
                        );
                        prompt = format!("{user_prompt}\n\n{}", retry.hint);
                    }
                    None => return Ok(value),
                }
            }
            Err(e) => {
                let full = e.to_string();
                last_error = if opts.redact_parse_errors {
                    describe_json_error(&e)
                } else {
                    full.clone()
                };
                log::warn!(
                    "{}-Antwort war kein gültiges JSON (Versuch {}): {}",
                    opts.noun,
                    attempt + 1,
                    last_error
                );
                prompt = format!(
                    "{user_prompt}\n\nYour previous reply could not be parsed as \
                     the required JSON object (error: {full}). Reply with \
                     ONLY a JSON object matching the schema, nothing else."
                );
            }
        }
    }
    Err(format!(
        "{}-Antwort war kein gültiges JSON: {last_error}",
        opts.noun
    ))
}

/// Ein Block der map-Stufe mit eigenem Retry-Budget. `ask_json` wiederholt
/// nur Struktur-Fehler; ein Transportfehler (Ollama kurz weg, Timeout) kommt
/// sofort zurueck und wuerde ohne diesen zweiten Anlauf den ganzen Lauf
/// kosten. `should_retry` entscheidet, ob nach einem Fehler ueberhaupt ein
/// weiterer Versuch lohnt (RAM-Fehler: nein); `|_| true` wiederholt immer.
pub async fn retry_chunk<T, F, Fut>(
    noun: &str,
    index: usize,
    total: usize,
    should_retry: impl Fn(&str) -> bool,
    mut attempt_once: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let mut last_error = String::new();
    for attempt in 1..=CHUNK_ATTEMPTS {
        match attempt_once().await {
            Ok(value) => return Ok(value),
            Err(e) => {
                last_error = e;
                log::warn!(
                    "{noun}: Block {}/{} fehlgeschlagen (Versuch {} von {}): {}",
                    index + 1,
                    total,
                    attempt,
                    CHUNK_ATTEMPTS,
                    last_error
                );
                if attempt < CHUNK_ATTEMPTS && !should_retry(&last_error) {
                    break;
                }
            }
        }
    }
    Err(last_error)
}

// -- Test-Hilfen (geteilt von minutes- und notes-Tests) ---------------------

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use crate::settings::{get_default_settings, AppSettings};

    /// Was der Mock auf eine Anfrage antwortet.
    pub enum MockReply {
        /// 200 mit diesem Body.
        Body(String),
        /// Fehlerstatus (z. B. 500) mit kurzem Text.
        Status(u16),
        /// Antwortet nie (haengender Server): Verbindung bleibt offen.
        Hang,
    }

    /// `choices[0].message.content` als Chat-Completions-Body.
    pub fn chat_body(content: &str) -> String {
        serde_json::json!({
            "choices": [{ "message": { "role": "assistant", "content": content } }]
        })
        .to_string()
    }

    /// Mock eines OpenAI-kompatiblen /chat/completions-Endpunkts (Muster
    /// `translator.rs`), der den kompletten Antwort-Body vorgibt und den
    /// Request — inklusive `response_format` — schlicht verwirft.
    pub async fn spawn_llm_mock(body: String) -> u16 {
        spawn_llm_mock_with(move |_| MockReply::Body(body.clone())).await
    }

    /// Wie `spawn_llm_mock`, aber die Antwort haengt vom Request-Body ab
    /// (der JSON-Text der Anfrage inklusive Prompt). Fuer Map-Reduce-Tests,
    /// bei denen jeder Block etwas anderes antworten soll.
    pub async fn spawn_llm_mock_with(
        handler: impl Fn(&str) -> MockReply + Send + Sync + 'static,
    ) -> u16 {
        let handler = Arc::new(handler);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4_194_304];
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
                                let request = String::from_utf8_lossy(
                                    &buf[header_end + 4..header_end + 4 + content_length],
                                )
                                .to_string();
                                let (status, body) = match handler(&request) {
                                    MockReply::Body(body) => ("200 OK".to_string(), body),
                                    MockReply::Status(code) => {
                                        (format!("{code} Mock Error"), "mock failure".to_string())
                                    }
                                    MockReply::Hang => std::future::pending().await,
                                };
                                let head = format!(
                                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
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

    pub fn settings_with_mock_provider(port: u16) -> AppSettings {
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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::test_support::*;
    use super::*;
    use crate::settings::get_default_settings;

    #[derive(Debug, Deserialize)]
    struct Answer {
        text: String,
    }

    fn opts(redact: bool) -> AskOptions<'static> {
        AskOptions {
            purpose: Purpose::Minutes,
            noun: "Test",
            redact_parse_errors: redact,
        }
    }

    fn schema(_local: bool) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    fn never(_: &Answer) -> Option<SemanticRetry> {
        None
    }

    #[test]
    fn strip_code_fence_removes_a_json_fence_and_leaves_plain_text() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("  {\"a\":1}  "), "{\"a\":1}");
        assert_eq!(strip_code_fence("```\n{}\n```"), "{}");
    }

    #[test]
    fn resolve_provider_reports_a_code_and_keeps_the_original_messages() {
        let mut settings = get_default_settings();
        settings.post_process_provider_id = "does-not-exist".into();
        let err = resolve_provider_coded(&settings).unwrap_err();
        assert_eq!(err.code, "no_provider");
        assert!(err.message.starts_with("Kein LLM-Provider konfiguriert"));

        let mut settings = get_default_settings();
        settings.post_process_provider_id = "custom".into();
        settings
            .post_process_models
            .insert("custom".into(), "  ".into());
        let err = resolve_provider_coded(&settings).unwrap_err();
        assert_eq!(err.code, "no_model");
        assert!(err.message.contains("kein Modell eingetragen"));
        assert_eq!(resolve_provider(&settings).unwrap_err(), err.message);
    }

    #[test]
    fn should_retry_never_repeats_a_memory_failure() {
        let ram = "Zu wenig freier Arbeitsspeicher: 1.0 GB frei, gebraucht werden etwa 4.0 GB";
        let plain = "API request failed with status 500 Internal Server Error: x";
        let reserve = crate::process_guard::RAM_RESERVE_MB;
        // (Fehler, freier RAM in MB, Erwartung)
        let table: [(&str, u64, bool); 7] = [
            (ram, 64_000, false),        // RAM-Fehler: nie, egal wie viel jetzt frei ist
            (ram, 0, false),             // auch bei nicht messbarem RAM
            (plain, 64_000, true),       // Transportfehler bei viel RAM: ja
            (plain, reserve, true),      // genau auf der Reserve: ja
            (plain, reserve - 1, false), // knapp darunter: nein
            (plain, 512, false),         // Speicherwaechter-Lage: nein
            (plain, 0, true),            // nicht messbar blockiert nie
        ];
        for (err, free, expected) in table {
            assert_eq!(should_retry(err, free), expected, "{err} / {free} MB");
        }
    }

    /// Haengt das Erkennungszeichen an der echten Meldung des RAM-Start-Gates:
    /// wuerde `process_guard` den Text aendern, liefe `should_retry` sonst
    /// still ins Leere.
    #[test]
    fn the_memory_marker_matches_the_real_start_gate_message() {
        // Unmessbarer RAM (0) blockiert nie; dann gibt es keine Meldung zu pruefen.
        if let Err(msg) = crate::process_guard::check_ram_for_start(u64::MAX / 4) {
            assert!(is_memory_error(&msg), "war: {msg}");
        }
    }

    #[tokio::test]
    async fn retry_chunk_does_not_retry_a_memory_error_but_retries_a_transport_error() {
        // RAM-Fehler: genau ein Versuch.
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = retry_chunk(
            "T",
            0,
            1,
            |e| should_retry(e, 64_000),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err("Zu wenig freier Arbeitsspeicher: 1 GB".to_string()) }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Transportfehler: das ganze Budget, dann der letzte Fehler.
        let calls = AtomicUsize::new(0);
        let result: Result<(), String> = retry_chunk(
            "T",
            0,
            1,
            |e| should_retry(e, 64_000),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err("HTTP request failed: connection refused".to_string()) }
            },
        )
        .await;
        assert_eq!(
            result.unwrap_err(),
            "HTTP request failed: connection refused"
        );
        assert_eq!(calls.load(Ordering::SeqCst), CHUNK_ATTEMPTS);

        // `|_| true` (Protokoll) wiederholt auch einen RAM-Fehler wie bisher.
        let calls = AtomicUsize::new(0);
        let _: Result<(), String> = retry_chunk(
            "T",
            0,
            1,
            |_| true,
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Err("Zu wenig freier Arbeitsspeicher".to_string()) }
            },
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), CHUNK_ATTEMPTS);
    }

    #[tokio::test]
    async fn retry_chunk_returns_the_first_success() {
        let calls = AtomicUsize::new(0);
        let result = retry_chunk(
            "T",
            0,
            1,
            |_| true,
            || {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    if n == 0 {
                        Err("kurz weg".to_string())
                    } else {
                        Ok(7)
                    }
                }
            },
        )
        .await;
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn ask_json_retries_once_on_invalid_json_then_gives_up() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_llm_mock_with(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            MockReply::Body(chat_body("das ist kein JSON"))
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let err = ask_json::<Answer>(&settings, &opts(false), "sys", &schema, "prompt", &never)
            .await
            .unwrap_err();
        assert!(
            err.starts_with("Test-Antwort war kein gültiges JSON"),
            "war: {err}"
        );
        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "ein Versuch plus ein Retry"
        );
    }

    #[tokio::test]
    async fn ask_json_repairs_with_the_error_text_and_accepts_the_second_answer() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_llm_mock_with(move |body| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                MockReply::Body(chat_body("```json\nnoch nicht```"))
            } else {
                assert!(
                    body.contains("could not be parsed"),
                    "der Retry nennt den Fehler"
                );
                MockReply::Body(chat_body(r#"{"text":"ok"}"#))
            }
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let answer = ask_json::<Answer>(&settings, &opts(false), "sys", &schema, "prompt", &never)
            .await
            .unwrap();
        assert_eq!(answer.text, "ok");
    }

    #[tokio::test]
    async fn a_semantic_retry_is_asked_once_and_only_in_the_first_attempt() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_llm_mock_with(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            MockReply::Body(chat_body(r#"{"text":""}"#))
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let empty_text = |a: &Answer| {
            a.text.trim().is_empty().then(|| SemanticRetry {
                reason: "leer".into(),
                hint: "Fill the text.".into(),
            })
        };
        let answer = ask_json::<Answer>(
            &settings,
            &opts(false),
            "sys",
            &schema,
            "prompt",
            &empty_text,
        )
        .await
        .unwrap();
        assert_eq!(
            answer.text, "",
            "im zweiten Versuch wird die Antwort genommen"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }

    /// Ein Typfehler nennt bei serde_json den Wert im Wortlaut. Der stammt
    /// aus Notizen oder Transkript und darf mit `redact_parse_errors` nicht
    /// in den zurueckgegebenen Fehler (und damit nicht ins Log).
    #[tokio::test]
    async fn redacted_parse_errors_do_not_quote_the_model_output() {
        #[derive(Debug, Deserialize)]
        struct Count {
            #[allow(dead_code)]
            n: u32,
        }
        let secret = "GEHEIMES-PROJEKT-ALPHA";
        let port = spawn_llm_mock_with(move |_| {
            // `n` ist eine Zahl -- hier ein String: Typfehler mit dem Wert.
            MockReply::Body(chat_body(&format!(r#"{{"n": "{secret}"}}"#)))
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let no_check = |_: &Count| None;

        let redacted = ask_json::<Count>(&settings, &opts(true), "sys", &schema, "p", &no_check)
            .await
            .unwrap_err();
        assert!(!redacted.contains(secret), "war: {redacted}");
        assert!(
            redacted.contains("Zeile"),
            "Klasse und Position bleiben: {redacted}"
        );

        let verbatim = ask_json::<Count>(&settings, &opts(false), "sys", &schema, "p", &no_check)
            .await
            .unwrap_err();
        assert!(
            verbatim.contains(secret),
            "Protokoll unveraendert: {verbatim}"
        );
    }

    #[tokio::test]
    async fn a_transport_error_is_not_retried_by_ask_json() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let port = spawn_llm_mock_with(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            MockReply::Status(500)
        })
        .await;
        let settings = settings_with_mock_provider(port);
        let err = ask_json::<Answer>(&settings, &opts(false), "sys", &schema, "p", &never)
            .await
            .unwrap_err();
        assert!(
            err.starts_with("Test-Erzeugung fehlgeschlagen"),
            "war: {err}"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    }
}
