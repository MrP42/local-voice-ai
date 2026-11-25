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

#[test]
fn draft_update_preserves_identity_codes_and_replaces_slots_atomically() {
    let (conn, mut poll) = fixture();
    conn.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    let old_subject = subject(&conn);
    poll.id = "T-EFGH".into();
    poll.status = Status::Offen;
    poll.titel = "Changed".into();
    poll.vorschlaege[0].nr = 3;
    poll.teilnehmende.push(tf::Person { adresse:"TWO@example.invalid".into(),name:"".into(),intern:false,pflicht:false });
    let view = update(&conn,"T-ABCD",poll.clone(),&[7;32],1).unwrap();
    assert_eq!(view.poll.id,"T-ABCD");
    assert_eq!(view.poll.status,PollStatus::Entwurf);
    assert_eq!(view.poll.vorschlaege[0].nr,3);
    assert_eq!(view.deliveries.len(),2);
    let slot: u8 = conn.query_row("SELECT nr FROM meeting_poll_slots",[],|r| r.get(0)).unwrap();
    assert_eq!(slot,3);
    let code: String = conn.query_row("SELECT code FROM meeting_poll_people WHERE address='one@example.invalid'",[],|r| r.get(0)).unwrap();
    assert_eq!(tf::betreff_marke("T-ABCD",&code).unwrap(),old_subject);
    let before = serde_json::to_value(get(&conn,"T-ABCD").unwrap()).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_update BEFORE INSERT ON meeting_poll_slots BEGIN SELECT RAISE(ABORT,'synthetic'); END;").unwrap();
    poll.teilnehmende.remove(0);
    poll.vorschlaege[0].nr = 4;
    assert!(update(&conn,"T-ABCD",poll,&[7;32],1).is_err());
    assert_eq!(serde_json::to_value(get(&conn,"T-ABCD").unwrap()).unwrap(),before);
    let slot: u8 = conn.query_row("SELECT nr FROM meeting_poll_slots",[],|r| r.get(0)).unwrap();
    assert_eq!(slot,3);
}

#[test]
fn update_rejects_non_drafts_and_invalid_definitions_without_changes() {
    let (conn, poll) = fixture();
    for status in ["offen","entschieden","abgebrochen"] {
        conn.execute("UPDATE meeting_polls SET status=?1",[status]).unwrap();
        assert_eq!(update(&conn,"T-ABCD",poll.clone(),&[7;32],1).unwrap_err(),"poll_not_draft");
    }
    conn.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    let before = serde_json::to_value(get(&conn,"T-ABCD").unwrap()).unwrap();
    for case in 0..6 {
        let mut bad = poll.clone();
        match case {
            0 => bad.frist = 1,
            1 => bad.vorschlaege.clear(),
            2 => bad.vorschlaege = vec![bad.vorschlaege[0].clone();9],
            3 => bad.dauer_min = 0,
            4 => bad.vorschlaege[0].nr = 9,
            _ => bad.teilnehmende.clear(),
        }
        assert!(update(&conn,"T-ABCD",bad,&[7;32],1).is_err());
        assert_eq!(serde_json::to_value(get(&conn,"T-ABCD").unwrap()).unwrap(),before);
    }
}

fn rank_owner() -> (String,String) { ("integration".into(),"tenant/account".into()) }

fn rank_window() -> ScheduleWindow {
    ScheduleWindow { start:"1970-01-01T00:00:00".into(),end:"1970-01-01T01:00:00".into(),time_zone:"UTC".into() }
}

fn rank_fixture() -> rusqlite::Connection {
    let (conn, mut poll) = fixture();
    conn.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    poll.vorschlaege.push(tf::Vorschlag { nr:2,start:1_800_000,ende:3_600_000,ortszeit:None });
    update(&conn,"T-ABCD",poll,&[7;32],1).unwrap();
    conn.execute("UPDATE meeting_polls SET status='offen'",[]).unwrap();
    conn
}

fn cache_rank(runtime: &PollRuntime, states: Vec<crate::managers::integrations::m365::polls::Availability>) {
    runtime.availability.lock().unwrap().insert("synthetic".into(),CachedAvailability {
        until:Instant::now()+Duration::from_secs(15*60),window:rank_window(),owner:rank_owner(),
        values:vec![PersonAvailability { address:"one@example.invalid".into(),intervals:states }],
    });
}

#[test]
fn rank_uses_only_live_account_bound_cache_and_marks_missing_availability() {
    use crate::managers::integrations::m365::polls::Availability::{Busy,Free};
    let conn = rank_fixture();
    let runtime = PollRuntime::default();
    let rank = || runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
    assert_eq!(rank()[0].nr,1);
    assert!(rank()[0].begruendung.contains("Kalender-Verfügbarkeit fehlt"));
    cache_rank(&runtime,vec![Busy,Free]);
    let cached = rank();
    assert_eq!(cached[0].nr,2);
    assert_eq!(cached[0].punkte,6);
    assert!(cached[1].ausgeschlossen);
    assert!(!cached[0].begruendung.contains("Kalender-Verfügbarkeit fehlt"));
    // Gegenprobe: derselbe Cache darf fuer ein anderes Konto nicht zaehlen.
    let foreign = ("integration".into(),"other-account".into());
    assert_eq!(runtime.rank_cached(&conn,"T-ABCD",Some(&foreign)).unwrap()[0].punkte,0);
    assert_eq!(runtime.rank_cached(&conn,"T-ABCD",None).unwrap()[0].punkte,0);
    runtime.availability.lock().unwrap().get_mut("synthetic").unwrap().until = Instant::now()-Duration::from_secs(1);
    assert_eq!(rank()[0].nr,1);
    assert_eq!(rank()[0].punkte,0);
}

#[test]
fn rank_changes_only_after_confirming_free_text_and_answers_override_cache() {
    use crate::managers::integrations::m365::polls::Availability::{Busy,Free};
    let conn = rank_fixture();
    let runtime = PollRuntime::default();
    cache_rank(&runtime,vec![Busy,Free]);
    let result = tf::Ergebnis { art:tf::ErgebnisArt::Antwort,quelle:Some(tf::Quelle::Freitext),sicher:false,
        werte:vec![tf::AntwortWert { vorschlag_nr:1,wert:tf::Wert::Passt },tf::AntwortWert { vorschlag_nr:2,wert:tf::Wert::PasstNicht }],grund:"Synthetic".into() };
    conn.execute("INSERT INTO meeting_poll_answers(poll_id,message_id,sender,person,received_at,result_json) VALUES ('T-ABCD','synthetic','one@example.invalid','one@example.invalid',1,?1)",[serde_json::to_string(&result).unwrap()]).unwrap();
    let rank = || runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
    assert_eq!(rank()[0].nr,2);
    let id = get(&conn,"T-ABCD").unwrap().history[0].id;
    confirm(&conn,"T-ABCD",id).unwrap();
    assert_eq!(rank()[0].nr,1);
    assert_eq!(rank()[0].punkte,6);
    assert!(rank()[1].ausgeschlossen);
    runtime.clear_availability();
    assert_eq!(rank()[0].nr,1);
    assert_eq!(rank()[0].punkte,6);
}

#[test]
fn rank_does_not_reuse_older_free_values_after_a_newer_unknown_result() {
    use crate::managers::integrations::m365::polls::Availability::{Free,Unknown};
    let conn = rank_fixture();
    let runtime = PollRuntime::default();
    cache_rank(&runtime,vec![Free,Free]);
    assert_eq!(runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap()[0].punkte,6);
    {
        let mut cache = runtime.availability.lock().unwrap();
        let until = cache["synthetic"].until + Duration::from_secs(1);
        cache.insert("newer".into(),CachedAvailability {
            until,window:rank_window(),owner:rank_owner(),
            values:vec![PersonAvailability { address:"one@example.invalid".into(),intervals:vec![Unknown,Unknown] }],
        });
    }
    let ranks = runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
    assert_eq!(ranks[0].punkte,0);
    assert!(ranks[0].begruendung.contains("Kalender-Verfügbarkeit fehlt"));
}

#[test]
fn rank_keeps_known_conflicts_in_slots_with_unknown_intervals() {
    use crate::managers::integrations::m365::polls::Availability::{Busy,Free,Oof,Unknown};
    let (conn, mut poll) = fixture();
    conn.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    poll.dauer_min = 60;
    poll.vorschlaege[0].ende = 3_600_000;
    update(&conn,"T-ABCD",poll,&[7;32],1).unwrap();
    let runtime = PollRuntime::default();
    for states in [vec![Busy,Unknown],vec![Unknown,Busy],vec![Oof,Unknown],vec![Unknown,Oof]] {
        cache_rank(&runtime,states);
        let ranks = runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
        assert!(ranks[0].ausgeschlossen);
        assert_eq!(ranks[0].punkte,0);
    }
    // Ohne bekannten Konflikt darf Unknown weder ausschliessen noch zusagen.
    cache_rank(&runtime,vec![Free,Unknown]);
    let ranks = runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
    assert!(!ranks[0].ausgeschlossen);
    assert_eq!(ranks[0].punkte,0);
}

#[tokio::test]
async fn rank_finishes_while_operation_is_held() {
    let conn = rank_fixture();
    let runtime = PollRuntime::default();
    let _held = runtime.operation.lock().await;
    let ranks = tokio::time::timeout(Duration::from_millis(250), async {
        runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner()))
    }).await.expect("rank waited for operation").unwrap();
    assert_eq!(ranks.len(),2);
}

#[test]
fn send_snapshot_blocks_a_concurrent_draft_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("synthetic.db");
    let (source, mut poll) = fixture();
    poll.vorschlaege.push(tf::Vorschlag { nr:2,start:1_800_000,ende:3_600_000,ortszeit:None });
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(schema::POLLS_MIGRATION).unwrap();
    insert_poll(&conn,"integration","tenant/account",&poll,&[7;32],0).unwrap();
    // Eine vor Versand abgeschlossene Aenderung muss im Versandstand stehen.
    poll.titel = "Latest synthetic draft".into();
    poll.teilnehmende[0].adresse = "latest@example.invalid".into();
    poll.vorschlaege[0].start = 3_600_000;
    poll.vorschlaege[0].ende = 5_400_000;
    update(&conn,"T-ABCD",poll.clone(),&[7;32],1).unwrap();
    let (opened, started) = std::sync::mpsc::channel();
    let (done, finished) = std::sync::mpsc::channel();
    let sender = std::thread::spawn(move || {
        let snapshot = open_for_send(&conn,"T-ABCD",1).unwrap();
        opened.send(snapshot).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    });
    let snapshot = started.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(snapshot.titel,"Latest synthetic draft");
    assert_eq!(snapshot.teilnehmende[0].adresse,"latest@example.invalid");
    assert_eq!(snapshot.vorschlaege[0].start,3_600_000);
    let updater = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(update(&updater,"T-ABCD",poll.clone(),&[7;32],2).unwrap_err(),"poll_not_draft");
    done.send(()).unwrap();
    sender.join().unwrap();
    // Gegenprobe: ein unbeanspruchter Entwurf bleibt bearbeitbar.
    source.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    assert!(update(&source,"T-ABCD",poll,&[7;32],2).is_ok());
}

#[test]
fn rank_cache_covers_entire_slot_and_never_scores_external_calendars() {
    use crate::managers::integrations::m365::polls::Availability::{Busy,Free,Unknown};
    let (conn, mut poll) = fixture();
    conn.execute("UPDATE meeting_polls SET status='entwurf'",[]).unwrap();
    poll.dauer_min = 60;
    poll.vorschlaege[0].ende = 3_600_000;
    update(&conn,"T-ABCD",poll.clone(),&[7;32],1).unwrap();
    let runtime = PollRuntime::default();
    let rank = || runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap();
    cache_rank(&runtime,vec![Free,Busy]);
    assert!(rank()[0].ausgeschlossen);
    cache_rank(&runtime,vec![Free,Unknown]);
    assert_eq!(rank()[0].punkte,0);
    assert!(!rank()[0].ausgeschlossen);
    cache_rank(&runtime,vec![Free]);
    assert_eq!(rank()[0].punkte,0);
    cache_rank(&runtime,vec![Free,Free]);
    assert_eq!(rank()[0].punkte,6);
    poll.teilnehmende[0].intern = false;
    update(&conn,"T-ABCD",poll,&[7;32],1).unwrap();
    assert_eq!(rank()[0].punkte,0);
}

#[test]
fn rank_cache_maps_local_zones_and_rejects_ambiguous_times() {
    use crate::managers::integrations::m365::polls::Availability::Free;
    let conn = rank_fixture();
    let runtime = PollRuntime::default();
    cache_rank(&runtime,vec![Free,Free]);
    {
        let mut cache = runtime.availability.lock().unwrap();
        let window = &mut cache.get_mut("synthetic").unwrap().window;
        window.start = "1970-01-01T01:00:00".into();
        window.end = "1970-01-01T02:00:00".into();
        window.time_zone = "Europe/Berlin".into();
    }
    assert_eq!(runtime.rank_cached(&conn,"T-ABCD",Some(&rank_owner())).unwrap()[0].punkte,6);
    let mut cache = runtime.availability.lock().unwrap();
    let entry = cache.get_mut("synthetic").unwrap();
    entry.window = ScheduleWindow { start:"2026-10-25T02:00:00".into(),end:"2026-10-25T03:00:00".into(),time_zone:"Europe/Berlin".into() };
    assert!(super::ranking::intervals(&entry.window).is_none());
    entry.window.time_zone = "Synthetic/Unknown".into();
    assert!(super::ranking::intervals(&entry.window).is_none());
}

#[test]
fn only_a_gate_denial_releases_the_send_claim() {
    use crate::managers::integrations::model::IntegrationError;
    let cases = [
        (Ok(GateOutcome::Denied { reason: "synthetic".into(), code: "direction" }), "pending"),
        (Ok(GateOutcome::Done(())), "sent"),
        (Ok(GateOutcome::Failed("synthetic".into())), "review"),
        (Ok(GateOutcome::Pending { approval_id: "synthetic".into() }), "review"),
        (Err(IntegrationError::Store("synthetic".into())), "review"),
    ];
    for (result, expected) in cases {
        let (conn, _) = fixture();
        conn.execute("UPDATE meeting_poll_people SET send_state='sending',sent_at=100",[]).unwrap();
        finish_send(&conn,"T-ABCD","one@example.invalid",&result).unwrap();
        let (state, since): (String,Option<i64>) = conn.query_row("SELECT send_state,sent_at FROM meeting_poll_people",[],|r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(state,expected);
        assert_eq!(since,if expected == "pending" { None } else { Some(100) });
        let claimed = conn.execute("UPDATE meeting_poll_people SET send_state='sending' WHERE send_state='pending'",[]).unwrap();
        assert_eq!(claimed,usize::from(expected == "pending"));
    }
}

#[test]
fn committed_page_notifies_before_a_later_error_and_not_on_replay_or_rollback() {
    use crate::managers::integrations::m365::polls::parse_delta;
    let (conn, _) = fixture();
    conn.execute("INSERT INTO meeting_poll_sync(integration_id,account_key) VALUES ('integration','tenant/account')",[]).unwrap();
    let base = "https://graph.microsoft.com/v1.0";
    let page = parse_delta(json!({"value":[message(&subject(&conn),"one@example.invalid","Alle Termine passen","page1")],
        "@odata.nextLink":format!("{base}/me/mailFolders/inbox/messages/delta?$skiptoken=SYNTHETIC")}),base).unwrap();
    let mut events = 0;
    let mut notify = || { events += 1; };
    let result: Result<(),String> = (|| {
        apply_page(&conn,("integration","tenant/account"),&[7;32],&page,None,|_,_| Ok(()),|_| {},&mut notify)?;
        // Seite 2 scheitert nach dem erfolgreichen Commit von Seite 1.
        Err("synthetic_429".into())
    })();
    assert!(result.is_err());
    assert_eq!(events,1);
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),1);
    apply_page(&conn,("integration","tenant/account"),&[7;32],&page,Some("poll_delta0"),|_,_| Ok(()),|_| {},|| { events += 1; }).unwrap();
    assert_eq!(events,1);
    let mut failed = page;
    failed.messages[0]["id"] = json!("rollback");
    assert!(apply_page(&conn,("integration","tenant/account"),&[7;32],&failed,Some("poll_delta1"),|_,_| Err("synthetic_store".into()),|_| {},|| { events += 1; }).is_err());
    assert_eq!(events,1);
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),1);
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
        |slot,token| { secrets.insert(slot.to_string(),token.to_string()); Ok(()) }, |_| {}, || {}).unwrap();
    assert!(changed);
    assert_eq!(secrets[&first],next);
    let page = parse_delta(json!({"value":[message(&subject(&conn),"one@example.invalid","Keiner passt","page2")],"@odata.deltaLink":delta}),base).unwrap();
    assert!(apply_page(&conn,("integration","tenant/account"),&[7;32],&page,Some(&first),|_,_| Err("synthetic_full_disk".into()),|_| {},|| {}).is_err());
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),1);
    let current: String = conn.query_row("SELECT delta_token FROM meeting_poll_sync",[],|r| r.get(0)).unwrap();
    assert_eq!(current,first);
    let (_,second) = apply_page(&conn,("integration","tenant/account"),&[7;32],&page,Some(&first),
        |slot,token| { secrets.insert(slot.to_string(),token.to_string()); Ok(()) }, |_| {}, || {}).unwrap();
    assert_ne!(first,second);
    assert_eq!(secrets[&second],delta);
    assert_eq!(get(&conn,"T-ABCD").unwrap().history.len(),2);
}

#[test]
fn availability_expiry_and_close_release_cached_data() {
    let runtime = PollRuntime::default();
    {
        let mut cache = runtime.availability.lock().unwrap();
        cache.insert("expired".into(),CachedAvailability { until:Instant::now()-Duration::from_secs(1),values:vec![],window:rank_window(),owner:rank_owner() });
        cache.insert("live".into(),CachedAvailability { until:Instant::now()+Duration::from_secs(15*60),values:vec![],window:rank_window(),owner:rank_owner() });
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
