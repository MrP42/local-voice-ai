//! Ausschliesslich strukturierte Umfragen/Antworten, keine Postfachinhalte.
use std::collections::BTreeSet;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use super::*;

pub const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

fn db<T>(r: rusqlite::Result<T>) -> Result<T, String> { r.map_err(|_| "poll_store".into()) }
fn json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    serde_json::from_str(s).map_err(|_| "poll_data".into())
}
fn encoded<T: serde::Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string(v).map_err(|_| "poll_data".into())
}

pub(super) fn salted_key(key: &[u8], salt: u32) -> Vec<u8> {
    if salt == 0 { return key.to_vec(); }
    let mut h = Sha256::new();
    h.update(b"terminfindung:salt:v1\0");
    h.update(key);
    h.update(salt.to_be_bytes());
    h.finalize().to_vec()
}

pub(super) fn unique_code(used: &mut BTreeSet<String>, mut make: impl FnMut(u32) -> Result<String, String>) -> Result<(String, u32), String> {
    for salt in 0..10_000 {
        let code = make(salt)?;
        if used.insert(code.clone()) { return Ok((code, salt)); }
    }
    Err("poll_code_collision".into())
}

pub(super) fn insert_poll(conn: &Connection, integration: &str, account: &str, u: &Umfrage, key: &[u8], now: i64) -> Result<(), String> {
    u.validiere().map_err(str::to_string)?;
    let tx = conn.unchecked_transaction().map_err(|_| "poll_store")?;
    db(tx.execute("INSERT INTO meeting_polls(id,integration_id,account_key,definition_json,status,created_at) VALUES (?1,?2,?3,?4,'entwurf',?5)",
        params![u.id, integration, account, encoded(u)?, now]))?;
    for v in &u.vorschlaege {
        db(tx.execute("INSERT INTO meeting_poll_slots(poll_id,nr,start,end) VALUES (?1,?2,?3,?4)", params![u.id,v.nr,v.start,v.ende]))?;
    }
    let mut used = BTreeSet::new();
    for p in &u.teilnehmende {
        let (code, salt) = unique_code(&mut used, |salt| tf::persoenlicher_code(&salted_key(key, salt), &u.id, &p.adresse).map_err(str::to_string))?;
        db(tx.execute("INSERT INTO meeting_poll_people(poll_id,address,code,salt) VALUES (?1,?2,?3,?4)",
            params![u.id, tf::normalisiere_adresse(&p.adresse), code, salt]))?;
    }
    db(tx.commit())
}

fn definition(conn: &Connection, id: &str) -> Result<PollDefinition, String> {
    let (definition, status): (String, String) = db(conn.query_row("SELECT definition_json,status FROM meeting_polls WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))))?;
    let mut poll: PollDefinition = json(&definition)?;
    poll.status = json(&format!("\"{status}\""))?;
    Ok(poll)
}

pub fn get(conn: &Connection, id: &str) -> Result<PollView, String> {
    let poll = definition(conn,id)?;
    let mut stmt = db(conn.prepare("SELECT id,sender,person,received_at,result_json,confirmed FROM meeting_poll_answers WHERE poll_id=?1 ORDER BY received_at,id"))?;
    let rows = db(stmt.query_map([id], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?))))?;
    let mut history = Vec::new();
    let mut answers: Vec<tf::Antwort> = Vec::new();
    for row in rows {
        let (id,sender,person,received_at,result,confirmed) = db(row)?;
        let mut result: tf::Ergebnis = json(&result)?;
        if confirmed { result.sicher = true; result.art = tf::ErgebnisArt::Antwort; }
        if result.sicher && result.art == tf::ErgebnisArt::Antwort {
            if let (Some(person), Some(quelle)) = (&person, result.quelle) {
                for value in &result.werte {
                    answers.retain(|a| a.person != *person || a.vorschlag_nr != value.vorschlag_nr);
                    answers.push(tf::Antwort { person: person.clone(), vorschlag_nr: value.vorschlag_nr, wert: value.wert,
                        quelle, sicher: true, eingegangen: received_at });
                }
            }
        }
        history.push(PollHistory { id, sender, person, received_at, result, confirmed });
    }
    let mut stmt = db(conn.prepare("SELECT address,send_state FROM meeting_poll_people WHERE poll_id=?1 ORDER BY address"))?;
    let deliveries = db(stmt.query_map([&poll.id], |r| Ok(PollDelivery { address:r.get(0)?, state:r.get(1)? })))?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "poll_store")?;
    Ok(PollView { poll, answers, history, deliveries })
}

pub fn list(conn: &Connection) -> Result<Vec<PollDefinition>, String> {
    let mut stmt = db(conn.prepare("SELECT id FROM meeting_polls ORDER BY created_at DESC,id"))?;
    let ids = db(stmt.query_map([], |r| r.get::<_,String>(0)))?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "poll_store")?;
    ids.iter().map(|id| definition(conn,id)).collect()
}

pub(super) fn binding(conn: &Connection, id: &str) -> Result<(String,String), String> {
    db(conn.query_row("SELECT integration_id,account_key FROM meeting_polls WHERE id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?))))
}

pub(super) fn has_open(conn: &Connection, integration: &str, account: &str) -> Result<bool,String> {
    db(conn.query_row("SELECT EXISTS(SELECT 1 FROM meeting_polls WHERE integration_id=?1 AND account_key=?2 AND status='offen')", params![integration,account], |r| r.get(0)))
}

/// Eine Seite samt Cursor atomar uebernehmen. Zwei feste Geheimnisfaecher
/// verhindern, dass ein Crash den noch aktiven Cursor ersetzt. Die Faecher sind
/// injizierbar, damit Tests weder Betriebskonto noch Geheimnisspeicher beruehren.
pub(super) fn apply_page(
    conn: &Connection,
    owner: (&str,&str),
    key: &[u8],
    page: &crate::managers::integrations::m365::polls::DeltaPage,
    current: Option<&str>,
    mut save: impl FnMut(&str,&str) -> Result<(),String>,
    mut delete: impl FnMut(&str),
) -> Result<(bool,String),String> {
    let (integration,account) = owner;
    let tx = conn.unchecked_transaction().map_err(|_| "poll_store")?;
    let mut changed = false;
    for mail in &page.messages { changed |= ingest(&tx,integration,account,key,mail)?; }
    let next = if current==Some("poll_delta0") { "poll_delta1" } else { "poll_delta0" };
    if let Err(error) = save(next,&page.cursor) { delete(next); return Err(error); }
    if !matches!(tx.execute("UPDATE meeting_poll_sync SET delta_token=?2,next_at=?3 WHERE integration_id=?1 AND account_key=?4",params![integration,next,now()+120_000,account]),Ok(1)) {
        delete(next);
        return Err("poll_store".into());
    }
    if tx.commit().is_err() { delete(next); return Err("poll_store".into()); }
    if let Some(old) = current { delete(old); }
    Ok((changed,next.to_string()))
}

fn review(reason: &str) -> tf::Ergebnis {
    tf::Ergebnis { art: tf::ErgebnisArt::Pruefen, werte: vec![], quelle: None, sicher: false, grund: reason.into() }
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Die from-Adresse stammt ausschliesslich von Graph. Kein Anzeigename, kein
/// Reply-To und keine Adresse aus dem Text darf diese Bindung ersetzen.
pub(super) fn ingest(conn: &Connection, integration: &str, account: &str, key: &[u8], mail: &Value) -> Result<bool,String> {
    if mail.get("@removed").is_some() { return Ok(false); }
    let Some(message_id) = text(mail,"id").filter(|s| s.len() <= 2048) else { return Ok(false); };
    let marker = text(mail,"subject").and_then(tf::finde_marke);
    let conversation = text(mail,"conversationId");
    let mut stmt = db(conn.prepare("SELECT id FROM meeting_polls WHERE integration_id=?1 AND account_key=?2 AND status='offen'"))?;
    let ids = db(stmt.query_map(params![integration,account], |r| r.get::<_,String>(0)))?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_| "poll_store")?;
    let mut changed = false;
    for id in ids {
        let conv_person: Option<String> = if let Some(c) = conversation {
            db(conn.query_row("SELECT address FROM meeting_poll_people WHERE poll_id=?1 AND conversation_id=?2", params![id,c], |r| r.get(0)).optional())?
        } else { None };
        if !marker.as_ref().is_some_and(|(poll,_)| poll == &id) && conv_person.is_none() { continue; }
        if db(conn.query_row("SELECT EXISTS(SELECT 1 FROM meeting_poll_answers WHERE poll_id=?1 AND message_id=?2)",params![id,message_id],|r| r.get::<_,bool>(0)))? { continue; }
        let poll = definition(conn,&id)?.into_core();
        let received_at = text(mail,"receivedDateTime").and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp_millis());
        let created_at: i64 = db(conn.query_row("SELECT created_at FROM meeting_polls WHERE id=?1",[&id],|r| r.get(0)))?;
        // Ein neu erzeugter Kurzcode darf keine historischen Antworten reaktivieren,
        // falls dieselbe kurze Umfrage-ID nach der Aufbewahrung erneut entsteht.
        if received_at.is_some_and(|t| t < created_at) { continue; }
        let sender = mail.pointer("/from/emailAddress/address").and_then(Value::as_str)
            .map(tf::normalisiere_adresse).filter(|s| valid_address(s)).unwrap_or_default();
        let known = poll.teilnehmende.iter().any(|p| p.adresse == sender);
        let invalid_count: u32 = db(conn.query_row("SELECT count(*) FROM meeting_poll_answers WHERE poll_id=?1 AND sender=?2 AND invalid_code=1", params![id,sender], |r| r.get(0)))?;
        let code_person: Option<String> = if let Some((poll,code)) = &marker {
            if poll == &id { db(conn.query_row("SELECT address FROM meeting_poll_people WHERE poll_id=?1 AND code=?2",params![id,code],|r| r.get(0)).optional())? } else { None }
        } else { None };
        let valid_code = if let Some((marker_id,code)) = &marker {
            let salt: Option<u32> = db(conn.query_row("SELECT salt FROM meeting_poll_people WHERE poll_id=?1 AND address=?2",params![id,sender],|r| r.get(0)).optional())?;
            marker_id == &id && salt.is_some_and(|salt| tf::pruefe_code(&salted_key(key,salt),&id,&sender,code))
        } else {
            // Eine normale Antwort ohne Marke darf nur zum gespeicherten
            // Gesendet-Thread UND zu dessen Empfaenger passen.
            conv_person.as_deref() == Some(sender.as_str()) && !sender.is_empty()
                && !text(mail,"subject").is_some_and(|s| s.contains("[Terminfindung "))
        };
        let invalid_code = !valid_code;
        let person = if known { Some(sender.clone()) } else { code_person.or(conv_person) };
        let auto = mail.get("internetMessageHeaders").and_then(Value::as_array).is_some_and(|headers| headers.iter().any(|h| {
            let name = text(h,"name").unwrap_or("");
            let value = text(h,"value").unwrap_or("").trim();
            (name.eq_ignore_ascii_case("Auto-Submitted") && !value.eq_ignore_ascii_case("no"))
                || (name.eq_ignore_ascii_case("X-Autoreply") || name.eq_ignore_ascii_case("X-Autorespond"))
        }));
        let mut result = if auto {
            tf::Ergebnis { art: tf::ErgebnisArt::Ignorieren, grund: "Automatische Abwesenheitsantwort.".into(), ..review("") }
        } else if let Some(body) = mail.get("body") {
            let content = text(body,"content").unwrap_or("");
            if text(body,"contentType").is_some_and(|s| s.eq_ignore_ascii_case("text")) {
                tf::lese_antwort(content,&poll)
            } else if content.len() <= tf::MAX_ANTWORT_BYTES {
                let plain = crate::media::html_to_text(content);
                let mut r = tf::lese_antwort(&plain,&poll);
                r.sicher = false;
                r.grund = "HTML-Antwort: Bitte bestaetigen.".into();
                r
            } else { review("Antwort zu lang.") }
        } else { review("Keine neue Antwort.") };
        let forwarded = text(mail,"subject").is_some_and(|s| {
            let lower = s.to_lowercase();
            let prefix = lower.split("[terminfindung ").next().unwrap_or("");
            prefix.split(':').any(|p| matches!(p.trim(),"fw"|"fwd"|"wg"))
                || prefix.trim_start().starts_with("forwarded")
                || prefix.trim_start().starts_with("weitergeleitet")
        });
        if result.art != tf::ErgebnisArt::Ignorieren {
            let reason = if invalid_count > 5 { Some("Versuchslimit: Bitte pruefen.") }
                else if forwarded { Some("Weiterleitung: Bitte pruefen.") }
                else if !valid_code || !known { Some("Absender oder Code ungueltig: Bitte pruefen.") }
                else if received_at.is_none() { Some("Antwortzeit fehlt: Bitte pruefen.") }
                else { None };
            if let Some(reason) = reason { result.art = tf::ErgebnisArt::Pruefen; result.sicher = false; result.grund = reason.into(); }
        }
        db(conn.execute("INSERT INTO meeting_poll_answers(poll_id,message_id,sender,person,received_at,result_json,invalid_code) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![id,message_id,sender,person,received_at.unwrap_or(0),encoded(&result)?,invalid_code]))?;
        changed = true;
    }
    Ok(changed)
}

pub fn confirm(conn: &Connection, id: &str, answer_id: i64) -> Result<(),String> {
    let view = get(conn,id)?;
    if view.poll.status != PollStatus::Offen { return Err("poll_closed".into()); }
    let row = view.history.iter().find(|h| h.id == answer_id).ok_or("poll_answer_missing")?;
    if row.result.art == tf::ErgebnisArt::Ignorieren || row.result.werte.is_empty()
        || !row.person.as_ref().is_some_and(|a| view.poll.teilnehmende.iter().any(|p| &p.adresse == a))
    { return Err("poll_answer_set_required".into()); }
    db(conn.execute("UPDATE meeting_poll_answers SET confirmed=1 WHERE poll_id=?1 AND id=?2",params![id,answer_id]))?;
    Ok(())
}

pub fn set_answer(conn: &Connection, id: &str, person: &str, values: Vec<tf::AntwortWert>, now: i64) -> Result<(),String> {
    let view = get(conn,id)?;
    let person = tf::normalisiere_adresse(person);
    let mut seen = BTreeSet::new();
    if view.poll.status != PollStatus::Offen || !view.poll.teilnehmende.iter().any(|p| p.adresse == person)
        || values.is_empty() || values.iter().any(|v| !seen.insert(v.vorschlag_nr) || !view.poll.vorschlaege.iter().any(|s| s.nr == v.vorschlag_nr))
    { return Err("poll_answer_invalid".into()); }
    let result = tf::Ergebnis { art:tf::ErgebnisArt::Antwort,werte:values,quelle:Some(tf::Quelle::Auswahl),sicher:true,grund:"Manuell gesetzt.".into() };
    db(conn.execute("INSERT INTO meeting_poll_answers(poll_id,message_id,sender,person,received_at,result_json,confirmed) VALUES (?1,?2,?3,?3,?4,?5,1)",
        params![id,format!("manual:{}",ulid::Ulid::new()),person,now,encoded(&result)?]))?;
    Ok(())
}

pub fn close(conn: &Connection, id: &str, decided: bool, now: i64) -> Result<(),String> {
    let changed = db(conn.execute("UPDATE meeting_polls SET status=?2,closed_at=?3 WHERE id=?1 AND closed_at IS NULL", params![id,if decided {"entschieden"} else {"abgebrochen"},now]))?;
    if changed == 0 { return Err("poll_closed".into()); }
    Ok(())
}

pub fn purge(conn: &Connection, now: i64) -> Result<(),String> {
    db(conn.execute("DELETE FROM meeting_polls WHERE closed_at IS NOT NULL AND closed_at<=?1",[now.saturating_sub(RETENTION_MS)]))?;
    Ok(())
}
