//! HTTP-Rohtransport für OpenAI-kompatible Anbieter (lva#604, Teilpaket 3).
//!
//! Drei Versuchsarten, je **ein** Request an das gebundene [`Ziel`]:
//! [`chat_einmal`] (Antwort am Stück), [`strom`] (SSE) und [`ohne_strom`] (der Rückfall, den
//! `versenden` nach einem abgelehnten oder vor dem ersten Token abgebrochenen Strom als
//! **neuen** Versuch startet). CLI-Ziele gehen nicht hierher, sondern nach
//! [`cli_chat`](super::cli_chat).
//!
//! Transportregeln: kein Redirect für jedes Ziel; die lokale Route ohne Proxy (auch keinen aus
//! der Umgebung), Cloud mit der Proxy-Wahl aus `netz::proxy`. Fehlerantworten werden bis zu
//! [`FEHLER_KOERPER_MAX`] Byte gelesen, auf Kontextüberlauf **klassifiziert** und verworfen;
//! kein Anbietertext gelangt in einen Fehler oder ins Log.

use std::time::Duration;

use futures_util::StreamExt;
use log::debug;
use reqwest::header::{HeaderMap, CONTENT_TYPE};

use super::{Ausgabe, Fehlschlag, Kontext, Versuch};
use crate::llm_client::{
    apply_limits, build_headers, clean_local_content, deterministic_sampling, is_ollama,
    stream_request_body, thinking_off_kwargs, ChatCompletionRequest, ChatCompletionResponse,
    ChatMessage, ChatReply, JsonSchema, ResponseFormat, SseDecoder, SseEvent, StreamLimits,
    StreamMessage, STREAM_FIRST_TOKEN_TIMEOUT, STREAM_IDLE_TIMEOUT, STREAM_MAX_CHARS,
};
use crate::llm_versand::anfrage::ChatInhalt;
use crate::llm_versand::fehler::VersandFehler;
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;
use crate::managers::usage::TokenUsage;
use crate::netz::proxy::Weg;

/// So viele Byte einer Fehlerantwort werden für die Klassifikation gelesen, der Rest verworfen.
const FEHLER_KOERPER_MAX: usize = 64 * 1024;

/// Kennzeichen eines Kontextüberlaufs in der Fehlerantwort (llama-server, OpenAI, Ollama,
/// vLLM). Treffer ergeben [`VersandFehler::Kontextueberlauf`]; der Text selbst geht nirgends hin.
const UEBERLAUF_MARKEN: [&str; 7] = [
    "exceed_context_size",
    "exceeds the available context size",
    "context_length_exceeded",
    "context length",
    "context window",
    "exceeds the context",
    "too many tokens",
];

/// Baut den Client für ein gebundenes Ziel: kein Redirect, die lokale Route ohne Proxy.
///
/// `headers`: Kopfzeilen des Anbieters (`None` bei Aufrufen ohne Zugangsdaten); `timeout`:
/// Gesamtzeit eines Requests (`None`: keine, wie bei Chat und Strom).
pub(in crate::llm_versand) async fn client(
    ziel: &Ziel,
    headers: Option<HeaderMap>,
    timeout: Option<Duration>,
) -> Result<reqwest::Client, VersandFehler> {
    let mut weg = Weg::Standard;
    if !ziel.ist_lokal() {
        #[cfg(not(test))]
        {
            weg = crate::netz::proxy::weg_fuer_async(basis(ziel)?).await;
        }
        // Im Test niemals PAC/WPAD auflösen; der vorhandene Haken hat Vorrang.
        #[cfg(test)]
        if let Some(proxy) = crate::llm_versand::testhaken::cloud_proxy() {
            weg = Weg::Ueber(proxy.to_string());
        }
    }
    client_mit_weg(ziel, headers, timeout, &weg)
}

/// Baut den Client ohne Systemauflösung mit einem vorgegebenen Weg.
pub(in crate::llm_versand) fn client_mit_weg(
    ziel: &Ziel,
    headers: Option<HeaderMap>,
    timeout: Option<Duration>,
    weg: &Weg,
) -> Result<reqwest::Client, VersandFehler> {
    let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
    if let Some(headers) = headers {
        builder = builder.default_headers(headers);
    }
    if ziel.ist_lokal() {
        builder = builder.no_proxy();
    } else {
        builder = crate::netz::proxy::anwenden(builder, weg);
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(|_| {
        log::warn!("LLM-Client nicht aufbaubar");
        VersandFehler::Transport
    })
}

/// Client mit den Kopfzeilen des Anbieters aus dem Kontext.
async fn anbieter_client(ziel: &Ziel, ctx: &Kontext<'_>) -> Result<reqwest::Client, VersandFehler> {
    let headers = build_headers(ctx.anbieter, ctx.api_key).map_err(|_| {
        // Der Fehlertext nennt nur das Feld, nie den Schlüssel; trotzdem nicht ins Log.
        log::warn!("LLM-Client: Kopfzeilen des Anbieters ungültig");
        VersandFehler::Transport
    })?;
    client(ziel, Some(headers), None).await
}

/// Nur die eingeordnete Meldung weitergeben, niemals Rohtext oder Adresse.
pub(super) fn netzmeldung(fehler: reqwest::Error) -> String {
    let roh = format!("{fehler:?}");
    let ausgabe = crate::netz::fehler::ausgabe(&roh);
    if ausgabe.code == "other" {
        "Netzanfrage fehlgeschlagen.".to_string()
    } else {
        ausgabe.meldung
    }
}

/// Senden gescheitert: Zeit abgelaufen oder keine Verbindung. Ohne den Text des Fehlers.
pub(in crate::llm_versand) fn sendefehler(fehler: &reqwest::Error) -> VersandFehler {
    if fehler.is_timeout() {
        VersandFehler::Zeitueberschreitung
    } else {
        VersandFehler::Transport
    }
}

/// Liest höchstens [`FEHLER_KOERPER_MAX`] Byte der Antwort.
pub(super) async fn koerper_anfang(mut antwort: reqwest::Response) -> Vec<u8> {
    let mut puffer = Vec::new();
    while puffer.len() < FEHLER_KOERPER_MAX {
        match antwort.chunk().await {
            Ok(Some(stueck)) => puffer.extend_from_slice(&stueck),
            _ => break,
        }
    }
    puffer.truncate(FEHLER_KOERPER_MAX);
    puffer
}

/// Hat die Fehlerantwort einen Kontextüberlauf gemeldet? Reine Klassifikation.
pub(in crate::llm_versand) fn ist_ueberlauf(koerper: &[u8]) -> bool {
    let text = String::from_utf8_lossy(koerper).to_ascii_lowercase();
    UEBERLAUF_MARKEN.iter().any(|marke| text.contains(marke))
}

/// Fehler einer nicht erfolgreichen Antwort: Status, oder Kontextüberlauf, wenn der Text es
/// verrät. Der Text wird gelesen und verworfen.
pub(in crate::llm_versand) async fn statusfehler(antwort: reqwest::Response) -> VersandFehler {
    let status = antwort.status().as_u16();
    let koerper = koerper_anfang(antwort).await;
    if ist_ueberlauf(&koerper) {
        log::info!("Anfrage abgelehnt (Status {status}): Kontextüberlauf");
        return VersandFehler::Kontextueberlauf;
    }
    log::info!("Anfrage abgelehnt (Status {status})");
    VersandFehler::Http { status }
}

pub(super) fn basis(ziel: &Ziel) -> Result<&str, VersandFehler> {
    ziel.basis_url().ok_or(VersandFehler::Transport)
}

/// Eine Antwort am Stück. Ein Request, an die gebundene Basis-URL; die Route wird nicht neu
/// aufgelöst.
pub(in crate::llm_versand) async fn chat_einmal(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    ctx: &Kontext<'_>,
    chat: &ChatInhalt,
) -> Versuch<ChatReply> {
    chat_mit_limit(freigabe, ziel, quellen, ctx, chat, None).await
}

pub(in crate::llm_versand) async fn chat_mit_limit(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    ctx: &Kontext<'_>,
    chat: &ChatInhalt,
    max_bytes: Option<usize>,
) -> Versuch<ChatReply> {
    freigabe.einloesen(ziel, quellen)?;
    let base_url = basis(ziel)?.trim_end_matches('/');
    let url = format!("{base_url}/chat/completions");
    // „Lokal“ für die Anfragegestalt (Sampling, Denkschalter) ist wie bisher der lokale Anbieter;
    // die Sicherheitsgrenze (Proxy, Prüfung) ist `ziel.ist_lokal()`.
    let local = crate::managers::llm::is_local(ctx.anbieter);
    let sampling = deterministic_sampling(local, ctx.zweck);
    debug!(
        "Sending chat completion request to: {}",
        ziel.adresse_fuer_log()
    );
    let client = anbieter_client(ziel, ctx).await?;

    let mut messages = Vec::new();
    if let Some(system) = &chat.system {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: system.clone(),
        });
    }
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: chat.nutzer.clone(),
    });
    let response_format = chat.schema.clone().map(|schema| ResponseFormat {
        format_type: "json_schema".to_string(),
        json_schema: JsonSchema {
            name: "transcription_output".to_string(),
            strict: true,
            schema,
        },
    });
    let request_body = ChatCompletionRequest {
        model: ziel.modell().to_string(),
        messages,
        response_format,
        reasoning_effort: chat.reasoning_effort.clone(),
        reasoning: chat.reasoning.clone(),
        chat_template_kwargs: thinking_off_kwargs(local, ziel.modell()),
        temperature: sampling.0,
        seed: sampling.1,
    };

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| Fehlschlag::Fehler(sendefehler(&e)))?;
    if !response.status().is_success() {
        return Err(Fehlschlag::Fehler(statusfehler(response).await));
    }
    let completion: ChatCompletionResponse = if let Some(limit) = max_bytes {
        let bytes = begrenzter_koerper(response.bytes_stream(), limit).await?;
        serde_json::from_slice(&bytes).map_err(|_| Fehlschlag::Fehler(VersandFehler::Protokoll))?
    } else {
        response.json().await.map_err(|_| {
            log::info!("Antwort des Anbieters nicht lesbar");
            Fehlschlag::Fehler(VersandFehler::Protokoll)
        })?
    };

    let tokens = TokenUsage::from(completion.usage);
    let first = completion.choices.first();
    let content = first.and_then(|choice| choice.message.content.clone());
    let truncated = first
        .and_then(|choice| choice.finish_reason.as_deref())
        .is_some_and(|reason| reason == "length");
    Ok((
        ChatReply {
            content: clean_local_content(local, content),
            truncated,
        },
        tokens,
    ))
}

// Vor dem Kopieren prüfen; auch ohne Content-Length bleibt der Sammelpuffer begrenzt.
async fn begrenzter_koerper<S, B, E>(chunks: S, limit: usize) -> Result<Vec<u8>, VersandFehler>
where
    S: futures_util::Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
{
    futures_util::pin_mut!(chunks);
    let mut bytes = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| VersandFehler::Transport)?;
        let chunk = chunk.as_ref();
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(VersandFehler::Protokoll);
        }
        bytes.extend_from_slice(chunk);
    }
    Ok(bytes)
}

/// Antwort als Strom (SSE), ein Request. Lehnt der Server Streaming ab (Fehlerstatus) oder
/// reißt der Strom **vor dem ersten Token** ab, ist das [`Fehlschlag::OhneStromNeu`]: `versenden`
/// entscheidet, ob ein neuer Versuch ohne Strom (mit eigener Prüfung) folgt. Schickt der Server
/// trotz `stream: true` eine ganze JSON-Antwort, wird sie als ein Stück genommen. Nach dem
/// ersten Token ist ein Abbruch ein Fehler (der Text war schon zu sehen).
pub(in crate::llm_versand) async fn strom(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    ctx: &Kontext<'_>,
    nachrichten: &[StreamMessage],
    grenzen: &StreamLimits,
    ausgabe: &Ausgabe<'_>,
) -> Versuch<String> {
    freigabe.einloesen(ziel, quellen)?;
    let url = format!("{}/chat/completions", basis(ziel)?.trim_end_matches('/'));
    debug!(
        "Sending streaming chat completion request to: {}",
        ziel.adresse_fuer_log()
    );
    let client = anbieter_client(ziel, ctx).await?;
    let local = crate::managers::llm::is_local(ctx.anbieter);
    let mut body = stream_request_body(ziel.modell(), nachrichten, true, local);
    apply_limits(&mut body, is_ollama(ctx.anbieter), grenzen);

    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| Fehlschlag::Fehler(sendefehler(&e)))?;
    let status = response.status();
    if !status.is_success() {
        log::info!(
            "Streaming abgelehnt ({}), neuer Versuch ohne Streaming erlaubt",
            status.as_u16()
        );
        return Err(Fehlschlag::OhneStromNeu);
    }
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));
    if !is_sse {
        // Der Server hat `stream` ignoriert und eine ganze Antwort geschickt.
        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|_| Fehlschlag::Fehler(VersandFehler::Protokoll))?;
        return Ok(ganze_antwort(completion, ausgabe));
    }

    let mut decoder = SseDecoder::default();
    let mut text = String::new();
    let mut text_chars = 0usize;
    let mut tokens = TokenUsage::default();
    let mut finished = false;
    let mut bytes = response.bytes_stream();
    'read: loop {
        let wait = if text.is_empty() {
            STREAM_FIRST_TOKEN_TIMEOUT
        } else {
            STREAM_IDLE_TIMEOUT
        };
        let (events, ended) = match tokio::time::timeout(wait, bytes.next()).await {
            Err(_) => return Err(Fehlschlag::Fehler(VersandFehler::Zeitueberschreitung)),
            Ok(None) => (decoder.finish(), true),
            Ok(Some(Err(_))) => {
                if text.is_empty() {
                    log::info!(
                        "Stream vor dem ersten Token abgebrochen, neuer Versuch ohne Streaming erlaubt"
                    );
                    return Err(Fehlschlag::OhneStromNeu);
                }
                return Err(Fehlschlag::Fehler(VersandFehler::Protokoll));
            }
            Ok(Some(Ok(chunk))) => (
                decoder
                    .push(&chunk)
                    .map_err(|_| Fehlschlag::Fehler(VersandFehler::Protokoll))?,
                false,
            ),
        };
        for event in events {
            match event {
                SseEvent::Delta(delta) => {
                    let delta_chars = delta.chars().count();
                    if text_chars + delta_chars > STREAM_MAX_CHARS {
                        log::warn!(
                            "Stream: Antwort ueber {STREAM_MAX_CHARS} Zeichen, abgeschnitten"
                        );
                        if grenzen.strict {
                            return Err(Fehlschlag::Fehler(VersandFehler::Protokoll));
                        }
                        break 'read;
                    }
                    text_chars += delta_chars;
                    text.push_str(&delta);
                    (ausgabe.on_delta)(&delta);
                }
                SseEvent::Reasoning(gedanke) => {
                    if let Some(senke) = &ausgabe.senke {
                        senke(&gedanke);
                    }
                }
                SseEvent::Usage(u) => tokens = u,
                SseEvent::Done => {
                    finished = true;
                    break 'read;
                }
                SseEvent::Error(_) => {
                    // Der Typ des Fehlerereignisses ist Anbietertext: nicht weiterreichen.
                    log::info!("Stream: Fehlerereignis vom Server");
                    return Err(Fehlschlag::Fehler(VersandFehler::Protokoll));
                }
            }
        }
        if ended {
            break;
        }
    }
    if grenzen.strict && !finished {
        return Err(Fehlschlag::Fehler(VersandFehler::Protokoll));
    }
    Ok((text, tokens))
}

/// Der Rückfall: dieselben Nachrichten, `stream: false`, ein Request. Die ganze Antwort geht als
/// ein Stück an den Rückruf. Ein **neuer** Versuch: `versenden` hat ihn frisch geprüft.
pub(in crate::llm_versand) async fn ohne_strom(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    ctx: &Kontext<'_>,
    nachrichten: &[StreamMessage],
    grenzen: &StreamLimits,
    ausgabe: &Ausgabe<'_>,
) -> Versuch<String> {
    freigabe.einloesen(ziel, quellen)?;
    let url = format!("{}/chat/completions", basis(ziel)?.trim_end_matches('/'));
    let client = anbieter_client(ziel, ctx).await?;
    let local = crate::managers::llm::is_local(ctx.anbieter);
    let mut body = stream_request_body(ziel.modell(), nachrichten, false, local);
    apply_limits(&mut body, is_ollama(ctx.anbieter), grenzen);
    let response = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| Fehlschlag::Fehler(sendefehler(&e)))?;
    if !response.status().is_success() {
        return Err(Fehlschlag::Fehler(statusfehler(response).await));
    }
    let completion: ChatCompletionResponse = response
        .json()
        .await
        .map_err(|_| Fehlschlag::Fehler(VersandFehler::Protokoll))?;
    Ok(ganze_antwort(completion, ausgabe))
}

/// Eine ganze Antwort als ein Stück an den Rückruf (höchstens [`STREAM_MAX_CHARS`] Zeichen).
fn ganze_antwort(
    completion: ChatCompletionResponse,
    ausgabe: &Ausgabe<'_>,
) -> (String, TokenUsage) {
    let text: String = completion
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default()
        .chars()
        .take(STREAM_MAX_CHARS)
        .collect();
    if !text.is_empty() {
        (ausgabe.on_delta)(&text);
    }
    (text, TokenUsage::from(completion.usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::netz::proxy::tests::{antwort, gesehen, testserver};

    #[tokio::test]
    async fn koerperlimit_gilt_ueber_stueckgrenzen_und_stoppt_sofort() {
        let genau = futures_util::stream::iter([Ok::<_, ()>(vec![0; 3]), Ok(vec![0; 5])]);
        assert_eq!(begrenzter_koerper(genau, 8).await.unwrap().len(), 8);
        let mut gelesen = 0;
        let zu_gross = futures_util::stream::iter([vec![0; 3], vec![0; 6], vec![0; 1]])
            .map(|chunk| {
                gelesen += 1;
                assert!(gelesen <= 2, "nach Überschreitung nicht weiterlesen");
                Ok::<_, ()>(chunk)
            });
        assert_eq!(begrenzter_koerper(zu_gross, 8).await, Err(VersandFehler::Protokoll));
        assert_eq!(gelesen, 2);
        let ein_stueck = futures_util::stream::iter([Ok::<_, ()>(vec![0; 9])]);
        assert_eq!(begrenzter_koerper(ein_stueck, 8).await, Err(VersandFehler::Protokoll));
    }

    #[tokio::test]
    async fn cloud_nutzt_den_vorgegebenen_proxy() {
        let (proxy, log) = testserver(antwort("204 No Content", "")).await;
        let ziel = Ziel::http("custom", "http://example.invalid/", "m");
        let client = client_mit_weg(
            &ziel,
            None,
            Some(Duration::from_secs(5)),
            &Weg::Ueber(proxy),
        )
        .unwrap();
        let response = client.get(basis(&ziel).unwrap()).send().await.unwrap();
        assert_eq!(response.status().as_u16(), 204);
        assert_eq!(gesehen(&log), ["GET http://example.invalid/ HTTP/1.1"]);
    }

    #[tokio::test]
    async fn direkt_kontaktiert_den_proxy_nicht() {
        let (proxy, log) = testserver(antwort("204 No Content", "")).await;
        let ziel = Ziel::http("custom", "http://example.invalid/", "m");
        // Gegenprobe: derselbe Cloud-Host ist über diesen Proxy erreichbar.
        let ueber = client_mit_weg(
            &ziel,
            None,
            Some(Duration::from_secs(5)),
            &Weg::Ueber(proxy),
        )
        .unwrap();
        ueber.get(basis(&ziel).unwrap()).send().await.unwrap();
        assert_eq!(gesehen(&log).len(), 1);

        let direkt =
            client_mit_weg(&ziel, None, Some(Duration::from_millis(500)), &Weg::Direkt).unwrap();
        assert!(direkt.get(basis(&ziel).unwrap()).send().await.is_err());
        assert_eq!(
            gesehen(&log).len(),
            1,
            "direkt darf keinen Treffer hinzufügen"
        );
    }

    #[tokio::test]
    async fn lokal_ignoriert_den_vorgegebenen_proxy() {
        let (proxy, proxy_log) = testserver(antwort("502 Bad Gateway", "")).await;
        let (adresse, ziel_log) = testserver(antwort("204 No Content", "")).await;
        let ziel = Ziel::http("custom", &adresse, "m");
        let client = client_mit_weg(
            &ziel,
            None,
            Some(Duration::from_secs(5)),
            &Weg::Ueber(proxy),
        )
        .unwrap();
        let response = client.get(basis(&ziel).unwrap()).send().await.unwrap();
        assert_eq!(response.status().as_u16(), 204);
        assert_eq!(gesehen(&ziel_log), ["GET / HTTP/1.1"]);
        assert!(gesehen(&proxy_log).is_empty());
    }

    #[tokio::test]
    async fn netzmeldung_gibt_keine_adresse_oder_geheimnisse_aus() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let fehler = client
            .get("http://127.0.0.1:1/GEHEIM?key=GEHEIM")
            .send()
            .await
            .unwrap_err();
        let meldung = netzmeldung(fehler);
        assert_eq!(
            meldung,
            crate::netz::fehler::Netzfehler::KeineVerbindung.meldung()
        );
        assert!(!meldung.contains("GEHEIM"));
        assert!(!meldung.contains("127.0.0.1"));
    }

    #[test]
    fn ueberlauf_wird_an_den_marken_erkannt_und_sonst_nicht() {
        for text in [
            r#"{"error":{"type":"exceed_context_size_error"}}"#,
            "the request exceeds the available context size",
            "This model's maximum context length is 8192 tokens",
            "Context window exceeded",
            "Too many tokens in the prompt",
        ] {
            assert!(ist_ueberlauf(text.as_bytes()), "{text}");
        }
        for text in ["Internal Server Error", "rate limited", ""] {
            assert!(!ist_ueberlauf(text.as_bytes()), "{text}");
        }
    }
}
