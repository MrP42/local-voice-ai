//! Hilfen fuer Tests ausserhalb von `planning` (Final-Pass, Recorder, Import): eine Besprechung,
//! die ueber ihren Kalendertermin zu einer Planung gehoert. Nur synthetische Daten
//! (Domains `example.*`).

use besprechungsplanung::invitation::Gestaltung;
use besprechungsplanung::model::{
    AgendaItem, Consent, ConsentChannel, ConsentState, GoalType, Hints, Plan, PlanInput, Roles,
};

use super::{
    adapter_c, einladung_erzeugen, einwilligung_setzen, plan_anlegen, plan_laden, MeetingStore,
};
use crate::managers::calendar::model::{event_key, Attendee, CalEvent, CalendarKind};
use crate::managers::meetings::store::SyncMeta;

pub(crate) const NOW: i64 = 1_790_000_000_000;
const QUELLE: &str = "testhilfe-quelle";
pub(crate) const ANNA: &str = "anna@example.com";
pub(crate) const BERT: &str = "bert@example.org";

fn eingabe() -> PlanInput {
    PlanInput {
        title: "Abstimmung Posteingang".into(),
        goal_type: GoalType::Entscheiden,
        goal_text: "Wir entscheiden, ob der Pilot im Januar startet.".into(),
        expected_result: None,
        duration_min: 25,
        classification: "intern".into(),
        agenda: vec![AgendaItem {
            minutes: 25,
            topic: "Optionen".into(),
            owner: None,
        }],
        roles: Roles::default(),
        materials: vec![],
        hints: Hints::default(),
        transcription_planned: true,
        minutes_template_id: None,
    }
}

fn person(email: &str) -> Attendee {
    Attendee {
        email: Some(email.into()),
        name: None,
        organizer: false,
        is_self: false,
        partstat: None,
    }
}

/// Die Einwilligung von `person` (Zustimmung mit Nachweis).
pub(crate) fn zustimmung(plan_id: &str, person: &str) -> Consent {
    Consent {
        plan_id: plan_id.into(),
        person: person.into(),
        state: ConsentState::Zugestimmt,
        channel: Some(ConsentChannel::Einladung),
        recorded_at: Some(NOW),
    }
}

/// Der Widerruf von `person` (mit Nachweis).
pub(crate) fn widerruf(plan_id: &str, person: &str) -> Consent {
    Consent {
        plan_id: plan_id.into(),
        person: person.into(),
        state: ConsentState::Abgelehnt,
        channel: Some(ConsentChannel::MuendlichBeiStart),
        recorded_at: Some(NOW + 1),
    }
}

/// Legt eine Planung (Termin `uid`, zwei Teilnehmende, Einwilligungen `offen`) an, verknuepft sie
/// ueber den Kalender-Cache und haengt die Besprechung `meeting_id` an den Termin. Mit
/// `zugestimmt` haben beide zugestimmt. Danach entscheidet das Tor fuer diese Besprechung.
pub(crate) fn plan_fuer_besprechung(
    store: &MeetingStore,
    meeting_id: &str,
    uid: &str,
    zugestimmt: bool,
) -> (Plan, CalEvent) {
    let _ = store.calendar_source_add(QUELLE, CalendarKind::Ics, "Kalender", None, NOW);
    let p = plan_anlegen(store, &eingabe(), NOW).unwrap();
    let e = einladung_erzeugen(store, &p.id, &Gestaltung::default(), NOW).unwrap();
    let start = NOW + 48 * 3_600_000;
    let t = CalEvent {
        key: event_key(QUELLE, uid, start),
        source_id: QUELLE.into(),
        uid: uid.into(),
        title: "Abstimmung".into(),
        starts_at: start,
        ends_at: start + 3_600_000,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: Some(format!("Hallo,\n\n{}\n\nTeams.", e.text)),
        instance_start: None,
        attendees: vec![person(ANNA), person(BERT)],
    };
    store
        .calendar_replace_events(
            QUELLE,
            std::slice::from_ref(&t),
            &SyncMeta {
                has_attendee_data: true,
                etag: None,
                last_modified: None,
                now_ms: NOW,
            },
        )
        .unwrap();
    assert_eq!(adapter_c::abgleich_lauf(store, NOW).unwrap().geaendert, 1);
    let im_cache = store.calendar_event(&t.key).unwrap().unwrap();
    store
        .link_meeting_event(meeting_id, &im_cache, "prompt", NOW)
        .unwrap();
    if zugestimmt {
        for wer in [ANNA, BERT] {
            einwilligung_setzen(store, &zustimmung(&p.id, wer), NOW).unwrap();
        }
    }
    (plan_laden(store, &p.id).unwrap().unwrap(), im_cache)
}

/// Legt eine Planung mit EIGENEM Termin `uid` an (eigene Kalenderquelle je Termin: mehrere
/// Planungen stehen so nebeneinander im Cache), zwei Teilnehmende; mit `zugestimmt` haben beide
/// zugestimmt. Die Besprechung wird NICHT verknuepft: der Termin ist fuer den Start bestimmt.
pub(crate) fn plan_mit_termin(
    store: &MeetingStore,
    uid: &str,
    zugestimmt: bool,
) -> (Plan, CalEvent) {
    let quelle = format!("{QUELLE}-{uid}");
    let _ = store.calendar_source_add(&quelle, CalendarKind::Ics, "Kalender", None, NOW);
    let p = plan_anlegen(store, &eingabe(), NOW).unwrap();
    let e = einladung_erzeugen(store, &p.id, &Gestaltung::default(), NOW).unwrap();
    let start = NOW + 48 * 3_600_000;
    let t = CalEvent {
        key: event_key(&quelle, uid, start),
        source_id: quelle.clone(),
        uid: uid.into(),
        title: "Abstimmung".into(),
        starts_at: start,
        ends_at: start + 3_600_000,
        all_day: false,
        cancelled: false,
        location: None,
        join_url: None,
        description: Some(format!("Hallo,\n\n{}\n\nTeams.", e.text)),
        instance_start: None,
        attendees: vec![person(ANNA), person(BERT)],
    };
    store
        .calendar_replace_events(
            &quelle,
            std::slice::from_ref(&t),
            &SyncMeta {
                has_attendee_data: true,
                etag: None,
                last_modified: None,
                now_ms: NOW,
            },
        )
        .unwrap();
    assert_eq!(adapter_c::abgleich_lauf(store, NOW).unwrap().geaendert, 1);
    let im_cache = store.calendar_event(&t.key).unwrap().unwrap();
    if zugestimmt {
        for wer in [ANNA, BERT] {
            einwilligung_setzen(store, &zustimmung(&p.id, wer), NOW).unwrap();
        }
    }
    (plan_laden(store, &p.id).unwrap().unwrap(), im_cache)
}
