//! Bewertung einer Messung: erreichbar / eingeschränkt / nicht erreichbar,
//! Grund und die Auswirkung in einem Satz (lva#496). Rein.

use super::{Category, Failure, ProbeOutcome, Reason, Row, State, Target, SLOW_MS};

/// Namen, die im Aussteller eines Zertifikats auf einen TLS-aufbrechenden
/// Proxy oder eine Firewall mit Inhaltsprüfung deuten (klein geschrieben).
/// Eine Liste bekannter Hersteller und Wörter, kein Beweis: ein Unternehmens-
/// Proxy mit eigenem Namen bleibt unerkannt, deshalb sagt die Oberfläche
/// „vermutlich“ und das Fehlerbild „Zertifikat nicht vertraut“ trägt denselben Hinweis.
const INSPECTION_MARKERS: [&str; 25] = [
    "zscaler",
    "netskope",
    "forcepoint",
    "websense",
    "fortinet",
    "fortigate",
    "palo alto",
    "sophos",
    "bluecoat",
    "blue coat",
    "symantec web",
    "barracuda",
    "check point",
    "checkpoint",
    "cisco umbrella",
    "iboss",
    "sonicwall",
    "watchguard",
    "untangle",
    "squid",
    "proxy",
    "firewall",
    "inspection",
    "intercept",
    "mitm",
];

/// Deutet der Aussteller auf einen TLS-aufbrechenden Proxy?
pub fn looks_like_inspection(issuer: &str) -> bool {
    let lower = issuer.to_lowercase();
    INSPECTION_MARKERS.iter().any(|m| lower.contains(m))
}

/// Zustand und Grund einer Messung.
pub fn judge(outcome: &ProbeOutcome) -> (State, Option<Reason>) {
    if let Some(failure) = outcome.failure {
        let reason = match failure {
            Failure::Dns => Reason::DnsFailed,
            Failure::Timeout(_) => Reason::Timeout,
            Failure::Connect => Reason::ConnectFailed,
            Failure::TlsCertificate => Reason::TlsCertificate,
            Failure::Tls => Reason::TlsFailed,
            Failure::Http => Reason::HttpFailed,
        };
        return (State::Unreachable, Some(reason));
    }
    let Some(status) = outcome.http_status else {
        return (State::Unreachable, Some(Reason::HttpFailed));
    };
    if status == 407 {
        return (State::Limited, Some(Reason::ProxyAuth));
    }
    if outcome.issuer.as_deref().is_some_and(looks_like_inspection) {
        return (State::Limited, Some(Reason::TlsInspection));
    }
    if (500..=599).contains(&status) {
        return (State::Limited, Some(Reason::Http5xx));
    }
    if outcome.total_ms >= SLOW_MS {
        return (State::Limited, Some(Reason::Slow));
    }
    (State::Reachable, None)
}

/// Ergebniszeile einer Gegenstelle.
pub fn row(target: &Target, outcome: &ProbeOutcome) -> Row {
    let (state, reason) = judge(outcome);
    // Der Aussteller steht nur da, wo er gebraucht wird; sonst bleibt der Bericht schlank.
    let issuer = (reason == Some(Reason::TlsInspection))
        .then(|| outcome.issuer.clone())
        .flatten();
    Row {
        category: target.category,
        host: target.endpoint.host.clone(),
        port: target.endpoint.port,
        tls: target.endpoint.tls,
        state,
        reason,
        dns_ms: outcome.dns_ms,
        tls_ms: outcome.tls_ms,
        http_status: outcome.http_status,
        total_ms: outcome.total_ms,
        issuer,
    }
}

/// Name der Funktion für die Überschrift einer Zeile im Bericht.
pub fn category_name(category: Category) -> &'static str {
    match category {
        Category::SyncHub => "Geräteabgleich",
        Category::Update => "Update-Quelle",
        Category::Llm => "Sprachmodell-Anbieter",
        Category::Tts => "Online-Stimme",
        Category::CalendarGraph => "Kalender (Microsoft 365)",
        Category::CalendarIcs => "Kalender (ICS)",
        Category::ModelDownload => "Modell-Downloads",
    }
}

/// Die Auswirkung in einem Satz. Dieselben Sätze stehen für die Oberfläche in
/// den Übersetzungen unter `netzcheck.impact.<Kategorie>.<Zustand>`.
pub fn impact_text(category: Category, state: State) -> &'static str {
    use Category::*;
    use State::*;
    match (category, state) {
        (SyncHub, Reachable) => "Der Geräteabgleich sollte funktionieren.",
        (SyncHub, Limited) => "Der Geräteabgleich funktioniert vermutlich, ist aber langsam oder unzuverlässig.",
        (SyncHub, Unreachable) => "Der Geräteabgleich funktioniert nicht; Änderungen bleiben auf diesem Gerät.",
        (Update, Reachable) => "Neue Versionen werden gefunden.",
        (Update, Limited) => "Neue Versionen werden vermutlich gefunden, die Prüfung ist aber langsam oder unzuverlässig.",
        (Update, Unreachable) => "Neue Versionen werden nicht gefunden; Updates müssen von Hand eingespielt werden.",
        (Llm, Reachable) => "Die Sprachmodelle dieses Anbieters sollten nutzbar sein.",
        (Llm, Limited) => "Die Sprachmodelle dieses Anbieters sind vermutlich nutzbar, antworten aber langsam oder fehlerhaft.",
        (Llm, Unreachable) => "Zusammenfassungen und Nachbearbeitung mit diesem Anbieter sind nicht möglich.",
        (Tts, Reachable) => "Vorlesen mit dieser Online-Stimme sollte funktionieren.",
        (Tts, Limited) => "Vorlesen mit dieser Online-Stimme funktioniert vermutlich, aber langsam oder mit Aussetzern.",
        (Tts, Unreachable) => "Vorlesen mit dieser Online-Stimme ist nicht möglich.",
        (CalendarGraph, Reachable) => "Termine aus Outlook / Microsoft 365 sollten sich aktualisieren.",
        (CalendarGraph, Limited) => "Termine aus Outlook / Microsoft 365 aktualisieren sich vermutlich, aber langsam oder unvollständig.",
        (CalendarGraph, Unreachable) => "Termine aus Outlook / Microsoft 365 werden nicht aktualisiert.",
        (CalendarIcs, Reachable) => "Termine aus diesem Kalender sollten sich aktualisieren.",
        (CalendarIcs, Limited) => "Termine aus diesem Kalender aktualisieren sich vermutlich, aber langsam oder unvollständig.",
        (CalendarIcs, Unreachable) => "Termine aus diesem Kalender werden nicht aktualisiert.",
        (ModelDownload, Reachable) => "Neue Modelle lassen sich herunterladen.",
        (ModelDownload, Limited) => "Modell-Downloads funktionieren vermutlich, sind aber langsam oder brechen ab.",
        (ModelDownload, Unreachable) => "Neue Modelle lassen sich nicht herunterladen; bereits geladene Modelle laufen weiter.",
    }
}

/// Kurzer Grund für den Bericht.
pub fn reason_text(reason: Reason) -> &'static str {
    match reason {
        Reason::DnsFailed => "Namensauflösung fehlgeschlagen",
        Reason::Timeout => "keine Antwort innerhalb des Zeitlimits",
        Reason::ConnectFailed => "Verbindung abgelehnt oder Ziel nicht erreichbar",
        Reason::TlsCertificate => {
            "Zertifikat nicht vertrauenswürdig (möglicherweise ein TLS-aufbrechender Proxy oder ein fehlendes Stammzertifikat)"
        }
        Reason::TlsFailed => "TLS-Aufbau fehlgeschlagen",
        Reason::HttpFailed => "HTTP-Anfrage fehlgeschlagen",
        Reason::Http5xx => "Server oder Proxy meldet einen Fehler (HTTP 5xx)",
        Reason::ProxyAuth => "Der Proxy verlangt eine Anmeldung (HTTP 407)",
        Reason::TlsInspection => {
            "Das Zertifikat stammt vermutlich von einem TLS-aufbrechenden Proxy"
        }
        Reason::Slow => "antwortet langsam",
    }
}

pub fn state_text(state: State) -> &'static str {
    match state {
        State::Reachable => "erreichbar",
        State::Limited => "eingeschränkt",
        State::Unreachable => "nicht erreichbar",
    }
}

#[cfg(test)]
mod tests {
    use super::super::redact::Endpoint;
    use super::*;
    use crate::netzcheck::Stage;

    fn ok(status: u16, total: u32, issuer: Option<&str>) -> ProbeOutcome {
        ProbeOutcome {
            dns_ms: Some(4),
            tls_ms: Some(30),
            http_status: Some(status),
            http_ms: Some(10),
            total_ms: total,
            failure: None,
            issuer: issuer.map(str::to_string),
        }
    }

    fn failed(f: Failure) -> ProbeOutcome {
        ProbeOutcome {
            total_ms: 100,
            failure: Some(f),
            ..ProbeOutcome::default()
        }
    }

    #[test]
    fn antwort_ist_erreichbar_auch_bei_404_und_401() {
        for s in [200, 204, 301, 401, 403, 404, 421, 429] {
            assert_eq!(
                judge(&ok(s, 120, Some("DigiCert Global G2"))),
                (State::Reachable, None),
                "{s}"
            );
        }
    }

    #[test]
    fn fehler_sind_nicht_erreichbar_mit_grund() {
        let cases = [
            (Failure::Dns, Reason::DnsFailed),
            (Failure::Timeout(Stage::Dns), Reason::Timeout),
            (Failure::Timeout(Stage::Unknown), Reason::Timeout),
            (Failure::Connect, Reason::ConnectFailed),
            (Failure::TlsCertificate, Reason::TlsCertificate),
            (Failure::Tls, Reason::TlsFailed),
            (Failure::Http, Reason::HttpFailed),
        ];
        for (f, r) in cases {
            assert_eq!(judge(&failed(f)), (State::Unreachable, Some(r)));
        }
    }

    #[test]
    fn eingeschraenkt_bei_proxy_anmeldung_5xx_langsam_und_inspektion() {
        assert_eq!(
            judge(&ok(407, 50, None)),
            (State::Limited, Some(Reason::ProxyAuth))
        );
        assert_eq!(
            judge(&ok(502, 50, None)),
            (State::Limited, Some(Reason::Http5xx))
        );
        assert_eq!(
            judge(&ok(503, 50, None)),
            (State::Limited, Some(Reason::Http5xx))
        );
        assert_eq!(
            judge(&ok(200, SLOW_MS, None)),
            (State::Limited, Some(Reason::Slow))
        );
        assert_eq!(judge(&ok(200, SLOW_MS - 1, None)), (State::Reachable, None));
        assert_eq!(
            judge(&ok(200, 80, Some("Zscaler Root CA"))),
            (State::Limited, Some(Reason::TlsInspection))
        );
    }

    #[test]
    fn ohne_fehler_und_ohne_status_ist_nicht_erreichbar() {
        assert_eq!(
            judge(&ProbeOutcome::default()),
            (State::Unreachable, Some(Reason::HttpFailed))
        );
    }

    #[test]
    fn inspektions_erkennung_ohne_fehlalarm_bei_oeffentlichen_cas() {
        for issuer in [
            "Zscaler Intermediate Root CA",
            "Palo Alto Networks Decryption CA",
            "FortiGate CA",
            "Firma Web Proxy CA",
            "Sophos SSL CA",
            "CCR agent-proxy interception CA (production) (Anthropic)",
        ] {
            assert!(looks_like_inspection(issuer), "{issuer}");
        }
        for issuer in [
            "DigiCert Global G2 TLS RSA SHA256 2020 CA1",
            "Let's Encrypt R11",
            "Sectigo Public Server Authentication CA DV R36",
            "Microsoft Azure RSA TLS Issuing CA 03",
            "GTS CA 1P5",
            "Amazon RSA 2048 M02",
        ] {
            assert!(!looks_like_inspection(issuer), "{issuer}");
        }
    }

    #[test]
    fn zeile_traegt_aussteller_nur_bei_inspektionsverdacht() {
        let target = Target {
            category: Category::Llm,
            endpoint: Endpoint {
                host: "api.example.com".into(),
                port: 443,
                tls: true,
            },
        };
        let normal = row(&target, &ok(200, 90, Some("DigiCert")));
        assert_eq!(normal.issuer, None);
        assert_eq!(normal.state, State::Reachable);
        let mitm = row(&target, &ok(200, 90, Some("Zscaler Root CA")));
        assert_eq!(mitm.issuer.as_deref(), Some("Zscaler Root CA"));
        assert_eq!(mitm.state, State::Limited);
        assert_eq!((mitm.host.as_str(), mitm.port), ("api.example.com", 443));
    }

    #[test]
    fn jede_kategorie_hat_drei_saetze_mit_umlauten_und_punkt() {
        use Category::*;
        for c in [
            SyncHub,
            Update,
            Llm,
            Tts,
            CalendarGraph,
            CalendarIcs,
            ModelDownload,
        ] {
            for s in [State::Reachable, State::Limited, State::Unreachable] {
                let t = impact_text(c, s);
                assert!(t.ends_with('.'), "{t}");
            }
            assert_ne!(
                impact_text(c, State::Reachable),
                impact_text(c, State::Unreachable)
            );
        }
    }
}
