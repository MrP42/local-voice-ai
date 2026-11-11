//! G2 (#497): Anschluss des Hinweises an Teilnehmende an den Besprechungsspeicher.
//!
//! Die Regeln, Textbausteine und das Datenformat stehen in
//! `managers::compliance::notice` (rein, ohne Speicher). Hier liegt nur, was den
//! Nachweis in `meetings.metadata_json` schreibt und liest, den Nachweis aus dem
//! Startdialog an die neue Besprechung haengt und die Zeile fuer den
//! Protokollexport bildet. Keine zweite Ablage: der Nachweis steht neben den
//! uebrigen Angaben der Besprechung, die Einwilligung (`consent_confirmed_at`)
//! bleibt unberuehrt.
//!
//! Datenschutz: der Hinweistext enthaelt keine Namen der Teilnehmenden; ins Log
//! geht nie der Text, nur Anzahl und Weg.

use std::sync::{Mutex, PoisonError};

use log::{info, warn};

use super::store::MeetingStore;
use crate::managers::compliance::notice::{
    self as nc, NoticeLang, NoticeRecord, PendingNotices, NOTICE_KEY,
};

/// Alle Schreiber des Nachweises gehen durch diese Sperre: Lesen, Ergaenzen und
/// Schreiben von `metadata_json[NOTICE_KEY]` sind im Speicher zwei Schritte;
/// ohne die Sperre ginge von zwei gleichzeitigen Nachweisen (Kopieren und
/// Einfuegen, oder der Nachweis aus dem Startdialog und ein neuer) einer
/// verloren. Es gibt keinen anderen Schreiber dieses Schluessels (der lokale
/// MCP-Server oeffnet den Speicher nur lesend). Die Sperre haelt nur
/// Daten-Reihenfolge: eine Vergiftung darf nie ein Schreiben verhindern.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Haengt einen Nachweis an die Besprechung `id`.
pub fn record_on_meeting(
    store: &MeetingStore,
    id: &str,
    record: &NoticeRecord,
) -> Result<(), String> {
    record_many_on_meeting(store, id, std::slice::from_ref(record))
}

/// Haengt mehrere Nachweise in EINEM Schreibvorgang an die Besprechung `id`
/// (aeltester zuerst).
pub fn record_many_on_meeting(
    store: &MeetingStore,
    id: &str,
    records: &[NoticeRecord],
) -> Result<(), String> {
    if records.is_empty() {
        return Ok(());
    }
    let _guard = WRITE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let meta = store
        .metadata_json(id)
        .map_err(|e| format!("store_failed: {e}"))?;
    let value = nc::with_records(meta.as_ref(), records);
    store
        .set_metadata_key(id, NOTICE_KEY, value)
        .map_err(|e| format!("store_failed: {e}"))
}

/// Die Nachweise der Besprechung `id` (aelteste zuerst; leer bei Fehlern).
pub fn records_of(store: &MeetingStore, id: &str) -> Vec<NoticeRecord> {
    match store.metadata_json(id) {
        Ok(meta) => nc::records_in(meta.as_ref()),
        Err(e) => {
            warn!("meetings: Nachweise von {id} nicht lesbar: {e}");
            Vec::new()
        }
    }
}

/// Nach erfolgreichem Start: die Nachweise (`evidence`, vor dem Start aus dem
/// Dialog `token` entnommen, siehe `nc::evidence_for_start`) gehoeren jetzt zur
/// neuen Besprechung, und der Dialog ist verbraucht: eine zweite Aufnahme
/// bekommt ihn nicht. Scheitert das Schreiben (Datentraeger voll, Speicher
/// gesperrt), bleibt die Aufnahme unberuehrt; der Nachweis geht dann nur
/// verloren, es gibt eine Warnung, und die Oberflaeche der Aufnahme zeigt
/// "noch kein Hinweis" statt etwas zu behaupten.
pub fn finish_start(
    pending: &PendingNotices,
    store: &MeetingStore,
    id: &str,
    token: Option<&str>,
    evidence: &[NoticeRecord],
) {
    if !evidence.is_empty() {
        match record_many_on_meeting(store, id, evidence) {
            Ok(()) => info!(
                "meetings: {} Nachweis(e) zum Hinweis an Teilnehmende an {id} gehaengt",
                evidence.len()
            ),
            Err(e) => warn!("meetings: Nachweis zum Hinweis nicht gespeichert ({e})"),
        }
    }
    if let Some(token) = token {
        pending.discard(token);
    }
}

/// Die Zeile fuer den Protokollexport (`None`: kein Nachweis oder ausgeblendet)
/// in der Sprache `lang` des Exports. `when` formatiert Unix-Sekunden als
/// Ortszeit.
pub fn export_line_for(
    store: &MeetingStore,
    id: &str,
    lang: NoticeLang,
    when: impl Fn(i64) -> String,
) -> Option<String> {
    nc::export_line(&records_of(store, id), lang, when)
}

/// Unix-Sekunden als Ortszeit mit Zeitzone: "2026-11-11 14:32 (UTC+01:00)".
pub fn local_stamp(at: i64) -> String {
    chrono::DateTime::from_timestamp(at, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M (UTC%:z)")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::compliance::notice::{
        evidence_for_start, MeetingNoticeSettings, NoticeApp, NoticeWay, CODE_NOTICE_REQUIRED,
    };
    use crate::managers::meetings::store::MeetingSource;
    use std::sync::Arc;

    fn store() -> MeetingStore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("meetings.db");
        let s = MeetingStore::open_at(&path).unwrap();
        std::mem::forget(dir);
        s
    }

    fn record(at: i64, way: NoticeWay) -> NoticeRecord {
        NoticeRecord {
            at,
            way,
            app: NoticeApp::Webex,
            lang: NoticeLang::En,
            text: "Note".to_string(),
            in_export: true,
            target: None,
        }
    }

    fn required() -> MeetingNoticeSettings {
        MeetingNoticeSettings {
            required: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_record_survives_a_round_trip_through_the_store_and_keeps_other_metadata() {
        let s = store();
        let m = s
            .create_meeting("Nachweis", MeetingSource::Live, Some(1))
            .unwrap();
        s.set_metadata_key(&m.id, "aec", serde_json::json!({"frames": 5}))
            .unwrap();
        record_on_meeting(&s, &m.id, &record(100, NoticeWay::Clipboard)).unwrap();
        let pasted = NoticeRecord {
            target: Some("Microsoft Teams".to_string()),
            ..record(200, NoticeWay::Paste)
        };
        record_on_meeting(&s, &m.id, &pasted).unwrap();
        let back = records_of(&s, &m.id);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].way, NoticeWay::Clipboard);
        assert_eq!(back[1].way, NoticeWay::Paste);
        assert_eq!(back[1].text, "Note");
        assert_eq!(back[1].target.as_deref(), Some("Microsoft Teams"));
        let meta = s.metadata_json(&m.id).unwrap().unwrap();
        assert_eq!(meta["aec"]["frames"], 5, "fremde Schluessel bleiben");
    }

    #[test]
    fn a_meeting_without_evidence_has_no_records_and_no_line() {
        let s = store();
        let m = s
            .create_meeting("Leer", MeetingSource::Live, Some(1))
            .unwrap();
        assert!(records_of(&s, &m.id).is_empty());
        assert_eq!(
            export_line_for(&s, &m.id, NoticeLang::De, |t| t.to_string()),
            None
        );
        assert!(records_of(&s, "gibt-es-nicht").is_empty());
    }

    #[test]
    fn the_export_line_comes_from_the_stored_records_in_the_export_language() {
        let s = store();
        let m = s
            .create_meeting("Zeile", MeetingSource::Live, Some(1))
            .unwrap();
        record_on_meeting(&s, &m.id, &record(42, NoticeWay::Oral)).unwrap();
        let de = export_line_for(&s, &m.id, NoticeLang::De, |t| format!("T{t}")).unwrap();
        assert_eq!(de, "Hinweis an Teilnehmende: am T42 mündlich bestätigt.");
        let en = export_line_for(&s, &m.id, NoticeLang::En, |t| format!("T{t}")).unwrap();
        assert_eq!(
            en,
            "Notice to participants: confirmed as given orally on T42."
        );
    }

    /// Befund 10: zwei gleichzeitige Nachweise gehen nicht verloren (Lesen,
    /// Ergaenzen, Schreiben unter einer Sperre).
    #[test]
    fn concurrent_records_are_all_kept() {
        let s = Arc::new(store());
        let m = s
            .create_meeting("Parallel", MeetingSource::Live, Some(1))
            .unwrap();
        let id = m.id.clone();
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let s = Arc::clone(&s);
                let id = id.clone();
                std::thread::spawn(move || {
                    record_on_meeting(&s, &id, &record(1_000 + i, NoticeWay::Clipboard)).unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let mut times: Vec<i64> = records_of(&s, &id).iter().map(|r| r.at).collect();
        times.sort_unstable();
        assert_eq!(times, (1_000..1_008).collect::<Vec<i64>>());
    }

    #[test]
    fn several_records_are_written_in_one_step_oldest_first() {
        let s = store();
        let m = s
            .create_meeting("Mehrere", MeetingSource::Live, Some(1))
            .unwrap();
        record_many_on_meeting(
            &s,
            &m.id,
            &[record(1, NoticeWay::Clipboard), record(2, NoticeWay::Oral)],
        )
        .unwrap();
        record_many_on_meeting(&s, &m.id, &[]).unwrap();
        let back = records_of(&s, &m.id);
        assert_eq!(back.iter().map(|r| r.at).collect::<Vec<_>>(), vec![1, 2]);
        assert!(record_many_on_meeting(&s, "gibt-es-nicht", &[record(3, NoticeWay::Oral)]).is_err());
    }

    /// Der Nachweis aus dem Startdialog liegt nach dem Start an der neuen
    /// Besprechung, und nur einmal; die zweite Aufnahme bekommt keinen alten.
    #[test]
    fn evidence_moves_to_the_new_meeting_once_and_a_second_recording_gets_none() {
        let s = store();
        let first = s
            .create_meeting("Erste", MeetingSource::Live, Some(1))
            .unwrap();
        let second = s
            .create_meeting("Zweite", MeetingSource::Live, Some(1))
            .unwrap();
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", record(1_000, NoticeWay::Clipboard)).unwrap();

        let evidence = evidence_for_start(&pending, &required(), Some("T"), 1_010).unwrap();
        finish_start(&pending, &s, &first.id, Some("T"), &evidence);
        assert_eq!(records_of(&s, &first.id).len(), 1);

        assert_eq!(
            evidence_for_start(&pending, &required(), Some("T"), 1_020),
            Err(CODE_NOTICE_REQUIRED.to_string()),
            "dasselbe Token ein zweites Mal"
        );
        finish_start(&pending, &s, &second.id, Some("T"), &[]);
        assert!(records_of(&s, &second.id).is_empty(), "nichts Altes");
    }

    /// Ein Start ohne Token (Hinweisfenster, Ablauf, Agent) haengt nichts an und
    /// verbraucht den offenen Startdialog nicht.
    #[test]
    fn a_start_without_a_token_leaves_the_open_dialog_alone() {
        let s = store();
        let m = s
            .create_meeting("Fremd", MeetingSource::Live, Some(1))
            .unwrap();
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", record(1_000, NoticeWay::Oral)).unwrap();
        let evidence = evidence_for_start(&pending, &MeetingNoticeSettings::default(), None, 1_001)
            .unwrap();
        assert!(evidence.is_empty());
        finish_start(&pending, &s, &m.id, None, &evidence);
        assert!(records_of(&s, &m.id).is_empty());
        assert_eq!(pending.fresh(Some("T"), 1_002).len(), 1);
    }

    /// Scheitert das Schreiben, geht der Nachweis verloren, aber nichts
    /// bricht ab, und der Dialog ist trotzdem verbraucht.
    #[test]
    fn a_failed_write_never_breaks_the_start_and_still_consumes_the_dialog() {
        let s = store();
        let pending = PendingNotices::new();
        pending.open("T".to_string());
        pending.add("T", record(1_000, NoticeWay::Oral)).unwrap();
        let evidence = pending.fresh(Some("T"), 1_001);
        finish_start(&pending, &s, "gibt-es-nicht", Some("T"), &evidence);
        assert!(!pending.is_open("T"));
    }

    /// Befund 9: Ortszeit mit Zeitzone, "2026-11-11 14:32 (UTC+01:00)".
    #[test]
    fn the_stamp_carries_the_time_zone() {
        let stamp = local_stamp(1_790_000_000);
        let (date, zone) = stamp.split_once(" (UTC").expect(&stamp);
        assert_eq!(date.len(), "2026-11-11 14:32".len(), "{stamp}");
        assert!(date.starts_with("2026-"), "{stamp}");
        assert!(zone.ends_with(')'), "{stamp}");
        let offset = zone.trim_end_matches(')');
        assert_eq!(offset.len(), "+01:00".len(), "{stamp}");
        assert!(offset.starts_with('+') || offset.starts_with('-'), "{stamp}");
        assert_eq!(&offset[3..4], ":", "{stamp}");
    }
}
