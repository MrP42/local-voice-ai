use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::*;
use crate::managers::integrations::gate::GateOutcome;
use crate::managers::integrations::model::{Caller, Kind, NewIntegration};
use crate::managers::integrations::test_support::Fx;
use crate::managers::integrations::{audit, store, targets};

const PASSWORD: &str = "geheim-App-Passwort-123";
const IDENTITY: &[u8] = include_bytes!("test_identity.p12");
const CERT_PEM: &[u8] = include_bytes!("test_cert.pem");

// ---------------------------------------------------------------------------
// Test-SMTP-Server (ein Verbindungsaufbau, protokolliert alles)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Script {
    /// Faehigkeiten ausser STARTTLS.
    caps: Vec<&'static str>,
    user: &'static str,
    pass: &'static str,
    /// Mit Identitaet: TLS ist moeglich.
    tls: bool,
    implicit_tls: bool,
    advertise_starttls: bool,
    inject_after_starttls: bool,
    reject_rcpt: Vec<&'static str>,
    reject_data: bool,
    silent: bool,
    long_line: bool,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            caps: vec!["AUTH PLAIN LOGIN", "8BITMIME"],
            user: "patrick",
            pass: PASSWORD,
            tls: false,
            implicit_tls: false,
            advertise_starttls: false,
            inject_after_starttls: false,
            reject_rcpt: vec![],
            reject_data: false,
            silent: false,
            long_line: false,
        }
    }
}

struct Mock {
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
    message: Arc<Mutex<Option<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl Mock {
    fn start(script: Script) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let message = Arc::new(Mutex::new(None));
        let (l2, m2) = (log.clone(), message.clone());
        let handle = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve(stream, script, l2, m2);
            }
        });
        Mock {
            port,
            log,
            message,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> (Vec<String>, Option<String>) {
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let log = self.log.lock().unwrap().clone();
        let msg = self.message.lock().unwrap().clone();
        (log, msg)
    }
}

fn acceptor() -> native_tls::TlsAcceptor {
    let identity =
        native_tls::Identity::from_pkcs12(IDENTITY, "test").expect("PKCS12 der Testidentitaet");
    native_tls::TlsAcceptor::new(identity).expect("TLS-Acceptor")
}

fn say(r: &mut BufReader<Transport>, text: &str) {
    let w = r.get_mut();
    let _ = w.write_all(text.as_bytes());
    let _ = w.flush();
}

fn upgrade_server(
    r: BufReader<Transport>,
    acc: &native_tls::TlsAcceptor,
) -> Option<BufReader<Transport>> {
    let Transport::Plain(stream) = r.into_inner() else {
        return None;
    };
    let tls = acc.accept(stream).ok()?;
    Some(BufReader::new(Transport::Tls(Box::new(tls))))
}

fn serve(
    stream: TcpStream,
    s: Script,
    log: Arc<Mutex<Vec<String>>>,
    message: Arc<Mutex<Option<String>>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let acc = s.tls.then(acceptor);
    let mut r = if s.implicit_tls {
        let Some(a) = &acc else { return };
        let Ok(tls) = a.accept(stream) else { return };
        BufReader::new(Transport::Tls(Box::new(tls)))
    } else {
        BufReader::new(Transport::Plain(stream))
    };
    if s.silent {
        std::thread::sleep(Duration::from_millis(1500));
        return;
    }
    if s.long_line {
        say(&mut r, &format!("220 {}\r\n", "x".repeat(MAX_LINE + 10)));
        return;
    }
    say(&mut r, "220 mock.test ESMTP bereit\r\n");
    let mut encrypted = s.implicit_tls;
    loop {
        let mut line = String::new();
        match r.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end().to_string();
        log.lock().unwrap().push(line.clone());
        let up = line.to_uppercase();
        if up.starts_with("EHLO") {
            let mut caps: Vec<String> = s.caps.iter().map(|c| c.to_string()).collect();
            if s.advertise_starttls && !encrypted {
                caps.push("STARTTLS".to_string());
            }
            let mut out = "250-mock.test\r\n".to_string();
            for (i, c) in caps.iter().enumerate() {
                let sep = if i + 1 == caps.len() { ' ' } else { '-' };
                out.push_str(&format!("250{sep}{c}\r\n"));
            }
            if caps.is_empty() {
                out = "250 mock.test\r\n".to_string();
            }
            say(&mut r, &out);
        } else if up == "STARTTLS" {
            if s.inject_after_starttls {
                say(&mut r, "220 Bereit\r\n220 boese eingeschleust\r\n");
            } else {
                say(&mut r, "220 Bereit\r\n");
            }
            let Some(a) = &acc else { return };
            let Some(next) = upgrade_server(r, a) else {
                return;
            };
            r = next;
            encrypted = true;
        } else if up.starts_with("AUTH PLAIN") {
            let b64 = line[10..].trim();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap_or_default();
            let want = format!("\0{}\0{}", s.user, s.pass);
            if decoded == want.as_bytes() {
                say(&mut r, "235 2.7.0 angemeldet\r\n");
            } else {
                say(&mut r, "535 5.7.8 Anmeldung abgelehnt\r\n");
            }
        } else if up == "AUTH LOGIN" {
            say(&mut r, "334 VXNlcm5hbWU6\r\n");
            let mut u = String::new();
            let _ = r.read_line(&mut u);
            say(&mut r, "334 UGFzc3dvcmQ6\r\n");
            let mut p = String::new();
            let _ = r.read_line(&mut p);
            let b = base64::engine::general_purpose::STANDARD;
            let ok = b.decode(u.trim()).unwrap_or_default() == s.user.as_bytes()
                && b.decode(p.trim()).unwrap_or_default() == s.pass.as_bytes();
            log.lock()
                .unwrap()
                .push("(LOGIN-Antworten gelesen)".to_string());
            say(&mut r, if ok { "235 ok\r\n" } else { "535 nein\r\n" });
        } else if up.starts_with("MAIL FROM") {
            say(&mut r, "250 2.1.0 Absender ok\r\n");
        } else if up.starts_with("RCPT TO") {
            if s.reject_rcpt.iter().any(|a| line.contains(a)) {
                say(&mut r, "550 5.1.1 Empfaenger unbekannt\r\n");
            } else {
                say(&mut r, "250 2.1.5 Empfaenger ok\r\n");
            }
        } else if up == "DATA" {
            say(&mut r, "354 weiter\r\n");
            let mut body = String::new();
            loop {
                let mut l = String::new();
                if r.read_line(&mut l).unwrap_or(0) == 0 {
                    return;
                }
                if l == ".\r\n" {
                    break;
                }
                // Punkt-Verdopplung rueckgaengig machen.
                body.push_str(
                    l.strip_prefix('.')
                        .filter(|_| l.starts_with(".."))
                        .unwrap_or(&l),
                );
            }
            *message.lock().unwrap() = Some(body);
            say(
                &mut r,
                if s.reject_data {
                    "554 5.7.1 Spam\r\n"
                } else {
                    "250 2.0.0 angenommen\r\n"
                },
            );
        } else if up == "QUIT" {
            say(&mut r, "221 tschuess\r\n");
            return;
        } else {
            say(&mut r, "500 unbekannt\r\n");
        }
    }
}

fn cfg(port: u16, security: Security) -> SmtpConfig {
    SmtpConfig {
        host: "127.0.0.1".to_string(),
        port,
        security,
        username: "patrick".to_string(),
        from_address: "patrick@example.de".to_string(),
        from_name: "Patrick Wolff".to_string(),
    }
}

fn msg() -> MailMessage {
    MailMessage {
        to: vec!["kunde@example.com".to_string()],
        cc: vec!["kollegin@example.com".to_string()],
        subject: "Protokoll Wochenmeeting".to_string(),
        body_text: "Hallo\n.Punkt am Zeilenanfang\nTschuess".to_string(),
        body_html: Some("<p>Hallo</p>".to_string()),
    }
}

fn opts() -> ConnectOpts {
    ConnectOpts {
        connect_timeout: Duration::from_secs(3),
        io_timeout: Duration::from_secs(3),
        extra_root_pem: Some(CERT_PEM.to_vec()),
    }
}

fn opts_without_trust() -> ConnectOpts {
    ConnectOpts {
        extra_root_pem: None,
        ..opts()
    }
}

fn auth_lines(log: &[String]) -> Vec<&String> {
    log.iter()
        .filter(|l| l.to_uppercase().starts_with("AUTH"))
        .collect()
}

// ---------------------------------------------------------------------------
// Konfiguration und Nachricht
// ---------------------------------------------------------------------------

#[test]
fn config_round_trips_and_never_contains_a_password() {
    let c = cfg(587, Security::Starttls);
    let json = c.to_json();
    assert!(json.get("password").is_none());
    assert!(!json.to_string().contains(PASSWORD));
    let back = SmtpConfig::from_config_json(&json.to_string()).unwrap();
    assert_eq!(back, c);
    // Das Register nimmt die Konfiguration an (keine Geheimnisfelder).
    assert!(store::validate_config(&json).is_ok());
}

#[test]
fn config_validation_rejects_bad_hosts_ports_and_addresses() {
    let mut c = cfg(587, Security::Starttls);
    c.host = "".into();
    assert!(c.validate().is_err());
    c.host = "mail.example.de/pfad".into();
    assert!(c.validate().is_err());
    c.host = "user@mail.example.de".into();
    assert!(c.validate().is_err());
    let mut c = cfg(587, Security::Starttls);
    c.port = 0;
    assert!(c.validate().is_err());
    let mut c = cfg(587, Security::Starttls);
    c.from_address = "kein-at-zeichen".into();
    assert!(c.validate().is_err());
    c.from_address = "ä@example.de".into();
    assert!(c.validate().is_err());
    let mut c = cfg(587, Security::Starttls);
    c.from_name = "Zeilen\numbruch".into();
    assert!(c.validate().is_err());
    c = cfg(587, Security::Starttls);
    c.host = "mail.example.de".into();
    assert!(c.validate().is_ok());
}

#[test]
fn plain_text_is_only_allowed_towards_this_machine() {
    let mut c = cfg(25, Security::Plain);
    assert!(c.validate().is_ok());
    c.host = "localhost".into();
    assert!(c.validate().is_ok());
    c.host = "mail.example.de".into();
    assert_eq!(c.validate().unwrap_err(), SmtpError::PlainNotAllowed);
    c.host = "192.168.1.20".into();
    assert_eq!(c.validate().unwrap_err(), SmtpError::PlainNotAllowed);
}

#[test]
fn messages_with_header_injection_or_odd_recipients_are_refused() {
    let mut m = msg();
    m.to = vec!["a@example.com\r\nBcc: boese@example.com".to_string()];
    assert!(m.validate().is_err());
    let mut m = msg();
    m.to = vec!["a@example.com, b@example.com".to_string()];
    assert!(m.validate().is_err());
    let mut m = msg();
    m.to = vec!["ü@example.com".to_string()];
    assert!(m.validate().is_err());
    let mut m = msg();
    m.to = vec![];
    assert!(m.validate().is_err());
    let mut m = msg();
    m.subject = "Betreff\r\nBcc: x@example.com".to_string();
    assert!(m.validate().is_err());
    let mut m = msg();
    m.subject = "  ".to_string();
    assert!(m.validate().is_err());
    let mut m = msg();
    m.to = (0..=MAX_RECIPIENTS)
        .map(|i| format!("p{i}@example.com"))
        .collect();
    assert!(m.validate().is_err());
    let mut m = msg();
    m.body_text = " ".to_string();
    assert!(m.validate().is_err());
}

#[test]
fn the_built_message_has_the_expected_headers_and_encodes_umlauts() {
    let mut m = msg();
    m.subject = "Protokoll für Q3".to_string();
    let date = DateTime::from_timestamp(1_790_000_000, 0).unwrap();
    let (bytes, id) = build_message(&cfg(587, Security::Starttls), &m, date, 0xABCD).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.contains("From: \"Patrick Wolff\" <patrick@example.de>"),
        "{text}"
    );
    assert!(text.contains("To: <kunde@example.com>"), "{text}");
    assert!(text.contains("Cc: <kollegin@example.com>"), "{text}");
    assert!(text.contains(&format!("Message-ID: <{id}>")), "{text}");
    assert!(id.ends_with("@example.de"));
    assert!(text.contains("Date: "));
    assert!(
        text.contains("Subject: =?utf-8?"),
        "Betreff nicht kodiert: {text}"
    );
    assert!(!text.contains("Bcc"));
    assert!(text.contains("multipart/alternative"));
    assert!(text.contains("<p>Hallo</p>"));
}

#[test]
fn dot_stuffing_and_line_endings_follow_the_protocol() {
    let out = dot_stuff(b"eins\n.zwei\r\n..drei\nvier");
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "eins\r\n..zwei\r\n...drei\r\nvier\r\n"
    );
    assert_eq!(dot_stuff(b"x\r\n"), b"x\r\n");
}

// ---------------------------------------------------------------------------
// Versand gegen den Test-Server
// ---------------------------------------------------------------------------

#[test]
fn a_mail_is_delivered_with_auth_plain_and_dot_stuffing() {
    let mock = Mock::start(Script::default());
    let r = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap();
    assert_eq!(r.recipients, 2);
    let (log, message) = mock.finish();
    let message = message.expect("Nachricht angekommen");
    assert!(
        log.iter().any(|l| l == "MAIL FROM:<patrick@example.de>"),
        "{log:?}"
    );
    assert!(log.iter().any(|l| l == "RCPT TO:<kunde@example.com>"));
    assert!(log.iter().any(|l| l == "RCPT TO:<kollegin@example.com>"));
    assert!(
        message.contains("Subject: Protokoll Wochenmeeting"),
        "{message}"
    );
    // Die Zeile mit fuehrendem Punkt kam unverfaelscht an (Verdopplung auf dem Weg).
    assert!(
        message.contains("\r\n.Punkt am Zeilenanfang\r\n"),
        "{message}"
    );
    assert_eq!(auth_lines(&log).len(), 1);
    assert!(auth_lines(&log)[0].starts_with("AUTH PLAIN "));
    assert_eq!(log.last().map(String::as_str), Some("QUIT"));
}

#[test]
fn auth_login_is_the_fallback_when_plain_is_not_offered() {
    let mock = Mock::start(Script {
        caps: vec!["AUTH LOGIN"],
        ..Script::default()
    });
    send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap();
    let (log, message) = mock.finish();
    assert!(message.is_some());
    assert!(log.iter().any(|l| l == "AUTH LOGIN"));
}

#[test]
fn a_wrong_password_stops_before_any_mail_and_never_echoes_the_password() {
    let mock = Mock::start(Script::default());
    let err = send(
        &cfg(mock.port, Security::Plain),
        "falsch-123",
        &msg(),
        &opts(),
    )
    .unwrap_err();
    assert_eq!(err, SmtpError::AuthFailed);
    assert_eq!(err.code(), "smtp_auth_failed");
    assert!(!err.to_string().contains("falsch-123"));
    let (log, message) = mock.finish();
    assert!(message.is_none());
    assert!(!log.iter().any(|l| l.starts_with("MAIL FROM")), "{log:?}");
}

#[test]
fn a_rejected_recipient_is_reported_and_no_data_is_sent() {
    let mock = Mock::start(Script {
        reject_rcpt: vec!["kollegin@example.com"],
        ..Script::default()
    });
    let err = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap_err();
    match &err {
        SmtpError::Rejected { stage, text } => {
            assert_eq!(*stage, "recipient");
            assert!(text.starts_with("550"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    assert!(err.to_string().contains("Empfänger abgelehnt"));
    let (log, message) = mock.finish();
    assert!(message.is_none());
    assert!(!log.iter().any(|l| l == "DATA"));
}

#[test]
fn a_refused_message_body_is_reported() {
    let mock = Mock::start(Script {
        reject_data: true,
        ..Script::default()
    });
    let err = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap_err();
    assert!(
        matches!(err, SmtpError::Rejected { stage: "data", .. }),
        "{err:?}"
    );
}

#[test]
fn a_missing_password_fails_before_connecting_the_login() {
    let mock = Mock::start(Script::default());
    let err = send(&cfg(mock.port, Security::Plain), "", &msg(), &opts()).unwrap_err();
    assert_eq!(err, SmtpError::PasswordMissing);
    let (log, _) = mock.finish();
    assert!(auth_lines(&log).is_empty());
}

#[test]
fn the_login_is_refused_when_the_server_offers_no_known_mechanism() {
    let mock = Mock::start(Script {
        caps: vec!["AUTH CRAM-MD5"],
        ..Script::default()
    });
    let err = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap_err();
    assert_eq!(err, SmtpError::AuthUnsupported);
}

#[test]
fn starttls_without_server_support_never_sends_the_password_in_clear() {
    // Der Server bietet kein STARTTLS an: Abbruch, keine Anmeldung.
    let mock = Mock::start(Script::default());
    let err = send(
        &cfg(mock.port, Security::Starttls),
        PASSWORD,
        &msg(),
        &opts(),
    )
    .unwrap_err();
    assert_eq!(err, SmtpError::StarttlsUnsupported);
    assert!(err.to_string().contains("kein Passwort unverschlüsselt"));
    let (log, _) = mock.finish();
    assert!(
        auth_lines(&log).is_empty(),
        "Passwort ging im Klartext raus: {log:?}"
    );
    assert!(!log.iter().any(|l| l.contains(PASSWORD)));
}

#[test]
fn nothing_listening_is_a_clear_connect_error() {
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let err = send(&cfg(port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap_err();
    assert!(
        matches!(err, SmtpError::Connect(_) | SmtpError::Timeout),
        "{err:?}"
    );
    assert!(!err.to_string().contains(PASSWORD));
}

#[test]
fn a_silent_server_runs_into_the_time_limit() {
    let mock = Mock::start(Script {
        silent: true,
        ..Script::default()
    });
    let mut o = opts();
    o.io_timeout = Duration::from_millis(300);
    let err = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &o).unwrap_err();
    assert_eq!(err, SmtpError::Timeout);
    let _ = mock.finish();
}

#[test]
fn an_endless_reply_line_is_cut_off() {
    let mock = Mock::start(Script {
        long_line: true,
        ..Script::default()
    });
    let err = send(&cfg(mock.port, Security::Plain), PASSWORD, &msg(), &opts()).unwrap_err();
    assert!(matches!(err, SmtpError::Protocol(_)), "{err:?}");
    let _ = mock.finish();
}

#[test]
fn the_connection_test_logs_in_without_sending_a_mail() {
    let mock = Mock::start(Script::default());
    test(&cfg(mock.port, Security::Plain), PASSWORD, &opts()).unwrap();
    let (log, message) = mock.finish();
    assert!(message.is_none());
    assert!(!log
        .iter()
        .any(|l| l.starts_with("MAIL FROM") || l == "DATA"));
    assert_eq!(auth_lines(&log).len(), 1);
}

// ---------------------------------------------------------------------------
// Verschluesselung (TLS und STARTTLS gegen den Test-Server mit eigenem Zertifikat)
// ---------------------------------------------------------------------------

#[test]
fn implicit_tls_delivers_a_mail_over_an_encrypted_connection() {
    let mock = Mock::start(Script {
        tls: true,
        implicit_tls: true,
        ..Script::default()
    });
    let r = send(&cfg(mock.port, Security::Tls), PASSWORD, &msg(), &opts()).unwrap();
    assert_eq!(r.recipients, 2);
    let (_, message) = mock.finish();
    assert!(message.is_some());
}

#[test]
fn starttls_upgrades_before_the_login() {
    let mock = Mock::start(Script {
        tls: true,
        advertise_starttls: true,
        ..Script::default()
    });
    send(
        &cfg(mock.port, Security::Starttls),
        PASSWORD,
        &msg(),
        &opts(),
    )
    .unwrap();
    let (log, message) = mock.finish();
    assert!(message.is_some());
    let starttls = log
        .iter()
        .position(|l| l == "STARTTLS")
        .expect("STARTTLS gesendet");
    let auth = log
        .iter()
        .position(|l| l.to_uppercase().starts_with("AUTH"))
        .expect("Anmeldung");
    assert!(starttls < auth, "Anmeldung vor STARTTLS: {log:?}");
    // Nach dem Hochstufen wird neu begruesst (EHLO vor und nach STARTTLS).
    assert_eq!(log.iter().filter(|l| l.starts_with("EHLO")).count(), 2);
}

#[test]
fn an_untrusted_certificate_is_refused_and_no_password_is_sent() {
    let mock = Mock::start(Script {
        tls: true,
        implicit_tls: true,
        ..Script::default()
    });
    let err = send(
        &cfg(mock.port, Security::Tls),
        PASSWORD,
        &msg(),
        &opts_without_trust(),
    )
    .unwrap_err();
    assert!(matches!(err, SmtpError::Tls(_)), "{err:?}");
    assert_eq!(err.code(), "smtp_tls_failed");
    let (log, _) = mock.finish();
    assert!(
        log.is_empty(),
        "Der Server sah Daten trotz ungeprueftem Zertifikat: {log:?}"
    );
}

#[test]
fn data_injected_before_the_tls_handshake_aborts_the_connection() {
    let mock = Mock::start(Script {
        tls: true,
        advertise_starttls: true,
        inject_after_starttls: true,
        ..Script::default()
    });
    let err = send(
        &cfg(mock.port, Security::Starttls),
        PASSWORD,
        &msg(),
        &opts(),
    )
    .unwrap_err();
    assert!(matches!(err, SmtpError::Tls(_)), "{err:?}");
    let (log, _) = mock.finish();
    assert!(auth_lines(&log).is_empty());
}

// ---------------------------------------------------------------------------
// Durch das Tor: Rechte, Freigabe, Audit
// ---------------------------------------------------------------------------

fn smtp_integration(fx: &Fx, port: u16) -> String {
    let conn = fx.conn();
    let mut n = NewIntegration::new(Kind::Smtp, "Mein Postfach");
    n.config = cfg(port, Security::Plain).to_json();
    store::create(&conn, &n, 1_000).unwrap().id
}

fn password(_: &Integration, slot: &str) -> Result<Option<Zeroizing<String>>, String> {
    assert_eq!(slot, "password");
    Ok(Some(Zeroizing::new(PASSWORD.to_string())))
}

use crate::managers::integrations::model::Integration;

#[test]
fn the_user_sends_without_approval_and_the_action_is_audited() {
    let fx = Fx::new();
    let mock = Mock::start(Script::default());
    let id = smtp_integration(&fx, mock.port);
    let conn = fx.conn();
    let out = targets::send_mail(
        &conn,
        Caller::User,
        &id,
        &msg(),
        None,
        &password,
        &opts(),
        5_000,
    )
    .unwrap();
    assert!(matches!(out, GateOutcome::Done(_)), "{out:?}");
    let (_, message) = mock.finish();
    assert!(message.is_some());
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].caller, "user");
    assert_eq!(rows[0].capability.as_deref(), Some("mail.send"));
    assert_eq!(rows[0].outcome, "ok");
    assert_eq!(rows[0].target.as_deref(), Some("kunde@example.com (+1)"));
    // Weder Passwort noch Text noch Betreff stehen im Protokoll.
    let dump = format!("{rows:?}");
    assert!(!dump.contains(PASSWORD));
    assert!(!dump.contains("Punkt am Zeilenanfang"));
    // Der Eintrag zeigt den Erfolg.
    assert!(store::get(&conn, &id)
        .unwrap()
        .unwrap()
        .last_ok_at
        .is_some());
}

#[test]
fn a_workflow_needs_an_approval_and_the_approval_is_bound_to_that_mail() {
    let fx = Fx::new();
    let mock = Mock::start(Script::default());
    let id = smtp_integration(&fx, mock.port);
    let conn = fx.conn();
    let pending = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &msg(),
        None,
        &password,
        &opts(),
        5_000,
    )
    .unwrap();
    let GateOutcome::Pending { approval_id } = pending else {
        panic!("{pending:?}");
    };
    // Die Vorschau zeigt beide Empfaenger vollstaendig.
    let approval = crate::managers::integrations::approvals::get(&conn, &approval_id)
        .unwrap()
        .unwrap();
    let preview = approval.args_preview.unwrap();
    assert!(
        preview.contains("kunde@example.com") && preview.contains("kollegin@example.com"),
        "{preview}"
    );

    crate::managers::integrations::approvals::decide(&conn, &approval_id, true, 6_000).unwrap();
    // Eine andere Mail mit derselben Freigabe: abgelehnt, es geht nichts raus.
    let mut other = msg();
    other.to = vec!["fremder@example.org".to_string()];
    let denied = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &other,
        Some(&approval_id),
        &password,
        &opts(),
        7_000,
    )
    .unwrap();
    assert!(
        matches!(
            denied,
            GateOutcome::Denied {
                code: "approval_mismatch",
                ..
            }
        ),
        "{denied:?}"
    );
    // Die genehmigte Mail geht.
    let done = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &msg(),
        Some(&approval_id),
        &password,
        &opts(),
        8_000,
    )
    .unwrap();
    assert!(matches!(done, GateOutcome::Done(_)), "{done:?}");
    let (log, message) = mock.finish();
    assert!(message.is_some());
    assert!(!log.iter().any(|l| l.contains("fremder@example.org")));
    // Zweites Einloesen derselben Freigabe: nicht moeglich.
    let again = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &msg(),
        Some(&approval_id),
        &password,
        &opts(),
        9_000,
    )
    .unwrap();
    assert!(matches!(again, GateOutcome::Denied { .. }), "{again:?}");
}

#[test]
fn an_external_agent_is_denied_by_default_and_nothing_connects() {
    let fx = Fx::new();
    let mock = Mock::start(Script::default());
    let id = smtp_integration(&fx, mock.port);
    let conn = fx.conn();
    let out = targets::send_mail(
        &conn,
        Caller::AgentExternal,
        &id,
        &msg(),
        None,
        &password,
        &opts(),
        5_000,
    )
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "grant_off",
                ..
            }
        ),
        "{out:?}"
    );
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    assert_eq!(rows[0].outcome, "denied");
    // Der Server hat nie eine Verbindung gesehen: Mock beenden, ohne dass etwas ankam.
    drop(TcpStream::connect(("127.0.0.1", mock.port)));
    let (log, message) = mock.finish();
    assert!(log.is_empty() && message.is_none());
}

#[test]
fn a_switched_off_integration_denies_even_the_user() {
    let fx = Fx::new();
    let id = smtp_integration(&fx, 1);
    let conn = fx.conn();
    store::update(
        &conn,
        &id,
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        2_000,
    )
    .unwrap();
    let out = targets::send_mail(
        &conn,
        Caller::User,
        &id,
        &msg(),
        None,
        &password,
        &opts(),
        5_000,
    )
    .unwrap();
    assert!(
        matches!(
            out,
            GateOutcome::Denied {
                code: "integration_disabled",
                ..
            }
        ),
        "{out:?}"
    );
}

#[test]
fn a_failed_send_is_audited_with_a_clean_reason_and_marks_the_entry() {
    let fx = Fx::new();
    let mock = Mock::start(Script::default());
    let id = smtp_integration(&fx, mock.port);
    let conn = fx.conn();
    let wrong = |_: &Integration, _: &str| Ok(Some(Zeroizing::new("falsch-123".to_string())));
    let out = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &msg(),
        None,
        &wrong,
        &opts(),
        5_000,
    )
    .unwrap();
    let GateOutcome::Pending { approval_id } = out else {
        panic!()
    };
    crate::managers::integrations::approvals::decide(&conn, &approval_id, true, 5_500).unwrap();
    let out = targets::send_mail(
        &conn,
        Caller::Workflow,
        &id,
        &msg(),
        Some(&approval_id),
        &wrong,
        &opts(),
        6_000,
    )
    .unwrap();
    assert!(matches!(out, GateOutcome::Failed(_)), "{out:?}");
    let rows = audit::list(&conn, &audit::AuditFilter::default(), 50).unwrap();
    let last = rows
        .iter()
        .find(|r| r.outcome == "error")
        .expect("Fehlereintrag");
    assert!(last.detail_json.as_deref().unwrap().contains("Anmeldung"));
    assert!(!format!("{rows:?}").contains("falsch-123"));
    assert!(store::get(&conn, &id)
        .unwrap()
        .unwrap()
        .last_error
        .is_some());
    let _ = mock.finish();
}

#[test]
fn a_missing_password_in_the_store_is_a_clear_failure() {
    let fx = Fx::new();
    let id = smtp_integration(&fx, 1);
    let conn = fx.conn();
    let none = |_: &Integration, _: &str| Ok(None);
    let out = targets::send_mail(
        &conn,
        Caller::User,
        &id,
        &msg(),
        None,
        &none,
        &opts(),
        5_000,
    )
    .unwrap();
    let GateOutcome::Failed(text) = out else {
        panic!("{out:?}")
    };
    assert!(text.contains("Passwort fehlt"), "{text}");
}

#[test]
fn the_test_command_marks_success_and_failure_at_the_entry() {
    let fx = Fx::new();
    let mock = Mock::start(Script::default());
    let id = smtp_integration(&fx, mock.port);
    let conn = fx.conn();
    let ok =
        targets::test_target(&conn, &id, &password, &opts(), &Default::default(), 5_000).unwrap();
    assert!(ok.ok);
    assert_eq!(ok.code, "smtp_ok");
    let _ = mock.finish();
    assert!(store::get(&conn, &id)
        .unwrap()
        .unwrap()
        .last_ok_at
        .is_some());

    let bad =
        targets::test_target(&conn, &id, &password, &opts(), &Default::default(), 6_000).unwrap();
    assert!(!bad.ok);
    assert!(bad.code.starts_with("smtp_"));
    assert!(bad.detail.is_some());
    assert!(store::get(&conn, &id)
        .unwrap()
        .unwrap()
        .last_error
        .is_some());
}
