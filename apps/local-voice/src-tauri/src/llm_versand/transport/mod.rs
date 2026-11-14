//! Privater Rohtransport des Versandmoduls (lva#604, Teilpaket 3).
//!
//! Hierher ist der gesamte Weg ins Netz und zu den CLIs **verschoben** worden, der bis Teilpaket 2
//! in `llm_client.rs` und `managers/llm/cli.rs` verstreut lag:
//!
//! | Modul | Inhalt | früher |
//! |---|---|---|
//! | [`http`] | Chat am Stück, Strom, Strom-Rückfall, Client-Bau | `llm_client` (`send_inner`, `stream_inner`, `complete_once`, `create_client`, `create_pinned_local_client`) |
//! | [`cli_chat`] | ein CLI-Chatversuch (Optionen, Denkabgriff, Klassifikation) | `llm_client::send_cli` |
//! | [`cli`] | Prozessstart, Vorschauprüfung, Beobachter | `managers::llm::cli` (`call`, `call_with_timeout`, `call_with_options`, `call_inner`, `preview_precheck`, `verify_preview`) |
//! | [`bild`] | Bild und Prompt | `llm_client::post_image_prompt` |
//! | [`ollama`] | natives Ollama, Entladen | `llm_client::send_ollama_native`, `ollama_unload` |
//! | [`modelle`] | Modellliste | `llm_client::fetch_models` |
//! | [`json`] | rohes JSON-POST für Agent, Einbettungen, Tokenzählung | eigene Clients in `agent/runtime.rs`, `managers/meetings/search/embed.rs`, `managers/llm/mod.rs` (`tokenize_count`) |
//!
//! Regeln für dieses Modul (der Compiler erzwingt sie):
//!
//! - Alles hier ist höchstens so sichtbar wie `pub(super)` von hier aus, also nur für
//!   `llm_versand`. In den Untermodulen heißt das `pub(in crate::llm_versand)`: dort würde
//!   `pub(super)` nur bis `transport` reichen. Es gibt keine Reexporte, weder hier noch in
//!   `llm_versand`; die Untermodule sind privat.
//! - Jeder Transportaufruf nimmt die **besitzende** `Versandfreigabe` entgegen und löst sie
//!   gegen Ziel und Quellen ein (Zielwechsel bei gleicher Anbieter-ID wird abgewiesen). Er macht
//!   **genau einen** Versuch an das gebundene [`Ziel`](super::Ziel), ohne die Route neu aufzulösen.
//!   Fallbacks (Strom ohne Strom, CLI-Update) orchestriert `versenden`, jeder als neuer Versuch
//!   mit eigener Prüfung; der Transport meldet nur, dass einer **erlaubt** wäre ([`Fehlschlag`]).
//! - Die lokale Route (Ziel `Lokal`) läuft ohne Proxy, Cloud mit dem Umgebungsproxy wie bisher;
//!   **kein** Ziel folgt einem Redirect.
//! - Anbieter-Antworttext gelangt nie in einen Fehler oder ein Log: Fehlerantworten werden
//!   hier gelesen, **klassifiziert** (Kontextüberlauf, Status) und verworfen.

pub(super) mod bild;
mod cli;
pub(super) mod cli_chat;
pub(super) mod http;
pub(super) mod json;
pub(super) mod modelle;
pub(super) mod ollama;

use crate::llm_client::DenkSenke;
use crate::llm_versand::fehler::VersandFehler;
use crate::managers::usage::{Purpose, TokenUsage};
use crate::settings::PostProcessProvider;

/// Die CLI-Einstiege für die rohen Aufrufer ohne Besprechungsinhalt (Buch-Gespräch, Probelauf).
/// Die Implementierung bleibt privat in [`cli`]; hier stehen nur die Durchreichen.
pub(super) mod cli_roh {
    use std::path::Path;
    use std::time::Duration;

    use crate::llm_versand::fehler::VersandFehler;
    use crate::llm_versand::pruefer::Versandfreigabe;
    use crate::llm_versand::quellen::Quellen;
    use crate::llm_versand::ziel::Ziel;
    use crate::managers::llm::cli::{CallOptions, Cli, CliReply};

    /// Wie `cli::call_with_timeout`, gegen das gebundene Ziel. Blockiert.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::llm_versand) fn mit_zeitgrenze(
        freigabe: Versandfreigabe,
        ziel: &Ziel,
        quellen: &Quellen,
        cli: Cli,
        binary: &Path,
        system: Option<&str>,
        user: &str,
        timeout: Duration,
    ) -> Result<CliReply, String> {
        einloesen(freigabe, ziel, quellen)?;
        super::cli::call_with_timeout(cli, binary, ziel.modell(), system, user, timeout)
    }

    /// Wie `cli::call_with_options`, gegen das gebundene Ziel. Blockiert.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::llm_versand) fn mit_optionen(
        freigabe: Versandfreigabe,
        ziel: &Ziel,
        quellen: &Quellen,
        cli: Cli,
        binary: &Path,
        system: Option<&str>,
        user: &str,
        timeout: Duration,
        opts: &CallOptions,
    ) -> Result<CliReply, String> {
        einloesen(freigabe, ziel, quellen)?;
        super::cli::call_with_options(cli, binary, ziel.modell(), system, user, timeout, opts)
    }

    fn einloesen(freigabe: Versandfreigabe, ziel: &Ziel, quellen: &Quellen) -> Result<(), String> {
        freigabe
            .einloesen(ziel, quellen)
            .map_err(|fehler: VersandFehler| fehler.to_string())
    }
}

/// Was ein Versuch braucht außer Ziel, Quellen und Freigabe.
pub(in crate::llm_versand) struct Kontext<'a> {
    /// Anbieter: Kopfzeilen, Ollama-Erkennung, „lokal“ für Sampling und Denkschalter.
    pub(in crate::llm_versand) anbieter: &'a PostProcessProvider,
    /// Zugangsschlüssel.
    pub(in crate::llm_versand) api_key: &'a str,
    /// Zweck (deterministisches Sampling nur für bestimmte Zwecke).
    pub(in crate::llm_versand) zweck: Purpose,
}

/// Wohin die Inhaltsausgaben eines Versuchs gehen. `versenden` setzt hier entweder die echten
/// Empfänger (direkt) oder Puffer (Cloud mit Besprechungsquellen); der Transport weiß nichts
/// davon.
pub(in crate::llm_versand) struct Ausgabe<'a> {
    /// Textstücke der Antwort.
    pub(in crate::llm_versand) on_delta: &'a (dyn Fn(&str) + Send + Sync),
    /// Denktexte des Modells (Anzeige; nie gesprochen, nie Teil der Antwort).
    pub(in crate::llm_versand) senke: Option<DenkSenke>,
}

/// Warum ein Versuch fehlschlug, und was danach **erlaubt** wäre. Erlaubt heißt nicht getan:
/// jeder Folgeversuch braucht eine neue Prüfung, und `Gesperrt` verbietet ihn ganz.
#[derive(Debug)]
pub(in crate::llm_versand) enum Fehlschlag {
    /// Endgültig. Klassifiziert, ohne Anbietertext.
    Fehler(VersandFehler),
    /// Der Strom wurde abgelehnt oder riss vor dem ersten Token ab: ein neuer Versuch ohne
    /// Strom ist erlaubt (früher `complete_once`).
    OhneStromNeu,
    /// Die CLI ist zu alt (Codex): nach einem Update ist ein neuer Versuch erlaubt (früher
    /// `on_cli_error` und `ensure_codex_updated` in `send_cli`).
    CliVeraltet,
}

impl From<VersandFehler> for Fehlschlag {
    fn from(fehler: VersandFehler) -> Self {
        Self::Fehler(fehler)
    }
}

impl Fehlschlag {
    /// Der Fehler, wenn **kein** Folgeversuch stattfindet (er ist nicht erlaubt, nicht möglich
    /// oder der Fallback hat selbst versagt). Die beiden Rückfallarten haben dann keinen
    /// eigenen Text: ein abgelehnter Strom ohne möglichen Rückfall ist eine unbrauchbare Antwort,
    /// eine zu alte CLI ohne Update bleibt `CliUpdate`.
    pub(in crate::llm_versand) fn endgueltig(self) -> VersandFehler {
        match self {
            Self::Fehler(fehler) => fehler,
            Self::OhneStromNeu => VersandFehler::Protokoll,
            Self::CliVeraltet => VersandFehler::CliUpdate,
        }
    }
}

/// Ergebnis eines Versuchs: der Wert samt Verbrauch.
pub(in crate::llm_versand) type Versuch<T> = Result<(T, TokenUsage), Fehlschlag>;

#[cfg(test)]
mod netz_tests;
