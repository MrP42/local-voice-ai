//! TF2: Terminfindung ohne automatische Versandaktionen. Der Hintergrund liest
//! ausschliesslich Antworten auf offene, kontogebundene Umfragen.
pub mod schema;
mod store;
pub use store::*;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use besprechungsplanung::terminfindung as tf;
use tf::{Umfrage, Status};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use sha2::{Digest, Sha256};
use rand::{Rng, RngCore};
use rusqlite::{params, OptionalExtension};
use tauri::Manager;
use tauri_specta::Event;
use crate::managers::integrations::{audit, gate::GateOutcome, model::{AuditOutcome, Caller, Capability, IntegrationError, NewAudit}, store as integrations};
use crate::managers::integrations::m365::{actions, Acct, M365Error, M365Service};
use crate::managers::integrations::m365::mail::{MailBody, MailMessage};
use crate::managers::integrations::m365::polls::{PersonAvailability, ScheduleWindow};
use crate::managers::calendar::secret::{self, SecretRef};
use super::store::MeetingStore;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PollHistory {
    pub id: i64,
    pub sender: String,
    pub person: Option<String>,
    pub received_at: i64,
    pub result: tf::Ergebnis,
    pub confirmed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PollDelivery { pub address: String, pub state: String }

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PollView {
    pub poll: PollDefinition,
    pub answers: Vec<tf::Antwort>,
    pub history: Vec<PollHistory>,
    pub deliveries: Vec<PollDelivery>,
}

// Eigener Randtyp: TF1s `Status` kollidiert in Specta mit dem Sprachdienst.
// Die TF1-Dateien bleiben waehrend ihrer parallelen Fixrunde unveraendert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum PollStatus { Entwurf, Offen, Entschieden, Abgebrochen }

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PollDefinition {
    pub id: String,
    pub plan_id: Option<String>,
    pub titel: String,
    pub titel_an_externe: bool,
    pub dauer_min: u32,
    pub frist: i64,
    pub status: PollStatus,
    pub vorschlaege: Vec<tf::Vorschlag>,
    pub teilnehmende: Vec<tf::Person>,
}

impl PollDefinition {
    pub fn into_core(self) -> Umfrage {
        Umfrage { id:self.id, plan_id:self.plan_id, titel:self.titel, titel_an_externe:self.titel_an_externe,
            dauer_min:self.dauer_min, frist:self.frist,
            status:match self.status { PollStatus::Entwurf=>Status::Entwurf, PollStatus::Offen=>Status::Offen,
                PollStatus::Entschieden=>Status::Entschieden, PollStatus::Abgebrochen=>Status::Abgebrochen },
            vorschlaege:self.vorschlaege,teilnehmende:self.teilnehmende }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct PollUpdated { pub poll_id: Option<String> }

struct CachedAvailability { until: Instant, values: Vec<PersonAvailability> }

#[derive(Default)]
pub struct PollRuntime {
    // Auch Versand/Schliessen und Delta-Verarbeitung werden serialisiert.
    pub operation: tokio::sync::Mutex<()>,
    availability: Mutex<HashMap<String, CachedAvailability>>,
}

pub fn now() -> i64 { chrono::Utc::now().timestamp_millis() }

pub fn valid_address(address: &str) -> bool {
    // Derselbe Validator wie bei allen M365-Mails; der TF1-Validator bleibt privat.
    crate::managers::meetings::mail::valid_address(address)
        && !address.chars().any(|c| c.is_whitespace() || matches!(c,'<'|'>'|'"'))
        && address.len() <= 254
}

fn connection(store: &MeetingStore) -> Result<rusqlite::Connection, String> {
    store.get_connection().map_err(|_| "poll_store".into())
}

/// Bindung an Client, Mandant und echte Konto-Adresse; keine Anzeigenamen.
pub fn account(svc: &M365Service, conn: &rusqlite::Connection, integration: &str) -> Result<(Acct,String,String),String> {
    let i = integrations::get(conn,integration).map_err(|_| "poll_store")?.ok_or("poll_account_missing")?;
    let a = Acct::from_integration(i).map_err(|e| e.wire())?;
    let stored = svc.vault.load(&a.integ).map_err(|_| "poll_account_missing")?.ok_or("poll_account_missing")?;
    if stored.client_id != a.cfg.client_id || stored.tenant != a.cfg.tenant { return Err("poll_account_changed".into()); }
    let own = stored.address.as_deref().map(tf::normalisiere_adresse).filter(|s| valid_address(s)).ok_or("poll_account_missing")?;
    let key = format!("{:x}", Sha256::digest(format!("{}\0{}\0{}",a.cfg.client_id,a.cfg.tenant,own).as_bytes()));
    Ok((a,key,own))
}

fn bound_account(svc: &M365Service, conn: &rusqlite::Connection, id: &str) -> Result<(Acct,String,String),String> {
    let (integration, expected) = binding(conn,id)?;
    let current = account(svc,conn,&integration)?;
    if current.1 != expected { return Err("poll_account_changed".into()); }
    Ok(current)
}

fn key_for(integration: &str, create: bool) -> Result<zeroize::Zeroizing<Vec<u8>>,String> {
    let r = SecretRef::integration(integration,"poll_key");
    if let Some(key) = secret::secret_get_ref(&r).map_err(|_| "poll_secret")? {
        if key.len() == 32 { return Ok(key); }
        return Err("poll_secret".into());
    }
    if !create { return Err("poll_secret".into()); }
    let mut key = zeroize::Zeroizing::new(vec![0;32]);
    rand::rngs::OsRng.fill_bytes(&mut key);
    secret::secret_put_ref(&r,&key).map_err(|_| "poll_secret")?;
    Ok(key)
}

fn clear_cursors(integration: &str) {
    for slot in ["poll_delta0","poll_delta1"] {
        secret::secret_delete_ref(&SecretRef::integration(integration,slot));
    }
}

fn audit_read(conn: &rusqlite::Connection, integration: &str, capability: Capability, outcome: AuditOutcome) -> Result<(),String> {
    audit::record_at(conn,&NewAudit {
        caller: Caller::User.as_str().into(), integration_id:Some(integration.into()), capability:Some(capability.as_str().into()),
        target:None, outcome, detail:Some(json!({"phase":"poll_read"})),
    },now()).map_err(|_| "poll_audit".to_string())?;
    Ok(())
}

fn read_allowed(a: &Acct, cap: Capability) -> Result<(),String> {
    a.require(cap).map_err(|e| e.wire())?;
    if !a.integ.enabled || !a.integ.direction.permits(cap.access()) { return Err("poll_capability_off".into()); }
    Ok(())
}

pub fn create(svc: &M365Service, store: &MeetingStore, integration: &str, mut poll: Umfrage) -> Result<PollView,String> {
    let conn = connection(store)?;
    purge(&conn,now())?;
    let (_,account,_) = account(svc,&conn,integration)?;
    let key = key_for(integration,true)?;
    poll.status = Status::Entwurf;
    for person in &mut poll.teilnehmende { person.adresse = tf::normalisiere_adresse(&person.adresse); }
    if poll.teilnehmende.is_empty() || poll.frist <= now() { return Err("poll_invalid".into()); }
    for _ in 0..100 {
        let mut number = rand::rngs::OsRng.gen_range(0..36_u32.pow(4));
        let mut code = [b'0';4];
        for byte in code.iter_mut().rev() { *byte = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"[(number % 36) as usize]; number /= 36; }
        poll.id = format!("T-{}",String::from_utf8_lossy(&code));
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM meeting_polls WHERE id=?1)",[&poll.id],|r| r.get(0)).map_err(|_| "poll_store")?;
        if exists { continue; }
        insert_poll(&conn,integration,&account,&poll,&key,now())?;
        return get(&conn,&poll.id);
    }
    Err("poll_id_collision".into())
}

fn finish_send(conn: &rusqlite::Connection, id: &str, address: &str, result: &Result<GateOutcome<()>,IntegrationError>) -> Result<(),String> {
    let state = match result {
        Ok(GateOutcome::Done(())) => "sent",
        // Nur Denied beweist, dass das Tor die Aktion nicht ausgefuehrt hat.
        // Fehler und etwaige ausstehende Freigaben bleiben gegen Doppelversand gesperrt.
        Ok(GateOutcome::Denied { .. }) => "pending",
        _ => "review",
    };
    conn.execute("UPDATE meeting_poll_people SET send_state=?3,sent_at=CASE WHEN ?3='pending' THEN NULL ELSE sent_at END WHERE poll_id=?1 AND address=?2 AND send_state='sending'",params![id,address,state]).map_err(|_| "poll_store")?;
    Ok(())
}

impl PollRuntime {
    pub fn clear_availability(&self) {
        if let Ok(mut cache) = self.availability.lock() { cache.clear(); }
    }

    fn prune_availability(&self) {
        if let Ok(mut cache) = self.availability.lock() { cache.retain(|_,v| v.until > Instant::now()); }
    }

    pub async fn availability(&self, svc: &M365Service, store: &MeetingStore, id: &str, window: ScheduleWindow) -> Result<Vec<PersonAvailability>,String> {
        let (a, account, addresses) = {
            let conn = connection(store)?;
            let (a,key,own) = bound_account(svc,&conn,id)?;
            read_allowed(&a,Capability::Verfuegbarkeit)?;
            let mut addresses = vec![own];
            for p in get(&conn,id)?.poll.teilnehmende.into_iter().filter(|p| p.intern) {
                if !addresses.contains(&p.adresse) { addresses.push(p.adresse); }
            }
            (a,key,addresses)
        };
        window.intervals().map_err(|e| e.wire())?;
        let cache_key = format!("{}:{}:{}",a.integ.id,account,serde_json::to_string(&(&window,&addresses)).map_err(|_| "poll_data")?);
        {
            let mut cache = self.availability.lock().map_err(|_| "poll_lock")?;
            cache.retain(|_,v| v.until > Instant::now());
            if let Some(value) = cache.get(&cache_key) { return Ok(value.values.clone()); }
        }
        audit_read(&connection(store)?,&a.integ.id,Capability::Verfuegbarkeit,AuditOutcome::Pending)?;
        let result = svc.poll_schedule(&a,&addresses[0],&addresses,&window).await;
        audit_read(&connection(store)?,&a.integ.id,Capability::Verfuegbarkeit,if result.is_ok() {AuditOutcome::Ok} else {AuditOutcome::Error})?;
        let values = result.map_err(|e| e.wire())?;
        let conn = connection(store)?;
        let current = bound_account(svc,&conn,id)?;
        if current.1 != account { return Err("poll_account_changed".into()); }
        read_allowed(&current.0,Capability::Verfuegbarkeit)?;
        let mut cache = self.availability.lock().map_err(|_| "poll_lock")?;
        if cache.len() >= 128 { cache.clear(); }
        cache.insert(cache_key,CachedAvailability { until:Instant::now()+Duration::from_secs(15*60),values:values.clone() });
        Ok(values)
    }

    /// Einziger Einstieg fuer den Versand, ausschliesslich aus poll_send.
    pub async fn send(&self, app: &tauri::AppHandle, svc: Arc<M365Service>, store: Arc<MeetingStore>, id: &str) -> Result<PollView,String> {
        let (a,key,own,poll) = {
            let conn = connection(&store)?;
            let (a,key,own) = bound_account(&svc,&conn,id)?;
            read_allowed(&a,Capability::AntwortenLesen)?;
            a.require(Capability::MailSend).map_err(|e| e.wire())?;
            let poll = get(&conn,id)?.poll.into_core();
            if !matches!(poll.status,Status::Entwurf|Status::Offen) || poll.vorschlaege.len()<2 || poll.frist<=now() { return Err("poll_not_sendable".into()); }
            let opened = conn.execute("UPDATE meeting_polls SET status='offen' WHERE id=?1 AND closed_at IS NULL AND status IN ('entwurf','offen')",[id]).map_err(|_| "poll_store")?;
            if opened != 1 { return Err("poll_closed".into()); }
            (a,key,own,poll)
        };
        updated(app,id);
        for person in &poll.teilnehmende {
            let (code,state): (String,String) = {
                let conn = connection(&store)?;
                let current = bound_account(&svc,&conn,id)?;
                if current.1 != key { return Err("poll_account_changed".into()); }
                conn.query_row("SELECT code,send_state FROM meeting_poll_people WHERE poll_id=?1 AND address=?2",params![id,person.adresse],|r| Ok((r.get(0)?,r.get(1)?))).map_err(|_| "poll_store")?
            };
            let subject = tf::betreff_marke(id,&code).map_err(str::to_string)?;
            if state == "pending" {
                let mut body = tf::anfrage_text(&poll,!person.intern).map_err(str::to_string)?;
                for link in tf::antwort_links(&own,&poll,&code).map_err(str::to_string)? { body.push_str(&format!("\n{}: {}\n",link.label,link.url)); }
                let message = MailMessage::new(&[person.adresse.clone()],&[],&subject,MailBody::Text(body)).map_err(|e| e.wire())?;
                let started = now();
                // Dauerhafter Claim VOR dem Netz: Crash/unklarer Ausgang darf nie
                // einen zweiten Versand bei erneutem poll_send verursachen.
                let claimed = connection(&store)?.execute("UPDATE meeting_poll_people SET send_state='sending',sent_at=?3 WHERE poll_id=?1 AND address=?2 AND send_state='pending' AND EXISTS (SELECT 1 FROM meeting_polls WHERE id=?1 AND status='offen')",params![id,person.adresse,started]).map_err(|_| "poll_store")?;
                if claimed != 1 { return Err("poll_send_review".into()); }
                let result = actions::send_poll_mail(svc.clone(),store.clone(),a.clone(),own.clone(),message).await;
                let sent = matches!(result,Ok(GateOutcome::Done(())));
                finish_send(&connection(&store)?,id,&person.adresse,&result)?;
                updated(app,id);
                if !sent { return Err("poll_send_review".into()); }
            }
            // Auch beim naechsten ausdruecklichen Klick fehlende conversationId
            // nachholen; niemals eine bereits beanspruchte Mail erneut senden.
            let (state,since,conversation): (String,Option<i64>,Option<String>) = connection(&store)?.query_row("SELECT send_state,sent_at,conversation_id FROM meeting_poll_people WHERE poll_id=?1 AND address=?2",params![id,person.adresse],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_| "poll_store")?;
            if matches!(state.as_str(),"sent"|"sending"|"review") && conversation.is_none() {
                if let Some(since) = since {
                    if let Some(conversation) = svc.poll_sent_conversation(&a,&own,&subject,&person.adresse,since).await.map_err(|e| e.wire())? {
                        connection(&store)?.execute("UPDATE meeting_poll_people SET conversation_id=?3,send_state='sent' WHERE poll_id=?1 AND address=?2",params![id,person.adresse,conversation]).map_err(|_| "poll_store")?;
                    }
                }
            }
        }
        updated(app,id);
        get(&connection(&store)?,id)
    }

    pub async fn sync(&self, svc: &M365Service, store: &MeetingStore, integration: &str, mut notify: impl FnMut()) -> Result<bool,String> {
        let (account_key,mut cursor_ref) = {
            let _guard = self.operation.lock().await;
            let conn = connection(store)?;
            let (a,key,_) = account(svc,&conn,integration)?;
            if !has_open(&conn,integration,&key)? { return Ok(false); }
            read_allowed(&a,Capability::AntwortenLesen)?;
            let old: Option<(String,Option<String>,i64)> = conn.query_row("SELECT account_key,delta_token,next_at FROM meeting_poll_sync WHERE integration_id=?1",[integration],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| "poll_store")?;
            if old.as_ref().is_some_and(|(account,_,next)| account==&key && *next>now()) { return Ok(false); }
            let cursor_ref = old.as_ref().filter(|(account,_,_)| account==&key).and_then(|(_,token,_)| token.clone());
            if old.as_ref().is_some_and(|(old_key,_,_)| old_key != &key) { clear_cursors(integration); }
            conn.execute("INSERT INTO meeting_poll_sync(integration_id,account_key,next_at) VALUES (?1,?2,?3) ON CONFLICT(integration_id) DO UPDATE SET account_key=excluded.account_key,delta_token=?4,next_at=excluded.next_at",params![integration,key,now()+120_000,cursor_ref]).map_err(|_| "poll_store")?;
            (key,cursor_ref)
        };
        let key = key_for(integration,false)?;
        // Sent Items ist nach sendMail eventuell noch nicht konsistent. Fehlende
        // Thread-IDs spaeter nur LESEND nachholen; ein Versand wird nie wiederholt.
        let mut changed = self.recover_conversations(svc,store,integration,&account_key,&mut notify).await?;
        for _ in 0..60 {
            let active = {
                let _guard = self.operation.lock().await;
                let conn = connection(store)?;
                if !has_open(&conn,integration,&account_key)? { break; }
                let current = account(svc,&conn,integration)?;
                if current.1 != account_key { return Err("poll_account_changed".into()); }
                read_allowed(&current.0,Capability::AntwortenLesen)?;
                audit_read(&conn,integration,Capability::AntwortenLesen,AuditOutcome::Pending)?;
                (current.0,current.2)
            };
            let cursor = if let Some(r) = &cursor_ref {
                if !matches!(r.as_str(),"poll_delta0"|"poll_delta1") { return Err("poll_secret".into()); }
                let bytes = secret::secret_get_ref(&SecretRef::integration(integration,r)).map_err(|_| "poll_secret")?.ok_or("poll_secret")?;
                Some(zeroize::Zeroizing::new(String::from_utf8(bytes.to_vec()).map_err(|_| "poll_secret")?))
            } else { None };
            let page = match svc.poll_delta_page(&active.0,&active.1,cursor.as_ref().map(|s| s.as_str())).await {
                Ok(page) => page,
                Err(e) => {
                    let _guard = self.operation.lock().await;
                    let conn = connection(store)?;
                    let wait = match &e { M365Error::Throttled { retry_after } => retry_after.as_millis().min(i64::MAX as u128) as i64, _ => 240_000 };
                    let gone = matches!(e,M365Error::Http {status:410,..});
                    conn.execute("UPDATE meeting_poll_sync SET next_at=?2,delta_token=CASE WHEN ?3 THEN NULL ELSE delta_token END WHERE integration_id=?1 AND account_key=?4",params![integration,now().saturating_add(wait.max(120_000)),gone,account_key]).map_err(|_| "poll_store")?;
                    if gone { clear_cursors(integration); }
                    audit_read(&conn,integration,Capability::AntwortenLesen,AuditOutcome::Error)?;
                    return Err(e.wire());
                }
            };
            let _guard = self.operation.lock().await;
            let conn = connection(store)?;
            let current = account(svc,&conn,integration)?;
            if current.1 != account_key { return Err("poll_account_changed".into()); }
            read_allowed(&current.0,Capability::AntwortenLesen)?;
            if !has_open(&conn,integration,&account_key)? { break; }
            let (page_changed,next_ref) = apply_page(&conn,(integration,&account_key),&key,&page,cursor_ref.as_deref(),
                |slot,token| secret::secret_put_ref(&SecretRef::integration(integration,slot),token.as_bytes()).map_err(|_| "poll_secret".to_string()),
                |slot| secret::secret_delete_ref(&SecretRef::integration(integration,slot)),&mut notify)?;
            changed |= page_changed;
            cursor_ref = Some(next_ref);
            audit_read(&conn,integration,Capability::AntwortenLesen,AuditOutcome::Ok)?;
            if page.finished { break; }
        }
        Ok(changed)
    }

    async fn recover_conversations(&self, svc: &M365Service, store: &MeetingStore, integration: &str, account_key: &str, notify: &mut impl FnMut()) -> Result<bool,String> {
        let pending: Vec<(String,String,String,i64)> = {
            let _guard = self.operation.lock().await;
            let conn = connection(store)?;
            let mut stmt = conn.prepare("SELECT p.poll_id,p.address,p.code,p.sent_at FROM meeting_poll_people p JOIN meeting_polls u ON u.id=p.poll_id WHERE u.integration_id=?1 AND u.account_key=?2 AND u.status='offen' AND p.send_state IN ('sent','sending','review') AND p.conversation_id IS NULL AND p.sent_at IS NOT NULL ORDER BY p.conversation_checked_at,p.sent_at LIMIT 20").map_err(|_| "poll_store")?;
            let values = stmt.query_map(params![integration,account_key],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_| "poll_store")?
                .collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "poll_store")?;
            values
        };
        let mut changed = false;
        for (id,address,code,since) in pending {
            let (a,own) = {
                let _guard = self.operation.lock().await;
                let conn = connection(store)?;
                if !has_open(&conn,integration,account_key)? { break; }
                if get(&conn,&id)?.poll.status != PollStatus::Offen { continue; }
                let (a,key,own) = account(svc,&conn,integration)?;
                if key != account_key { return Err("poll_account_changed".into()); }
                read_allowed(&a,Capability::AntwortenLesen)?;
                conn.execute("UPDATE meeting_poll_people SET conversation_checked_at=?3 WHERE poll_id=?1 AND address=?2",params![id,address,now()]).map_err(|_| "poll_store")?;
                audit_read(&conn,integration,Capability::AntwortenLesen,AuditOutcome::Pending)?;
                (a,own)
            };
            let subject = tf::betreff_marke(&id,&code).map_err(str::to_string)?;
            let result = svc.poll_sent_conversation(&a,&own,&subject,&address,since).await;
            let _guard = self.operation.lock().await;
            let conn = connection(store)?;
            audit_read(&conn,integration,Capability::AntwortenLesen,if result.is_ok() {AuditOutcome::Ok} else {AuditOutcome::Error})?;
            let conversation = result.map_err(|e| e.wire())?;
            let current = account(svc,&conn,integration)?;
            if current.1 != account_key { return Err("poll_account_changed".into()); }
            read_allowed(&current.0,Capability::AntwortenLesen)?;
            if let Some(conversation) = conversation {
                let count = conn.execute("UPDATE meeting_poll_people SET conversation_id=?3,send_state='sent' WHERE poll_id=?1 AND address=?2 AND EXISTS (SELECT 1 FROM meeting_polls WHERE id=?1 AND status='offen')",params![id,address,conversation]).map_err(|_| "poll_store")?;
                changed |= count > 0;
                if count > 0 { notify(); }
            }
        }
        Ok(changed)
    }
}

pub fn updated(app: &tauri::AppHandle, id: &str) {
    let _ = PollUpdated { poll_id: (!id.is_empty()).then(|| id.to_string()) }.emit(app);
}

pub fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            // Kein Versand in diesem Arbeiter. Ohne offene Umfrage kein Graph-Aufruf.
            tokio::time::sleep(Duration::from_secs(120)).await;
            let store = app.state::<Arc<MeetingStore>>();
            let svc = app.state::<Arc<M365Service>>();
            let runtime = app.state::<Arc<PollRuntime>>();
            runtime.prune_availability();
            let accounts = {
                let _guard = runtime.operation.lock().await;
                (|| -> Result<Vec<String>,String> {
                let conn = connection(&store)?;
                purge(&conn,now())?;
                // Nach Abschluss auch den Cursor und seinen Geheimnisverweis entfernen.
                let mut stmt = conn.prepare("SELECT integration_id,delta_token FROM meeting_poll_sync WHERE NOT EXISTS (SELECT 1 FROM meeting_polls p WHERE p.integration_id=meeting_poll_sync.integration_id AND p.status='offen')").map_err(|_| "poll_store")?;
                let idle = stmt.query_map([],|r| Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).map_err(|_| "poll_store")?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "poll_store")?;
                for (integration,_) in idle {
                    conn.execute("DELETE FROM meeting_poll_sync WHERE integration_id=?1",[&integration]).map_err(|_| "poll_store")?;
                    clear_cursors(&integration);
                }
                let mut stmt = conn.prepare("SELECT DISTINCT integration_id FROM meeting_polls WHERE status='offen'").map_err(|_| "poll_store")?;
                let values = stmt.query_map([],|r| r.get(0)).map_err(|_| "poll_store")?.collect::<rusqlite::Result<Vec<String>>>().map_err(|_| "poll_store")?;
                Ok(values)
                })()
            };
            if let Ok(accounts) = accounts {
                for integration in accounts {
                    let _ = runtime.sync(&svc,&store,&integration,|| updated(&app,"")).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests;
