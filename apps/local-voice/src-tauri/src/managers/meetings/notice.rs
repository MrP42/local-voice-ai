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

use log::{info, warn};

use super::store::MeetingStore;
use crate::managers::compliance::notice::{self as nc, NoticeRecord, NOTICE_KEY};

/// Haengt einen Nachweis an die Besprechung `id`.
pub fn record_on_meeting(
    store: &MeetingStore,
    id: &str,
    record: &NoticeRecord,
) -> Result<(), String> {
    let meta = store
        .metadata_json(id)
        .map_err(|e| format!("store_failed: {e}"))?;
    let value = nc::with_record(meta.as_ref(), record);
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

/// Nach erfolgreichem Start: die Nachweise aus dem Startdialog gehoeren jetzt
/// zur neuen Besprechung. Scheitert das Schreiben, bleibt die Aufnahme
/// unberuehrt (der Nachweis geht dann nur verloren, es gibt eine Warnung).
pub fn attach_pending(store: &MeetingStore, id: &str, now: i64) {
    let pending = nc::PENDING.take_fresh(now);
    if pending.is_empty() {
        return;
    }
    let count = pending.len();
    for record in &pending {
        if let Err(e) = record_on_meeting(store, id, record) {
            warn!("meetings: Nachweis zum Hinweis nicht gespeichert ({e})");
            return;
        }
    }
    info!("meetings: {count} Nachweis(e) zum Hinweis an Teilnehmende an {id} gehaengt");
}

/// Die Zeile fuer den Protokollexport (`None`: kein Nachweis oder ausgeblendet).
/// `when` formatiert Unix-Sekunden als Ortszeit.
pub fn export_line_for(
    store: &MeetingStore,
    id: &str,
    when: impl Fn(i64) -> String,
) -> Option<String> {
    nc::export_line(&records_of(store, id), when)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::compliance::notice::{NoticeApp, NoticeLang, NoticeWay};
    use crate::managers::meetings::store::MeetingSource;

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
        record_on_meeting(&s, &m.id, &record(200, NoticeWay::Paste)).unwrap();
        let back = records_of(&s, &m.id);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].way, NoticeWay::Clipboard);
        assert_eq!(back[1].way, NoticeWay::Paste);
        assert_eq!(back[1].text, "Note");
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
        assert_eq!(export_line_for(&s, &m.id, |t| t.to_string()), None);
        assert!(records_of(&s, "gibt-es-nicht").is_empty());
    }

    #[test]
    fn the_export_line_comes_from_the_stored_records() {
        let s = store();
        let m = s
            .create_meeting("Zeile", MeetingSource::Live, Some(1))
            .unwrap();
        record_on_meeting(&s, &m.id, &record(42, NoticeWay::Oral)).unwrap();
        let line = export_line_for(&s, &m.id, |t| format!("T{t}")).unwrap();
        assert_eq!(line, "Hinweis an Teilnehmende: am T42 mündlich bestätigt.");
    }

    /// Der Nachweis aus dem Startdialog liegt nach dem Start an der neuen
    /// Besprechung, und nur einmal.
    #[test]
    fn pending_evidence_moves_to_the_new_meeting_exactly_once() {
        let s = store();
        let m = s
            .create_meeting("Start", MeetingSource::Live, Some(1))
            .unwrap();
        // Der Prozess-weite Satz ist geteilt: nur dieser Test fasst ihn an.
        nc::PENDING.clear();
        nc::PENDING.add(record(1_000, NoticeWay::Clipboard));
        attach_pending(&s, &m.id, 1_010);
        assert_eq!(records_of(&s, &m.id).len(), 1);
        attach_pending(&s, &m.id, 1_020);
        assert_eq!(records_of(&s, &m.id).len(), 1, "kein zweites Mal");
        // Ein abgelaufener Nachweis wird nicht uebernommen.
        nc::PENDING.add(record(1_000, NoticeWay::Oral));
        attach_pending(&s, &m.id, 1_000 + nc::PENDING_TTL_SECS);
        assert_eq!(records_of(&s, &m.id).len(), 1);
    }
}
