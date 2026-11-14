//! Ein CLI-Chatversuch (Claude Code, Codex) für `versenden` (lva#604, Teilpaket 3).
//!
//! Früher `llm_client::send_cli`. Hier steht nur **ein** Versuch: Binärdatei suchen, Optionen aus
//! den Aufruf-Kontexten (`CLI_TIMEOUT`, `CLI_CANCEL`, `CLI_MAX_TOKENS`, `CLI_REQUIRE_USAGE`,
//! `CLI_PREVIEW`, `CLI_PREVIEW_SWITCH`) lesen, den Prozess auf einem Blocking-Thread starten
//! (siehe [`cli`](super::cli)) und das Ergebnis klassifizieren. Das Aktualisieren einer zu
//! alten Codex-CLI und der Neuversuch danach liegen **außerhalb** (`versenden`): dieser Versuch
//! meldet nur [`Fehlschlag::CliVeraltet`].
//!
//! Der Fehlertext der CLI (`cli_failed: <letzte stderr-Zeile>`) ist Fremdtext. [`klassifiziere`]
//! macht daraus einen [`VersandFehler`] aus festen Codes und höchstens einem festen Hinweis;
//! der Text selbst geht weder in den Fehler noch ins Log noch in die Ursachenmerkung.

use serde_json::Value;

use super::cli as prozess;
use super::{Fehlschlag, Versuch};
use crate::llm_client::{
    denksenke, ChatReply, CliMeta, DenkSenke, CLI_CANCEL, CLI_MAX_TOKENS, CLI_META, CLI_PREVIEW,
    CLI_PREVIEW_SWITCH, CLI_REQUIRE_USAGE, CLI_TIMEOUT,
};
use crate::llm_versand::fehler::{cli_code, VersandFehler};
use crate::llm_versand::pruefer::Versandfreigabe;
use crate::llm_versand::quellen::Quellen;
use crate::llm_versand::ziel::Ziel;
use crate::managers::llm::cli::{self, CallOptions, Cli, CliReply, Zeilenabgriff};
use crate::managers::llm::{cli_update, preview};
use crate::managers::usage::TokenUsage;

/// Die CLI zum Ziel (`cli://claude`, `cli://codex`); `None` bei HTTP-Zielen und unbekannten
/// Programmen.
pub(in crate::llm_versand) fn cli_des_ziels(ziel: &Ziel) -> Option<Cli> {
    ziel.cli_programm()
        .and_then(|programm| Cli::from_base_url(&format!("cli://{programm}")))
}

/// Klassifiziert den Fehlertext einer CLI. Reine Funktion, ohne Text im Ergebnis.
pub(in crate::llm_versand) fn klassifiziere(which: Cli, fehler: &str) -> VersandFehler {
    if cli::is_cancelled(fehler) {
        return VersandFehler::Abgebrochen;
    }
    if let Some(grund) = preview::rejection_code(fehler) {
        return VersandFehler::Cli {
            code: match grund {
                "budget" => cli_code::VORSCHAU_BUDGET,
                "cancelled" => cli_code::VORSCHAU_ABGEBROCHEN,
                _ => cli_code::VORSCHAU_ABGELEHNT,
            },
            hinweis: None,
        };
    }
    if cli_update::on_cli_error(which, fehler) {
        return VersandFehler::CliUpdate;
    }
    let code = [
        cli_code::NICHT_ANGEMELDET,
        cli_code::LIMIT,
        cli_code::FEHLT,
        cli_code::AUSGABE,
        cli_code::MODELL,
        cli_code::ZEIT,
        cli_code::FEHLGESCHLAGEN,
    ]
    .into_iter()
    .find(|code| fehler.starts_with(code))
    .unwrap_or(cli_code::FEHLGESCHLAGEN);
    let klein = fehler.to_ascii_lowercase();
    let hinweis = cli_code::HINWEISE
        .into_iter()
        .find(|hinweis| klein.contains(hinweis));
    VersandFehler::Cli { code, hinweis }
}

/// ```json … ``` um eine JSON-Antwort entfernen.
fn strip_code_fence(text: &str) -> String {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t.to_string();
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    rest.strip_suffix("```").unwrap_or(rest).trim().to_string()
}

/// Wie bisher `llm_client::call_cli`: Zeitgrenze und Optionen aus den Aufruf-Kontexten. Ohne
/// Zeitgrenze, Abbruchsignal, Ausgabegrenze und Vorschau läuft der Aufruf mit den
/// Standardoptionen (früher `cli::call`); das ist vorbestehendes Verhalten und bleibt so.
fn call_cli(
    which: Cli,
    model: &str,
    binary: &std::path::Path,
    system: Option<&str>,
    user: &str,
    limit: Option<std::time::Duration>,
    opts: &CallOptions,
) -> Result<CliReply, String> {
    let standard = CallOptions::default();
    let (timeout, opts) = match limit {
        Some(t) => (t, opts),
        None if opts.cancel.is_some() || opts.max_tokens.is_some() || opts.preview.is_some() => {
            (cli::CALL_TIMEOUT, opts)
        }
        None => (cli::CALL_TIMEOUT, &standard),
    };
    prozess::call_with_options(which, binary, model, system, user, timeout, opts)
}

/// Ein CLI-Chatversuch. Ein Schema wandert in den Systemtext (die CLIs kennen keine
/// strukturierte Ausgabe); ein Codeblock um das JSON wird entfernt.
///
/// `senke`: Empfänger für Denkschritte der Codex-CLI (lva#485), schon in der Form, die
/// `versenden` gewählt hat (direkt oder gepuffert).
#[allow(clippy::too_many_arguments)]
pub(in crate::llm_versand) async fn einmal(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    which: Cli,
    user_content: &str,
    system_prompt: Option<&str>,
    json_schema: Option<&Value>,
    senke: Option<DenkSenke>,
) -> Versuch<ChatReply> {
    let binary = cli::locate(which).ok_or_else(|| {
        // Die nächste Modellauflösung sucht neu und weicht auf den Ersatz aus.
        cli::forget_installed();
        Fehlschlag::Fehler(VersandFehler::Cli {
            code: cli_code::FEHLT,
            hinweis: None,
        })
    })?;
    let wants_json = json_schema.is_some();
    let system = match json_schema {
        Some(schema) => Some(format!(
            "{}\n\nAntworte ausschliesslich mit einem JSON-Objekt nach diesem JSON-Schema, \
             ohne Erklaerung und ohne Codeblock:\n{}",
            system_prompt.unwrap_or_default(),
            schema
        )),
        None => system_prompt.map(str::to_string),
    };
    let limit = CLI_TIMEOUT.try_with(|t| *t).ok();
    let opts = CallOptions {
        cancel: CLI_CANCEL.try_with(|c| c.clone()).ok(),
        max_tokens: CLI_MAX_TOKENS.try_with(|n| *n).ok(),
        require_usage: CLI_REQUIRE_USAGE.try_with(|b| *b).unwrap_or(false),
        preview: CLI_PREVIEW.try_with(|p| p.clone()).ok(),
        preview_switch: CLI_PREVIEW_SWITCH.try_with(|p| p.clone()).ok(),
        // Codex meldet Denkschritte als eigene JSONL-Zeilen: sofort weiterreichen (an den
        // Empfänger, den `versenden` gewählt hat).
        stdout_zeile: senke.filter(|_| which == Cli::Codex).map(|senke| {
            Zeilenabgriff(std::sync::Arc::new(move |zeile: &str| {
                if let Some(gedanke) = cli::codex_gedanke(zeile) {
                    senke(&gedanke);
                }
            }))
        }),
    };
    let preview_on = opts.preview.is_some();
    // Die Freigabe wird hier besitzend verbraucht: ein Versuch, ein Prozess.
    freigabe.einloesen(ziel, quellen)?;
    let model = ziel.modell().to_string();
    let aufruf = {
        let (binary, system, user, opts) = (
            binary.clone(),
            system.clone(),
            user_content.to_string(),
            opts.clone(),
        );
        tokio::task::spawn_blocking(move || {
            call_cli(
                which,
                &model,
                &binary,
                system.as_deref(),
                &user,
                limit,
                &opts,
            )
        })
        .await
    };
    let ergebnis = match aufruf {
        Ok(ergebnis) => ergebnis,
        Err(_) => {
            return Err(Fehlschlag::Fehler(VersandFehler::Cli {
                code: cli_code::FEHLGESCHLAGEN,
                hinweis: None,
            }))
        }
    };
    let reply = match ergebnis {
        Ok(reply) => reply,
        Err(e) => {
            // Nach einem Abbruch weder merken noch aktualisieren noch wiederholen.
            let aborted = cli::is_cancelled(&e)
                || opts
                    .cancel
                    .as_ref()
                    .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst));
            let fehler = klassifiziere(which, &e);
            if aborted {
                return Err(Fehlschlag::Fehler(VersandFehler::Abgebrochen));
            }
            // Ursache merken (für die Fehlermeldung des Protokolls): nur die app-eigene Fassung,
            // nie der Text der CLI.
            cli_update::remember_error(&ursache(&fehler));
            // Vorschau: nie aktualisieren und wiederholen. Ein Update änderte Version und
            // Programm und machte die Freigabe ungültig; die Antwort wäre nie verwendbar.
            if opts.preview.is_some() {
                return Err(Fehlschlag::Fehler(fehler));
            }
            // Ist die CLI zu alt, aktualisiert `versenden` sie und versucht es als neuen Versuch.
            if fehler == VersandFehler::CliUpdate {
                return Err(Fehlschlag::CliVeraltet);
            }
            return Err(Fehlschlag::Fehler(fehler));
        }
    };
    // Vorschau: bis hierher kann der Schalter ausgeschaltet worden sein; dann geht der Text
    // weder an die Oberfläche noch an die Sprache.
    if preview_on && !CLI_PREVIEW_SWITCH.try_with(|s| s.is_on()).unwrap_or(false) {
        return Err(Fehlschlag::Fehler(VersandFehler::Cli {
            code: cli_code::VORSCHAU_ABGEBROCHEN,
            hinweis: None,
        }));
    }
    let _ = CLI_META.try_with(|m| {
        if let Ok(mut m) = m.lock() {
            *m = CliMeta {
                model: reply.model.clone(),
                prompt_tokens: reply.prompt_tokens,
                completion_tokens: reply.completion_tokens,
            };
        }
    });
    let text = if wants_json {
        strip_code_fence(&reply.text)
    } else {
        reply.text
    };
    Ok((
        ChatReply {
            content: Some(text),
            truncated: false,
        },
        TokenUsage {
            prompt_tokens: reply.prompt_tokens,
            completion_tokens: reply.completion_tokens,
        },
    ))
}

/// Die Ursache eines CLI-Fehlers als Zusatz zu einer Fehlermeldung: app-eigener Text, mit dem
/// Code vorn (`recent_error_note` zeigt den Teil nach dem ersten „: “).
fn ursache(fehler: &VersandFehler) -> String {
    fehler.alte_textform()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn die_cli_zum_ziel_kommt_aus_dem_programm() {
        assert_eq!(
            cli_des_ziels(&Ziel::cli("claude_cli", "claude", "sonnet")),
            Some(Cli::Claude)
        );
        assert_eq!(
            cli_des_ziels(&Ziel::cli("codex_cli", "codex", "gpt")),
            Some(Cli::Codex)
        );
        assert_eq!(cli_des_ziels(&Ziel::cli("x", "unbekannt", "m")), None);
        assert_eq!(
            cli_des_ziels(&Ziel::http("openai", "https://api.example.com/v1", "m")),
            None
        );
    }

    #[test]
    fn cli_fehler_werden_ohne_text_klassifiziert() {
        let k = |cli, text: &str| klassifiziere(cli, text);
        assert_eq!(
            k(Cli::Claude, "cli_cancelled: GEHEIM"),
            VersandFehler::Abgebrochen
        );
        assert_eq!(
            k(Cli::Codex, "cli_model_not_in_plan: GEHEIM"),
            VersandFehler::CliUpdate
        );
        // Claude kennt kein Update: der Code bleibt.
        assert_eq!(
            k(Cli::Claude, "cli_model_not_in_plan: GEHEIM"),
            VersandFehler::Cli {
                code: cli_code::MODELL,
                hinweis: None
            }
        );
        assert_eq!(
            k(Cli::Claude, "cli_not_logged_in: GEHEIM"),
            VersandFehler::Cli {
                code: cli_code::NICHT_ANGEMELDET,
                hinweis: None
            }
        );
        assert_eq!(
            k(
                Cli::Claude,
                "cli_failed: Selected model is at capacity GEHEIM"
            ),
            VersandFehler::Cli {
                code: cli_code::FEHLGESCHLAGEN,
                hinweis: Some("at capacity")
            }
        );
        assert_eq!(
            k(Cli::Claude, "cli_preview_rejected:budget:budget_precheck"),
            VersandFehler::Cli {
                code: cli_code::VORSCHAU_BUDGET,
                hinweis: None
            }
        );
        // Unbekanntes wird zu `cli_failed`, nie durchgereicht.
        let unbekannt = k(Cli::Claude, "irgendwas GEHEIM");
        assert!(!unbekannt.to_string().contains("GEHEIM"), "{unbekannt}");
        for text in ["cli_failed: GEHEIM", "cli_limit_reached: GEHEIM"] {
            assert!(!k(Cli::Codex, text).to_string().contains("GEHEIM"));
        }
    }

    #[test]
    fn ein_codeblock_um_das_json_wird_entfernt() {
        assert_eq!(strip_code_fence("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_code_fence("{\"a\":1}"), "{\"a\":1}");
    }
}
