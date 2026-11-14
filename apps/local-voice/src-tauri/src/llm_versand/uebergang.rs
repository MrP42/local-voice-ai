//! Übergangsfassade bis lva#601 (lva#604, Teilpaket 3).
//!
//! Die bestehenden Aufrufer rufen weiter Funktionen mit **den heutigen Signaturen und dem
//! bisherigen Verhalten**; sie laufen aber über den Rohtransport von `llm_versand` und nicht
//! mehr über eigene Netzwege. Jede Funktion ist `#[deprecated(note = "entfällt mit #601")]`; die
//! Aufrufer tragen `#[allow(deprecated)]` an der Aufrufstelle, damit jede Stelle, die #601
//! umstellen muss, mit `grep` auffindbar bleibt ([`AUFRUFER`] hält sie vollständig fest, ein
//! Test hält die Liste mit dem Quelltext zusammen).
//!
//! # Was die Fassade **nicht** tut
//!
//! Sie prüft **keine Besprechungsquellen**: es gibt hier noch keine Herkunftsangabe, und die
//! Besprechungspfade werden ausdrücklich **nicht** als [`Quellen::keine`] umetikettiert. Der
//! Prüfer läuft nicht ([`Pruefung::Uebergang`]); der Transport bindet seine Freigabe an
//! [`quellen_uebergang`](super::pruefer::quellen_uebergang) (unbestimmt: die Fassade kennt die
//! Herkunft nicht). Das heißt: **bis #601 die Fassade entfernt, gibt es für diese Aufrufer
//! keinen Schutz durch den Versandvertrag**, nur den bestehenden (Compliance, Budget,
//! Lokalschutz einzelner Aufrufer). Erst `versenden` mit belegter Herkunft schützt flächendeckend.
//! Folge: kein Abschlussvergleich und kein Streamingpuffer auf diesem Weg.
//!
//! # Was sich gegenüber früher ändert (erlaubt, TP3)
//!
//! - Kein Redirect für jedes Ziel; die lokale Route (Loopback) ohne Proxy.
//! - Compliance und Budget bei **jedem** Versuch, auch bei den Rückfällen (Strom ohne Strom,
//!   CLI nach dem Update) und beim natives-Ollama-Weg.
//! - Kein Anbieter- und kein CLI-Text in Fehlern und im Log. Fehler tragen [`VersandFehler`]
//!   in der Textform [`VersandFehler::alte_textform`]: dieselben Präfixe und Marken, an denen die
//!   Aufrufer heute Fehlerarten erkennen (Kontextüberlauf, `HTTP request failed`,
//!   `cli_not_logged_in:`, Zeitlimit …), aber ohne Antworttext.
//! - Natives Ollama und `ollama_unload` laufen mit der aus dem Host abgeleiteten Zielart (nicht
//!   mehr an `contains("localhost")` gebunden), ohne Redirect, lokal ohne Proxy.
//!
//! Das Auflösen der Route (der lokale Server wird bei Bedarf gestartet) geschieht **vor** dem
//! Versand und ist nicht Teil des einen Transportversuchs; der Transport löst sie nie neu auf.
//! Compliance und Budget laufen deshalb **auch davor** ([`vorpruefung`]), wie im Aufruf vor
//! lva#604: ein gesperrter oder budgetgesperrter Aufruf startet nie ein lokales Modell. Die
//! Prüfung je Transportversuch bleibt zusätzlich bestehen.
#![allow(deprecated)]

use std::path::Path;
use std::time::Duration;

use super::anfrage::{Anfrage, ChatInhalt, Inhalt, JsonAntwort};
use super::fehler::VersandFehler;
use super::pruefer::{freigabe_uebergang, quellen_uebergang};
#[cfg(test)]
use super::testhaken;
use super::transport::{bild, cli_roh, json, modelle, ollama};
use super::versand::{ausfuehren, regelwerk_und_budget, Lauf, Pruefung};
use super::ziel::Ziel;
use crate::llm_client::{
    ollama_native_url, ChatReply, ImageReply, ReasoningConfig, StreamLimits, StreamMessage,
};
use crate::managers::llm::cli::{CallOptions, Cli, CliReply};
use crate::managers::usage::Purpose;
use crate::settings::PostProcessProvider;

/// Ob ein Aufrufer Besprechungsinhalt tragen kann (Einstufung wie im Inventar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Besprechung {
    /// Text aus Besprechungen geht in den Aufruf.
    Ja,
    /// Der Weg kann Besprechungsinhalt tragen, die Herkunft ist heute nicht belegt: in #601
    /// `Quellen::unbestimmt()`, bis eine Herkunft ermittelt ist.
    Moeglich,
    /// Nein, mit Begründung im Inventar.
    Nein,
}

/// Ein Aufrufer der Fassade.
#[derive(Clone, Copy, Debug)]
pub struct Aufrufer {
    /// Quelldatei relativ zu `src/`.
    pub datei: &'static str,
    /// Eintrag im Inventar (`docs/llm-versand-inventar.md`).
    pub inventar: &'static str,
    /// Einstufung.
    pub besprechung: Besprechung,
}

const fn aufrufer(
    datei: &'static str,
    inventar: &'static str,
    besprechung: Besprechung,
) -> Aufrufer {
    Aufrufer {
        datei,
        inventar,
        besprechung,
    }
}

/// **Vollständige Aufruferliste** der Fassade (jede Quelldatei außerhalb von `llm_versand`, die
/// sie benutzt). lva#601 stellt jede Zeile auf [`versenden`](super::versenden) mit belegter
/// Herkunft um und entfernt die Fassade. Der Test `aufruferliste_ist_vollstaendig` vergleicht die
/// Liste mit dem Quelltext.
pub const AUFRUFER: &[Aufrufer] = &[
    // Besprechungsinhalt: ja
    aufrufer("managers/meetings/llm_call.rs", "B1", Besprechung::Ja),
    aufrufer("managers/meetings/followup.rs", "B8", Besprechung::Ja),
    aufrufer(
        "managers/meetings/chat/controller.rs",
        "B9",
        Besprechung::Ja,
    ),
    aufrufer("summarizer.rs", "B10", Besprechung::Ja),
    aufrufer("managers/meetings/slides/vision.rs", "E1", Besprechung::Ja),
    aufrufer("agent/runtime.rs", "B11", Besprechung::Ja),
    aufrufer("managers/meetings/search/embed.rs", "B12", Besprechung::Ja),
    aufrufer("managers/llm/mod.rs", "B13", Besprechung::Ja),
    // Besprechungsinhalt: möglich (Herkunft heute nicht belegt)
    aufrufer("projekte/zustand.rs", "C1", Besprechung::Moeglich),
    aufrufer("voice_call/turn.rs", "C2", Besprechung::Moeglich),
    aufrufer("voice_call/mod.rs", "C3", Besprechung::Moeglich),
    aufrufer("voice_call/projekt.rs", "C4", Besprechung::Moeglich),
    // Besprechungsinhalt: nein (Begründung im Inventar)
    aufrufer("actions.rs", "D1", Besprechung::Nein),
    aufrufer("books.rs", "D2", Besprechung::Nein),
    aufrufer("book_ai/talk/mod.rs", "D3", Besprechung::Nein),
    aufrufer("sync/booktalk_relay.rs", "D4", Besprechung::Nein),
    aufrufer("book_ai/talk/adapter.rs", "D6", Besprechung::Nein),
    aufrufer("translator.rs", "D8", Besprechung::Nein),
    aufrufer("tagging.rs", "D9", Besprechung::Nein),
    aufrufer("managers/llm/updates.rs", "D11", Besprechung::Nein),
    aufrufer("commands/tts.rs", "E2", Besprechung::Nein),
    aufrufer("commands/llm.rs", "E4", Besprechung::Nein),
    aufrufer("shortcut/mod.rs", "E4", Besprechung::Nein),
];

/// Regelwerk vor dem Budget, **bevor** die Route aufgelöst wird: wie der Aufruf vor lva#604 darf
/// ein verweigerter Aufruf nicht einmal den lokalen Server starten. Die Verweigerung bleibt
/// ungebucht und kommt in der alten Textform (die Meldung der Prüfung) zurück.
fn vorpruefung(provider: &PostProcessProvider, model: &str) -> Result<(), String> {
    regelwerk_und_budget(provider, model).map_err(|fehler| fehler.alte_textform())
}

/// Die aufgelöste Route zum Anbieter: CLI-Anbieter ohne Netz, sonst die Adresse, die der
/// Anbieter **jetzt** trägt (für den lokalen Anbieter die des laufenden Servers; er wird bei
/// Bedarf gestartet).
async fn ziel_aufloesen(provider: &PostProcessProvider, model: &str) -> Result<Ziel, String> {
    #[cfg(test)]
    testhaken::aufloesung_zaehlen();
    if Cli::from_base_url(&provider.base_url).is_some() {
        return Ok(Ziel::aus_provider(provider, model));
    }
    let adresse = crate::managers::llm::resolve_base_url(provider, model).await?;
    Ok(Ziel::http(&provider.id, &adresse, model))
}

/// Chat-Anfrage an einen OpenAI-kompatiblen Anbieter. Liefert `Ok(Some(text))`, `Ok(None)` ohne
/// Inhalt oder den Fehler.
#[deprecated(note = "entfällt mit #601")]
pub async fn send_chat_completion(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    prompt: String,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_with_schema(
        purpose,
        provider,
        api_key,
        model,
        prompt,
        None,
        None,
        reasoning_effort,
        reasoning,
    )
    .await
}

/// Wie [`send_chat_completion`], mit Systemtext und JSON-Schema für strukturierte Ausgabe.
#[deprecated(note = "entfällt mit #601")]
#[allow(clippy::too_many_arguments)]
pub async fn send_chat_completion_with_schema(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<serde_json::Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<Option<String>, String> {
    send_chat_completion_checked(
        purpose,
        provider,
        api_key,
        model,
        user_content,
        system_prompt,
        json_schema,
        reasoning_effort,
        reasoning,
    )
    .await
    .map(|reply| reply.content)
}

/// Wie [`send_chat_completion_with_schema`], liefert aber auch, ob die Antwort abgeschnitten
/// wurde.
#[deprecated(note = "entfällt mit #601")]
#[allow(clippy::too_many_arguments)]
pub async fn send_chat_completion_checked(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    user_content: String,
    system_prompt: Option<String>,
    json_schema: Option<serde_json::Value>,
    reasoning_effort: Option<String>,
    reasoning: Option<ReasoningConfig>,
) -> Result<ChatReply, String> {
    vorpruefung(provider, model)?;
    let ziel = ziel_aufloesen(provider, model).await?;
    let anfrage = Anfrage {
        zweck: purpose,
        anbieter: provider,
        api_key,
        inhalt: Inhalt::Chat(ChatInhalt {
            nutzer: user_content,
            system: system_prompt,
            schema: json_schema,
            reasoning_effort,
            reasoning,
        }),
    };
    // Auf den Haufen: die Zustandsmaschine des ganzen Versands soll nicht im Future jedes
    // Aufrufers (Workflows, Notizen, Agent) mitwandern und dort den Stapel füllen.
    Box::pin(ausfuehren(
        Pruefung::Uebergang,
        &ziel,
        &quellen_uebergang(),
        anfrage,
    ))
    .await
    .map(|antwort| ChatReply {
        content: antwort.text,
        truncated: antwort.abgeschnitten,
    })
    .map_err(|fehler| fehler.alte_textform())
}

/// Chat-Anfrage mit Streaming (SSE). `on_delta` bekommt jedes Textstück in Reihenfolge; das
/// Ergebnis ist der ganze Text. Rückfall ohne Streaming, wenn der Anbieter Streaming ablehnt oder
/// der Strom vor dem ersten Token abreißt (ein neuer Versuch mit eigener Prüfung).
#[deprecated(note = "entfällt mit #601")]
pub async fn send_chat_completion_stream(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<String, String> {
    send_chat_completion_stream_limited(
        purpose,
        provider,
        api_key,
        model,
        messages,
        on_delta,
        &StreamLimits::default(),
    )
    .await
}

/// Wie [`send_chat_completion_stream`], mit Ausgabegrenze und Strenge.
#[deprecated(note = "entfällt mit #601")]
pub async fn send_chat_completion_stream_limited(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
    limits: &StreamLimits,
) -> Result<String, String> {
    strom_kern(
        purpose, provider, api_key, model, messages, on_delta, limits, None,
    )
    .await
}

/// Wie [`send_chat_completion_stream`]; mit `pinned_local` (der schon aufgelösten Adresse des
/// eingebauten Modells) nur genau dorthin, ohne erneutes Auflösen oder Starten; sonst nur an das
/// eingebaute lokale Modell: Loopback-Adresse, ohne Proxy, ohne Umleitung, ohne CLI. Sonst Fehler
/// statt Versand.
#[deprecated(note = "entfällt mit #601")]
pub async fn send_chat_completion_stream_routed(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
    pinned_local: Option<&str>,
) -> Result<String, String> {
    strom_kern(
        purpose,
        provider,
        api_key,
        model,
        messages,
        on_delta,
        &StreamLimits::default(),
        pinned_local,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn strom_kern(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    messages: Vec<StreamMessage>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
    limits: &StreamLimits,
    pinned_local: Option<&str>,
) -> Result<String, String> {
    vorpruefung(provider, model)?;
    let ziel = match pinned_local {
        Some(adresse) => {
            if !crate::managers::llm::is_local(provider) {
                return Err("Kontext nur an das eingebaute lokale Modell".into());
            }
            // Gebunden: genau die Adresse, für die geprüft wurde (kein Neustart zwischen Prüfung
            // und Versand).
            let ziel = Ziel::http(&provider.id, adresse, model);
            if !ziel.ist_lokal() {
                return Err("Lokales Modell nicht auf Loopback".into());
            }
            ziel
        }
        None => ziel_aufloesen(provider, model).await?,
    };
    let anfrage = Anfrage {
        zweck: purpose,
        anbieter: provider,
        api_key,
        inhalt: Inhalt::Strom {
            nachrichten: messages,
            grenzen: *limits,
            on_delta,
        },
    };
    // Auf den Haufen: die Zustandsmaschine des ganzen Versands soll nicht im Future jedes
    // Aufrufers (Workflows, Notizen, Agent) mitwandern und dort den Stapel füllen.
    Box::pin(ausfuehren(
        Pruefung::Uebergang,
        &ziel,
        &quellen_uebergang(),
        anfrage,
    ))
    .await
    .map(|antwort| antwort.text.unwrap_or_default())
    .map_err(|fehler| fehler.alte_textform())
}

/// Modellliste eines Anbieters. Der lokale Anbieter listet, was auf der Platte liegt (kein
/// Server nötig), CLI-Anbieter ihre feste Liste; sonst `GET <basis>/models`.
#[deprecated(note = "entfällt mit #601")]
pub async fn fetch_models(
    provider: &PostProcessProvider,
    api_key: String,
) -> Result<Vec<String>, String> {
    if crate::managers::llm::is_local(provider) {
        return Ok(crate::managers::llm::downloaded_model_ids());
    }
    if let Some(cli) = Cli::from_base_url(&provider.base_url) {
        return Ok(cli.models().into_iter().map(|m| m.id).collect());
    }
    let ziel = Ziel::http(&provider.id, &provider.base_url, "");
    modelle::liste(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        provider,
        &api_key,
    )
    .await
}

/// Eine Anfrage über Ollamas nativen Endpunkt (`url`), wahlweise ohne GPU. Compliance, Budget
/// und Buchung wie bei jedem Modellaufruf.
#[deprecated(note = "entfällt mit #601")]
pub async fn send_ollama_native(
    purpose: Purpose,
    provider: &PostProcessProvider,
    url: &str,
    model: &str,
    prompt: String,
    cpu_only: bool,
) -> Result<Option<String>, String> {
    let ziel = Ziel::http(&provider.id, url, model);
    let quellen = quellen_uebergang();
    let mut lauf = Lauf::neu(Pruefung::Uebergang, &ziel, &quellen, provider, purpose);
    let ergebnis = Box::pin(
        lauf.versuch(|freigabe| ollama::nativ(freigabe, &ziel, &quellen, prompt, cpu_only)),
    )
    .await
    .and_then(|versuch| versuch.map_err(|fehlschlag| fehlschlag.endgueltig()));
    lauf.buchen(&ergebnis).await;
    ergebnis.map_err(|fehler| fehler.alte_textform())
}

/// Ein bei Ollama geladenes Modell sofort entladen (best effort; Scheitern ist nur ein
/// Protokolleintrag).
#[deprecated(note = "entfällt mit #601")]
pub async fn ollama_unload(base_url: &str, model: &str) {
    let Some(chat_url) = ollama_native_url(base_url) else {
        return;
    };
    let url = chat_url.replace("/api/chat", "/api/generate");
    let ziel = Ziel::http("ollama", &url, model);
    ollama::entladen(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
    )
    .await;
}

/// Ein JSON-POST an `<Basis von ziel><pfad>` für Aufrufer mit eigener Antwortauswertung (Agent
/// B11, Einbettungen B12, Tokenzählung B13). Compliance und Budget prüft, wie bisher, der Aufrufer
/// selbst; die Fassade ersetzt nur seinen eigenen Client durch den Rohtransport (kein Redirect,
/// lokal ohne Proxy). Fehlerantworten kommen als Status samt Überlaufmarke, ihr Text nie.
#[deprecated(note = "entfällt mit #601")]
pub async fn json_post(
    ziel: &Ziel,
    pfad: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<JsonAntwort, VersandFehler> {
    json::post(
        freigabe_uebergang(ziel).freigabe,
        ziel,
        &quellen_uebergang(),
        pfad,
        body,
        timeout,
    )
    .await
}

/// Schickt `image` samt `prompt` an `<base_url>/chat/completions` des Bildservers (der
/// eingebaute lokale Server mit Projektor). Fehlertexte tragen einen Code als Präfix. Ohne
/// Compliance und Budget wie bisher (nur lokal, kein Verbrauch).
#[deprecated(note = "entfällt mit #601")]
pub async fn post_image_prompt(
    base_url: &str,
    model: &str,
    image: &Path,
    prompt: &str,
    max_tokens: u32,
) -> Result<ImageReply, String> {
    let ziel = Ziel::eingebaut(model, base_url);
    bild::senden(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        image,
        prompt,
        max_tokens,
    )
    .await
}

/// Das CLI-Ziel für die rohen CLI-Einstiege.
fn cli_ziel(cli: Cli, model: &str) -> Ziel {
    match cli {
        Cli::Claude => Ziel::cli("claude_cli", "claude", model),
        Cli::Codex => Ziel::cli("codex_cli", "codex", model),
    }
}

/// Ein CLI-Aufruf mit Zeitgrenze (Probelauf). Blockiert; im Blocking-Thread rufen. Ohne
/// Compliance und Budget wie bisher (fester Inhalt, kein Modellaufruf mit Nutzerdaten).
#[deprecated(note = "entfällt mit #601")]
pub fn cli_mit_zeitgrenze(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
) -> Result<CliReply, String> {
    let ziel = cli_ziel(cli, model);
    cli_roh::mit_zeitgrenze(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        cli,
        binary,
        system,
        user,
        timeout,
    )
}

/// Ein CLI-Aufruf mit Optionen (Buch-Gespräch: Abbruch, Ausgabegrenze, Nutzungsangabe).
/// Blockiert; im Blocking-Thread rufen.
#[deprecated(note = "entfällt mit #601")]
#[allow(clippy::too_many_arguments)]
pub fn cli_mit_optionen(
    cli: Cli,
    binary: &Path,
    model: &str,
    system: Option<&str>,
    user: &str,
    timeout: Duration,
    opts: &CallOptions,
) -> Result<CliReply, String> {
    let ziel = cli_ziel(cli, model);
    cli_roh::mit_optionen(
        freigabe_uebergang(&ziel).freigabe,
        &ziel,
        &quellen_uebergang(),
        cli,
        binary,
        system,
        user,
        timeout,
        opts,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::llm_versand::testhilfe::{anbieter, mock, RohAntwort};

    fn quelltext_dateien(wurzel: &Path, treffer: &mut Vec<std::path::PathBuf>) {
        let Ok(eintraege) = std::fs::read_dir(wurzel) else {
            return;
        };
        for eintrag in eintraege.flatten() {
            let pfad = eintrag.path();
            if pfad.is_dir() {
                quelltext_dateien(&pfad, treffer);
            } else if pfad.extension().is_some_and(|e| e == "rs") {
                treffer.push(pfad);
            }
        }
    }

    /// Die Aufruferliste ist vollständig und hat keine Karteileichen: jede Quelldatei außerhalb
    /// von `llm_versand`, die die Fassade benutzt, steht in [`AUFRUFER`], und jede Zeile dort
    /// benutzt sie wirklich. Testdateien zählen mit (sie sind Aufrufer wie jede andere Stelle).
    #[test]
    fn aufruferliste_ist_vollstaendig() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut dateien = Vec::new();
        quelltext_dateien(&src, &mut dateien);
        let mut benutzt: Vec<String> = dateien
            .iter()
            .filter(|pfad| !pfad.starts_with(src.join("llm_versand")))
            .filter(|pfad| {
                std::fs::read_to_string(pfad)
                    .is_ok_and(|text| text.contains("llm_versand::uebergang"))
            })
            .map(|pfad| {
                pfad.strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        benutzt.sort();
        let mut gelistet: Vec<String> = AUFRUFER.iter().map(|a| a.datei.to_string()).collect();
        gelistet.sort();
        gelistet.dedup();
        assert_eq!(
            benutzt, gelistet,
            "AUFRUFER und Quelltext weichen ab (links: benutzt, rechts: gelistet)"
        );
    }

    #[test]
    fn aufruferliste_hat_zu_jedem_eintrag_eine_inventarnummer() {
        for a in AUFRUFER {
            assert!(
                a.inventar.len() >= 2 && a.inventar.as_bytes()[0].is_ascii_uppercase(),
                "{a:?}"
            );
        }
        // Besprechungsinhalt ja: genau die Gruppen B und E1.
        for a in AUFRUFER.iter().filter(|a| a.besprechung == Besprechung::Ja) {
            assert!(a.inventar.starts_with('B') || a.inventar == "E1", "{a:?}");
        }
    }

    #[test]
    fn die_fassade_bindet_an_unbestimmt_und_nie_an_keine() {
        let ziel = Ziel::http("openai", "https://api.example.com/v1", "m");
        let freigabe = freigabe_uebergang(&ziel).freigabe;
        // Gegen `keine()` ist die Freigabe der Fassade nicht einlösbar: die Fassade behauptet
        // nicht, es gehe kein Besprechungsinhalt hinaus.
        assert!(freigabe
            .einloesen(&ziel, &crate::llm_versand::Quellen::keine())
            .is_err());
        let freigabe = freigabe_uebergang(&ziel).freigabe;
        assert!(freigabe.einloesen(&ziel, &quellen_uebergang()).is_ok());
    }

    /// Zählt die Prüfungen und verweigert höchstens eine (`"compliance"` oder `"budget"`).
    fn regelwerk(verweigert: Option<&'static str>) -> Rc<RefCell<Vec<&'static str>>> {
        let geprueft = Rc::new(RefCell::new(Vec::new()));
        let gemerkt = Rc::clone(&geprueft);
        testhaken::setze_regelwerk(move |art, _anbieter, _modell| {
            gemerkt.borrow_mut().push(art);
            match verweigert == Some(art) {
                true => Err(format!("{art} verweigert")),
                false => Ok(()),
            }
        });
        geprueft
    }

    fn nachrichten() -> Vec<StreamMessage> {
        vec![StreamMessage::new("user", "Frage")]
    }

    #[tokio::test]
    async fn ein_budgetgesperrter_chat_loest_die_route_nicht_auf() {
        let _wache = testhaken::wache();
        let geprueft = regelwerk(Some("budget"));
        let anbieter = anbieter("custom", "http://cloud.example/v1");

        let ergebnis = send_chat_completion_checked(
            Purpose::Chat,
            &anbieter,
            String::new(),
            "m",
            "Frage".into(),
            None,
            None,
            None,
            None,
        )
        .await;

        assert_eq!(ergebnis.unwrap_err(), "budget verweigert");
        assert_eq!(testhaken::aufloesungen(), 0, "die Route blieb unaufgelöst");
        assert_eq!(*geprueft.borrow(), ["compliance", "budget"]);
    }

    #[tokio::test]
    async fn ein_gesperrter_anbieter_wird_vor_dem_budget_und_der_route_abgelehnt() {
        let _wache = testhaken::wache();
        let geprueft = regelwerk(Some("compliance"));
        let anbieter = anbieter("custom", "http://cloud.example/v1");

        let ergebnis = send_chat_completion(
            Purpose::Chat,
            &anbieter,
            String::new(),
            "m",
            "Frage".into(),
            None,
            None,
        )
        .await;

        assert_eq!(ergebnis.unwrap_err(), "compliance verweigert");
        assert_eq!(testhaken::aufloesungen(), 0);
        assert_eq!(
            *geprueft.borrow(),
            ["compliance"],
            "Budget nie vor Regelwerk"
        );
    }

    #[tokio::test]
    async fn ein_budgetgesperrter_strom_loest_die_route_nicht_auf() {
        let _wache = testhaken::wache();
        let geprueft = regelwerk(Some("budget"));
        let anbieter = anbieter("custom", "http://cloud.example/v1");
        let ausgabe = std::sync::Mutex::new(String::new());
        let on_delta = |stueck: &str| ausgabe.lock().unwrap().push_str(stueck);

        let ergebnis = send_chat_completion_stream(
            Purpose::Chat,
            &anbieter,
            String::new(),
            "m",
            nachrichten(),
            &on_delta,
        )
        .await;

        assert_eq!(ergebnis.unwrap_err(), "budget verweigert");
        assert_eq!(testhaken::aufloesungen(), 0);
        assert_eq!(*geprueft.borrow(), ["compliance", "budget"]);
        assert!(ausgabe.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn auch_der_gebundene_lokale_strom_prueft_vor_allem_anderen() {
        let _wache = testhaken::wache();
        regelwerk(Some("compliance"));
        // Kein lokaler Anbieter: ohne Regelwerk käme "Kontext nur an das eingebaute lokale
        // Modell"; die Verweigerung des Regelwerks geht voraus, wie vor lva#604.
        let anbieter = anbieter("custom", "http://cloud.example/v1");

        let ergebnis = send_chat_completion_stream_routed(
            Purpose::Chat,
            &anbieter,
            String::new(),
            "m",
            nachrichten(),
            &|_: &str| {},
            Some("http://127.0.0.1:1/v1"),
        )
        .await;

        assert_eq!(ergebnis.unwrap_err(), "compliance verweigert");
        assert_eq!(testhaken::aufloesungen(), 0);
    }

    #[tokio::test]
    async fn nach_bestandener_vorpruefung_wird_aufgeloest_und_je_versuch_erneut_geprueft() {
        let _wache = testhaken::wache();
        let geprueft = regelwerk(None);
        let server = mock(|_| RohAntwort::json("Antwort", "stop", 3, 2)).await;
        let anbieter = anbieter("custom", &server.basis());

        let antwort = send_chat_completion_checked(
            Purpose::Chat,
            &anbieter,
            String::new(),
            "m",
            "Frage".into(),
            None,
            None,
            None,
            None,
        )
        .await
        .expect("Antwort");

        assert_eq!(antwort.content.as_deref(), Some("Antwort"));
        assert_eq!(testhaken::aufloesungen(), 1);
        assert_eq!(server.treffer(), 1);
        assert_eq!(
            *geprueft.borrow(),
            ["compliance", "budget", "compliance", "budget"],
            "Vorprüfung vor der Route, danach die Prüfung des Versuchs"
        );
    }

    #[test]
    fn das_cli_ziel_folgt_dem_programm() {
        assert_eq!(
            cli_ziel(Cli::Claude, "sonnet").cli_programm(),
            Some("claude")
        );
        assert_eq!(cli_ziel(Cli::Codex, "gpt").cli_programm(), Some("codex"));
        assert!(cli_ziel(Cli::Codex, "gpt").ist_cloud());
    }
}
