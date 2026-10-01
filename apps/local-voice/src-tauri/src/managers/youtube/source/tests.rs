use std::time::Duration;

use super::*;
use crate::managers::integrations::audit::{self, AuditFilter};
use crate::managers::integrations::model::{Caller, Capability, Direction, IntegrationPatch, Kind};
use crate::managers::integrations::store as register;
use crate::managers::integrations::test_support::Fx;
use crate::managers::provenance::{self, ActorKind, SubjectKind};
use crate::managers::youtube::link::normalize_link;
use crate::managers::youtube::test_support::{delayed, json_ok, serve, status};
use rusqlite::Connection;

const ID: &str = "dQw4w9WgXcQ";
const LINK: &str = "https://youtu.be/dQw4w9WgXcQ?si=TRACKING&t=30";

const GOOD: &str = r#"{
  "title": "Lastgang verstehen: Spitzen glätten",
  "author_name": "Wolff Applied AI",
  "author_url": "https://www.youtube.com/@wolffappliedai",
  "type": "video",
  "thumbnail_url": "https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"
}"#;

fn opts(base: &str) -> AddOptions {
    AddOptions {
        oembed_base: base.to_string(),
        fetch: oembed::FetchOpts {
            timeout: Duration::from_secs(5),
            max_bytes: 64 * 1024,
            use_env_proxy: false,
        },
    }
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn youtube_audit(conn: &Connection) -> Vec<crate::managers::integrations::model::AuditEntry> {
    audit::list(
        conn,
        &AuditFilter {
            integration_id: Some(INTEGRATION_ID.to_string()),
            ..Default::default()
        },
        100,
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Anlegen mit allem, was dazugehoert
// ---------------------------------------------------------------------------

#[tokio::test]
async fn adding_a_link_creates_a_youtube_meeting_in_the_chosen_project() {
    let fx = Fx::new();
    let project = fx.store.folder_save(None, "Podcast", None).unwrap();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;

    let added = add_youtube_source(&fx.store, LINK, Some(&project.id), &opts(&base))
        .await
        .unwrap();

    let m = &added.meeting;
    assert_eq!(m.source, "youtube");
    assert_eq!(m.title, "Lastgang verstehen: Spitzen glätten");
    assert_eq!(m.status, "ready", "nichts laeuft: keine Verarbeitung");
    assert!(m.mic_audio_path.is_none() && m.system_audio_path.is_none());
    assert!(m.source_path.is_none(), "kein Dateipfad vortaeuschen");
    assert_eq!(seen.lock().unwrap().len(), 1, "genau ein Abruf");

    // In der Datenbank steht dasselbe, und im gewaehlten Projekt.
    let stored = fx.store.get_meeting(&m.id).unwrap().expect("gespeichert");
    assert_eq!(stored.source, "youtube");
    assert_eq!(
        fx.store.meeting_folder_ids(&m.id).unwrap(),
        vec![project.id]
    );

    // Die Quelle ist lesbar und bereinigt (ohne si=, mit Startzeit).
    let source = read_source(&fx.store, &m.id).unwrap().expect("Quelle");
    assert_eq!(source.video_id, ID);
    assert_eq!(source.url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
    assert_eq!(source.title, "Lastgang verstehen: Spitzen glätten");
    assert_eq!(source.channel, "Wolff Applied AI");
    assert_eq!(source.start_s, Some(30));
    assert_eq!(
        source.thumbnail_url.as_deref(),
        Some("https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg")
    );
    let raw = fx.store.metadata_json(&m.id).unwrap().unwrap().to_string();
    assert!(
        !raw.contains("TRACKING"),
        "Verfolgungsparameter gehoeren nicht in die Daten: {raw}"
    );
}

#[tokio::test]
async fn the_provenance_names_youtube_as_the_source() {
    let fx = Fx::new();
    let (base, _) = serve(vec![json_ok(GOOD)]).await;
    let added = add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .unwrap();

    let conn = fx.conn();
    let entries = provenance::list(&conn, SubjectKind::Transcript, &added.meeting.id).unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
    let e = &entries[0];
    assert_eq!(e.operation, "youtube_source");
    assert_eq!(e.actor_kind, Some(ActorKind::User));
    assert_eq!(e.provider.as_deref(), Some("youtube"));
    assert_eq!(e.sources.len(), 1);
    assert_eq!(e.sources[0].kind, "youtube");
    assert_eq!(e.sources[0].reference, ID);
    assert_eq!(
        e.sources[0].title.as_deref(),
        Some("Lastgang verstehen: Spitzen glätten")
    );
    assert_eq!(
        e.sources[0].url.as_deref(),
        Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
    );
    let params: serde_json::Value =
        serde_json::from_str(e.params_json.as_deref().unwrap()).unwrap();
    assert_eq!(params["channel"], "Wolff Applied AI");
}

#[tokio::test]
async fn the_register_and_the_audit_record_the_network_step() {
    let fx = Fx::new();
    let (base, _) = serve(vec![json_ok(GOOD)]).await;
    add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .unwrap();

    let conn = fx.conn();
    let integration = register::get(&conn, INTEGRATION_ID)
        .unwrap()
        .expect("Eintrag");
    assert_eq!(integration.kind, Kind::Youtube);
    assert_eq!(integration.direction, Direction::Read);
    assert!(integration.enabled);
    assert!(integration.last_ok_at.is_some());
    assert_eq!(integration.last_error, None);

    let rows = youtube_audit(&conn);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.caller, "user");
    assert_eq!(row.capability.as_deref(), Some("media.fetch"));
    assert_eq!(row.outcome, "ok");
    assert_eq!(row.target.as_deref(), Some("youtube:dQw4w9WgXcQ"));
    let detail = row.detail_json.clone().unwrap_or_default();
    assert!(
        !detail.contains("TRACKING") && !detail.contains("http"),
        "{detail}"
    );
}

#[tokio::test]
async fn the_integration_is_created_once_and_a_user_rename_survives() {
    let fx = Fx::new();
    let conn = fx.conn();
    let first = ensure_integration(&conn, 1_000).unwrap();
    assert_eq!(first.id, INTEGRATION_ID);
    assert_eq!(first.label, "YouTube");
    assert_eq!(first.data_class.as_deref(), Some("internal"));

    register::update(
        &conn,
        INTEGRATION_ID,
        &IntegrationPatch {
            label: Some("Mein YouTube".to_string()),
            ..Default::default()
        },
        2_000,
    )
    .unwrap();
    let again = ensure_integration(&conn, 3_000).unwrap();
    assert_eq!(again.label, "Mein YouTube");
    let n = register::list(&conn)
        .unwrap()
        .iter()
        .filter(|i| i.kind == Kind::Youtube)
        .count();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// Fehlerfaelle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_failed_fetch_leaves_nothing_but_an_error_row() {
    let fx = Fx::new();
    let (base, _) = serve(vec![status(404, "Not Found")]).await;
    let err = add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .unwrap_err();
    assert_eq!(err, YoutubeError::Unavailable);

    let conn = fx.conn();
    assert_eq!(count(&conn, "meetings"), 0, "keine halbe Besprechung");
    assert_eq!(count(&conn, "provenance"), 0);
    let rows = youtube_audit(&conn);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].outcome, "error");
    let integration = register::get(&conn, INTEGRATION_ID).unwrap().unwrap();
    assert!(integration.last_error.is_some());
    assert_eq!(integration.last_ok_at, None);
}

#[tokio::test]
async fn a_timeout_is_reported_quickly_and_the_video_can_be_retried() {
    let fx = Fx::new();
    let (base, _) = serve(vec![Vec::new()]).await;
    let mut o = opts(&base);
    o.fetch.timeout = Duration::from_millis(250);
    let err = add_youtube_source(&fx.store, LINK, None, &o)
        .await
        .unwrap_err();
    assert_eq!(err, YoutubeError::Timeout);

    // Die Sperre je Video ist wieder frei: ein zweiter Versuch laeuft.
    let (base2, _) = serve(vec![json_ok(GOOD)]).await;
    let added = add_youtube_source(&fx.store, LINK, None, &opts(&base2)).await;
    assert!(added.is_ok(), "{added:?}");
}

#[tokio::test]
async fn a_second_add_of_the_same_video_while_the_first_runs_is_refused() {
    let fx = Fx::new();
    let (base, seen) = serve(vec![delayed(500, json_ok(GOOD))]).await;
    let o = opts(&base);
    let (a, b) = tokio::join!(
        add_youtube_source(&fx.store, LINK, None, &o),
        add_youtube_source(
            &fx.store,
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            None,
            &o
        ),
    );
    let results = [a, b];
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let busy = results
        .iter()
        .filter(|r| matches!(r, Err(YoutubeError::Busy)))
        .count();
    assert_eq!((ok, busy), (1, 1), "{results:?}");
    assert_eq!(count(&fx.conn(), "meetings"), 1, "genau eine Besprechung");
    assert_eq!(seen.lock().unwrap().len(), 1, "genau ein Abruf");

    // Danach ist das Video frei: dasselbe Video darf bewusst ein zweites Mal
    // angelegt werden (anderes Projekt, andere Notizen).
    let second = add_youtube_source(&fx.store, LINK, None, &o).await;
    assert!(second.is_ok(), "{second:?}");
    assert_eq!(count(&fx.conn(), "meetings"), 2);
}

#[tokio::test]
async fn different_videos_do_not_block_each_other() {
    let fx = Fx::new();
    let (base, _) = serve(vec![delayed(300, json_ok(GOOD))]).await;
    let o = opts(&base);
    let (a, b) = tokio::join!(
        add_youtube_source(&fx.store, "https://youtu.be/aaaaaaaaaaa", None, &o),
        add_youtube_source(&fx.store, "https://youtu.be/bbbbbbbbbbb", None, &o),
    );
    assert!(a.is_ok() && b.is_ok(), "{a:?} {b:?}");
    assert_eq!(count(&fx.conn(), "meetings"), 2);
}

#[tokio::test]
async fn an_unknown_project_is_refused_before_any_traffic() {
    let fx = Fx::new();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    let err = add_youtube_source(&fx.store, LINK, Some("gibt-es-nicht"), &opts(&base))
        .await
        .unwrap_err();
    assert_eq!(err, YoutubeError::Project);
    assert!(seen.lock().unwrap().is_empty(), "kein Abruf");
    let conn = fx.conn();
    assert_eq!(count(&conn, "meetings"), 0);
    assert_eq!(count(&conn, "audit_log"), 0);
}

#[test]
fn a_project_that_vanishes_before_the_write_leaves_no_meeting_behind() {
    let fx = Fx::new();
    let video = normalize_link(LINK).unwrap();
    let meta = VideoMeta {
        title: "t".into(),
        channel: "c".into(),
        channel_url: None,
        thumbnail_url: None,
    };
    let mut conn = fx.conn();
    let err = create_meeting(&mut conn, &video, &meta, Some("weg"), 1_000).unwrap_err();
    assert_eq!(err, YoutubeError::Project);
    assert_eq!(count(&conn, "meetings"), 0);
    assert_eq!(count(&conn, "provenance"), 0);
}

#[test]
fn a_failure_in_the_last_step_rolls_the_whole_creation_back() {
    let fx = Fx::new();
    let project = fx.store.folder_save(None, "P", None).unwrap();
    let video = normalize_link(LINK).unwrap();
    let meta = VideoMeta {
        title: "t".into(),
        channel: "c".into(),
        channel_url: None,
        thumbnail_url: None,
    };
    let mut conn = fx.conn();
    // Die Provenienz ist der letzte Schritt: ohne ihre Tabelle scheitert er,
    // und Besprechung samt Projektzuordnung duerfen nicht stehen bleiben.
    conn.execute_batch("DROP TABLE provenance;").unwrap();
    let err = create_meeting(&mut conn, &video, &meta, Some(&project.id), 1_000).unwrap_err();
    assert!(matches!(err, YoutubeError::Store(_)), "{err:?}");
    assert_eq!(count(&conn, "meetings"), 0, "Besprechung zurueckgerollt");
    assert_eq!(
        count(&conn, "meeting_folder_items"),
        0,
        "Zuordnung zurueckgerollt"
    );
}

#[tokio::test]
async fn without_a_writable_audit_nothing_goes_to_the_network() {
    let fx = Fx::new();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    // Platte voll / Datenbank gesperrt: das Audit laesst sich nicht schreiben.
    fx.conn()
        .execute_batch(
            "CREATE TRIGGER audit_full BEFORE INSERT ON audit_log
             BEGIN SELECT RAISE(ABORT, 'database or disk is full'); END;",
        )
        .unwrap();
    let err = add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .unwrap_err();
    assert!(matches!(err, YoutubeError::Store(_)), "{err:?}");
    assert!(seen.lock().unwrap().is_empty(), "fail closed: kein Abruf");
    assert_eq!(count(&fx.conn(), "meetings"), 0);
}

#[tokio::test]
async fn a_disabled_integration_blocks_the_add_before_any_traffic() {
    let fx = Fx::new();
    {
        let conn = fx.conn();
        ensure_integration(&conn, 1_000).unwrap();
        register::update(
            &conn,
            INTEGRATION_ID,
            &IntegrationPatch {
                enabled: Some(false),
                ..Default::default()
            },
            2_000,
        )
        .unwrap();
    }
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    let err = add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .unwrap_err();
    assert!(matches!(err, YoutubeError::Disabled(_)), "{err:?}");
    assert_eq!(err.code(), "youtube_disabled");
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(count(&fx.conn(), "meetings"), 0);

    // Wieder eingeschaltet geht es.
    register::update(
        &fx.conn(),
        INTEGRATION_ID,
        &IntegrationPatch {
            enabled: Some(true),
            ..Default::default()
        },
        3_000,
    )
    .unwrap();
    assert!(add_youtube_source(&fx.store, LINK, None, &opts(&base))
        .await
        .is_ok());
}

#[tokio::test]
async fn invalid_links_touch_neither_network_nor_database() {
    let fx = Fx::new();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    for (raw, code) in [
        ("https://vimeo.com/1", "youtube_not_youtube"),
        (
            "https://www.youtube.com/playlist?list=PLx",
            "youtube_playlist",
        ),
        ("https://www.youtube.com/@kanal", "youtube_channel"),
        ("", "youtube_empty"),
    ] {
        let err = add_youtube_source(&fx.store, raw, None, &opts(&base))
            .await
            .unwrap_err();
        assert_eq!(err.code(), code, "{raw}");
    }
    assert!(seen.lock().unwrap().is_empty());
    let conn = fx.conn();
    assert_eq!(count(&conn, "meetings"), 0);
    assert_eq!(count(&conn, "audit_log"), 0);
}

// ---------------------------------------------------------------------------
// Lesen
// ---------------------------------------------------------------------------

#[test]
fn a_meeting_from_another_source_has_no_youtube_source() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting(
            "Import",
            crate::managers::meetings::store::MeetingSource::Import,
            Some(1),
        )
        .unwrap();
    assert_eq!(read_source(&fx.store, &m.id).unwrap(), None);
    // Unbekannte Besprechung: auch keine Quelle, kein Absturz.
    assert_eq!(
        read_source(&fx.store, "gibt-es-nicht").unwrap_or(None),
        None
    );
}

#[test]
fn damaged_source_data_reads_as_no_source_instead_of_failing() {
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting(
            "Kaputt",
            crate::managers::meetings::store::MeetingSource::Youtube,
            None,
        )
        .unwrap();
    for bad in [
        serde_json::json!("text"),
        serde_json::json!({ "video_id": 5 }),
        serde_json::json!({ "video_id": "zu-kurz", "title": "t", "url": "u" }),
    ] {
        fx.store.set_metadata_key(&m.id, METADATA_KEY, bad).unwrap();
        assert_eq!(read_source(&fx.store, &m.id).unwrap(), None);
    }
}

#[test]
fn the_meeting_source_constant_matches_the_store() {
    use crate::managers::meetings::store::MeetingSource;
    assert_eq!(MeetingSource::Youtube.as_str(), SOURCE_KIND);
    let fx = Fx::new();
    let m = fx
        .store
        .create_meeting("YT", MeetingSource::Youtube, None)
        .unwrap();
    assert_eq!(m.source, SOURCE_KIND);
    assert_eq!(m.status, "ready");
}

#[test]
fn the_capability_used_for_the_network_step_is_allowed_for_the_user_only() {
    // Die Vorgabe des Registers: fuer den Nutzer erlaubt, fuer alle anderen aus.
    let fx = Fx::new();
    let conn = fx.conn();
    let integration = ensure_integration(&conn, 1_000).unwrap();
    let grants = register::grants_for(&conn, &integration.id).unwrap();
    use crate::managers::integrations::grants::effective_mode;
    use crate::managers::integrations::model::GrantMode;
    assert_eq!(
        effective_mode(
            &integration,
            Capability::MediaFetch,
            Caller::User,
            &grants,
            None
        ),
        GrantMode::Allow
    );
    for caller in [Caller::Workflow, Caller::AgentExternal, Caller::AgentLocal] {
        assert_eq!(
            effective_mode(&integration, Capability::MediaFetch, caller, &grants, None),
            GrantMode::Off,
            "{caller:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// G1 (#70): ein Link fuellt einen leeren Eintrag
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_link_fills_the_empty_entry_instead_of_creating_a_new_meeting() {
    let fx = Fx::new();
    let project = fx.store.folder_save(None, "Podcast", None).unwrap();
    let target = fx
        .store
        .create_empty_meeting("Neue Besprechung", Some(&project.id))
        .unwrap();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;

    let added = add_youtube_source_into(&fx.store, LINK, None, Some(&target.id), &opts(&base))
        .await
        .unwrap();

    let m = &added.meeting;
    assert_eq!(m.id, target.id, "derselbe Eintrag");
    assert_eq!((m.source.as_str(), m.status.as_str()), ("youtube", "ready"));
    assert_eq!(m.title, "Lastgang verstehen: Spitzen glätten", "Standardtitel ersetzt");
    assert_eq!(seen.lock().unwrap().len(), 1, "ein Abruf, wie sonst");
    assert_eq!(fx.store.meeting_folder_ids(&m.id).unwrap(), vec![project.id.clone()]);
    assert_eq!(fx.store.list_meetings(0, 10).unwrap().len(), 1, "keine zweite Besprechung");
    // Die Quelle ist wie bei jedem Link lesbar, der Marker des leeren Eintrags ist weg.
    let source = read_source(&fx.store, &m.id).unwrap().expect("Quelle");
    assert_eq!(source.video_id, ID);
    let meta = fx.store.metadata_json(&m.id).unwrap().unwrap();
    assert!(meta.get("empty_default_title").is_none(), "{meta}");
    // Herkunft und Audit wie sonst.
    let conn = fx.conn();
    let entries = provenance::list(&conn, SubjectKind::Transcript, &m.id).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].operation, "youtube_source");
    let audit = youtube_audit(&conn);
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].outcome, "ok");
}

#[tokio::test]
async fn a_renamed_empty_entry_keeps_the_title_of_the_user_when_a_link_arrives() {
    let fx = Fx::new();
    let target = fx.store.create_empty_meeting("Neue Besprechung", None).unwrap();
    fx.store.set_title(&target.id, "Mein Titel").unwrap();
    let (base, _) = serve(vec![json_ok(GOOD)]).await;
    let added = add_youtube_source_into(&fx.store, LINK, None, Some(&target.id), &opts(&base))
        .await
        .unwrap();
    assert_eq!(added.meeting.title, "Mein Titel");
    assert_eq!(added.meeting.source, "youtube");
}

#[tokio::test]
async fn a_target_that_is_not_empty_is_refused_before_any_traffic() {
    let fx = Fx::new();
    let live = fx
        .store
        .create_meeting("Live", MeetingSource::Live, Some(1))
        .unwrap();
    let (base, seen) = serve(vec![json_ok(GOOD)]).await;
    let err = add_youtube_source_into(&fx.store, LINK, None, Some(&live.id), &opts(&base))
        .await
        .unwrap_err();
    assert_eq!(err, YoutubeError::Target("target_not_empty"));
    assert_eq!(err.to_command_error(), "target_not_empty");
    let err = add_youtube_source_into(&fx.store, LINK, None, Some("gibt-es-nicht"), &opts(&base))
        .await
        .unwrap_err();
    assert_eq!(err, YoutubeError::Target("meeting_not_found"));
    assert!(seen.lock().unwrap().is_empty(), "kein Abruf");
    let conn = fx.conn();
    assert_eq!(count(&conn, "audit_log"), 0, "kein Audit-Eintrag");
    assert_eq!(fx.store.get_meeting(&live.id).unwrap().unwrap().source, "live");
}

#[tokio::test]
async fn a_second_link_for_the_same_entry_is_refused_after_the_first_filled_it() {
    let fx = Fx::new();
    let target = fx.store.create_empty_meeting("Neue Besprechung", None).unwrap();
    let (base, _) = serve(vec![json_ok(GOOD), json_ok(GOOD)]).await;
    add_youtube_source_into(&fx.store, LINK, None, Some(&target.id), &opts(&base))
        .await
        .unwrap();
    let err = add_youtube_source_into(
        &fx.store,
        "https://youtu.be/aaaaaaaaaaa",
        None,
        Some(&target.id),
        &opts(&base),
    )
    .await
    .unwrap_err();
    assert_eq!(err, YoutubeError::Target("target_not_empty"));
    assert_eq!(
        read_source(&fx.store, &target.id).unwrap().unwrap().video_id,
        ID,
        "der erste Link bleibt"
    );
}

#[test]
fn an_entry_that_gets_filled_elsewhere_during_the_write_rolls_back_without_a_trace() {
    let fx = Fx::new();
    let target = fx.store.create_empty_meeting("Neue Besprechung", None).unwrap();
    let video = normalize_link(LINK).unwrap();
    let meta = VideoMeta {
        title: "t".into(),
        channel: "c".into(),
        channel_url: None,
        thumbnail_url: None,
    };
    let mut conn = fx.conn();
    // Die Provenienz ist der letzte Schritt: scheitert er, bleibt der Eintrag leer.
    conn.execute_batch("DROP TABLE provenance;").unwrap();
    let err = create_meeting_into(&mut conn, &video, &meta, None, Some(&target.id), 1_000)
        .unwrap_err();
    assert!(matches!(err, YoutubeError::Store(_)), "{err:?}");
    assert!(fx.store.is_empty_meeting(&target.id).unwrap(), "weiterhin leer");
}
