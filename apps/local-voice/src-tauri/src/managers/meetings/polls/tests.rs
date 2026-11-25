use super::*;
use serde_json::json;

fn fixture() -> (rusqlite::Connection, Umfrage) {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    conn.execute_batch(schema::POLLS_MIGRATION).unwrap();
    let u = Umfrage {
        id: "T-ABCD".into(), plan_id: None, titel: "Synthetic".into(), titel_an_externe: false,
        dauer_min: 30, frist: 9_000_000, status: Status::Entwurf,
        vorschlaege: vec![tf::Vorschlag { nr: 1, start: 0, ende: 1_800_000, ortszeit: None }],
        teilnehmende: vec![tf::Person { adresse: "one@example.invalid".into(), name: "".into(), intern: true, pflicht: true }],
    };
    insert_poll(&conn, "integration", "tenant/account", &u, &[7; 32], 0).unwrap();
    conn.execute("UPDATE meeting_polls SET status='offen'", []).unwrap();
    (conn, u)
}

fn message(subject: &str, from: &str, body: &str, id: &str) -> Value {
    json!({"id":id,"subject":subject,"from":{"emailAddress":{"address":from,"name":"one@example.invalid"}},
        "receivedDateTime":"2026-12-01T10:00:00Z", "body":{"contentType":"text","content":body}})
}

fn subject(conn: &rusqlite::Connection) -> String {
    let code: String = conn.query_row("SELECT code FROM meeting_poll_people", [], |r| r.get(0)).unwrap();
    tf::betreff_marke("T-ABCD", &code).unwrap()
}

#[test]
fn sender_binding_uses_graph_address_not_display_name() {
    let (conn, _) = fixture();
    let s = subject(&conn);
    ingest(&conn, "integration", "tenant/account", &[7; 32], &message(&s, "other@example.invalid", "Alle Termine passen", "m1")).unwrap();
    let p = get(&conn, "T-ABCD").unwrap();
    assert!(!p.history[0].result.sicher);
    assert_eq!(p.history[0].result.art, tf::ErgebnisArt::Pruefen);
    ingest(&conn, "integration", "tenant/account", &[7; 32], &message(&s, "one@example.invalid", "Alle Termine passen", "m2")).unwrap();
    assert!(get(&conn, "T-ABCD").unwrap().history[1].result.sicher);
}

#[test]
fn sixth_invalid_code_locks_sender_but_not_another_poll() {
    let (conn, _) = fixture();
    for n in 0..6 {
        ingest(&conn, "integration", "tenant/account", &[7; 32], &message("[Terminfindung T-ABCD-ZZZZ]", "one@example.invalid", "Alle Termine passen", &format!("bad{n}"))).unwrap();
    }
    ingest(&conn, "integration", "tenant/account", &[7; 32], &message(&subject(&conn), "one@example.invalid", "Alle Termine passen", "good")).unwrap();
    let p = get(&conn, "T-ABCD").unwrap();
    assert_eq!(p.history.len(), 7);
    assert!(!p.history[6].result.sicher);
    assert_eq!(p.history[6].result.grund, "Versuchslimit: Bitte pruefen.");
    let mut other = p.poll.into_core();
    other.id = "T-EFGH".into();
    insert_poll(&conn,"integration","tenant/account",&other,&[7;32],0).unwrap();
    conn.execute("UPDATE meeting_polls SET status='offen' WHERE id='T-EFGH'",[]).unwrap();
    let code: String = conn.query_row("SELECT code FROM meeting_poll_people WHERE poll_id='T-EFGH'",[],|r| r.get(0)).unwrap();
    let subject = tf::betreff_marke("T-EFGH",&code).unwrap();
    ingest(&conn,"integration","tenant/account",&[7;32],&message(&subject,"one@example.invalid","Alle Termine passen","other")).unwrap();
    assert!(get(&conn,"T-EFGH").unwrap().history[0].result.sicher);
}

#[test]
fn quoted_reply_auto_reply_dedup_and_tenant_isolation() {
    let (conn, _) = fixture();
    let s = subject(&conn);
    let m = message(&s, "one@example.invalid", "Alle Termine passen\n> Keiner passt", "m1");
    assert!(!ingest(&conn, "integration", "other-tenant", &[7; 32], &m).unwrap());
    assert!(ingest(&conn, "integration", "tenant/account", &[7; 32], &m).unwrap());
    assert!(!ingest(&conn, "integration", "tenant/account", &[7; 32], &m).unwrap());
    let mut auto = message(&s, "one@example.invalid", "Alle Termine passen", "m2");
    auto["internetMessageHeaders"] = json!([{"name":"Auto-Submitted","value":"auto-replied"}]);
    ingest(&conn, "integration", "tenant/account", &[7; 32], &auto).unwrap();
    let p = get(&conn, "T-ABCD").unwrap();
    assert!(p.history[0].result.sicher);
    assert_eq!(p.history[0].result.werte[0].wert, tf::Wert::Passt);
    assert_eq!(p.history[1].result.art, tf::ErgebnisArt::Ignorieren);
}

#[test]
fn no_open_poll_means_no_polling_and_retention_is_after_close() {
    let (conn, _) = fixture();
    assert!(has_open(&conn, "integration", "tenant/account").unwrap());
    close(&conn, "T-ABCD", false, 100).unwrap();
    assert!(!has_open(&conn, "integration", "tenant/account").unwrap());
    purge(&conn, 100 + RETENTION_MS - 1).unwrap();
    assert_eq!(list(&conn).unwrap().len(), 1);
    purge(&conn, 100 + RETENTION_MS).unwrap();
    assert!(list(&conn).unwrap().is_empty());
}

#[test]
fn collision_increases_salt_and_a_newer_unsafe_reply_does_not_replace_a_safe_one() {
    let mut used = std::collections::BTreeSet::from(["AAAA".to_string()]);
    let (code, salt) = unique_code(&mut used, |salt| Ok(if salt == 0 { "AAAA" } else { "BBBB" }.into())).unwrap();
    assert_eq!((code.as_str(), salt), ("BBBB", 1));
    let (conn, _) = fixture();
    ingest(&conn, "integration", "tenant/account", &[7; 32], &message(&subject(&conn), "one@example.invalid", "Alle Termine passen", "m1")).unwrap();
    ingest(&conn, "integration", "tenant/account", &[7; 32], &message(&subject(&conn), "one@example.invalid", "nein", "m2")).unwrap();
    let p = get(&conn, "T-ABCD").unwrap();
    assert_eq!(p.answers[0].wert, tf::Wert::Passt);
    confirm(&conn, "T-ABCD", p.history[1].id).unwrap();
    assert_eq!(get(&conn, "T-ABCD").unwrap().answers[0].wert, tf::Wert::PasstNicht);
}

#[test]
fn historical_reply_and_unrelated_mail_are_not_retained() {
    let (conn, _) = fixture();
    let mut old = message(&subject(&conn),"one@example.invalid","Alle Termine passen","old");
    old["receivedDateTime"] = json!("1969-12-31T23:59:59Z");
    assert!(!ingest(&conn,"integration","tenant/account",&[7;32],&old).unwrap());
    assert!(!ingest(&conn,"integration","tenant/account",&[7;32],&message("Private unrelated mail","one@example.invalid","SECRET","unrelated")).unwrap());
    assert!(get(&conn,"T-ABCD").unwrap().history.is_empty());
}

#[test]
fn conversation_fallback_is_bound_to_its_recipient_and_html_is_never_sure() {
    let (conn, _) = fixture();
    conn.execute("UPDATE meeting_poll_people SET conversation_id='synthetic-thread'",[]).unwrap();
    let mut reply = message("Re: reply without marker","one@example.invalid","Alle Termine passen","conversation");
    reply["conversationId"] = json!("synthetic-thread");
    ingest(&conn,"integration","tenant/account",&[7;32],&reply).unwrap();
    assert!(get(&conn,"T-ABCD").unwrap().history[0].result.sicher);
    reply["id"] = json!("forwarded");
    reply["from"]["emailAddress"]["address"] = json!("other@example.invalid");
    ingest(&conn,"integration","tenant/account",&[7;32],&reply).unwrap();
    assert!(!get(&conn,"T-ABCD").unwrap().history[1].result.sicher);
    let mut html = message(&subject(&conn),"one@example.invalid","<p>Alle Termine passen</p>","html");
    html["body"]["contentType"] = json!("html");
    ingest(&conn,"integration","tenant/account",&[7;32],&html).unwrap();
    assert!(!get(&conn,"T-ABCD").unwrap().history[2].result.sicher);
}

#[test]
fn delta_followup_token_and_secret_failure_are_atomic_without_network() {
    use crate::managers::integrations::m365::polls::parse_delta;
    let (conn, _) = fixture();
    conn.execute("INSERT INTO meeting_poll_sync(integration_id,account_key) VALUES ('integration','tenant/account')",[]).unwrap();
    let base = "https://graph.microsoft.com/v1.0";
    let next = format!("{base}/me/mailFolders/inbox/messages/delta?$skiptoken=SYNTHETIC");
    let delta = format!("{base}/me/mailFolders/inbox/messages/delta?$deltatoken=SYNTHETIC");
    let page = parse_delta(json!({"value":[message(&subject(&conn),"one@example.invalid","Alle Termine passen","page1")],"@odata.nextLink":next}),base).unwrap();
    let mut secrets = std::collections::HashMap::new();
    let (changed, first) = apply_page(&conn,("integration","tenant/account"),&[7;32],&page,None,
        |slot,token| { secrets.insert(slot.to_string(),token.to_string()); Ok(()) }, |_| {}).unwrap();
    assert!(changed);
    assert_eq!(secrets[&first],next);
    let page = parse_delta(json!({"value":[message(&subject(&conn),"one@example.invalid","Keiner passt","page2")],"@odata.deltaLink":delta}),base).unwrap();
    assert!(apply_page(&conn,("integration","tenant/account"),&[7;32],&page,Some(&first),|_,_| Err("synthetic_full_disk".into()),|_| {}).is_err());
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),1);
    let current: String = conn.query_row("SELECT delta_token FROM meeting_poll_sync",[],|r| r.get(0)).unwrap();
    assert_eq!(current,first);
    let (_,second) = apply_page(&conn,("integration","tenant/account"),&[7;32],&page,Some(&first),
        |slot,token| { secrets.insert(slot.to_string(),token.to_string()); Ok(()) }, |_| {}).unwrap();
    assert_ne!(first,second);
    assert_eq!(secrets[&second],delta);
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),2);
}

#[test]
fn availability_expiry_and_close_release_cached_data() {
    let runtime = PollRuntime::default();
    {
        let mut cache = runtime.availability.lock().unwrap();
        cache.insert("expired".into(),CachedAvailability { until:Instant::now()-Duration::from_secs(1),values:vec![] });
        cache.insert("live".into(),CachedAvailability { until:Instant::now()+Duration::from_secs(15*60),values:vec![] });
    }
    runtime.prune_availability();
    assert_eq!(runtime.availability.lock().unwrap().len(),1);
    runtime.clear_availability();
    assert!(runtime.availability.lock().unwrap().is_empty());
}

#[test]
fn forwarded_subject_stays_in_review_even_with_valid_sender_and_code() {
    let (conn, _) = fixture();
    let marker = subject(&conn);
    for (n,prefix) in ["FW:","Fwd:","WG:","Re: FW:"].iter().enumerate() {
        ingest(&conn,"integration","tenant/account",&[7;32],&message(&format!("{prefix} {marker}"),"one@example.invalid","Alle Termine passen",&format!("forward{n}"))).unwrap();
    }
    let view = get(&conn,"T-ABCD").unwrap();
    assert!(view.history.iter().all(|h| !h.result.sicher && h.result.art == tf::ErgebnisArt::Pruefen));
    ingest(&conn,"integration","tenant/account",&[7;32],&message(&format!("Re: {marker}"),"one@example.invalid","Alle Termine passen","normal")).unwrap();
    assert!(get(&conn,"T-ABCD").unwrap().history.last().unwrap().result.sicher);
}
