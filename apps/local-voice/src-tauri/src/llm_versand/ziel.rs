//! Das konkret aufgelöste Ziel eines Modellaufrufs (lva#604, Teilpaket 1).
//!
//! [`Ziel`] fasst zusammen, **wohin** ein Aufruf tatsächlich geht: Anbieter-ID, Basis-URL
//! bzw. CLI-Programm, Modell und [`ZielArt`]. Die Art wird **abgeleitet, nie gesetzt**: es
//! gibt keinen Konstruktor mit freiem „lokal“-Schalter.
//!
//! Regel für [`ZielArt::Lokal`] (lva#604, „Vertrag“):
//! - Lokal ist nur eine **http(s)-Adresse auf Loopback** nach
//!   [`is_loopback_url`](crate::llm_client::is_loopback_url) (`127.0.0.0/8`, `::1`,
//!   `localhost`). Das eingebaute lokale Modell fällt darunter, weil seine aufgelöste
//!   Adresse (`ensure_local`, Platzhalter `http://127.0.0.1:0/v1`) Loopback ist; die bloße
//!   Anbieter-ID `local` macht nichts lokal. Zeigt ein Anbieter mit der ID `local` auf eine
//!   fremde Adresse, ist das Ziel Cloud.
//! - **Cloud ist alles andere:** LAN-Adressen (`192.168.x.x`, `10.x.x.x`), sonstige
//!   Hostnamen (auch `foo.local`), CLI-Programme, unbekannte Anbieter, leere und kaputte URLs
//!   sowie jedes andere Schema als `http`/`https` (so wird `cli://localhost` nicht lokal).
//! - Ein Cloud-Gateway auf Loopback (z. B. ein lokaler Proxy zu einem Cloud-Anbieter) zählt
//!   als lokal. Diese Grenze ist akzeptiert und dokumentiert (lva#604).
//! - `managers::llm::is_local` (erkennt an Anbieter-ID oder Platzhalter) wird hierfür
//!   bewusst **nicht** benutzt: es sieht nicht auf die tatsächliche Adresse.
//!
//! Die Freigabe wird an genau dieses Ziel gebunden; ein Zielwechsel (auch bei gleicher
//! Anbieter-ID) macht sie ungültig. Dafür ist [`Ziel`] vergleichbar (`PartialEq`).

use std::fmt;

use url::Url;

use crate::llm_client::is_loopback_url;
use crate::managers::llm::LOCAL_PROVIDER_ID;
use crate::settings::PostProcessProvider;

/// Vorsatz der CLI-Anbieter (`cli://claude`, `cli://codex`).
const CLI_SCHEMA: &str = "cli://";

/// Wohin ein Aufruf geht, grob: auf diesen Rechner oder darüber hinaus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZielArt {
    /// Loopback-Adresse (nach `is_loopback_url`) oder das eingebaute lokale Modell.
    Lokal,
    /// Alles andere.
    Cloud,
}

/// Wo das Ziel erreichbar ist. Privat, damit niemand Art und Adresse getrennt setzt.
#[derive(Clone, PartialEq, Eq)]
enum Adresse {
    /// Basis-URL eines HTTP-Anbieters.
    Http(String),
    /// Programmname einer CLI (ohne `cli://`).
    Cli(String),
}

/// Das konkret aufgelöste Ziel. Felder privat; Konstruktoren leiten die [`ZielArt`] ab.
#[derive(Clone, PartialEq, Eq)]
pub struct Ziel {
    anbieter_id: String,
    adresse: Adresse,
    modell: String,
    art: ZielArt,
}

/// http(s) UND Loopback. Ein anderes Schema (`cli://localhost`, `ftp://127.0.0.1`) ist nie
/// lokal.
fn ist_http_loopback(url: &str) -> bool {
    let Ok(geparst) = Url::parse(url.trim()) else {
        return false;
    };
    matches!(geparst.scheme(), "http" | "https") && is_loopback_url(url)
}

impl Ziel {
    /// HTTP-Ziel mit seiner (aufgelösten) Basis-URL. Die Art folgt allein aus der URL.
    pub fn http(anbieter_id: &str, basis_url: &str, modell: &str) -> Self {
        let basis_url = basis_url.trim();
        let art = if ist_http_loopback(basis_url) {
            ZielArt::Lokal
        } else {
            ZielArt::Cloud
        };
        Self {
            anbieter_id: anbieter_id.to_owned(),
            adresse: Adresse::Http(basis_url.to_owned()),
            modell: modell.to_owned(),
            art,
        }
    }

    /// CLI-Ziel (`claude`, `codex`). Immer Cloud: die CLI spricht mit dem Dienst ihres
    /// Herstellers, und ihr Prozess ist von hier aus nicht zu kontrollieren.
    pub fn cli(anbieter_id: &str, programm: &str, modell: &str) -> Self {
        Self {
            anbieter_id: anbieter_id.to_owned(),
            adresse: Adresse::Cli(programm.trim().to_owned()),
            modell: modell.to_owned(),
            art: ZielArt::Cloud,
        }
    }

    /// Das eingebaute lokale Modell mit der **aufgelösten** Adresse seines Servers
    /// (`managers::llm::ensure_local`). Es gibt kein Lokal-Flag: zeigt die Adresse nicht auf
    /// Loopback, ist das Ziel Cloud.
    pub fn eingebaut(modell: &str, aufgeloeste_url: &str) -> Self {
        Self::http(LOCAL_PROVIDER_ID, aufgeloeste_url, modell)
    }

    /// Ziel eines konfigurierten Anbieters mit der Adresse, die er **jetzt** trägt.
    /// `cli://…` ergibt ein CLI-Ziel, alles andere ein HTTP-Ziel. Für den eingebauten
    /// Anbieter (Platzhalter-Port 0) vorher die Adresse auflösen und [`Ziel::eingebaut`]
    /// benutzen; der Platzhalter selbst ist zwar Loopback, aber nicht erreichbar.
    pub fn aus_provider(provider: &PostProcessProvider, modell: &str) -> Self {
        let url = provider.base_url.trim();
        match url.get(..CLI_SCHEMA.len()) {
            Some(vorsatz) if vorsatz.eq_ignore_ascii_case(CLI_SCHEMA) => {
                Self::cli(&provider.id, &url[CLI_SCHEMA.len()..], modell)
            }
            _ => Self::http(&provider.id, url, modell),
        }
    }

    /// Anbieter-ID wie in den Einstellungen.
    pub fn anbieter_id(&self) -> &str {
        &self.anbieter_id
    }

    /// Modellname (bei CLIs mit Effort-Anhang wie in den Einstellungen).
    pub fn modell(&self) -> &str {
        &self.modell
    }

    /// Lokal oder Cloud.
    pub fn art(&self) -> ZielArt {
        self.art
    }

    /// Ist das Ziel lokal (Loopback oder eingebautes Modell)?
    pub fn ist_lokal(&self) -> bool {
        self.art == ZielArt::Lokal
    }

    /// Ist das Ziel Cloud (alles, was nicht lokal ist)?
    pub fn ist_cloud(&self) -> bool {
        self.art == ZielArt::Cloud
    }

    /// Basis-URL eines HTTP-Ziels, genau wie gebunden; `None` bei CLI-Zielen. Nur für den
    /// Transport: **nie loggen** (kann Zugangsdaten enthalten), dafür
    /// [`Ziel::adresse_fuer_log`].
    pub fn basis_url(&self) -> Option<&str> {
        match &self.adresse {
            Adresse::Http(url) => Some(url),
            Adresse::Cli(_) => None,
        }
    }

    /// Programmname eines CLI-Ziels; `None` bei HTTP-Zielen.
    pub fn cli_programm(&self) -> Option<&str> {
        match &self.adresse {
            Adresse::Cli(programm) => Some(programm),
            Adresse::Http(_) => None,
        }
    }

    /// Adresse ohne Zugangsdaten, Pfad und Anfrageteil: `schema://host[:port]`, bei CLIs
    /// `cli://programm`, bei unlesbaren URLs `<ungueltig>`. Sicher für Logzeilen.
    pub fn adresse_fuer_log(&self) -> String {
        match &self.adresse {
            Adresse::Cli(programm) => format!("{CLI_SCHEMA}{programm}"),
            Adresse::Http(url) => match Url::parse(url) {
                Ok(geparst) => match (geparst.host_str(), geparst.port()) {
                    (Some(host), Some(port)) => format!("{}://{host}:{port}", geparst.scheme()),
                    (Some(host), None) => format!("{}://{host}", geparst.scheme()),
                    (None, _) => "<ungueltig>".to_owned(),
                },
                Err(_) => "<ungueltig>".to_owned(),
            },
        }
    }
}

impl fmt::Debug for Ziel {
    /// Ohne Zugangsdaten und ohne Pfad: `Debug` landet leicht in einer Logzeile.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ziel")
            .field("anbieter_id", &self.anbieter_id)
            .field("adresse", &self.adresse_fuer_log())
            .field("modell", &self.modell)
            .field("art", &self.art)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(url: &str) -> Ziel {
        Ziel::http("x", url, "m")
    }

    fn provider(id: &str, url: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_owned(),
            label: id.to_owned(),
            base_url: url.to_owned(),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    #[test]
    fn loopback_ist_lokal() {
        for url in [
            "http://127.0.0.1:8080/v1",
            "http://127.0.0.1/v1",
            "http://127.1.2.3:9/v1",
            "http://[::1]:11434/v1",
            "http://localhost:11434/v1",
            "https://localhost/v1",
            "http://LOCALHOST:1/x",
            "  http://127.0.0.1:8080/v1  ",
            "http://127.0.0.1:0/v1",
        ] {
            assert_eq!(http(url).art(), ZielArt::Lokal, "{url}");
            assert!(http(url).ist_lokal() && !http(url).ist_cloud(), "{url}");
        }
    }

    #[test]
    fn lan_hostnamen_und_oeffentliche_hosts_sind_cloud() {
        for url in [
            "http://192.168.1.20:11434/v1",
            "http://10.0.0.5:8080/v1",
            "http://172.16.0.1/v1",
            "http://foo.local:8080/v1",
            "http://nas/v1",
            "https://api.example.com/v1",
            "https://api.openai.com/v1",
            "http://localhost.example.com/v1",
            "http://127.0.0.1.example.com/v1",
            "http://foo.localhost/v1",
            "http://0.0.0.0:8080/v1",
        ] {
            assert_eq!(http(url).art(), ZielArt::Cloud, "{url}");
            assert!(http(url).ist_cloud(), "{url}");
        }
    }

    #[test]
    fn kaputte_und_leere_urls_sind_cloud() {
        for url in [
            "",
            "   ",
            "kein url",
            "127.0.0.1:8080/v1",
            "localhost",
            "http://",
            "http:///v1",
            "://127.0.0.1",
        ] {
            assert_eq!(http(url).art(), ZielArt::Cloud, "{url:?}");
        }
    }

    #[test]
    fn nur_http_und_https_koennen_lokal_sein() {
        for url in [
            "cli://localhost",
            "ftp://127.0.0.1/x",
            "file://localhost/etc",
            "ws://127.0.0.1:1/x",
            "unix://localhost",
        ] {
            assert_eq!(http(url).art(), ZielArt::Cloud, "{url}");
        }
    }

    #[test]
    fn die_adresse_entscheidet_nicht_ein_trick_in_der_url() {
        for url in [
            // Nutzerinfo vor dem Host: der Host ist evil.example.
            "http://127.0.0.1@evil.example/v1",
            "http://localhost:80@evil.example/v1",
            "http://user:pw@192.168.0.1/v1",
            // IPv4-mapped IPv6 gilt nicht als Loopback (streng).
            "http://[::ffff:127.0.0.1]:8080/v1",
        ] {
            assert_eq!(http(url).art(), ZielArt::Cloud, "{url}");
        }
        // Nutzerinfo beim echten Loopback ändert nichts.
        assert_eq!(
            http("http://user:pw@127.0.0.1:8080/v1").art(),
            ZielArt::Lokal
        );
    }

    #[test]
    fn cli_ist_immer_cloud() {
        let z = Ziel::cli("claude_cli", "claude", "sonnet");
        assert_eq!(z.art(), ZielArt::Cloud);
        assert_eq!(z.cli_programm(), Some("claude"));
        assert_eq!(z.basis_url(), None);
        // Auch ein Programmname „localhost“ macht nichts lokal.
        assert_eq!(Ziel::cli("c", "localhost", "m").art(), ZielArt::Cloud);
    }

    #[test]
    fn aus_provider_loest_cli_und_http_auf() {
        let claude = Ziel::aus_provider(&provider("claude_cli", "cli://claude"), "sonnet");
        assert_eq!(claude.art(), ZielArt::Cloud);
        assert_eq!(claude.cli_programm(), Some("claude"));
        assert_eq!(claude.anbieter_id(), "claude_cli");
        assert_eq!(claude.modell(), "sonnet");

        let codex = Ziel::aus_provider(&provider("codex_cli", "CLI://codex"), "gpt");
        assert_eq!(codex.cli_programm(), Some("codex"));
        assert_eq!(codex.art(), ZielArt::Cloud);

        let ollama = Ziel::aus_provider(&provider("ollama", "http://localhost:11434/v1"), "m");
        assert_eq!(ollama.art(), ZielArt::Lokal);
        assert_eq!(ollama.basis_url(), Some("http://localhost:11434/v1"));
        assert_eq!(ollama.cli_programm(), None);

        let lan = Ziel::aus_provider(&provider("ollama", "http://192.168.1.20:11434/v1"), "m");
        assert_eq!(lan.art(), ZielArt::Cloud);

        let cloud = Ziel::aus_provider(&provider("openai", "https://api.openai.com/v1"), "gpt");
        assert_eq!(cloud.art(), ZielArt::Cloud);

        let kaputt = Ziel::aus_provider(&provider("custom", "kein url"), "m");
        assert_eq!(kaputt.art(), ZielArt::Cloud);

        let leer = Ziel::aus_provider(&provider("", ""), "");
        assert_eq!(leer.art(), ZielArt::Cloud);
    }

    #[test]
    fn eingebautes_modell_ist_lokal_nur_mit_loopback_adresse() {
        // Platzhalter der Vorlage und aufgelöste Serveradresse.
        let platzhalter = Ziel::aus_provider(&provider("local", "http://127.0.0.1:0/v1"), "q4");
        assert_eq!(platzhalter.art(), ZielArt::Lokal);
        let aufgeloest = Ziel::eingebaut("q4", "http://127.0.0.1:51234/v1");
        assert_eq!(aufgeloest.art(), ZielArt::Lokal);
        assert_eq!(aufgeloest.anbieter_id(), LOCAL_PROVIDER_ID);

        // Kein Lokal-Flag: eine Adresse außerhalb von Loopback macht auch das „eingebaute“
        // Modell zu Cloud, und die bloße ID `local` ändert daran nichts.
        assert_eq!(
            Ziel::eingebaut("q4", "http://192.168.1.9:8080/v1").art(),
            ZielArt::Cloud
        );
        assert_eq!(
            Ziel::aus_provider(&provider("local", "https://evil.example/v1"), "q4").art(),
            ZielArt::Cloud
        );
    }

    #[test]
    fn zielwechsel_bei_gleicher_anbieter_id_ist_ungleich() {
        let a = Ziel::http("ollama", "http://127.0.0.1:11434/v1", "m");
        assert_eq!(a, Ziel::http("ollama", "http://127.0.0.1:11434/v1", "m"));
        assert_ne!(a, Ziel::http("ollama", "http://127.0.0.1:11435/v1", "m"));
        assert_ne!(a, Ziel::http("ollama", "https://api.example.com/v1", "m"));
        assert_ne!(
            a,
            Ziel::http("ollama", "http://127.0.0.1:11434/v1", "anderes")
        );
        assert_ne!(a, Ziel::http("anderer", "http://127.0.0.1:11434/v1", "m"));
        // Gleiche Adresse als CLI ist ein anderes Ziel.
        assert_ne!(
            Ziel::cli("c", "claude", "m"),
            Ziel::http("c", "claude", "m")
        );
    }

    #[test]
    fn log_und_debug_zeigen_keine_zugangsdaten() {
        let z = Ziel::http(
            "gateway",
            "https://user:geheim@api.example.com:8443/v1/chat?key=token123",
            "m",
        );
        assert_eq!(z.adresse_fuer_log(), "https://api.example.com:8443");
        let debug = format!("{z:?}");
        assert!(!debug.contains("geheim"), "{debug}");
        assert!(!debug.contains("token123"), "{debug}");
        assert!(!debug.contains("/v1/chat"), "{debug}");
        assert!(debug.contains("api.example.com"), "{debug}");

        assert_eq!(http("kein url").adresse_fuer_log(), "<ungueltig>");
        assert_eq!(
            Ziel::cli("c", "claude", "m").adresse_fuer_log(),
            "cli://claude"
        );
    }
}
