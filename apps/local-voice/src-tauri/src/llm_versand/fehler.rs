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
    /// Der Anbieter antwortete mit diesem HTTP-Status. Nie mit seinem Antworttext.
    Http { status: u16 },
    /// Verbindungsfehler, Protokollfehler, nicht lesbare Antwort. Ohne Anbietertext.
    Transport,
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

    /// Stabile Kennung der Fehlerart ohne Zahlen und Meldungen; für Zähler und Logzeilen.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Kontextueberlauf => "kontextueberlauf",
            Self::UngueltigesJson => "ungueltiges_json",
            Self::ZuWenigRam => "zu_wenig_ram",
            Self::Absturz => "absturz",
            Self::Abgebrochen => "abgebrochen",
            Self::CliUpdate => "cli_update",
            Self::Http { .. } => "http",
            Self::Transport => "transport",
            Self::Zeitueberschreitung => "zeitueberschreitung",
            Self::Budget { .. } => "budget",
            Self::Compliance { .. } => "compliance",
            Self::Gesperrt { code, .. } => code,
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
            VersandFehler::Http { status: 400 },
            VersandFehler::Transport,
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
                | VersandFehler::Http { .. }
                | VersandFehler::Transport
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
