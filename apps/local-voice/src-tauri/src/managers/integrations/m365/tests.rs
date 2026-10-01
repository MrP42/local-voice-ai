//! Tests des Microsoft-365-Kontos (A5). Kein Netz, kein Browser, keine Fenster:
//! Anmelde-Server und Graph sind ein lokaler Test-Server (`test_server`), der
//! „Browser“ ist ein Roh-TCP-Aufruf an den Loopback-Listener.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::TimeZone;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::account::{StoredAccount, Vault};
use super::actions;
use super::config::{scope_of, M365Config, BASE_SCOPES};
use super::drive::{
    clean_folder, clean_segment, Conflict, UploadRequest, UploadSource, UploadVia, CHUNK,
    SIMPLE_MAX,
};
use super::error::M365Error;
use super::event::{EventRef, NoteOutcome};
use super::mail::{
    message_from_draft, MailAttachment, MailBody, MailMessage, MAX_ATTACHMENTS, MAX_ATTACHMENT_BYTES,
};
use super::service::{Acct, M365Service};
use super::status::{status_of, AccountState};
use super::test_server::{serve, Req, Resp, Seen};
use super::FilesMode;
use crate::managers::calendar::graph::Endpoints;
use crate::managers::integrations::gate::GateOutcome;
use crate::managers::integrations::model::{
    Caller, Capability, GrantMode, Integration, Kind, NewIntegration,
};
use crate::managers::integrations::test_support::Fx;
use crate::managers::integrations::{approvals, audit, grants, store};
use crate::managers::meetings::mail::MailDraft;
use crate::managers::meetings::store::MeetingStore;

const CLIENT: &str = "11111111-2222-3333-4444-555555555555";
const OTHER_CLIENT: &str = "99999999-2222-3333-4444-555555555555";
const RT: &str = "RT-geheim-0001";

const MAIL: Capability = Capability::MailSend;
const FILES: Capability = Capability::FilesWrite;
const CAL: Capability = Capability::CalendarWrite;

// ---------------------------------------------------------------------------
// Hilfen
// ---------------------------------------------------------------------------

fn eps(base: &str) -> Endpoints {
    Endpoints {
        authority: base.to_string(),
        graph: format!("{base}/v1.0"),
        use_env_proxy: false,
    }
}

struct World {
    store: Arc<MeetingStore>,
    vault: Vault,
    svc: Arc<M365Service>,
    id: String,
    dir: PathBuf,
    _tmp: tempfile::TempDir,
}

impl World {
    fn new(base: &str, caps: &[Capability], signed_in: bool) -> Self {
        Self::with(eps(base), caps, FilesMode::Full, signed_in)
    }

    fn with(ep: Endpoints, caps: &[Capability], mode: FilesMode, signed_in: bool) -> Self {
        let fx = Fx::new();
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("secrets");
        let vault = Vault::in_dir(dir.clone());
        let cfg = M365Config::new(CLIENT, "common", caps, mode, "Local Voice AI").unwrap();
        let mut n = NewIntegration::new(Kind::M365, "Mein Konto");
        n.config = cfg.to_json();
        let integ = store::create(&fx.conn(), &n, 1_000).unwrap();
        let mut svc = M365Service::new(ep, vault.clone());
        svc.retry_pause = Duration::from_millis(5);
        let w = Self {
            store: Arc::new(fx.store),
            vault,
            svc: Arc::new(svc),
            id: integ.id.clone(),
            dir,
            _tmp: tmp,
        };
        if signed_in {
            w.sign_in_directly(RT);
        }
        w
    }

    fn integration(&self) -> Integration {
        store::get(&self.store.get_connection().unwrap(), &self.id)
            .unwrap()
            .unwrap()
    }

    fn acct(&self) -> Acct {
        Acct::from_integration(self.integration()).unwrap()
    }

    fn cfg(&self) -> M365Config {
        self.acct().cfg
    }

    /// Legt ein Konto so ab, wie es nach der Anmeldung dort liegt.
    fn sign_in_directly(&self, refresh: &str) {
        let cfg = self.cfg();
        let account = StoredAccount::new(
            cfg.client_id.clone(),
            cfg.tenant.clone(),
            cfg.scope_string(),
            refresh.to_string(),
            Some("ich@example.com".to_string()),
            Some("Ich".to_string()),
        );
        self.vault.save(&self.integration(), &account).unwrap();
    }

    /// Aendert die Konfiguration (Faehigkeiten, Client-ID) wie die Oberflaeche.
    fn reconfigure(&self, client: &str, caps: &[Capability]) {
        let old = self.cfg();
        let cfg =
            M365Config::new(client, &old.tenant, caps, old.files_mode, &old.files_folder).unwrap();
        let patch = crate::managers::integrations::model::IntegrationPatch {
            config: Some(cfg.to_json()),
            ..Default::default()
        };
        store::update(
            &self.store.get_connection().unwrap(),
            &self.id,
            &patch,
            2_000,
        )
        .unwrap();
    }

    fn audit_rows(&self) -> Vec<crate::managers::integrations::model::AuditEntry> {
        audit::list(
            &self.store.get_connection().unwrap(),
            &audit::AuditFilter::default(),
            500,
        )
        .unwrap()
    }
}

fn token_resp(at: &str, rt: Option<&str>) -> Resp {
    let mut v = json!({ "access_token": at, "expires_in": 3600, "token_type": "Bearer" });
    if let Some(rt) = rt {
        v["refresh_token"] = json!(rt);
    }
    Resp::json(200, v)
}

fn is_token(req: &Req) -> bool {
    req.method == "POST" && req.path().ends_with("/oauth2/v2.0/token")
}

fn count(seen: &Seen, pred: impl Fn(&Req) -> bool) -> usize {
    seen.lock().unwrap().iter().filter(|r| pred(r)).count()
}

fn count_path(seen: &Seen, method: &str, path: &str) -> usize {
    count(seen, |r| r.method == method && r.path() == path)
}

fn msg(to: &str) -> MailMessage {
    MailMessage::new(
        &[to.to_string()],
        &[],
        "Follow-up Planung",
        MailBody::Text("Hallo,\nhier die Beschlüsse.".to_string()),
    )
    .unwrap()
}

fn ok_mail_server(tokens: Arc<AtomicUsize>) -> impl Fn(&Req) -> Resp + Send + Sync + 'static {
    move |req| {
        if is_token(req) {
            let n = tokens.fetch_add(1, Ordering::SeqCst) + 1;
            return token_resp(&format!("AT-{n}"), None);
        }
        if req.method == "POST" && req.path() == "/v1.0/me/sendMail" {
            return Resp::empty(202);
        }
        Resp::empty(404)
    }
}

async fn raw_get(port: u16, target: &str, host: &str) {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(
        format!("GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out).await;
}

/// Der „Browser“: ruft die Weiterleitung mit Code und `state` auf und merkt sich
/// die Anmelde-Adresse.
fn fake_browser(opened: Arc<Mutex<Option<String>>>) -> impl FnOnce(String) -> Result<(), String> {
    move |url: String| {
        let q: HashMap<String, String> = url::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        *opened.lock().unwrap() = Some(url.clone());
        let port: u16 = q["redirect_uri"]
            .rsplit(':')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let state = q["state"].clone();
        tokio::spawn(async move {
            raw_get(
                port,
                &format!("/?code=auth-code-1&state={state}"),
                &format!("localhost:{port}"),
            )
            .await;
        });
        Ok(())
    }
}

fn query_of(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

fn me_ok() -> Resp {
    Resp::json(
        200,
        json!({ "mail": "Ich@Example.com", "displayName": "Ich Selbst" }),
    )
}

// ---------------------------------------------------------------------------
// Konfiguration und Scopes je Faehigkeit
// ---------------------------------------------------------------------------

#[test]
fn scopes_follow_the_enabled_capabilities() {
    let mail_only = M365Config::new(CLIENT, "", &[MAIL], FilesMode::Full, "").unwrap();
    assert_eq!(
        mail_only.scope_string(),
        "offline_access User.Read Mail.Send"
    );
    let files_only = M365Config::new(CLIENT, "", &[FILES], FilesMode::Full, "").unwrap();
    assert_eq!(
        files_only.scope_string(),
        "offline_access User.Read Files.ReadWrite"
    );
    let cal_only = M365Config::new(CLIENT, "", &[CAL], FilesMode::Full, "").unwrap();
    assert_eq!(
        cal_only.scope_string(),
        "offline_access User.Read Calendars.ReadWrite"
    );
    // Reihenfolge der Eingabe ist egal, Dubletten fallen weg.
    let all = M365Config::new(CLIENT, "", &[CAL, MAIL, FILES, MAIL], FilesMode::Full, "").unwrap();
    assert_eq!(
        all.scope_string(),
        "offline_access User.Read Mail.Send Files.ReadWrite Calendars.ReadWrite"
    );
}

#[test]
fn no_capability_requests_only_the_base_scopes() {
    let none = M365Config::new(CLIENT, "", &[], FilesMode::Full, "").unwrap();
    assert_eq!(none.required_scopes(), BASE_SCOPES.to_vec());
    for scope in none.required_scopes() {
        assert!(
            !scope.starts_with("Mail.")
                && !scope.starts_with("Files.")
                && !scope.starts_with("Calendars."),
            "{scope}"
        );
    }
}

#[test]
fn the_app_folder_mode_asks_for_the_app_folder_scope_only() {
    let cfg = M365Config::new(CLIENT, "consumers", &[FILES], FilesMode::AppFolder, "").unwrap();
    assert_eq!(
        cfg.scope_string(),
        "offline_access User.Read Files.ReadWrite.AppFolder"
    );
    assert_eq!(scope_of(FILES, FilesMode::Full), Some("Files.ReadWrite"));
    assert_eq!(
        scope_of(FILES, FilesMode::AppFolder),
        Some("Files.ReadWrite.AppFolder")
    );
}

#[test]
fn capabilities_the_account_does_not_implement_are_refused() {
    for cap in [
        Capability::CalendarRead,
        Capability::FilesRead,
        Capability::YoutubeAdd,
        Capability::RecordingStart,
    ] {
        let r = M365Config::new(CLIENT, "", &[cap], FilesMode::Full, "");
        assert!(matches!(r, Err(M365Error::Invalid(_))), "{cap:?}");
        assert_eq!(scope_of(cap, FilesMode::Full), None);
    }
}

#[test]
fn config_round_trips_and_the_defaults_are_safe() {
    let cfg = M365Config::new(
        CLIENT,
        "Contoso.com",
        &[MAIL, FILES],
        FilesMode::Full,
        "Berichte/2026",
    )
    .unwrap();
    let again = M365Config::from_json(&cfg.to_json().to_string()).unwrap();
    assert_eq!(cfg, again);
    assert_eq!(again.tenant, "contoso.com");
    // Eine leere Konfiguration schaltet nichts ein und braucht eine Client-ID.
    let empty = M365Config::from_json("{}").unwrap();
    assert!(empty.capabilities.is_empty());
    assert!(empty.client_id.is_empty());
    assert_eq!(empty.tenant, "common");
    assert!(M365Config::from_json("kein json").is_err());
}

#[test]
fn the_client_id_must_be_a_guid_and_the_config_has_no_secret() {
    assert!(M365Config::new("nicht-gueltig", "", &[], FilesMode::Full, "").is_err());
    assert!(M365Config::new("", "", &[], FilesMode::Full, "").is_ok());
    // Die Registry lehnt Geheimnisse in der Konfiguration ab: diese hier besteht.
    let cfg = M365Config::new(CLIENT, "", &[MAIL, FILES, CAL], FilesMode::Full, "Ablage").unwrap();
    store::validate_config(&cfg.to_json()).unwrap();
}

// ---------------------------------------------------------------------------
// Rechte: eingeschaltete Faehigkeiten
// ---------------------------------------------------------------------------

#[test]
fn a_capability_that_is_not_enabled_is_off_even_for_the_user() {
    let w = World::new("http://127.0.0.1:1", &[MAIL], false);
    let i = w.integration();
    let none = HashMap::new();
    let (mode, reason) = grants::explain(&i, MAIL, Caller::User, &none, None);
    assert_eq!(mode, GrantMode::Allow);
    assert!(reason.is_none());
    for cap in [FILES, CAL, Capability::CalendarRead, Capability::FilesRead] {
        for caller in [Caller::User, Caller::Workflow, Caller::AgentExternal] {
            let (mode, reason) = grants::explain(&i, cap, caller, &none, None);
            assert_eq!(mode, GrantMode::Off, "{cap:?} {caller:?}");
            assert_eq!(reason, Some(grants::OffReason::CapabilityNotEnabled));
        }
    }
}

#[test]
fn without_the_key_everything_the_kind_offers_stays_enabled() {
    let fx = Fx::new();
    let folder = crate::managers::integrations::test_support::folder(&fx.conn(), "Ablage");
    assert!(grants::capability_enabled(&folder, Capability::FilesWrite));
    // Mit kaputtem Wert ist NICHTS eingeschaltet (fail closed).
    let mut broken = folder.clone();
    broken.config_json = r#"{"enabled_capabilities":"alle"}"#.to_string();
    assert!(!grants::capability_enabled(&broken, Capability::FilesWrite));
    broken.config_json = r#"{"enabled_capabilities": [oops"#.to_string();
    assert!(!grants::capability_enabled(&broken, Capability::FilesWrite));
}

// ---------------------------------------------------------------------------
// Anmeldung
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sign_in_asks_only_for_the_enabled_scopes_and_stores_the_account() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", Some("RT-neu"));
        }
        if req.method == "GET" && req.path() == "/v1.0/me" {
            return me_ok();
        }
        Resp::empty(404)
    })
    .await;
    let w = World::new(&base, &[MAIL, FILES], false);
    let opened = Arc::new(Mutex::new(None));
    let info = w
        .svc
        .sign_in(
            &w.acct(),
            fake_browser(opened.clone()),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert_eq!(info.address, "ich@example.com");
    assert_eq!(info.name.as_deref(), Some("Ich Selbst"));

    let url = opened.lock().unwrap().clone().unwrap();
    let q = query_of(&url);
    assert_eq!(
        q["scope"],
        "offline_access User.Read Mail.Send Files.ReadWrite"
    );
    assert!(!q["scope"].contains("Calendars"));
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(q["redirect_uri"].starts_with("http://localhost:"));
    assert_eq!(q["client_id"], CLIENT);
    assert!(url.starts_with(&format!("{base}/common/oauth2/v2.0/authorize?")));

    // Token-Anfrage: dieselben Scopes, PKCE-Beweis, kein Client-Geheimnis.
    let token_req = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| is_token(r))
        .cloned()
        .unwrap();
    let form = token_req.form();
    assert_eq!(form["scope"], q["scope"]);
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["redirect_uri"], q["redirect_uri"]);
    assert!(form.contains_key("code_verifier"));
    assert!(!form.contains_key("client_secret"));
    // Profil mit dem frischen Zugriffstoken.
    let me_req = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path() == "/v1.0/me")
        .cloned()
        .unwrap();
    assert_eq!(me_req.bearer(), Some("AT-1"));

    // Das Konto liegt verschluesselt im Geheimnisspeicher, mit den Scopes.
    let account = w.vault.load(&w.integration()).unwrap().unwrap();
    assert_eq!(account.refresh_token(), "RT-neu");
    assert_eq!(account.scope, q["scope"]);
    assert_eq!(account.client_id, CLIENT);
    let s = status_of(&w.integration(), &w.vault, false).unwrap();
    assert_eq!(s.state, AccountState::Ready);
    assert_eq!(s.account.as_deref(), Some("ich@example.com"));
}

#[tokio::test]
async fn sign_in_needs_a_client_id_and_a_capability_before_any_network() {
    let (base, seen) = serve(|_| Resp::empty(500)).await;
    let w = World::new(&base, &[], false);
    let r = w
        .svc
        .sign_in(&w.acct(), |_| Ok(()), Duration::from_secs(1))
        .await;
    assert_eq!(r, Err(M365Error::NoCapability));
    w.reconfigure("", &[MAIL]);
    let r = w
        .svc
        .sign_in(&w.acct(), |_| Ok(()), Duration::from_secs(1))
        .await;
    assert_eq!(r, Err(M365Error::NotConfigured));
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_second_sign_in_is_refused_while_one_runs_and_cancel_ends_it_cleanly() {
    let (base, seen) = serve(|_| Resp::empty(500)).await;
    let w = World::new(&base, &[MAIL], false);
    let (svc, a) = (w.svc.clone(), w.acct());
    let first =
        tokio::spawn(async move { svc.sign_in(&a, |_| Ok(()), Duration::from_secs(30)).await });
    for _ in 0..200 {
        if w.svc.signing_in() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(w.svc.signing_in());
    let second = w
        .svc
        .sign_in(&w.acct(), |_| Ok(()), Duration::from_secs(1))
        .await;
    assert!(matches!(second, Err(M365Error::Config(_))), "{second:?}");
    assert!(w.svc.cancel_sign_in());
    assert_eq!(first.await.unwrap(), Err(M365Error::Cancelled));
    assert!(!w.svc.signing_in());
    // Nichts geschrieben, kein Token-Austausch.
    assert!(w.vault.load(&w.integration()).unwrap().is_none());
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sign_in_without_a_browser_or_after_the_timeout_leaves_no_token() {
    let (base, _seen) = serve(|_| Resp::empty(500)).await;
    let w = World::new(&base, &[MAIL], false);
    let r = w
        .svc
        .sign_in(
            &w.acct(),
            |_| Err("kein Browser".to_string()),
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(r, Err(M365Error::Browser(_))), "{r:?}");
    let r = w
        .svc
        .sign_in(&w.acct(), |_| Ok(()), Duration::from_millis(150))
        .await;
    assert_eq!(r, Err(M365Error::SignInTimeout));
    assert!(w.vault.load(&w.integration()).unwrap().is_none());
    assert!(!w.svc.signing_in());
}

#[tokio::test]
async fn a_failing_profile_after_the_token_stores_nothing() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", Some("RT-neu"));
        }
        Resp::empty(500)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let before = w.vault.load(&w.integration()).unwrap().unwrap();
    let r = w
        .svc
        .sign_in(
            &w.acct(),
            fake_browser(Default::default()),
            Duration::from_secs(10),
        )
        .await;
    assert!(r.is_err(), "{r:?}");
    // Der alte Stand bleibt: alles oder nichts.
    let after = w.vault.load(&w.integration()).unwrap().unwrap();
    assert_eq!(after.refresh_token(), before.refresh_token());
}

#[tokio::test]
async fn a_failed_token_write_during_sign_in_keeps_the_old_token() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", Some("RT-neu"));
        }
        me_ok()
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    w.vault.fail_writes.store(true, Ordering::SeqCst);
    let r = w
        .svc
        .sign_in(
            &w.acct(),
            fake_browser(Default::default()),
            Duration::from_secs(10),
        )
        .await;
    assert!(matches!(r, Err(M365Error::Store(_))), "{r:?}");
    w.vault.fail_writes.store(false, Ordering::SeqCst);
    assert_eq!(
        w.vault
            .load(&w.integration())
            .unwrap()
            .unwrap()
            .refresh_token(),
        RT
    );
}

// ---------------------------------------------------------------------------
// Token: Erneuern, 401, Zustimmung
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_refresh_asks_only_for_the_scopes_that_are_still_enabled() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL, FILES, CAL], true);
    // Der Nutzer schaltet Dateien und Termine wieder aus.
    w.reconfigure(CLIENT, &[MAIL]);
    w.svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap();
    let token_req = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| is_token(r))
        .cloned()
        .unwrap();
    let form = token_req.form();
    assert_eq!(form["scope"], "offline_access User.Read Mail.Send");
    assert_eq!(form["grant_type"], "refresh_token");
    assert_eq!(form["refresh_token"], RT);
    assert_eq!(form["client_id"], CLIENT);
}

#[tokio::test]
async fn a_refresh_rotates_the_refresh_token() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", Some("RT-rotiert"));
        }
        Resp::empty(202)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    w.svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap();
    let account = w.vault.load(&w.integration()).unwrap().unwrap();
    assert_eq!(account.refresh_token(), "RT-rotiert");
    assert_eq!(account.client_id, CLIENT);
    assert_eq!(account.address.as_deref(), Some("ich@example.com"));
}

#[tokio::test]
async fn an_invalid_refresh_token_deletes_the_dead_token_and_asks_to_sign_in_again() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return Resp::json(
                400,
                json!({"error": "invalid_grant", "error_description": "AADSTS70008: abgelaufen\r\nTrace ID: abc"}),
            );
        }
        Resp::empty(202)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let r = w.svc.send_mail(&w.acct(), &msg("a@example.com")).await;
    assert_eq!(r, Err(M365Error::NeedsSignIn));
    assert_eq!(r.unwrap_err().wire(), "m365_needs_sign_in");
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 0);
    // Das tote Token ist weg: Zustand „neu anmelden“.
    assert!(w.vault.load(&w.integration()).unwrap().is_none());
    let s = status_of(&w.integration(), &w.vault, false).unwrap();
    assert_eq!(s.state, AccountState::NeedsSignIn);
    assert_eq!(s.secret, "missing");
    // Ein zweiter Aufruf geht gar nicht erst ans Netz.
    let hits = seen.lock().unwrap().len();
    assert_eq!(
        w.svc.send_mail(&w.acct(), &msg("a@example.com")).await,
        Err(M365Error::NeedsSignIn)
    );
    assert_eq!(seen.lock().unwrap().len(), hits);
}

#[tokio::test]
async fn enabling_a_capability_after_the_sign_in_needs_consent_without_calling_the_network() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    w.reconfigure(CLIENT, &[MAIL, FILES]);
    let r = w.svc.send_mail(&w.acct(), &msg("a@example.com")).await;
    assert_eq!(
        r,
        Err(M365Error::NeedsConsent {
            missing: vec!["Files.ReadWrite".to_string()]
        })
    );
    assert!(seen.lock().unwrap().is_empty());
    let s = status_of(&w.integration(), &w.vault, false).unwrap();
    assert_eq!(s.state, AccountState::NeedsConsent);
    assert_eq!(s.missing_scopes, vec!["Files.ReadWrite".to_string()]);
    assert_eq!(s.granted_scopes.len(), 3);
}

#[tokio::test]
async fn a_missing_or_broken_secret_needs_a_sign_in_and_never_works_with_an_empty_value() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], false);
    assert_eq!(
        w.svc.send_mail(&w.acct(), &msg("a@example.com")).await,
        Err(M365Error::NeedsSignIn)
    );
    // Eine kaputte Datei (anderer Benutzer, beschaedigt) ist dasselbe.
    std::fs::create_dir_all(&w.dir).unwrap();
    std::fs::write(w.dir.join(format!("int-{}-token.bin", w.id)), b"LVS1kaputt").unwrap();
    assert_eq!(
        w.svc.send_mail(&w.acct(), &msg("a@example.com")).await,
        Err(M365Error::NeedsSignIn)
    );
    let s = status_of(&w.integration(), &w.vault, false).unwrap();
    assert_eq!(s.state, AccountState::NeedsSignIn);
    assert_eq!(s.secret, "broken");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_changed_client_id_needs_a_new_sign_in() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    w.reconfigure(OTHER_CLIENT, &[MAIL]);
    assert_eq!(
        w.svc.send_mail(&w.acct(), &msg("a@example.com")).await,
        Err(M365Error::NeedsSignIn)
    );
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(
        status_of(&w.integration(), &w.vault, false).unwrap().state,
        AccountState::NeedsSignIn
    );
}

#[tokio::test]
async fn a_failed_token_write_keeps_the_old_token_and_the_call_goes_on() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", Some("RT-rotiert"));
        }
        Resp::empty(202)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    w.vault.fail_writes.store(true, Ordering::SeqCst);
    // Volle Platte beim Ablegen des neuen Erneuerungs-Tokens: die Mail geht trotzdem.
    w.svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap();
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
    w.vault.fail_writes.store(false, Ordering::SeqCst);
    assert_eq!(
        w.vault
            .load(&w.integration())
            .unwrap()
            .unwrap()
            .refresh_token(),
        RT,
        "das alte Token bleibt gueltig stehen"
    );
}

#[tokio::test]
async fn a_401_refreshes_once_and_retries_with_the_new_token() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let t2 = tokens.clone();
    let (base, seen) = serve(move |req| {
        if is_token(req) {
            let n = t2.fetch_add(1, Ordering::SeqCst) + 1;
            return token_resp(&format!("AT-{n}"), None);
        }
        if req.path() == "/v1.0/me/sendMail" {
            return if req.bearer() == Some("AT-1") {
                Resp::json(
                    401,
                    json!({"error": {"code": "InvalidAuthenticationToken"}}),
                )
            } else {
                Resp::empty(202)
            };
        }
        Resp::empty(404)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    w.svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap();
    assert_eq!(
        tokens.load(Ordering::SeqCst),
        2,
        "ein Erneuern am Anfang, eines nach dem 401"
    );
    let mails: Vec<Req> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path() == "/v1.0/me/sendMail")
        .cloned()
        .collect();
    assert_eq!(mails.len(), 2);
    assert_eq!(mails[0].bearer(), Some("AT-1"));
    assert_eq!(mails[1].bearer(), Some("AT-2"));
}

#[tokio::test]
async fn a_second_401_after_the_refresh_needs_a_sign_in_and_stops() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let t2 = tokens.clone();
    let (base, seen) = serve(move |req| {
        if is_token(req) {
            let n = t2.fetch_add(1, Ordering::SeqCst) + 1;
            return token_resp(&format!("AT-{n}"), None);
        }
        Resp::json(
            401,
            json!({"error": {"code": "InvalidAuthenticationToken"}}),
        )
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let r = w.svc.send_mail(&w.acct(), &msg("a@example.com")).await;
    assert_eq!(r, Err(M365Error::NeedsSignIn));
    assert_eq!(
        count_path(&seen, "POST", "/v1.0/me/sendMail"),
        2,
        "kein dritter Versuch"
    );
    assert_eq!(tokens.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn concurrent_401s_refresh_only_once() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let t2 = tokens.clone();
    let (base, seen) = serve(move |req| {
        if is_token(req) {
            let n = t2.fetch_add(1, Ordering::SeqCst) + 1;
            return token_resp(&format!("AT-{n}"), None);
        }
        if req.path() == "/v1.0/me/sendMail" {
            return if req.bearer() == Some("AT-1") {
                Resp::json(
                    401,
                    json!({"error": {"code": "InvalidAuthenticationToken"}}),
                )
            } else {
                Resp::empty(202)
            };
        }
        Resp::empty(404)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let a = w.acct();
    let m = msg("a@example.com");
    let results = futures_util::future::join_all((0..4).map(|_| w.svc.send_mail(&a, &m))).await;
    assert!(results.iter().all(|r| r.is_ok()), "{results:?}");
    assert_eq!(
        tokens.load(Ordering::SeqCst),
        2,
        "vier gleichzeitige Aufrufe, zwei Anfragen am Token-Endpunkt (Anfang und nach dem 401)"
    );
    assert_eq!(
        count(&seen, |r| r.path() == "/v1.0/me/sendMail"
            && r.bearer() == Some("AT-2")),
        4
    );
}

// ---------------------------------------------------------------------------
// Mail
// ---------------------------------------------------------------------------

#[tokio::test]
async fn send_mail_posts_the_documented_body_with_the_bearer_token() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let m = MailMessage::new(
        &[
            "Anna@Example.com".to_string(),
            "anna@example.com".to_string(),
        ],
        &["chef@example.com".to_string()],
        "Follow-up:\nPlanung",
        MailBody::Html("<p>Hallo</p>".to_string()),
    )
    .unwrap();
    w.svc.send_mail(&w.acct(), &m).await.unwrap();
    let req = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path() == "/v1.0/me/sendMail")
        .cloned()
        .unwrap();
    assert_eq!(req.bearer(), Some("AT-1"));
    assert!(req
        .header("content-type")
        .unwrap()
        .starts_with("application/json"));
    assert_eq!(
        req.json(),
        json!({
            "message": {
                "subject": "Follow-up: Planung",
                "body": { "contentType": "HTML", "content": "<p>Hallo</p>" },
                "toRecipients": [ { "emailAddress": { "address": "Anna@Example.com" } } ],
                "ccRecipients": [ { "emailAddress": { "address": "chef@example.com" } } ]
            },
            "saveToSentItems": true
        })
    );
}

#[test]
fn a_text_message_uses_the_text_content_type_and_no_cc_key() {
    let v = msg("a@example.com").to_graph_json();
    assert_eq!(v["message"]["body"]["contentType"], "Text");
    assert!(v["message"].get("ccRecipients").is_none());
}

#[tokio::test]
async fn a_mail_is_checked_before_the_network() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let body = || MailBody::Text("Text".to_string());
    let bad = |to: &[&str], subject: &str| {
        let to: Vec<String> = to.iter().map(|s| s.to_string()).collect();
        MailMessage::new(&to, &[], subject, body())
    };
    assert!(matches!(bad(&[], "S"), Err(M365Error::Invalid(_))));
    assert!(matches!(bad(&["kein-at"], "S"), Err(M365Error::Invalid(_))));
    assert!(matches!(
        bad(&["a@example.com", "b c@example.com"], "S"),
        Err(M365Error::Invalid(_))
    ));
    assert!(matches!(
        bad(&["a@example.com"], "  \n "),
        Err(M365Error::Invalid(_))
    ));
    let long = "x".repeat(300);
    assert!(matches!(
        bad(&["a@example.com"], &long),
        Err(M365Error::Invalid(_))
    ));
    let many: Vec<String> = (0..31).map(|i| format!("u{i}@example.com")).collect();
    assert!(MailMessage::new(&many, &[], "S", body()).is_err());
    assert!(MailMessage::new(
        &["a@example.com".into()],
        &[],
        "S",
        MailBody::Text("  ".into())
    )
    .is_err());
    // Eine Mail ohne den eingeschalteten Scope geht nicht ans Netz.
    w.reconfigure(CLIENT, &[FILES]);
    let r = w.svc.send_mail(&w.acct(), &msg("a@example.com")).await;
    assert_eq!(r, Err(M365Error::CapabilityOff(MAIL)));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_follow_up_draft_becomes_an_html_message_and_a_bad_address_aborts() {
    let draft = MailDraft {
        to: vec!["anna@example.com".to_string(), " ".to_string()],
        subject: "Follow-up".to_string(),
        body_text: "Hallo Anna,\n\n- Punkt eins\n- Punkt <zwei>".to_string(),
        body_html: String::new(),
    };
    let m = message_from_draft(&draft).unwrap();
    assert_eq!(m.to, vec!["anna@example.com".to_string()]);
    let MailBody::Html(h) = &m.body else {
        panic!("HTML erwartet")
    };
    assert!(h.contains("<li>") || h.contains("<br"), "{h}");
    assert!(h.contains("&lt;zwei&gt;"), "Zeichen maskiert: {h}");
    let mut bad = draft.clone();
    bad.to.push("keine adresse".to_string());
    assert!(matches!(
        message_from_draft(&bad),
        Err(M365Error::Invalid(_))
    ));
    let mut none = draft;
    none.to.clear();
    assert!(matches!(
        message_from_draft(&none),
        Err(M365Error::Invalid(_))
    ));
}

#[tokio::test]
async fn forbidden_is_reported_with_the_reason() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::json(
            403,
            json!({"error": {"code": "ErrorSendAsDenied", "message": "Das Senden als diese Person ist nicht erlaubt.\r\nrequest-id: 123"}}),
        )
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let e = w
        .svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap_err();
    assert_eq!(
        e,
        M365Error::Denied("Das Senden als diese Person ist nicht erlaubt.".to_string())
    );
    assert!(e.wire().starts_with("m365_denied|"));
}

#[tokio::test]
async fn throttling_reports_the_wait_and_does_not_retry() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::json(429, json!({"error": {"code": "TooManyRequests"}})).with("Retry-After", "120")
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let e = w
        .svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap_err();
    assert_eq!(
        e,
        M365Error::Throttled {
            retry_after: Duration::from_secs(120)
        }
    );
    assert_eq!(e.wire(), "m365_throttled|2");
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
}

#[tokio::test]
async fn with_no_network_a_mail_fails_as_not_sent() {
    // Anmelde-Server laeuft, Graph ist nicht erreichbar (geschlossener Port).
    let (token_base, _seen) = serve(|_| token_resp("AT-1", None)).await;
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", l.local_addr().unwrap())
    };
    let ep = Endpoints {
        authority: token_base,
        graph: format!("{closed}/v1.0"),
        use_env_proxy: false,
    };
    let w = World::with(ep, &[MAIL], FilesMode::Full, true);
    let r = w.svc.send_mail(&w.acct(), &msg("a@example.com")).await;
    assert!(matches!(r, Err(M365Error::Network(_))), "{r:?}");
}

#[tokio::test]
async fn a_connection_lost_after_sending_is_uncertain_and_never_retried() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::Drop
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let e = w
        .svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap_err();
    assert!(matches!(e, M365Error::Uncertain(_)), "{e:?}");
    assert_eq!(e.code(), "m365_uncertain");
    assert_eq!(
        count_path(&seen, "POST", "/v1.0/me/sendMail"),
        1,
        "nicht wiederholt"
    );
}

#[tokio::test]
async fn redirects_are_never_followed() {
    let (other, other_seen) = serve(|_| Resp::empty(202)).await;
    let target = format!("{other}/abgegriffen");
    let (base, _seen) = serve(move |req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::empty(302).with("Location", &target)
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let e = w
        .svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap_err();
    assert_eq!(
        e,
        M365Error::Http {
            status: 302,
            code: String::new()
        }
    );
    assert!(
        other_seen.lock().unwrap().is_empty(),
        "das Token ging nicht an den fremden Host"
    );
}

// ---------------------------------------------------------------------------
// OneDrive: klein (PUT)
// ---------------------------------------------------------------------------

fn small_upload(name: &str, bytes: &[u8]) -> UploadRequest {
    UploadRequest {
        name: name.to_string(),
        subfolder: String::new(),
        conflict: Conflict::Rename,
        source: UploadSource::Bytes(bytes.to_vec()),
    }
}

#[tokio::test]
async fn a_small_file_goes_up_with_one_put_and_never_overwrites_by_default() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        if req.method == "PUT" {
            return Resp::json(
                201,
                json!({"id": "ITEM1", "name": "Protokoll Q3.txt", "size": 11, "webUrl": "https://onedrive.example/x"}),
            );
        }
        Resp::empty(404)
    })
    .await;
    let w = World::new(&base, &[FILES], true);
    let up = w
        .svc
        .upload(
            &w.acct(),
            &small_upload("Protokoll Q3.txt", b"hallo welt!!"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(up.id, "ITEM1");
    assert_eq!(up.via, UploadVia::Simple);
    let put = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.method == "PUT")
        .cloned()
        .unwrap();
    assert_eq!(
        put.target,
        "/v1.0/me/drive/root:/Local%20Voice%20AI/Protokoll%20Q3.txt:/content?@microsoft.graph.conflictBehavior=rename"
    );
    assert_eq!(put.bearer(), Some("AT-1"));
    assert_eq!(put.body, b"hallo welt!!");
    assert_eq!(put.header("content-type"), Some("application/octet-stream"));
}

#[tokio::test]
async fn the_app_folder_mode_uploads_below_the_app_root() {
    let (base, seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::json(200, json!({"id": "I", "name": "a.txt", "size": 1}))
    })
    .await;
    let w = World::with(eps(&base), &[FILES], FilesMode::AppFolder, true);
    let mut req = small_upload("a.txt", b"x");
    req.subfolder = "Berichte/Q3".to_string();
    req.conflict = Conflict::Replace;
    w.svc.upload(&w.acct(), &req, None).await.unwrap();
    let put = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.method == "PUT")
        .cloned()
        .unwrap();
    assert_eq!(
        put.target,
        "/v1.0/me/drive/special/approot:/Local%20Voice%20AI/Berichte/Q3/a.txt:/content?@microsoft.graph.conflictBehavior=replace"
    );
    // Der Token-Antrag trug nur den App-Ordner-Scope.
    let token = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| is_token(r))
        .cloned()
        .unwrap();
    assert_eq!(
        token.form()["scope"],
        "offline_access User.Read Files.ReadWrite.AppFolder"
    );
}

#[tokio::test]
async fn file_names_and_folders_are_checked_before_the_network() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[FILES], true);
    for name in [
        "",
        "..",
        "a/b.txt",
        "a\\b.txt",
        "x:y.txt",
        "CON",
        "nul.txt",
        "~$lock.docx",
        "a?.txt",
        "tab\t.txt",
    ] {
        let r = w
            .svc
            .upload(&w.acct(), &small_upload(name, b"x"), None)
            .await;
        assert!(matches!(r, Err(M365Error::Invalid(_))), "{name:?}: {r:?}");
    }
    for folder in ["../geheim", "a/../b", "..", "C:/Windows", "a//..//b"] {
        let mut req = small_upload("ok.txt", b"x");
        req.subfolder = folder.to_string();
        let r = w.svc.upload(&w.acct(), &req, None).await;
        assert!(matches!(r, Err(M365Error::Invalid(_))), "{folder:?}: {r:?}");
    }
    assert!(seen.lock().unwrap().is_empty());
    // Zulaessiges bleibt zulaessig.
    assert_eq!(
        clean_segment("Bericht 2026-Q3 (Entwurf).docx").unwrap(),
        "Bericht 2026-Q3 (Entwurf).docx"
    );
    assert_eq!(
        clean_folder("\\Ablage\\Berichte/").unwrap(),
        "Ablage/Berichte"
    );
    assert_eq!(clean_folder("").unwrap(), "");
    // Kodierung: Umlaute, Leerzeichen und Sonderzeichen kommen kodiert in den Pfad.
    assert_eq!(
        super::drive::encode_segment("Büro Ü#1"),
        "B%C3%BCro%20%C3%9C%231"
    );
}

#[tokio::test]
async fn a_full_onedrive_is_reported_as_such() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::json(507, json!({"error": {"code": "quotaLimitReached"}}))
    })
    .await;
    let w = World::new(&base, &[FILES], true);
    let e = w
        .svc
        .upload(&w.acct(), &small_upload("a.txt", b"x"), None)
        .await
        .unwrap_err();
    assert_eq!(e, M365Error::StorageFull);
    assert_eq!(e.wire(), "m365_storage_full");
}

// ---------------------------------------------------------------------------
// OneDrive: gross (Upload-Sitzung)
// ---------------------------------------------------------------------------

/// Eine Datei mit nachvollziehbarem Inhalt.
fn big_file(dir: &std::path::Path, len: usize) -> (PathBuf, Vec<u8>) {
    let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let path = dir.join("gross.bin");
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

type Chunks = Arc<Mutex<Vec<(u64, u64, Vec<u8>)>>>;

/// Ein Server mit Upload-Sitzung. `put_hook` darf je PUT-Anfrage (Nummer ab 1) eine
/// eigene Antwort liefern.
fn session_server(
    chunks: Chunks,
    put_hook: impl Fn(usize, &Req) -> Option<Resp> + Send + Sync + 'static,
) -> impl Fn(&Req) -> Resp + Send + Sync + 'static {
    let puts = Arc::new(AtomicUsize::new(0));
    move |req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        if req.method == "POST" && req.path().ends_with("/createUploadSession") {
            let host = req.header("host").unwrap();
            return Resp::json(
                200,
                json!({"uploadUrl": format!("http://{host}/upload/s1"), "expirationDateTime": "2026-10-02T00:00:00Z"}),
            );
        }
        if req.path() == "/upload/s1" {
            match req.method.as_str() {
                "PUT" => {
                    let n = puts.fetch_add(1, Ordering::SeqCst) + 1;
                    if let Some(resp) = put_hook(n, req) {
                        return resp;
                    }
                    let range = req.header("content-range").unwrap().to_string();
                    let (span, total) = range
                        .strip_prefix("bytes ")
                        .unwrap()
                        .split_once('/')
                        .unwrap();
                    let (a, b) = span.split_once('-').unwrap();
                    let (a, b, total): (u64, u64, u64) = (
                        a.parse().unwrap(),
                        b.parse().unwrap(),
                        total.parse().unwrap(),
                    );
                    chunks.lock().unwrap().push((a, b, req.body.clone()));
                    return if b + 1 < total {
                        Resp::json(202, json!({"nextExpectedRanges": [format!("{}-", b + 1)]}))
                    } else {
                        Resp::json(
                            201,
                            json!({"id": "BIG1", "name": "gross.bin", "size": total, "webUrl": "https://onedrive.example/b"}),
                        )
                    };
                }
                "GET" => {
                    let next = chunks.lock().unwrap().last().map(|c| c.1 + 1).unwrap_or(0);
                    return Resp::json(200, json!({"nextExpectedRanges": [format!("{next}-")]}));
                }
                "DELETE" => return Resp::empty(204),
                _ => {}
            }
        }
        Resp::empty(404)
    }
}

fn file_request(path: &std::path::Path) -> UploadRequest {
    UploadRequest {
        name: "gross.bin".to_string(),
        subfolder: String::new(),
        conflict: Conflict::Rename,
        source: UploadSource::File(path.to_path_buf()),
    }
}

#[tokio::test]
async fn a_large_file_goes_through_an_upload_session_in_chunks_without_a_token_on_the_chunks() {
    let chunks: Chunks = Default::default();
    let (base, seen) = serve(session_server(chunks.clone(), |_, _| None)).await;
    let w = World::new(&base, &[FILES], true);
    let len = 2 * CHUNK as usize + 123_456;
    let (path, bytes) = big_file(w.dir.parent().unwrap(), len);
    let up = w
        .svc
        .upload(&w.acct(), &file_request(&path), None)
        .await
        .unwrap();
    assert_eq!(up.via, UploadVia::Session);
    assert_eq!(up.id, "BIG1");
    assert_eq!(up.size, len as u64);

    let create = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path().ends_with("/createUploadSession"))
        .cloned()
        .unwrap();
    assert_eq!(
        create.path(),
        "/v1.0/me/drive/root:/Local%20Voice%20AI/gross.bin:/createUploadSession"
    );
    assert_eq!(create.bearer(), Some("AT-1"));
    assert_eq!(
        create.json(),
        json!({"item": {"@microsoft.graph.conflictBehavior": "rename", "name": "gross.bin"}})
    );

    // Stuecke: Vielfache von 320 KiB (ausser dem letzten), lueckenlos, ohne Token.
    let got = chunks.lock().unwrap().clone();
    assert_eq!(got.len(), 3);
    for (i, (a, b, _)) in got.iter().enumerate() {
        assert_eq!(*a, i as u64 * CHUNK);
        if i < 2 {
            assert_eq!(b + 1 - a, CHUNK);
            assert_eq!(CHUNK % (320 * 1024), 0);
        }
    }
    let joined: Vec<u8> = got.iter().flat_map(|c| c.2.clone()).collect();
    assert_eq!(joined, bytes);
    for put in seen
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path() == "/upload/s1")
    {
        assert!(
            put.bearer().is_none(),
            "die uploadUrl bekommt nie ein Token"
        );
    }
}

#[tokio::test]
async fn an_upload_session_resumes_at_the_server_position_after_a_failed_chunk() {
    let chunks: Chunks = Default::default();
    // Der zweite Stueck-Versuch scheitert mit 503, danach laeuft es weiter.
    let (base, seen) = serve(session_server(chunks.clone(), |n, _| {
        (n == 2).then(|| Resp::empty(503))
    }))
    .await;
    let w = World::new(&base, &[FILES], true);
    let (path, bytes) = big_file(w.dir.parent().unwrap(), CHUNK as usize * 2 + 10);
    w.svc
        .upload(&w.acct(), &file_request(&path), None)
        .await
        .unwrap();
    let got = chunks.lock().unwrap().clone();
    assert_eq!(
        got.iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![0, CHUNK, 2 * CHUNK]
    );
    assert_eq!(
        got.iter().flat_map(|c| c.2.clone()).collect::<Vec<u8>>(),
        bytes
    );
    assert_eq!(
        count_path(&seen, "GET", "/upload/s1"),
        1,
        "Stand der Sitzung abgefragt"
    );
    assert_eq!(count_path(&seen, "DELETE", "/upload/s1"), 0);
}

#[tokio::test]
async fn an_upload_session_is_deleted_when_a_chunk_keeps_failing() {
    let chunks: Chunks = Default::default();
    let (base, seen) = serve(session_server(chunks.clone(), |_, _| {
        Some(Resp::empty(503))
    }))
    .await;
    let w = World::new(&base, &[FILES], true);
    let (path, _) = big_file(w.dir.parent().unwrap(), SIMPLE_MAX as usize + 10);
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&path), None)
        .await
        .unwrap_err();
    assert!(matches!(e, M365Error::Http { status: 503, .. }), "{e:?}");
    assert_eq!(
        count_path(&seen, "PUT", "/upload/s1"),
        4,
        "ein Versuch und drei Wiederholungen"
    );
    assert_eq!(
        count_path(&seen, "DELETE", "/upload/s1"),
        1,
        "nichts Halbes bleibt liegen"
    );
    assert!(chunks.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_full_onedrive_during_a_session_stops_it_and_deletes_the_session() {
    let chunks: Chunks = Default::default();
    let (base, seen) = serve(session_server(chunks, |n, _| {
        (n == 2).then(|| Resp::json(507, json!({"error": {"code": "quotaLimitReached"}})))
    }))
    .await;
    let w = World::new(&base, &[FILES], true);
    let (path, _) = big_file(w.dir.parent().unwrap(), CHUNK as usize * 2 + 10);
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&path), None)
        .await
        .unwrap_err();
    assert_eq!(e, M365Error::StorageFull);
    assert_eq!(count_path(&seen, "PUT", "/upload/s1"), 2);
    assert_eq!(count_path(&seen, "DELETE", "/upload/s1"), 1);
}

#[tokio::test]
async fn cancelling_an_upload_between_chunks_deletes_the_session() {
    let chunks: Chunks = Default::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let c2 = cancel.clone();
    let (base, seen) = serve(session_server(chunks.clone(), move |n, _| {
        if n == 1 {
            c2.store(true, Ordering::SeqCst);
        }
        None
    }))
    .await;
    let w = World::new(&base, &[FILES], true);
    let (path, _) = big_file(w.dir.parent().unwrap(), CHUNK as usize * 2 + 10);
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&path), Some(cancel.as_ref()))
        .await
        .unwrap_err();
    assert_eq!(e, M365Error::Cancelled);
    assert_eq!(chunks.lock().unwrap().len(), 1);
    assert_eq!(count_path(&seen, "DELETE", "/upload/s1"), 1);
}

#[tokio::test]
async fn a_file_that_changes_during_the_upload_is_detected_and_the_session_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, _) = big_file(tmp.path(), CHUNK as usize * 2 + 10);
    let p2 = path.clone();
    // Nach dem ersten Stueck wird die Datei kuerzer.
    let (base, seen) = serve(session_server(Default::default(), move |n, _| {
        if n == 1 {
            std::fs::write(&p2, b"zu kurz").unwrap();
        }
        None
    }))
    .await;
    let w = World::new(&base, &[FILES], true);
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&path), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&e, M365Error::Invalid(m) if m.contains("geändert")),
        "{e:?}"
    );
    assert_eq!(count_path(&seen, "DELETE", "/upload/s1"), 1);
}

#[test]
fn the_upload_url_must_be_https_without_credentials() {
    let prod = M365Service::new(Endpoints::production(), Vault::in_dir(PathBuf::from("x")));
    let ok = prod.checked_upload_url(
        &json!({"uploadUrl": "https://sn3302.up.1drv.com/up/fe6987415ace7X4e1eF866337"}),
    );
    assert!(ok.is_ok());
    for bad in [
        "http://sn3302.up.1drv.com/up/x",
        "ftp://example.com/x",
        "https://user:pw@example.com/x",
        "kein url",
        "",
    ] {
        assert!(
            prod.checked_upload_url(&json!({ "uploadUrl": bad }))
                .is_err(),
            "{bad:?}"
        );
    }
    assert!(prod.checked_upload_url(&json!({})).is_err());
    // Gegen einen lokalen Test-Server (http) ist http erlaubt, sonst nichts anderes.
    let local = M365Service::new(eps("http://127.0.0.1:9"), Vault::in_dir(PathBuf::from("x")));
    assert!(local
        .checked_upload_url(&json!({"uploadUrl": "http://127.0.0.1:9/u"}))
        .is_ok());
    assert!(local
        .checked_upload_url(&json!({"uploadUrl": "http://u:p@127.0.0.1:9/u"}))
        .is_err());
}

#[tokio::test]
async fn too_large_uploads_are_refused_before_the_network() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[FILES], true);
    let missing = w.dir.join("gibt-es-nicht.bin");
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&missing), None)
        .await
        .unwrap_err();
    assert!(matches!(e, M365Error::Invalid(_)), "{e:?}");
    let e = w
        .svc
        .upload(&w.acct(), &file_request(&w.dir), None)
        .await
        .unwrap_err();
    assert!(
        matches!(e, M365Error::Invalid(_)),
        "ein Ordner ist keine Datei: {e:?}"
    );
    assert!(seen.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Notiz am Termin
// ---------------------------------------------------------------------------

fn event_ref() -> EventRef {
    let start = chrono::Utc
        .with_ymd_and_hms(2026, 10, 5, 8, 0, 0)
        .unwrap()
        .timestamp_millis();
    EventRef {
        uid: "evt-uid-1".to_string(),
        starts_at: start,
        ends_at: start + 3_600_000,
    }
}

struct EventServer {
    patches: Arc<Mutex<Vec<Req>>>,
}

fn event_server(
    body: Value,
    patch_status: u16,
) -> (impl Fn(&Req) -> Resp + Send + Sync + 'static, EventServer) {
    let patches: Arc<Mutex<Vec<Req>>> = Default::default();
    let p2 = patches.clone();
    let f = move |req: &Req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        if req.path() == "/v1.0/me/calendarView" {
            return Resp::json(
                200,
                json!({"value": [
                    {"id": "ANDERER", "iCalUId": "evt-uid-2", "subject": "Anderer",
                     "start": {"dateTime": "2026-10-05T08:00:00.0000000", "timeZone": "UTC"},
                     "end": {"dateTime": "2026-10-05T09:00:00.0000000", "timeZone": "UTC"}},
                    {"id": "EVT=1", "iCalUId": "evt-uid-1", "subject": "Review",
                     "start": {"dateTime": "2026-10-05T08:00:00.0000000", "timeZone": "UTC"},
                     "end": {"dateTime": "2026-10-05T09:00:00.0000000", "timeZone": "UTC"}}
                ]}),
            );
        }
        if req.method == "GET" && req.path() == "/v1.0/me/events/EVT%3D1" {
            return Resp::json(200, body.clone());
        }
        if req.method == "PATCH" && req.path() == "/v1.0/me/events/EVT%3D1" {
            p2.lock().unwrap().push(req.clone());
            return if patch_status == 200 {
                Resp::json(200, json!({"id": "EVT=1"}))
            } else {
                Resp::json(
                    patch_status,
                    json!({"error": {"code": "ErrorAccessDenied", "message": "Nur der Organisator darf den Termin ändern."}}),
                )
            };
        }
        Resp::empty(404)
    };
    (f, EventServer { patches })
}

#[tokio::test]
async fn a_note_is_appended_to_a_html_event_and_the_invitation_stays() {
    let invitation = "<html><head></head><body><p>Einladung</p><a href=\"https://teams.example/join\">Teilnehmen</a></body></html>";
    let (handler, ev) = event_server(
        json!({"id": "EVT=1", "@odata.etag": "W/\"abc\"", "body": {"contentType": "html", "content": invitation}}),
        200,
    );
    let (base, seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    let out = w
        .svc
        .add_event_note(
            &w.acct(),
            &event_ref(),
            "Beschluss: <b>Release</b> am 12.\nOwner: Anna",
        )
        .await
        .unwrap();
    assert_eq!(out, NoteOutcome::Added);
    let patch = ev.patches.lock().unwrap()[0].clone();
    assert_eq!(patch.header("if-match"), Some("W/\"abc\""));
    let body = patch.json();
    assert_eq!(body["body"]["contentType"], "html");
    let content = body["body"]["content"].as_str().unwrap();
    assert!(content.starts_with("<html><head></head><body><p>Einladung</p><a href=\"https://teams.example/join\">Teilnehmen</a>"));
    assert!(content.contains("Beschluss: &lt;b&gt;Release&lt;/b&gt; am 12.<br>Owner: Anna"));
    assert!(content.ends_with("</body></html>"));
    assert!(content.contains("<!--lva-note:"));
    // Gesucht wurde im Zeitfenster des Termins, mit dem Scope-Token.
    let search = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path() == "/v1.0/me/calendarView")
        .cloned()
        .unwrap();
    assert!(
        search
            .query()
            .contains("startDateTime=2026-10-05T08:00:00Z"),
        "{}",
        search.query()
    );
    assert_eq!(search.bearer(), Some("AT-1"));
    let token = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| is_token(r))
        .cloned()
        .unwrap();
    assert_eq!(
        token.form()["scope"],
        "offline_access User.Read Calendars.ReadWrite"
    );
}

#[tokio::test]
async fn the_same_note_is_not_added_twice() {
    let note = "Beschluss: Release am 12.";
    let first = super::event::append_html("<html><body><p>Einladung</p></body></html>", note);
    let (handler, ev) = event_server(
        json!({"id": "EVT=1", "body": {"contentType": "html", "content": first}}),
        200,
    );
    let (base, _seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    let out = w
        .svc
        .add_event_note(&w.acct(), &event_ref(), note)
        .await
        .unwrap();
    assert_eq!(out, NoteOutcome::AlreadyThere);
    assert!(ev.patches.lock().unwrap().is_empty());
    // Auch wenn Outlook die Marke entfernt hat: der Text genuegt.
    assert!(super::event::already_has(
        "<html><body><p>Einladung</p><div><p>Beschluss: Release am 12.</p></div></body></html>",
        note,
        true
    ));
}

#[tokio::test]
async fn a_note_extends_a_plain_text_event() {
    let (handler, ev) = event_server(
        json!({"id": "EVT=1", "body": {"contentType": "text", "content": "Einladung\n"}}),
        200,
    );
    let (base, _seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    w.svc
        .add_event_note(&w.acct(), &event_ref(), "Notiz eins")
        .await
        .unwrap();
    let body = ev.patches.lock().unwrap()[0].json();
    assert_eq!(body["body"]["contentType"], "text");
    assert_eq!(
        body["body"]["content"],
        "Einladung\n\n--- Notiz aus Local Voice AI ---\nNotiz eins\n"
    );
}

#[tokio::test]
async fn an_attendee_who_is_not_the_organizer_gets_a_clear_denial() {
    let (handler, _ev) = event_server(
        json!({"id": "EVT=1", "body": {"contentType": "html", "content": "<p>x</p>"}}),
        403,
    );
    let (base, _seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    let e = w
        .svc
        .add_event_note(&w.acct(), &event_ref(), "Notiz")
        .await
        .unwrap_err();
    assert_eq!(
        e,
        M365Error::Denied("Nur der Organisator darf den Termin ändern.".to_string())
    );
}

#[tokio::test]
async fn an_unknown_event_is_not_found_and_an_empty_note_is_refused() {
    let (handler, ev) = event_server(json!({}), 200);
    let (base, seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    let mut other = event_ref();
    other.uid = "gibt-es-nicht".to_string();
    let e = w
        .svc
        .add_event_note(&w.acct(), &other, "Notiz")
        .await
        .unwrap_err();
    assert!(matches!(e, M365Error::NotFound(_)), "{e:?}");
    let before = seen.lock().unwrap().len();
    let e = w
        .svc
        .add_event_note(&w.acct(), &event_ref(), "  \n")
        .await
        .unwrap_err();
    assert!(matches!(e, M365Error::Invalid(_)));
    assert_eq!(
        seen.lock().unwrap().len(),
        before,
        "kein Netz fuer eine leere Notiz"
    );
    assert!(ev.patches.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Tor: Grants, Freigabe, Audit
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_mail_asks_first_and_sends_nothing_until_the_user_approves() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let m = msg("kunde@example.com");
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        m.clone(),
        None,
    )
    .await
    .unwrap();
    let GateOutcome::Pending { approval_id } = out else {
        panic!("Freigabe erwartet: {out:?}")
    };
    assert!(
        seen.lock().unwrap().is_empty(),
        "ohne Freigabe kein Netz, auch kein Token"
    );
    let conn = w.store.get_connection().unwrap();
    let pending = approvals::list_pending(&conn, chrono::Utc::now().timestamp_millis()).unwrap();
    assert_eq!(pending.len(), 1);
    let preview = pending[0].args_preview.clone().unwrap();
    assert!(preview.contains("kunde@example.com"), "{preview}");
    assert!(w
        .audit_rows()
        .iter()
        .any(|r| r.outcome == "pending" && r.caller == "workflow"));

    // Anderer Inhalt mit derselben Freigabe: abgelehnt (an die Argumente gebunden).
    approvals::decide(
        &conn,
        &approval_id,
        true,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    let other = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        msg("anderer@example.com"),
        Some(approval_id.clone()),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            other,
            GateOutcome::Denied {
                code: "approval_mismatch",
                ..
            }
        ),
        "{other:?}"
    );
    assert!(seen.lock().unwrap().is_empty());

    // Mit den genehmigten Argumenten laeuft die Aktion, genau einmal.
    let done = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        m.clone(),
        Some(approval_id.clone()),
    )
    .await
    .unwrap();
    assert!(matches!(done, GateOutcome::Done(())), "{done:?}");
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
    let again = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        m,
        Some(approval_id),
    )
    .await
    .unwrap();
    assert!(
        matches!(again, GateOutcome::Denied { .. }),
        "einmalig: {again:?}"
    );
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
    assert!(w
        .audit_rows()
        .iter()
        .any(|r| r.outcome == "ok" && r.capability.as_deref() == Some("mail.send")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_user_sends_without_an_approval_and_leaves_an_audit_row() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::User,
        &w.id,
        msg("anna@example.com"),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(out, GateOutcome::Done(())), "{out:?}");
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
    let rows = w.audit_rows();
    let row = rows
        .iter()
        .find(|r| r.caller == "user" && r.capability.as_deref() == Some("mail.send"))
        .unwrap();
    assert_eq!(row.outcome, "ok");
    assert_eq!(row.target.as_deref(), Some("anna@example.com"));
    // Der Eintrag zeigt „zuletzt ok“.
    assert!(w.integration().last_ok_at.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_capability_that_is_not_enabled_is_denied_by_the_gate_and_audited() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[FILES], true);
    for caller in [Caller::User, Caller::Workflow, Caller::AgentExternal] {
        let out = actions::send_mail(
            w.svc.clone(),
            w.store.clone(),
            caller,
            &w.id,
            msg("a@example.com"),
            None,
        )
        .await
        .unwrap();
        assert!(
            matches!(
                out,
                GateOutcome::Denied {
                    code: "capability_not_enabled",
                    ..
                }
            ),
            "{caller:?}: {out:?}"
        );
    }
    assert!(seen.lock().unwrap().is_empty());
    assert!(w.audit_rows().iter().any(|r| r.outcome == "denied"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grants_decide_for_agents_off_denies_allow_runs_directly() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let conn = w.store.get_connection().unwrap();
    // Externe Agenten: Vorgabe aus.
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::AgentExternal,
        &w.id,
        msg("a@example.com"),
        None,
    )
    .await
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
    // Der Nutzer erlaubt es dem Workflow ausdruecklich: laeuft ohne Freigabe.
    store::set_grant(&conn, &w.id, MAIL, Caller::Workflow, GrantMode::Allow).unwrap();
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        msg("a@example.com"),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(out, GateOutcome::Done(())), "{out:?}");
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
    // Und „aus“ gilt sofort.
    store::set_grant(&conn, &w.id, MAIL, Caller::Workflow, GrantMode::Off).unwrap();
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        msg("a@example.com"),
        None,
    )
    .await
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
    assert_eq!(count_path(&seen, "POST", "/v1.0/me/sendMail"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_action_is_audited_as_error_and_marked_on_the_integration() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-1", None);
        }
        Resp::json(500, json!({"error": {"code": "InternalServerError"}}))
    })
    .await;
    let w = World::new(&base, &[MAIL], true);
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::User,
        &w.id,
        msg("a@example.com"),
        None,
    )
    .await
    .unwrap();
    let GateOutcome::Failed(wire) = out else {
        panic!("Fehler erwartet: {out:?}")
    };
    assert_eq!(wire, "m365_http|500 InternalServerError");
    let row = w
        .audit_rows()
        .into_iter()
        .find(|r| r.caller == "user" && r.outcome == "error")
        .unwrap();
    assert!(row.detail_json.unwrap().contains("m365_http"));
    let integ = w.integration();
    assert!(integ.last_error.unwrap().contains("HTTP 500"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_upload_asks_first_and_shows_the_target_path_in_the_preview() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[FILES], true);
    let out = actions::upload_file(
        w.svc.clone(),
        w.store.clone(),
        Caller::AgentLocal,
        &w.id,
        small_upload("Protokoll.txt", b"inhalt"),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(matches!(out, GateOutcome::Pending { .. }), "{out:?}");
    assert!(seen.lock().unwrap().is_empty());
    let conn = w.store.get_connection().unwrap();
    let pending = approvals::list_pending(&conn, chrono::Utc::now().timestamp_millis()).unwrap();
    let preview = pending[0].args_preview.clone().unwrap();
    assert!(
        preview.contains("OneDrive:/Local Voice AI/Protokoll.txt"),
        "{preview}"
    );
    // Ein unzulaessiger Name kommt gar nicht erst ans Tor.
    let bad = actions::upload_file(
        w.svc.clone(),
        w.store.clone(),
        Caller::AgentLocal,
        &w.id,
        small_upload("../x.txt", b"inhalt"),
        None,
        None,
    )
    .await;
    assert!(bad.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_event_note_asks_first() {
    let (handler, ev) = event_server(
        json!({"id": "EVT=1", "body": {"contentType": "html", "content": "<p>x</p>"}}),
        200,
    );
    let (base, seen) = serve(handler).await;
    let w = World::new(&base, &[CAL], true);
    let out = actions::event_note(
        w.svc.clone(),
        w.store.clone(),
        Caller::Workflow,
        &w.id,
        event_ref(),
        "Notiz".to_string(),
        None,
    )
    .await
    .unwrap();
    assert!(matches!(out, GateOutcome::Pending { .. }), "{out:?}");
    assert!(seen.lock().unwrap().is_empty());
    assert!(ev.patches.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Zustand, Fehlercodes, Geheimhaltung
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_status_walks_through_the_states() {
    let w = World::new("http://127.0.0.1:1", &[], false);
    let st = |w: &World| status_of(&w.integration(), &w.vault, false).unwrap();
    assert_eq!(st(&w).state, AccountState::NoCapabilities);
    w.reconfigure("", &[MAIL]);
    assert_eq!(st(&w).state, AccountState::NotConfigured);
    w.reconfigure(CLIENT, &[MAIL]);
    assert_eq!(st(&w).state, AccountState::NeedsSignIn);
    w.sign_in_directly(RT);
    assert_eq!(st(&w).state, AccountState::Ready);
    w.reconfigure(CLIENT, &[MAIL, CAL]);
    let s = st(&w);
    assert_eq!(s.state, AccountState::NeedsConsent);
    assert_eq!(s.missing_scopes, vec!["Calendars.ReadWrite".to_string()]);
    assert_eq!(s.required_scopes.len(), 4);
    // Ausschalten verlangt keine neue Zustimmung.
    w.reconfigure(CLIENT, &[]);
    assert_eq!(st(&w).state, AccountState::NoCapabilities);
    assert!(
        status_of(&w.integration(), &w.vault, true)
            .unwrap()
            .signing_in
    );
}

#[tokio::test]
async fn no_token_reaches_the_status_the_register_the_audit_or_an_error_text() {
    let (base, _seen) = serve(|req| {
        if is_token(req) {
            return token_resp("AT-GEHEIM-ZUGRIFF", Some("RT-GEHEIM-ERNEUERN"));
        }
        if req.path() == "/v1.0/me" {
            return me_ok();
        }
        Resp::json(403, json!({"error": {"code": "ErrorAccessDenied", "message": "Zugriff mit Bearer AT-GEHEIM-ZUGRIFF abgelehnt"}}))
    })
    .await;
    let w = World::new(&base, &[MAIL], false);
    w.svc
        .sign_in(
            &w.acct(),
            fake_browser(Default::default()),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    let denied = w
        .svc
        .send_mail(&w.acct(), &msg("a@example.com"))
        .await
        .unwrap_err();
    let status =
        serde_json::to_string(&status_of(&w.integration(), &w.vault, false).unwrap()).unwrap();
    let register = w.integration().config_json;
    let integ_dump = serde_json::to_string(&w.integration()).unwrap();
    let wire = denied.wire();
    for text in [&status, &register, &integ_dump] {
        assert!(!text.contains("GEHEIM"), "{text}");
    }
    // Der Fehlertext aus Microsoft wird beim Eintragen ins Audit geschwaerzt.
    let out = actions::send_mail(
        w.svc.clone(),
        w.store.clone(),
        Caller::User,
        &w.id,
        msg("a@example.com"),
        None,
    )
    .await;
    let _ = (out, wire);
    for row in w.audit_rows() {
        let text = format!("{:?}", row);
        assert!(!text.contains("RT-GEHEIM"), "{text}");
    }
    // Die Datei im Geheimnisordner ist verschluesselt.
    let raw = std::fs::read(w.dir.join(format!("int-{}-token.bin", w.id))).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("RT-GEHEIM"));
}

#[test]
fn error_codes_are_stable_and_the_wire_form_has_no_separator_inside_details() {
    assert_eq!(M365Error::NeedsSignIn.wire(), "m365_needs_sign_in");
    assert_eq!(M365Error::StorageFull.wire(), "m365_storage_full");
    assert_eq!(
        M365Error::NeedsConsent {
            missing: vec!["Mail.Send".into(), "Files.ReadWrite".into()]
        }
        .wire(),
        "m365_needs_consent|Mail.Send Files.ReadWrite"
    );
    assert_eq!(
        M365Error::Http {
            status: 409,
            code: "nameAlreadyExists".into()
        }
        .wire(),
        "m365_http|409 nameAlreadyExists"
    );
    assert_eq!(M365Error::Denied("a|b".into()).wire(), "m365_denied|a/b");
    assert!(M365Error::NeedsSignIn.needs_sign_in());
    assert!(!M365Error::StorageFull.needs_sign_in());
    // Jeder Fehler hat einen deutschen Klartext.
    for e in [
        M365Error::NotConfigured,
        M365Error::NoCapability,
        M365Error::NeedsSignIn,
        M365Error::Timeout,
        M365Error::Cancelled,
        M365Error::SignInTimeout,
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn only_the_m365_kind_is_accepted_and_the_config_must_parse() {
    let fx = Fx::new();
    let folder = crate::managers::integrations::test_support::folder(&fx.conn(), "Ablage");
    assert!(matches!(
        Acct::from_integration(folder),
        Err(M365Error::Invalid(_))
    ));
    let mut i =
        crate::managers::integrations::test_support::of_kind(&fx.conn(), Kind::M365, "Konto");
    assert!(Acct::from_integration(i.clone()).is_ok());
    i.config_json = "kaputt".to_string();
    assert!(matches!(
        Acct::from_integration(i),
        Err(M365Error::Config(_))
    ));
}

// ---------------------------------------------------------------------------
// Anhaenge (B5)
// ---------------------------------------------------------------------------

fn attachment(name: &str, bytes: &[u8]) -> MailAttachment {
    MailAttachment {
        name: name.to_string(),
        content_type: "application/pdf".to_string(),
        bytes: bytes.to_vec(),
    }
}

#[test]
fn attachments_are_checked_before_the_network_and_nothing_is_cut() {
    let base = msg("a@example.com");
    assert!(base
        .clone()
        .with_attachments(vec![attachment("Protokoll.pdf", b"%PDF-1")])
        .is_ok());
    let many: Vec<MailAttachment> = (0..=MAX_ATTACHMENTS)
        .map(|i| attachment(&format!("{i}.pdf"), b"x"))
        .collect();
    assert!(matches!(
        base.clone().with_attachments(many),
        Err(M365Error::Invalid(m)) if m.contains("Zu viele")
    ));
    assert!(matches!(
        base.clone().with_attachments(vec![attachment("leer.pdf", b"")]),
        Err(M365Error::Invalid(m)) if m.contains("leer")
    ));
    for bad in ["", "   ", "a\nb.pdf", &"x".repeat(200)] {
        assert!(
            matches!(
                base.clone().with_attachments(vec![attachment(bad, b"x")]),
                Err(M365Error::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    let big = vec![0u8; MAX_ATTACHMENT_BYTES / 2 + 1];
    assert!(matches!(
        base.with_attachments(vec![attachment("a.pdf", &big), attachment("b.pdf", &big)]),
        Err(M365Error::Invalid(m)) if m.contains("zu groß")
    ));
}

#[tokio::test]
async fn an_attachment_goes_into_the_graph_request_as_a_file_attachment() {
    let tokens = Arc::new(AtomicUsize::new(0));
    let (base, seen) = serve(ok_mail_server(tokens)).await;
    let w = World::new(&base, &[MAIL], true);
    let m = msg("a@example.com")
        .with_attachments(vec![attachment("Protokoll Überprüfung.pdf", b"%PDF-1.7 Inhalt")])
        .unwrap();
    w.svc.send_mail(&w.acct(), &m).await.unwrap();
    let sent = seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| r.path() == "/v1.0/me/sendMail")
        .cloned()
        .unwrap()
        .json();
    let a = &sent["message"]["attachments"][0];
    assert_eq!(a["@odata.type"], "#microsoft.graph.fileAttachment");
    assert_eq!(a["name"], "Protokoll Überprüfung.pdf");
    assert_eq!(a["contentType"], "application/pdf");
    use base64::Engine as _;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(a["contentBytes"].as_str().unwrap())
            .unwrap(),
        b"%PDF-1.7 Inhalt"
    );
    // Ohne Anhang gibt es das Feld nicht.
    assert!(msg("a@example.com").to_graph_json()["message"]
        .get("attachments")
        .is_none());
}

#[test]
fn the_gate_arguments_name_the_attachments_but_never_carry_their_content() {
    let m = msg("a@example.com")
        .with_attachments(vec![attachment("Protokoll.pdf", b"GEHEIMER-INHALT")])
        .unwrap();
    let args = m.gate_args();
    assert_eq!(args["attachments"], json!(["Protokoll.pdf"]));
    assert!(!args.to_string().contains("GEHEIMER-INHALT"));
    assert!(
        !format!("{m:?}").contains("GEHEIMER-INHALT"),
        "auch nicht im Debug-Text"
    );
}
