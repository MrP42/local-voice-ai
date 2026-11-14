//! Fehlervertrag des Versandmoduls (lva#604, Teilpaket 1).
//!
//! [`VersandFehler`] behält die Fehlerarten, die Aufrufer heute am Fehlertext erkennen
//! (Kontextüberlauf, ungültiges JSON, RAM, Absturz, Abbruch, CLI-Update, HTTP, Transport,
//! Zeit, Budget, Compliance) und ergänzt das terminale [`VersandFehler::Gesperrt`].
//!
//! Regeln, die der Typ erzwingt:
//! - **Kein Anbietertext, kein Inhalt.** Die Varianten tragen nur Codes, Zahlen und
//!   app-eigene Meldungen (Budget, Compliance); `grund` ist eine `&'static str`-Konstante aus
//!   [`grund`], nie ein Freitext. Rohtexte kommen nie ins Log, nur Codes und Längen.
//! - **`Gesperrt` ist terminal** ([`VersandFehler::is_terminal`]): nie wiederholen, nie in
//!   Chunks zerlegen, nie reparieren, kein Fallback. Ein Aufrufer, der einen Fehler
//!   weiterreicht, reicht ihn als `Gesperrt` weiter und leitet daraus nichts anderes ab.
//! - **Ein kanonischer externer Code**: [`CODE_BESPRECHUNG_CLOUD_GESPERRT`]. Die äußere
//!   Textform ist `besprechung_cloud_gesperrt:<grund>` (`Display`), wie bei
//!   `server_crashed:<…>` in `managers::llm`.

use std::fmt;

/// Kanonischer Code der Besprechungs-Cloud-Sperre. Das ist der einzige externe Code für
/// „diese Besprechungsinhalte dürfen nicht an dieses Cloud-Ziel“; `gesperrt:<grund>` o. ä.
/// gibt es nicht.
pub const CODE_BESPRECHUNG_CLOUD_GESPERRT: &str = "besprechung_cloud_gesperrt";

/// Gründe für [`VersandFehler::Gesperrt`]. Feste Kennungen, keine Inhalte.
pub mod grund {
    /// Kundenprofil „nur lokal“ (`KiSperre::ganz_gesperrt`).
    pub const NUR_LOKAL: &str = "nur_lokal";
    /// Eine Quelle ist für KI-Clients nicht `sichtbar()`.
    pub const NICHT_SICHTBAR: &str = "nicht_sichtbar";
    /// Die Herkunft der Inhalte ist unbestimmt (`Quellen::unbestimmt`).
    pub const UNBESTIMMT: &str = "unbestimmt";
    /// Ein Lesefehler bei Richtlinie, Spiegel, Revision oder Quelle. Jeder Zweifel sperrt.
    pub const LESEFEHLER: &str = "lesefehler";
    /// Der Richtlinienspiegel in der Datenbank fehlt.
    pub const SPIEGEL_FEHLT: &str = "spiegel_fehlt";
    /// Die Richtlinie weicht vom Spiegel ab (eine Änderung ist unterwegs).
    pub const SPIEGEL_ABWEICHEND: &str = "spiegel_abweichend";
    /// Revision oder Richtlinie haben sich zwischen zwei Lesungen geändert (R1≠R2 nach
    /// erschöpften Prüfversuchen).
    pub const REVISION_GEWECHSELT: &str = "revision_gewechselt";
    /// Die Prüfung blieb nach der erlaubten Zahl an Versuchen ohne stabilen Stand.
    pub const PRUEFVERSUCHE_ERSCHOEPFT: &str = "pruefversuche_erschoepft";
    /// Abschlussvergleich: nach dem Linearisierungspunkt (R2) hat sich die Freigabegrundlage
    /// geändert (R3≠R2). Das Ergebnis wird verworfen.
    pub const WIDERRUF_NACH_START: &str = "widerruf_nach_start";
    /// Die Freigabe war für ein anderes Ziel ausgestellt als das, an das gesendet wird.
    pub const ZIEL_GEWECHSELT: &str = "ziel_gewechselt";
    /// Die Freigabe war für andere Quellen ausgestellt als die, die gesendet werden.
    pub const QUELLEN_GEWECHSELT: &str = "quellen_gewechselt";
}

/// Feste Codes der CLI-Fehler ([`VersandFehler::Cli`]). Das sind die Präfixe, an denen
/// Aufrufer die Fehlerart heute erkennen (`managers::llm::cli::classify_error`).
pub mod cli_code {
    /// Nicht angemeldet.
    pub const NICHT_ANGEMELDET: &str = "cli_not_logged_in";
    /// Nutzungsgrenze erreicht.
    pub const LIMIT: &str = "cli_limit_reached";
    /// Sonstiger Fehler der CLI.
    pub const FEHLGESCHLAGEN: &str = "cli_failed";
    /// CLI nicht installiert oder nicht startbar.
    pub const FEHLT: &str = "cli_missing";
    /// Antwort ohne gültige Nutzungsangabe oder nicht lesbar.
    pub const AUSGABE: &str = "cli_bad_output";
    /// Modell nicht angenommen (bei Codex als [`super::VersandFehler::CliUpdate`]).
    pub const MODELL: &str = "cli_model_not_in_plan";
    /// Zeitgrenze der CLI.
    pub const ZEIT: &str = "cli_timeout";
    /// Vorschau abgelehnt: Budget.
    pub const VORSCHAU_BUDGET: &str = "cli_preview_rejected:budget";
    /// Vorschau abgelehnt: Schalter aus oder abgebrochen.
    pub const VORSCHAU_ABGEBROCHEN: &str = "cli_preview_rejected:cancelled";
    /// Vorschau abgelehnt: sonst.
    pub const VORSCHAU_ABGELEHNT: &str = "cli_preview_rejected:llm_failed";

    /// Feste Hinweise für vorübergehende Fehler. Aufrufer wiederholen bei diesen Wörtern
    /// (`translator::is_transient_error`); die Liste ist geschlossen und app-eigen.
    pub const HINWEISE: [&str; 8] = [
        "at capacity",
        "overloaded",
        "rate limit",
        "rate_limit",
        "429",
        "503",
        "try again",
        "temporarily",
    ];
}

/// Alle Fehler, die ein Versand liefern kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersandFehler {
    /// Der Prompt passt nicht in den Kontext oder die Antwort wurde abgeschnitten. Dieselbe
    /// Anfrage scheitert wieder, eine kleinere nicht. Die Diagnose in
    /// `managers::meetings::llm_call` (`is_truncation_error`) hängt daran; `Http { status: 400 }`
    /// ersetzt sie nicht.
    Kontextueberlauf,
    /// Die Antwort ist kein gültiges JSON (bei verlangtem Schema).
    UngueltigesJson,
    /// Das lokale Modell passt nicht in den freien Speicher (RAM-Start-Gate).
    ZuWenigRam,
    /// Der lokale Server oder die CLI ist während des Aufrufs abgestürzt.
    Absturz,
    /// Der Aufruf wurde abgebrochen (Nutzer oder App).
    Abgebrochen,
    /// Die CLI ist zu alt und muss aktualisiert werden.
    CliUpdate,
    /// Die CLI meldete einen Fehler. Nur ihr app-eigener Code (und höchstens ein fester
    /// Hinweis aus einer geschlossenen Liste), **nie ihr Text**: der Text der CLI ist Fremdtext
    /// und wird im Transport klassifiziert und dann verworfen.
    Cli {
        /// Feste Kennung aus [`cli_code`].
        code: &'static str,
        /// Fester app-eigener Hinweis aus [`cli_code::HINWEISE`] (etwa „at capacity“), den
        /// Aufrufer für „vorübergehend, später erneut“ brauchen; sonst `None`.
        hinweis: Option<&'static str>,
    },
    /// Der Anbieter antwortete mit diesem HTTP-Status. Nie mit seinem Antworttext.
    Http { status: u16 },
    /// Die Verbindung kam nicht zustande oder riss beim Senden ab. Ohne Anbietertext.
    Transport,
    /// Die Verbindung stand, die Antwort war aber nicht verwertbar: nicht lesbar, Strom
    /// abgebrochen oder ohne Abschluss, Zeile oder Antwort zu lang, Fehlerereignis im Strom.
    /// Ohne Anbietertext. (Getrennt von `Transport`, weil Aufrufer „kein Netz“ von „Modell
    /// lieferte Unbrauchbares“ unterscheiden: `managers::meetings::chat::controller`.)
    Protokoll,
    /// Die Zeit für den Aufruf ist abgelaufen.
    Zeitueberschreitung,
    /// Das harte Monatsbudget ist erreicht (app-eigene Meldung, kein Anbietertext).
    Budget { meldung: String },
    /// Das Regelwerk (Compliance-Profil) sperrt das Ziel (app-eigene Meldung).
    Compliance { meldung: String },
    /// Terminal: die Besprechungsinhalte dürfen nicht an dieses Cloud-Ziel. Wird nie
    /// wiederholt, nie in Chunks zerlegt, nie repariert und löst keinen Fallback aus.
    Gesperrt {
        /// Immer [`CODE_BESPRECHUNG_CLOUD_GESPERRT`].
        code: &'static str,
        /// Eine Konstante aus [`grund`].
        grund: &'static str,
    },
}

impl VersandFehler {
    /// Die Besprechungs-Cloud-Sperre mit dem kanonischen Code und einem Grund aus [`grund`].
    pub fn gesperrt(grund: &'static str) -> Self {
        Self::Gesperrt {
            code: CODE_BESPRECHUNG_CLOUD_GESPERRT,
            grund,
        }
    }

    /// Terminal heißt: kein Wiederholen, kein Zerlegen, kein Reparieren, kein Fallback.
    /// Das trifft ausschließlich `Gesperrt`. (Ein Abbruch ist ebenfalls endgültig, aber ein
    /// Nutzerwunsch und kein Sperrurteil; ihn erkennen die Aufrufer wie bisher.)
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Gesperrt { .. })
    }

    /// Lohnt ein erneuter Aufruf mit derselben Anfrage (neuer Versuch mit eigener Prüfung)?
    ///
    /// Ja bei Fehlern, die der Anbieter beim nächsten Mal nicht mehr haben muss: unbrauchbare
    /// Antwort oder ungültiges JSON, Verbindung, Zeit, `408`/`429`/`5xx` und ein vorübergehender
    /// CLI-Fehler (fester Hinweis). **Nie** bei [`Gesperrt`](Self::Gesperrt) (terminal), Abbruch,
    /// Budget, Compliance, Kontextüberlauf (kleinerer Prompt statt derselbe), Speichermangel,
    /// Absturz und CLI-Update (das erledigt `versenden` selbst als eigenen Versuch). Der `match`
    /// ist erschöpfend: eine neue Variante verlangt eine Entscheidung.
    pub fn ist_wiederholbar(&self) -> bool {
        match self {
            Self::UngueltigesJson
            | Self::Protokoll
            | Self::Transport
            | Self::Zeitueberschreitung => true,
            Self::Http { status } => matches!(*status, 408 | 429) || *status >= 500,
            Self::Cli {
                hinweis: Some(_), ..
            } => true,
            Self::Cli { hinweis: None, .. }
            | Self::Kontextueberlauf
            | Self::ZuWenigRam
            | Self::Absturz
            | Self::Abgebrochen
            | Self::CliUpdate
            | Self::Budget { .. }
            | Self::Compliance { .. }
            | Self::Gesperrt { .. } => false,
        }
    }

    /// Stabile Kennung der Fehlerart ohne Zahlen und Meldungen; für Zähler und Logzeilen.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Kontextueberlauf => "kontextueberlauf",
            Self::UngueltigesJson => "ungueltiges_json",
            Self::ZuWenigRam => "zu_wenig_ram",
            Self::Absturz => "absturz",
            Self::Abgebrochen => "abgebrochen",
            Self::CliUpdate => "cli_update",
            Self::Cli { .. } => "cli",
            Self::Http { .. } => "http",
            Self::Transport => "transport",
            Self::Protokoll => "protokoll",
            Self::Zeitueberschreitung => "zeitueberschreitung",
            Self::Budget { .. } => "budget",
            Self::Compliance { .. } => "compliance",
            Self::Gesperrt { code, .. } => code,
        }
    }
}

impl VersandFehler {
    /// **Übergang bis lva#601:** die Textform, an der die bestehenden Aufrufer Fehlerarten
    /// erkennen (Präfixe und Marken wie `HTTP request failed`, `API request failed with status`,
    /// `exceed_context_size`, `cli_not_logged_in:`, `Zeitlimit`). Sie ist **app-eigen**: nie
    /// Text eines Anbieters oder einer CLI, nur feste Wörter und Zahlen. Die Übergangsfassade
    /// und das Verbrauchs-Ledger benutzen sie; mit #601 lesen die Aufrufer den Typ.
    pub(in crate::llm_versand) fn alte_textform(&self) -> String {
        match self {
            Self::Kontextueberlauf => "API request failed with status 400 Bad Request \
                (exceed_context_size_error: context length exceeded)"
                .to_string(),
            Self::UngueltigesJson => "Antwort kein gültiges JSON".to_string(),
            Self::ZuWenigRam => "Zu wenig freier Arbeitsspeicher für das lokale Modell".to_string(),
            Self::Absturz => "server_crashed: Das lokale Sprachmodell ist abgestürzt.".to_string(),
            Self::Abgebrochen => "cli_cancelled: Der Aufruf wurde abgebrochen".to_string(),
            Self::CliUpdate => "cli_model_not_in_plan: Dieses Modell nimmt der Server von der \
                installierten Codex-CLI nicht an. Meist hilft ein Update: npm i -g \
                @openai/codex@latest, oder den offiziellen Installer erneut ausfuehren"
                .to_string(),
            Self::Cli { code, hinweis } => {
                let text = match *code {
                    cli_code::NICHT_ANGEMELDET => "Bei der CLI ist keine Anmeldung vorhanden",
                    cli_code::LIMIT => "Die Nutzungsgrenze der CLI ist erreicht",
                    cli_code::FEHLT => "Die CLI des Anbieters ist nicht installiert",
                    cli_code::AUSGABE => "Die Antwort der CLI war nicht verwertbar",
                    cli_code::MODELL => "Dieses Modell nimmt die CLI nicht an",
                    cli_code::ZEIT => "Zeitlimit der CLI überschritten",
                    c if c.starts_with("cli_preview_rejected:") => "siehe_audit",
                    _ => "Die CLI meldete einen Fehler",
                };
                match hinweis {
                    Some(hinweis) => format!("{code}: {text} ({hinweis})"),
                    None => format!("{code}: {text}"),
                }
            }
            Self::Http { status } => {
                let status = reqwest::StatusCode::from_u16(*status)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| status.to_string());
                format!("API request failed with status {status}")
            }
            Self::Transport => "HTTP request failed: keine Verbindung".to_string(),
            Self::Protokoll => "Antwort des Anbieters nicht verwertbar".to_string(),
            Self::Zeitueberschreitung => "Zeitlimit überschritten".to_string(),
            Self::Budget { meldung } | Self::Compliance { meldung } => meldung.clone(),
            Self::Gesperrt { code, grund } => format!("{code}:{grund}"),
        }
    }
}

impl fmt::Display for VersandFehler {
    /// Äußere Textform: nur Codes, Zahlen und app-eigene Meldungen; nie Anbieter- oder
    /// Besprechungstext.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { status } => write!(f, "http:{status}"),
            Self::Budget { meldung } | Self::Compliance { meldung } => f.write_str(meldung),
            Self::Gesperrt { code, grund } => write!(f, "{code}:{grund}"),
            Self::Cli {
                code,
                hinweis: Some(hinweis),
            } => write!(f, "{code}:{hinweis}"),
            Self::Cli {
                code,
                hinweis: None,
            } => f.write_str(code),
            sonst => f.write_str(sonst.code()),
        }
    }
}

impl std::error::Error for VersandFehler {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eine Instanz je Variante. Der `match` unten ist erschöpfend: kommt eine Variante hinzu,
    /// bricht diese Datei, bis `alle()`, `is_terminal` und `code` bedacht sind.
    fn alle() -> Vec<VersandFehler> {
        let liste = vec![
            VersandFehler::Kontextueberlauf,
            VersandFehler::UngueltigesJson,
            VersandFehler::ZuWenigRam,
            VersandFehler::Absturz,
            VersandFehler::Abgebrochen,
            VersandFehler::CliUpdate,
            VersandFehler::Cli {
                code: cli_code::FEHLGESCHLAGEN,
                hinweis: None,
            },
            VersandFehler::Http { status: 400 },
            VersandFehler::Transport,
            VersandFehler::Protokoll,
            VersandFehler::Zeitueberschreitung,
            VersandFehler::Budget {
                meldung: "Budget erreicht".into(),
            },
            VersandFehler::Compliance {
                meldung: "Regelwerk sperrt".into(),
            },
            VersandFehler::gesperrt(grund::NUR_LOKAL),
        ];
        for f in &liste {
            match f {
                VersandFehler::Kontextueberlauf
                | VersandFehler::UngueltigesJson
                | VersandFehler::ZuWenigRam
                | VersandFehler::Absturz
                | VersandFehler::Abgebrochen
                | VersandFehler::CliUpdate
                | VersandFehler::Cli { .. }
                | VersandFehler::Http { .. }
                | VersandFehler::Transport
                | VersandFehler::Protokoll
                | VersandFehler::Zeitueberschreitung
                | VersandFehler::Budget { .. }
                | VersandFehler::Compliance { .. }
                | VersandFehler::Gesperrt { .. } => {}
            }
        }
        liste
    }

    #[test]
    fn nur_gesperrt_ist_terminal() {
        let terminal: Vec<_> = alle().into_iter().filter(|f| f.is_terminal()).collect();
        assert_eq!(terminal, vec![VersandFehler::gesperrt(grund::NUR_LOKAL)]);
    }

    #[test]
    fn gesperrt_traegt_den_kanonischen_code() {
        assert_eq!(
            CODE_BESPRECHUNG_CLOUD_GESPERRT,
            "besprechung_cloud_gesperrt"
        );
        for g in [
            grund::NUR_LOKAL,
            grund::NICHT_SICHTBAR,
            grund::UNBESTIMMT,
            grund::LESEFEHLER,
            grund::SPIEGEL_FEHLT,
            grund::SPIEGEL_ABWEICHEND,
            grund::REVISION_GEWECHSELT,
            grund::PRUEFVERSUCHE_ERSCHOEPFT,
            grund::WIDERRUF_NACH_START,
            grund::ZIEL_GEWECHSELT,
            grund::QUELLEN_GEWECHSELT,
        ] {
            let VersandFehler::Gesperrt { code, grund } = VersandFehler::gesperrt(g) else {
                panic!("gesperrt() muss Gesperrt liefern");
            };
            assert_eq!(code, CODE_BESPRECHUNG_CLOUD_GESPERRT);
            assert_eq!(grund, g);
        }
    }

    #[test]
    fn codes_sind_eindeutig_und_ohne_leerzeichen() {
        let codes: Vec<_> = alle().iter().map(VersandFehler::code).collect();
        let mut einmalig = codes.clone();
        einmalig.sort_unstable();
        einmalig.dedup();
        assert_eq!(einmalig.len(), codes.len(), "doppelter Code: {codes:?}");
        assert!(codes.iter().all(|c| !c.is_empty() && !c.contains(' ')));
    }

    #[test]
    fn textform_enthaelt_nur_codes_zahlen_und_app_meldungen() {
        assert_eq!(
            VersandFehler::gesperrt(grund::NUR_LOKAL).to_string(),
            "besprechung_cloud_gesperrt:nur_lokal"
        );
        assert_eq!(VersandFehler::Http { status: 502 }.to_string(), "http:502");
        assert_eq!(
            VersandFehler::Kontextueberlauf.to_string(),
            "kontextueberlauf"
        );
        assert_eq!(
            VersandFehler::Budget {
                meldung: "Budget erreicht".into()
            }
            .to_string(),
            "Budget erreicht"
        );
    }

    #[test]
    fn wiederholbar_ist_nie_terminal_und_nie_budget_oder_compliance() {
        for fehler in alle() {
            if fehler.is_terminal() {
                assert!(!fehler.ist_wiederholbar(), "{fehler}");
            }
            if matches!(
                fehler,
                VersandFehler::Budget { .. }
                    | VersandFehler::Compliance { .. }
                    | VersandFehler::Abgebrochen
                    | VersandFehler::Kontextueberlauf
            ) {
                assert!(!fehler.ist_wiederholbar(), "{fehler}");
            }
        }
        for status in [408, 429, 500, 503] {
            assert!(
                VersandFehler::Http { status }.ist_wiederholbar(),
                "{status}"
            );
        }
        for status in [400, 401, 404] {
            assert!(
                !VersandFehler::Http { status }.ist_wiederholbar(),
                "{status}"
            );
        }
        assert!(VersandFehler::UngueltigesJson.ist_wiederholbar());
        assert!(VersandFehler::Cli {
            code: cli_code::FEHLGESCHLAGEN,
            hinweis: Some("at capacity")
        }
        .ist_wiederholbar());
        assert!(!VersandFehler::Cli {
            code: cli_code::NICHT_ANGEMELDET,
            hinweis: None
        }
        .ist_wiederholbar());
    }

    #[test]
    fn cli_fehler_tragen_code_und_hoechstens_einen_festen_hinweis() {
        assert_eq!(
            VersandFehler::Cli {
                code: cli_code::NICHT_ANGEMELDET,
                hinweis: None
            }
            .to_string(),
            "cli_not_logged_in"
        );
        assert_eq!(
            VersandFehler::Cli {
                code: cli_code::FEHLGESCHLAGEN,
                hinweis: Some("at capacity")
            }
            .to_string(),
            "cli_failed:at capacity"
        );
        assert!(!VersandFehler::Cli {
            code: cli_code::LIMIT,
            hinweis: None
        }
        .is_terminal());
        assert!(cli_code::HINWEISE.iter().all(|h| !h.is_empty()));
    }

    #[test]
    fn kontextueberlauf_bleibt_von_http_400_unterscheidbar() {
        assert_ne!(
            VersandFehler::Kontextueberlauf,
            VersandFehler::Http { status: 400 }
        );
        assert_ne!(
            VersandFehler::Kontextueberlauf.code(),
            VersandFehler::Http { status: 400 }.code()
        );
    }
}
