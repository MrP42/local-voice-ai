//! Das Ledger der Ausloeser (B3).

use super::*;
use crate::managers::workflows::test_support::Fx;

fn full(key: &str, seen_at: i64) -> Entry {
    Entry {
        path_key: key.to_string(),
        size: Some(10),
        mtime: Some(20),
        content_hash: Some("abc".to_string()),
        run_id: Some("run-1".to_string()),
        seen_at,
    }
}

#[test]
fn an_entry_round_trips_and_a_second_put_replaces_it() {
    let fx = Fx::new();
    let conn = fx.conn();
    assert_eq!(get(&conn, "p:a").unwrap(), None);
    assert!(!exists(&conn, "p:a").unwrap());
    put(&conn, &full("p:a", 1)).unwrap();
    assert_eq!(get(&conn, "p:a").unwrap(), Some(full("p:a", 1)));
    assert!(exists(&conn, "p:a").unwrap());
    let mut changed = full("p:a", 5);
    changed.size = Some(99);
    changed.run_id = None;
    put(&conn, &changed).unwrap();
    assert_eq!(get(&conn, "p:a").unwrap(), Some(changed));
    assert_eq!(count(&conn, "p:").unwrap(), 1, "ersetzt, nicht verdoppelt");
}

#[test]
fn the_keys_are_scoped_per_workflow_and_ignore_case_in_paths() {
    assert_ne!(
        content_key("wf1", "h"),
        content_key("wf2", "h"),
        "zwei Ablaeufe haben je ihre Eintraege"
    );
    assert_eq!(
        path_key("wf", "ordner", "Eingang/Datei.WAV"),
        path_key("wf", "ordner", "eingang/datei.wav"),
        "ein NTFS-Pfad kennt keine Gross-/Kleinschreibung"
    );
    assert_ne!(video_key("a", "UCx", "v1"), video_key("b", "UCx", "v1"));
    assert!(channel_init_key("a", "UCx").starts_with("ytinit:"));
    assert!(
        !channel_init_key("a", "UCx").starts_with("yt:"),
        "die Anfangsmarke gehoert nicht zur Familie der Videos"
    );
}

#[test]
fn pruning_keeps_the_newest_of_one_family_and_leaves_the_others_alone() {
    let fx = Fx::new();
    let conn = fx.conn();
    for i in 0..10 {
        put(&conn, &full(&format!("p:wf:i:{i}"), 100 + i)).unwrap();
        put(&conn, &full(&format!("h:wf:{i}"), 100 + i)).unwrap();
    }
    put(&conn, &Entry::new("ytinit:wf:UC1", 1)).unwrap();
    let removed = prune_family(&conn, "p:", 4).unwrap();
    assert_eq!(removed, 6);
    assert_eq!(count(&conn, "p:").unwrap(), 4);
    // Die vier juengsten (6..9) bleiben.
    for i in 6..10 {
        assert!(exists(&conn, &format!("p:wf:i:{i}")).unwrap(), "{i}");
    }
    assert!(!exists(&conn, "p:wf:i:5").unwrap());
    assert_eq!(count(&conn, "h:").unwrap(), 10, "andere Familie unberuehrt");
    assert!(exists(&conn, "ytinit:wf:UC1").unwrap());
    // Unter dem Deckel: nichts zu tun.
    assert_eq!(prune_family(&conn, "p:", 4).unwrap(), 0);
}

#[test]
fn keys_with_sql_wildcards_do_not_confuse_the_family_match() {
    let fx = Fx::new();
    let conn = fx.conn();
    put(&conn, &full("p:wf:i:100%_done.wav", 1)).unwrap();
    put(&conn, &full("pxwf", 2)).unwrap();
    assert_eq!(count(&conn, "p:").unwrap(), 1);
    assert_eq!(prune_family(&conn, "p:", 0).unwrap(), 1);
    assert!(exists(&conn, "pxwf").unwrap());
}
