//! Netzfehler beim Download einordnen und verständlich melden (lva#573).
//!
//! Die Rohmeldungen (reqwest/hyper/hf-hub, teils als `{:?}` formatiert)
//! werden nur noch als „Details“ gezeigt. Eingeordnet wird über den Text der
//! gesamten Fehlerkette, weil hf-hub-Fehler beim Wiederholen in Strings
//! umgewandelt werden und der Typ dann verloren ist.

/// Art eines Download-Fehlers, so grob, wie der Nutzer sie unterscheiden muss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Netzfehler {
    /// Servername nicht auflösbar (kein Netz, oder Firmen-DNS ohne Proxy).
    NameNichtAufloesbar,
    /// Proxy verlangt eine Anmeldung (HTTP 407).
    ProxyAnmeldung,
    /// Proxy nicht erreichbar oder Tunnel abgelehnt.
    ProxyFehler,
    /// Verbindung abgelehnt, zurückgesetzt, Netz nicht erreichbar.
    KeineVerbindung,
    /// Zeitüberschreitung oder hängende Übertragung.
    Zeitueberschreitung,
    /// TLS-Zertifikat abgelehnt (oft Prüfung durch Firmenproxy).
    Zertifikat,
    /// Netz oder Server verweigert den Zugriff (HTTP 403/451).
    Gesperrt,
    /// Alles andere (Prüfsumme, Speicherplatz, unbekannt).
    Sonstiges,
}

impl Netzfehler {
    /// Stabiler Schlüssel für das Frontend (Übersetzung `modelDownload.fault.<code>`).
    pub fn code(self) -> &'static str {
        match self {
            Netzfehler::NameNichtAufloesbar => "dns",
            Netzfehler::ProxyAnmeldung => "proxyAuth",
            Netzfehler::ProxyFehler => "proxy",
            Netzfehler::KeineVerbindung => "offline",
            Netzfehler::Zeitueberschreitung => "timeout",
            Netzfehler::Zertifikat => "certificate",
            Netzfehler::Gesperrt => "blocked",
            Netzfehler::Sonstiges => "other",
        }
    }

    /// Lohnt automatisches Warten und erneutes Versuchen? Nur bei Fehlern,
    /// die ein zurückkehrendes Netz von selbst behebt.
    pub fn voruebergehend(self) -> bool {
        matches!(
            self,
            Netzfehler::NameNichtAufloesbar
                | Netzfehler::KeineVerbindung
                | Netzfehler::Zeitueberschreitung
                | Netzfehler::ProxyFehler
        )
    }

    /// Ist das überhaupt ein Netzproblem (und nicht z. B. eine kaputte Datei)?
    pub fn ist_netz(self) -> bool {
        self != Netzfehler::Sonstiges
    }

    /// Deutsche Meldung für den Nutzer, ohne technische Details.
    pub fn meldung(self) -> &'static str {
        match self {
            Netzfehler::NameNichtAufloesbar => {
                "Keine Verbindung zu Hugging Face: Dieser Rechner kann den Servernamen nicht auflösen. Entweder fehlt gerade das Internet, oder das Firmennetz lässt den Zugang nur über einen Proxy zu (Einstellungen › Allgemein › Proxy für Downloads)."
            }
            Netzfehler::ProxyAnmeldung => {
                "Der Proxy verlangt eine Anmeldung, die Local Voice nicht leisten kann. Bitte die IT \
                 fragen, ob huggingface.co ohne Anmeldung freigegeben werden kann."
            }
            Netzfehler::ProxyFehler => {
                "Der Proxy ist nicht erreichbar oder lehnt die Verbindung ab. Bitte die Proxy-Einstellung unter Einstellungen › Allgemein prüfen."
            }
            Netzfehler::KeineVerbindung => {
                "Keine Verbindung zum Download-Server. Bitte die Netzwerkverbindung prüfen; bereits geladene Teile bleiben erhalten."
            }
            Netzfehler::Zeitueberschreitung => {
                "Der Download-Server antwortet nicht. Bereits geladene Teile bleiben erhalten; ein neuer Start setzt dort fort."
            }
            Netzfehler::Zertifikat => {
                "Die sichere Verbindung wurde abgelehnt (Zertifikat). Häufig prüft ein Firmenproxy \
                 den Datenverkehr; dann muss die IT das Zertifikat auf diesem Rechner hinterlegen."
            }
            Netzfehler::Gesperrt => {
                "Das Netzwerk sperrt den Download (Zugriff verweigert). Bitte die IT bitten, \
                 huggingface.co freizugeben."
            }
            Netzfehler::Sonstiges => "Der Download ist fehlgeschlagen.",
        }
    }
}

/// Ordnet einen Fehlertext (am besten `format!("{:#}", err)` bzw. `{:?}`) ein.
pub fn einordnen(text: &str) -> Netzfehler {
    let t = text.to_ascii_lowercase();
    let hat = |muster: &[&str]| muster.iter().any(|m| t.contains(m));

    if hat(&["http 407", "proxy authentication required"]) {
        return Netzfehler::ProxyAnmeldung;
    }
    if hat(&[
        "certificate",
        "invalidcertificate",
        "unknownissuer",
        "sec_e_untrusted_root",
        "0x80090325",
        "self signed",
        "self-signed",
        "cert verify",
    ]) {
        return Netzfehler::Zertifikat;
    }
    if hat(&[
        "dns error",
        "os error 11001",
        "code: 11001",
        "code: 11002",
        "code: 11004",
        "host ist unbekannt",
        "no such host",
        "failed to lookup address",
        "name or service not known",
        "nodename nor servname",
        "temporary failure in name resolution",
        "no address associated with hostname",
    ]) {
        return Netzfehler::NameNichtAufloesbar;
    }
    if hat(&["tunnel", "proxy connect", "proxyconnect"]) {
        return Netzfehler::ProxyFehler;
    }
    if hat(&["http 403", "http 451"]) {
        return Netzfehler::Gesperrt;
    }
    if hat(&[
        "timed out",
        "timeout",
        "transfer stalled",
        "no response within",
        "no progress for",
    ]) {
        return Netzfehler::Zeitueberschreitung;
    }
    if hat(&[
        "connection refused",
        "connection reset",
        "connection aborted",
        "network is unreachable",
        "host is unreachable",
        "no route to host",
        "code: 10051",
        "code: 10054",
        "code: 10060",
        "code: 10061",
        "code: 10065",
        "error trying to connect",
        "tcp connect error",
        "connecterror",
        "kind: connect",
        "error sending request",
    ]) {
        return Netzfehler::KeineVerbindung;
    }
    Netzfehler::Sonstiges
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wörtlich aus dem Befund vom 13.11.2026 (Behörden-PC über VPN).
    const BEFUND: &str = r#"Download failed from Hugging Face (RequestError(reqwest::Error { kind: Request, url: "https://huggingface.co/handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf/resolve/6d44e540bc31b0de1dbe174a3cea87f53a7f22fb/nemotron-3.5-asr-streaming-0.6b-Q8_0.gguf", source: hyper_util::client::legacy::Error(Connect, ConnectError("dns error", Os { code: 11001, kind: Uncategorized, message: "Der angegebene Host ist unbekannt." })) })) and 1 mirror(s)"#;

    #[test]
    fn befund_ist_dns_und_voruebergehend() {
        let f = einordnen(BEFUND);
        assert_eq!(f, Netzfehler::NameNichtAufloesbar);
        assert!(f.voruebergehend());
        assert_eq!(f.code(), "dns");
        assert!(!f.meldung().contains("reqwest"));
    }

    #[test]
    fn typische_meldungen() {
        let faelle = [
            (
                "server returned HTTP 407 Proxy Authentication Required",
                Netzfehler::ProxyAnmeldung,
            ),
            (
                "invalid peer certificate: UnknownIssuer",
                Netzfehler::Zertifikat,
            ),
            ("SEC_E_UNTRUSTED_ROOT (0x80090325)", Netzfehler::Zertifikat),
            (
                "failed to lookup address information: Name or service not known",
                Netzfehler::NameNichtAufloesbar,
            ),
            (
                "transfer stalled: no data for 60s",
                Netzfehler::Zeitueberschreitung,
            ),
            ("operation timed out", Netzfehler::Zeitueberschreitung),
            (
                "tcp connect error: Connection refused (os error 111)",
                Netzfehler::KeineVerbindung,
            ),
            (
                "ConnectError(\"tcp connect error\", Os { code: 10061 })",
                Netzfehler::KeineVerbindung,
            ),
            ("server returned HTTP 403 Forbidden", Netzfehler::Gesperrt),
            ("unsuccessful tunnel", Netzfehler::ProxyFehler),
            (
                "Download verification failed for model x: file is corrupt",
                Netzfehler::Sonstiges,
            ),
            ("No space left on device", Netzfehler::Sonstiges),
        ];
        for (text, erwartet) in faelle {
            assert_eq!(einordnen(text), erwartet, "{text}");
        }
    }

    #[test]
    fn nur_netzausfaelle_werden_abgewartet() {
        assert!(!Netzfehler::ProxyAnmeldung.voruebergehend());
        assert!(!Netzfehler::Zertifikat.voruebergehend());
        assert!(!Netzfehler::Gesperrt.voruebergehend());
        assert!(!Netzfehler::Sonstiges.voruebergehend());
        assert!(!Netzfehler::Sonstiges.ist_netz());
        assert!(Netzfehler::KeineVerbindung.voruebergehend());
    }
}
