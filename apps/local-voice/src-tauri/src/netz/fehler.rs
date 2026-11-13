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
                "Keine Verbindung zum Download-Server. Bitte die Netzwerkverbindung prüfen und den Download erneut starten."
            }
            Netzfehler::Zeitueberschreitung => {
                "Der Download-Server antwortet nicht. Bitte später erneut versuchen."
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

/// Entfernt Geheimnisse aus einem Fehlertext, bevor er ins Log, in ein Event
/// oder in einen Befehlsfehler geht: Zugangsdaten und Query/Fragment in URLs
/// (signierte CDN-Adressen, `?token=`), `token=`/`signature=`/`password=`-Paare
/// und `Bearer`-Werte außerhalb von URLs.
pub fn schwaerzen(text: &str) -> String {
    use regex::Regex;
    use std::sync::LazyLock;
    static URL: LazyLock<Regex> = LazyLock::new(|| {
        // Authority und Pfad enden an Klammern (URL in Klammern), Query und
        // Fragment reichen bis zum naechsten Leerraum/Anfuehrungszeichen:
        // `?token=a(GEHEIM)` gehoert ganz zur Query.
        Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*://)([^\s"'<>(){}?#]+)([?#][^\s"'<>]*)?"#)
            .expect("URL-Muster")
    });
    static PAAR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)\b((?:access_|auth_|api_|refresh_|id_)?token|signature|sig|x-amz-signature|x-amz-credential|key|api_key|apikey|password|passwort|secret|client_secret|private_key|credential|auth|authorization|proxy-authorization|cookie|session(?:_?id)?)(\s*[=:]\s*)("?)[^\s"&,;)]+"#,
        )
        .expect("Paar-Muster")
    });
    static BEARER: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)\b(bearer)\s+[A-Za-z0-9._~+/=-]+|\b(basic)\s+[A-Za-z0-9+/=]{8,}")
            .expect("Bearer-Muster")
    });

    let ohne_urls = URL.replace_all(text, |c: &regex::Captures| {
        let schema = &c[1];
        let rest = &c[2];
        // Authority endet am ersten '/'.
        let ende = rest.find('/').unwrap_or(rest.len());
        let (authority, pfad) = rest.split_at(ende);
        let authority = match authority.rfind('@') {
            Some(i) => format!("…@{}", &authority[i + 1..]),
            None => authority.to_string(),
        };
        let query = if c.get(3).is_some() { "?…" } else { "" };
        format!("{schema}{authority}{pfad}{query}")
    });
    // Erst Bearer/Basic (sonst bliebe nach „Authorization: …“ der Wert stehen),
    // dann die Paare.
    let ohne_bearer = BEARER.replace_all(&ohne_urls, "$1$2 …");
    PAAR.replace_all(&ohne_bearer, "$1$2$3…").into_owned()
}

/// Was ein gescheiterter Download nach außen gibt (Log, Event, Befehlsfehler):
/// alles aus dem geschwärzten Rohtext, nie aus dem ungeschwärzten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ausgabe {
    /// Meldung für den Nutzer (bei Netzfehlern deutsch, sonst der geschwärzte Text).
    pub meldung: String,
    /// [`Netzfehler::code`].
    pub code: &'static str,
    /// Geschwärzter Rohtext für „Details“ und Log.
    pub detail: String,
}

/// Baut die [`Ausgabe`] aus dem Rohtext einer Fehlerkette (`{:#}`).
pub fn ausgabe(roh: &str) -> Ausgabe {
    let detail = schwaerzen(roh);
    let art = einordnen(&detail);
    let meldung = if art.ist_netz() {
        art.meldung().to_string()
    } else {
        detail.clone()
    };
    Ausgabe {
        meldung,
        code: art.code(),
        detail,
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
    fn geheimnisse_werden_geschwaerzt() {
        let roh = concat!(
            "Download failed (RequestError(reqwest::Error { kind: Request, url: ",
            "\"https://cas-bridge.xethub.hf.co/x/model.gguf?X-Amz-Signature=GEHEIM1&X-Amz-Credential=GEHEIM2\", ",
            "source: hyper_util::Error(Connect) })) and mirror http://nutzer:GEHEIM3@spiegel.example.org/m?token=GEHEIM4#frag; ",
            "header Authorization: Bearer GEHEIM5; access_token=GEHEIM6&x=1; password: GEHEIM7"
        );
        let sauber = schwaerzen(roh);
        for geheim in [
            "GEHEIM1", "GEHEIM2", "GEHEIM3", "GEHEIM4", "GEHEIM5", "GEHEIM6", "GEHEIM7", "nutzer",
        ] {
            assert!(!sauber.contains(geheim), "{geheim} in {sauber}");
        }
        assert!(sauber.contains("https://cas-bridge.xethub.hf.co/x/model.gguf?…"));
        assert!(sauber.contains("http://…@spiegel.example.org/m?…"));
        // Die Einordnung funktioniert auch auf dem geschwärzten Text.
        assert_eq!(
            einordnen(&schwaerzen(BEFUND)),
            Netzfehler::NameNichtAufloesbar
        );
        assert!(schwaerzen(BEFUND).contains("https://huggingface.co/handy-computer/"));
    }

    /// R2-B2: IPv6-Adressen, `auth=` und Kopfzeilen dürfen in keinem
    /// Ausgabefeld landen – auch nicht bei Fehlern ohne Netzbezug.
    #[test]
    fn ausgabe_ist_in_allen_feldern_geheimnisfrei() {
        let roh = concat!(
            "Mirror download failed: error sending request for url ",
            "(http://[::1]:8080/m.gguf?auth=GEHEIM1): verification failed; ",
            "file is corrupt; url=http://[fe80::1%25eth0]/x?token=GEHEIM2 ",
            "auth=GEHEIM3 Authorization: Bearer GEHEIM4 ",
            "Proxy-Authorization: Basic R0VIRUlNNTpwdw== cookie=GEHEIM6"
        );
        let a = ausgabe(roh);
        for feld in [&a.meldung, &a.detail] {
            for geheim in [
                "GEHEIM1",
                "GEHEIM2",
                "GEHEIM3",
                "GEHEIM4",
                "R0VIRUlNNTpwdw",
                "GEHEIM6",
            ] {
                assert!(!feld.contains(geheim), "{geheim} in {feld}");
            }
        }
        assert!(
            a.detail.contains("http://[::1]:8080/m.gguf?…"),
            "{}",
            a.detail
        );
        // Netzfehler: deutsche Meldung, Detail geschwärzt.
        let b = ausgabe(&format!("{BEFUND} http://[::1]/m?auth=GEHEIM7"));
        assert_eq!(b.code, "dns");
        assert!(!b.detail.contains("GEHEIM7") && !b.meldung.contains("GEHEIM7"));
    }

    #[test]
    fn query_mit_klammern_wird_ganz_geschwaerzt() {
        for roh in [
            "fehler bei https://spiegel.example/m?token=prefix(GEHEIM) weiter",
            "fehler bei https://spiegel.example/m?a=1&b=(x)GEHEIM",
            "fehler bei https://spiegel.example/m#frag(GEHEIM)",
            "(https://spiegel.example/m?sig=a(GEHEIM)b)",
        ] {
            let s = schwaerzen(roh);
            assert!(!s.contains("GEHEIM"), "{s}");
            assert!(s.contains("https://spiegel.example/m?…"), "{s}");
            let a = ausgabe(roh);
            assert!(!a.detail.contains("GEHEIM") && !a.meldung.contains("GEHEIM"));
        }
        // Pfad ohne Query bleibt lesbar, Klammer davor/dahinter bleibt stehen.
        assert_eq!(
            schwaerzen("(https://huggingface.co/a/b.gguf)"),
            "(https://huggingface.co/a/b.gguf)"
        );
    }

    #[test]
    fn meldungen_versprechen_keine_wiederholung() {
        for f in [
            Netzfehler::NameNichtAufloesbar,
            Netzfehler::ProxyAnmeldung,
            Netzfehler::ProxyFehler,
            Netzfehler::KeineVerbindung,
            Netzfehler::Zeitueberschreitung,
            Netzfehler::Zertifikat,
            Netzfehler::Gesperrt,
        ] {
            let m = f.meldung();
            assert!(!m.contains("automatisch"), "{m}");
            assert!(!m.contains("bleiben erhalten"), "{m}");
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
