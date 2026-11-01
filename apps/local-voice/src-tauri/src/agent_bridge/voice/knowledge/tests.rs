//! Tests der Wissenswerkzeuge mit einer Attrappe als Quelle (rein, ohne App).

use chrono::{Local, TimeZone};
use serde_json::{json, Value};

use super::*;

fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
    Local
        .with_ymd_and_hms(y, m, d, h, min, 0)
        .earliest()
        .expect("lokale Zeit")
        .timestamp_millis()
}

fn now() -> i64 {
    at(2026, 11, 1, 10, 0)
}

struct Fake {
    dictations: Vec<Dictation>,
    conversations: Vec<ConversationInfo>,
    entries: Vec<EntryInfo>,
    events: Vec<EventInfo>,
    meetings: Vec<MeetingInfo>,
    pages: Vec<PageInfo>,
    page: String,
}

fn dictation(id: i64, at_ms: i64, text: &str) -> Dictation {
    Dictation {
        id,
        at_ms,
        text: text.to_string(),
    }
}

fn conversation(id: &str, title: &str, updated_ms: i64, last: &str) -> ConversationInfo {
    ConversationInfo {
        id: id.to_string(),
        title: title.to_string(),
        updated_ms,
        entries: 4,
        last_text: last.to_string(),
    }
}

fn fake() -> Fake {
    Fake {
        // neueste zuerst
        dictations: vec![
            dictation(
                3,
                at(2026, 11, 1, 9, 15),
                "Grundsteuerbescheid für das Ordnungsamt prüfen",
            ),
            dictation(
                2,
                at(2026, 10, 31, 16, 0),
                "Einkaufsliste: Äpfel, Brot, Milch",
            ),
            dictation(1, at(2026, 10, 30, 8, 0), &"lang ".repeat(200)),
        ],
        conversations: vec![
            conversation(
                "conv_aktuell",
                "Laufendes Gespräch",
                at(2026, 11, 1, 9, 59),
                "Hallo",
            ),
            conversation(
                "conv_haushalt",
                "Haushaltsplan 2027",
                at(2026, 11, 1, 8, 30),
                "Wir sprechen über den Haushalt.",
            ),
            conversation(
                "conv_alt",
                "Urlaubsplanung",
                at(2026, 10, 20, 12, 0),
                "Ende Oktober",
            ),
        ],
        entries: vec![
            EntryInfo {
                conversation_id: "conv_aktuell".to_string(),
                title: "Laufendes Gespräch".to_string(),
                role: "user".to_string(),
                at_ms: at(2026, 11, 1, 9, 59),
                text: "Was stand im Haushaltsplan?".to_string(),
            },
            EntryInfo {
                conversation_id: "conv_haushalt".to_string(),
                title: "Haushaltsplan 2027".to_string(),
                role: "assistant".to_string(),
                at_ms: at(2026, 11, 1, 8, 30),
                text: "Der Haushaltsplan sieht 2027 neue Mittel für die Schule vor.".to_string(),
            },
            EntryInfo {
                conversation_id: "conv_haushalt".to_string(),
                title: "Haushaltsplan 2027".to_string(),
                role: "user".to_string(),
                at_ms: at(2026, 11, 1, 8, 29),
                text: "Erzähl mir etwas zum Haushaltsplan".to_string(),
            },
        ],
        events: vec![
            EventInfo {
                title: "Teamrunde".to_string(),
                starts_ms: at(2026, 11, 1, 14, 0),
                ends_ms: at(2026, 11, 1, 15, 0),
                all_day: false,
                location: Some("Raum 2".to_string()),
            },
            EventInfo {
                title: "Feiertag".to_string(),
                starts_ms: at(2026, 11, 1, 0, 0),
                ends_ms: at(2026, 11, 2, 0, 0),
                all_day: true,
                location: None,
            },
            EventInfo {
                title: "Nächste Woche".to_string(),
                starts_ms: at(2026, 11, 5, 9, 0),
                ends_ms: at(2026, 11, 5, 10, 0),
                all_day: false,
                location: None,
            },
        ],
        meetings: vec![MeetingInfo {
            id: "m1".to_string(),
            title: "Wochenplanung".to_string(),
            started_ms: at(2026, 11, 1, 9, 0),
            minutes: Some(32),
        }],
        pages: vec![PageInfo {
            id: "seite1".to_string(),
            title: "Rede".to_string(),
            modified_ms: at(2026, 10, 30, 12, 0),
            preview: "Sehr geehrte Damen und Herren".to_string(),
        }],
        page: "Satz. ".repeat(1000),
    }
}

impl KnowledgeSource for Fake {
    fn dictations(
        &self,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Dictation>, String> {
        Ok(self
            .dictations
            .iter()
            .filter(|d| from_ms.is_none_or(|f| d.at_ms >= f) && to_ms.is_none_or(|t| d.at_ms < t))
            .take(limit)
            .cloned()
            .collect())
    }
    fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
        Ok(self.conversations.clone())
    }
    fn conversation_entries(&self, limit: usize) -> Result<Vec<EntryInfo>, String> {
        Ok(self.entries.iter().take(limit).cloned().collect())
    }
    fn conversation(&self, id: &str) -> Result<ConversationView, String> {
        let c = self
            .conversations
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| "Das Gespräch gibt es nicht (mehr).".to_string())?;
        let mut entries: Vec<EntryLine> = (0..60)
            .map(|i| EntryLine {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                at_ms: at(2026, 11, 1, 8, 0) + i * 1000,
                text: format!("Beitrag {i} {}", "x".repeat(400)),
            })
            .collect();
        entries.last_mut().unwrap().text = "LETZTER".to_string();
        Ok(ConversationView {
            id: c.id.clone(),
            title: c.title.clone(),
            updated_ms: c.updated_ms,
            entries,
        })
    }
    fn calendar(&self, from_ms: i64, to_ms: i64) -> Result<Vec<EventInfo>, String> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.starts_ms < to_ms && e.ends_ms > from_ms)
            .cloned()
            .collect())
    }
    fn meetings(&self, from_ms: i64, to_ms: i64, limit: usize) -> Result<Vec<MeetingInfo>, String> {
        Ok(self
            .meetings
            .iter()
            .filter(|m| m.started_ms >= from_ms && m.started_ms < to_ms)
            .take(limit)
            .cloned()
            .collect())
    }
    fn pages(&self) -> Result<Vec<PageInfo>, String> {
        Ok(self.pages.clone())
    }
    fn page_text(&self, id: &str) -> Result<String, String> {
        if id == "seite1" {
            Ok(self.page.clone())
        } else {
            Err("Die Seite gibt es nicht.".to_string())
        }
    }
}

fn everything() -> Areas {
    Areas::from_settings(
        &["meetings", "dictations", "conversations", "calendar", "tts"].map(String::from),
        true,
    )
}

fn only(area: &str) -> Areas {
    Areas::from_settings(&[area.to_string()], false)
}

fn ctx(areas: Areas) -> Context<'static> {
    Context {
        areas,
        current_conversation: Some("conv_aktuell"),
        now_ms: now(),
    }
}

fn go(areas: Areas, tool: &str, args: Value) -> Result<Value, String> {
    run(&fake(), &ctx(areas), tool, &args)
}

#[test]
fn every_tool_is_listed_once_with_a_strict_schema() {
    let list = specs();
    let mut names: Vec<&str> = list.iter().map(|s| s.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), list.len());
    for s in &list {
        assert!(
            s.name.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{}",
            s.name
        );
        assert!(s.description.contains("Nur lesend"), "{}", s.name);
        assert_eq!(s.parameters["type"], "object");
        assert_eq!(s.parameters["additionalProperties"], false, "{}", s.name);
        assert!(find(s.name).is_some());
    }
    assert!(
        find("meetings_list").is_none(),
        "die Besprechungen gehören der Brücke"
    );
    assert!(find("start_recording").is_none());
}

#[test]
fn a_tool_without_its_area_reads_nothing() {
    let cases = [
        ("dictations_recent", json!({}), "dictations"),
        ("dictations_search", json!({"query": "x"}), "dictations"),
        ("conversations_list", json!({}), "conversations"),
        (
            "conversations_search",
            json!({"query": "x"}),
            "conversations",
        ),
        (
            "conversation_get",
            json!({"id": "conv_alt"}),
            "conversations",
        ),
        ("calendar_events", json!({}), "calendar"),
        ("tts_pages_list", json!({}), "tts"),
        ("tts_page_text", json!({"id": "seite1"}), "tts"),
    ];
    let all = ["dictations", "conversations", "calendar", "tts"];
    for (tool, args, area) in cases {
        assert!(
            go(only(area), tool, args.clone()).is_ok(),
            "{tool} mit {area}"
        );
        for other in all.iter().filter(|o| **o != area) {
            let err = go(only(other), tool, args.clone()).unwrap_err();
            assert!(err.contains("nicht freigegeben"), "{tool}: {err}");
            assert!(err.contains("Gesprächseinstellungen → Wissen"), "{err}");
        }
        assert!(go(Areas::none(), tool, args).is_err(), "{tool} ohne alles");
    }
    // Der Tagesüberblick braucht mindestens einen seiner Bereiche, nicht aber „Vorlesen“.
    assert!(go(Areas::none(), "day_overview", json!({})).is_err());
    assert!(go(only("tts"), "day_overview", json!({})).is_err());
    assert!(go(only("calendar"), "day_overview", json!({})).is_ok());
}

#[test]
fn recent_dictations_are_short_and_limited() {
    let v = go(everything(), "dictations_recent", json!({"limit": 2})).unwrap();
    assert_eq!(v["returned"], 2);
    assert_eq!(v["dictations"][0]["id"], 3);
    let all = go(everything(), "dictations_recent", json!({})).unwrap();
    assert_eq!(all["returned"], 3);
    let long = all["dictations"][2]["text"].as_str().unwrap();
    assert!(
        long.chars().count() <= EXCERPT_CHARS + 2,
        "{}",
        long.chars().count()
    );
    assert!(long.ends_with('…'));
    assert!(all["dictations"][0]["time"]
        .as_str()
        .unwrap()
        .starts_with("2026-11-01 09:15"));
}

#[test]
fn dictation_search_needs_all_words_and_ignores_case() {
    let v = go(
        everything(),
        "dictations_search",
        json!({"query": "ORDNUNGSAMT grundsteuerbescheid"}),
    )
    .unwrap();
    assert_eq!(v["returned"], 1);
    assert_eq!(v["dictations"][0]["id"], 3);
    // Umlaute werden ohne Rücksicht auf die Schreibweise gefunden.
    let v = go(everything(), "dictations_search", json!({"query": "äpfel"})).unwrap();
    assert_eq!(v["dictations"][0]["id"], 2);
    let v = go(
        everything(),
        "dictations_search",
        json!({"query": "ÄPFEL milch"}),
    )
    .unwrap();
    assert_eq!(v["returned"], 1);
    let none = go(
        everything(),
        "dictations_search",
        json!({"query": "Grundsteuer Äpfel"}),
    )
    .unwrap();
    assert_eq!(none["returned"], 0);
    assert!(go(everything(), "dictations_search", json!({})).is_err());
    assert!(go(everything(), "dictations_search", json!({"query": "   "})).is_err());
}

#[test]
fn the_excerpt_centres_on_the_match() {
    let text = format!("{} Treffer {}", "vorne ".repeat(80), "hinten ".repeat(80));
    let at = text.find("Treffer").unwrap();
    let e = excerpt(&text, text[..at].chars().count(), 100);
    assert!(e.contains("Treffer"), "{e}");
    assert!(e.starts_with('…') && e.ends_with('…'));
    assert!(e.chars().count() <= 102);
    assert_eq!(excerpt("kurz", 0, 100), "kurz");
}

#[test]
fn the_running_conversation_is_never_shown() {
    let list = go(everything(), "conversations_list", json!({})).unwrap();
    let ids: Vec<&str> = list["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["conv_haushalt", "conv_alt"]);
    let hits = go(
        everything(),
        "conversations_search",
        json!({"query": "haushaltsplan"}),
    )
    .unwrap();
    let ids: Vec<&str> = hits["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec!["conv_haushalt"],
        "ein Treffer je Gespräch, ohne das laufende"
    );
    let err = go(
        everything(),
        "conversation_get",
        json!({"id": "conv_aktuell"}),
    )
    .unwrap_err();
    assert!(err.contains("laufende Gespräch"), "{err}");
}

fn without_id(id: Option<&'static str>) -> Context<'static> {
    Context {
        current_conversation: id,
        ..ctx(everything())
    }
}

#[test]
fn without_the_running_conversation_id_no_conversation_tool_runs() {
    // Fail closed: ohne Kennung (vor dem Anlegen, leer, ungueltig) laeuft kein Gespraechswerkzeug.
    for id in [None, Some(""), Some("../x")] {
        let c = without_id(id);
        for (tool, args) in [
            ("conversations_list", json!({})),
            ("conversations_search", json!({"query": "haushalt"})),
            ("conversation_get", json!({"id": "conv_alt"})),
        ] {
            let err = run(&fake(), &c, tool, &args).unwrap_err();
            assert!(
                err.contains("noch nicht gespeichert"),
                "{tool} {id:?}: {err}"
            );
        }
        let day = run(&fake(), &c, "day_overview", &json!({})).unwrap();
        assert_eq!(day["conversations"], json!([]));
        assert!(day["conversations_hinweis"].is_string());
        assert!(day["events"].is_array(), "andere Bereiche laufen weiter");
    }
    let known = run(
        &fake(),
        &without_id(Some("conv_aktuell")),
        "conversations_list",
        &json!({}),
    );
    assert!(known.is_ok());
}

#[test]
fn unknown_argument_fields_are_reported_without_their_name() {
    let err = go(everything(), "calendar_events", json!({"GEHEIMER_NAME": 1})).unwrap_err();
    assert!(!err.contains("GEHEIMER"), "{err}");
    let err = go(
        everything(),
        "dictations_recent",
        json!({"zzz": 1, "limit": 2}),
    )
    .unwrap_err();
    assert!(!err.contains("zzz"), "{err}");
}

fn all_day(title: &str, start_ms: i64, end_ms: i64) -> EventInfo {
    EventInfo {
        title: title.to_string(),
        starts_ms: start_ms,
        ends_ms: end_ms,
        all_day: true,
        location: None,
    }
}

/// UTC-Mitternacht des Tages (so speichert Graph ganztaegige Termine).
fn utc_midnight(y: i32, m: u32, d: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(y, m, d, 0, 0, 0)
        .unwrap()
        .timestamp_millis()
}

fn titles_of(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap().to_string())
        .collect()
}

fn with_events(events: Vec<EventInfo>) -> Fake {
    let mut f = fake();
    f.events = events;
    f
}

#[test]
fn all_day_events_stay_on_their_own_day_whatever_the_source_stores() {
    // ICS: Mitternacht Ortszeit; Graph: Mitternacht UTC. Beide sind der 2.11.
    let f = with_events(vec![
        all_day("ICS-Tag", at(2026, 11, 2, 0, 0), at(2026, 11, 3, 0, 0)),
        all_day(
            "Graph-Tag",
            utc_midnight(2026, 11, 2),
            utc_midnight(2026, 11, 3),
        ),
    ]);
    let day = |d: &str| {
        let v = run(
            &f,
            &ctx(everything()),
            "day_overview",
            &json!({ "date": d }),
        )
        .unwrap();
        titles_of(&v["events"])
    };
    assert!(day("2026-11-01").is_empty(), "nicht am Vortag");
    assert!(day("2026-11-03").is_empty(), "nicht am Folgetag");
    assert_eq!(day("2026-11-02"), ["ICS-Tag", "Graph-Tag"]);
    let range = |from: &str, to: &str| {
        let v = run(
            &f,
            &ctx(everything()),
            "calendar_events",
            &json!({ "from": from, "to": to }),
        )
        .unwrap();
        (titles_of(&v["events"]), v)
    };
    assert!(range("2026-11-01", "2026-11-01").0.is_empty());
    assert!(range("2026-11-03", "2026-11-04").0.is_empty());
    let (hit, v) = range("2026-11-02", "2026-11-02");
    assert_eq!(hit.len(), 2);
    assert_eq!(v["events"][0]["day"], "2026-11-02");
    assert!(v["events"][0].get("until").is_none());
}

#[test]
fn a_multi_day_all_day_event_covers_each_of_its_days() {
    let f = with_events(vec![
        all_day("Klausur", at(2026, 11, 3, 0, 0), at(2026, 11, 6, 0, 0)),
        all_day(
            "Dienstreise",
            utc_midnight(2026, 11, 3),
            utc_midnight(2026, 11, 6),
        ),
    ]);
    let on = |d: &str| {
        let v = run(
            &f,
            &ctx(everything()),
            "day_overview",
            &json!({ "date": d }),
        )
        .unwrap();
        titles_of(&v["events"]).len()
    };
    assert_eq!(on("2026-11-02"), 0);
    assert_eq!(on("2026-11-03"), 2);
    assert_eq!(on("2026-11-05"), 2, "letzter Tag einschliesslich");
    assert_eq!(on("2026-11-06"), 0, "das Ende ist ausschliessend");
    let v = run(
        &f,
        &ctx(everything()),
        "calendar_events",
        &json!({"from": "2026-11-01", "to": "2026-11-10"}),
    )
    .unwrap();
    assert_eq!(v["events"][0]["day"], "2026-11-03");
    assert_eq!(v["events"][0]["until"], "2026-11-05");
}

#[test]
fn a_conversation_comes_back_compact_and_newest_last() {
    let v = go(everything(), "conversation_get", json!({"id": "conv_alt"})).unwrap();
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.last().unwrap()["text"], "LETZTER");
    let chars: usize = entries
        .iter()
        .map(|e| e["text"].as_str().unwrap().chars().count())
        .sum();
    assert!(chars <= CONVERSATION_BUDGET_CHARS, "{chars}");
    assert!(entries.len() < 30, "das Zeichenbudget begrenzt");
    assert_eq!(
        v["earlier_entries_omitted"].as_u64().unwrap() as usize + entries.len(),
        60
    );
    assert!(go(everything(), "conversation_get", json!({"id": "../x"})).is_err());
    assert!(go(
        everything(),
        "conversation_get",
        json!({"id": "gibt_es_nicht"})
    )
    .is_err());
    assert!(go(everything(), "conversation_get", json!({})).is_err());
}

#[test]
fn calendar_defaults_to_the_next_seven_days_and_reads_only_the_cache() {
    let v = go(everything(), "calendar_events", json!({})).unwrap();
    assert_eq!(v["from"], "2026-11-01");
    assert_eq!(v["to"], "2026-11-07");
    assert_eq!(v["returned"], 3);
    assert!(v["note"].as_str().unwrap().contains("Cache"));
    let one = go(
        everything(),
        "calendar_events",
        json!({"from": "2026-11-01", "to": "2026-11-01"}),
    )
    .unwrap();
    assert_eq!(one["returned"], 2);
    let titles: Vec<&str> = one["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        titles,
        vec!["Feiertag", "Teamrunde"],
        "zeitlich sortiert, Ganztägiges zuerst"
    );
    assert_eq!(one["events"][1]["location"], "Raum 2");
    assert_eq!(one["events"][0]["day"], "2026-11-01");
    assert!(one["events"][0].get("from").is_none());
    let tomorrow = go(
        everything(),
        "calendar_events",
        json!({"from": "morgen", "to": "morgen"}),
    )
    .unwrap();
    assert_eq!(tomorrow["from"], "2026-11-02");
}

#[test]
fn calendar_arguments_are_checked() {
    for args in [
        json!({"from": "2026-11-05", "to": "2026-11-01"}),
        json!({"from": "2026-01-01", "to": "2026-12-31"}),
        json!({"from": "nächste Woche"}),
        json!({"from": "2026-13-45"}),
        json!({"from": 5}),
        json!({"bis": "2026-11-05"}),
    ] {
        assert!(
            go(everything(), "calendar_events", args.clone()).is_err(),
            "{args}"
        );
    }
}

#[test]
fn the_day_overview_shows_only_shared_areas() {
    let v = go(everything(), "day_overview", json!({})).unwrap();
    assert_eq!(v["date"], "2026-11-01");
    assert_eq!(v["meetings"][0]["title"], "Wochenplanung");
    assert_eq!(v["meetings"][0]["minutes"], 32);
    assert_eq!(
        v["dictations"].as_array().unwrap().len(),
        1,
        "nur die von heute"
    );
    let convs: Vec<&str> = v["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(convs, vec!["conv_haushalt"], "ohne das laufende Gespräch");
    assert_eq!(v["events"].as_array().unwrap().len(), 2);
    assert!(v.get("nicht_freigegeben").is_none());

    let part = go(
        Areas::from_settings(&["calendar".to_string(), "dictations".to_string()], false),
        "day_overview",
        json!({"date": "2026-11-01"}),
    )
    .unwrap();
    assert!(part.get("meetings").is_none() && part.get("conversations").is_none());
    assert!(part.get("events").is_some() && part.get("dictations").is_some());
    assert_eq!(
        part["nicht_freigegeben"],
        json!(["Besprechungen", "frühere Gespräche"])
    );

    let other = go(everything(), "day_overview", json!({"date": "2026-10-31"})).unwrap();
    assert_eq!(other["dictations"].as_array().unwrap().len(), 1);
    assert!(other["meetings"].as_array().unwrap().is_empty());
    let yesterday = go(everything(), "day_overview", json!({"date": "gestern"})).unwrap();
    assert_eq!(yesterday["date"], "2026-10-31");
}

#[test]
fn the_day_overview_stays_small() {
    let mut f = fake();
    f.dictations = (0..40)
        .map(|i| dictation(i, at(2026, 11, 1, 8, 0), &"wort ".repeat(100)))
        .collect();
    f.events = (0..40)
        .map(|i| EventInfo {
            title: format!("Termin {i} {}", "t".repeat(300)),
            starts_ms: at(2026, 11, 1, 8, 0),
            ends_ms: at(2026, 11, 1, 9, 0),
            all_day: false,
            location: Some("o".repeat(300)),
        })
        .collect();
    let v = run(&f, &ctx(everything()), "day_overview", &json!({})).unwrap();
    assert!(
        v.to_string().chars().count() < 7_000,
        "{}",
        v.to_string().chars().count()
    );
    assert_eq!(v["events"].as_array().unwrap().len(), DAY_EVENTS);
    assert_eq!(v["weitere"]["events"], 40 - DAY_EVENTS);
    assert_eq!(v["weitere"]["dictations"], 1, "40 vorhanden, 7 abgefragt");
    assert!(v["weitere"].get("meetings").is_none());
    let small = go(everything(), "day_overview", json!({})).unwrap();
    assert!(small.get("weitere").is_none(), "nichts gekappt: kein Feld");
}

#[test]
fn tts_pages_are_listed_and_long_text_is_marked() {
    let list = go(everything(), "tts_pages_list", json!({})).unwrap();
    assert_eq!(list["pages"][0]["id"], "seite1");
    let page = go(everything(), "tts_page_text", json!({"id": "seite1"})).unwrap();
    assert_eq!(page["truncated"], true);
    assert_eq!(page["text"].as_str().unwrap().chars().count(), PAGE_CHARS);
    assert!(go(everything(), "tts_page_text", json!({"id": "x/../y"})).is_err());
    assert!(go(everything(), "tts_page_text", json!({"id": "fehlt"})).is_err());
}

#[test]
fn arguments_are_checked_strictly() {
    for (tool, args) in [
        ("dictations_recent", json!({"limit": 0})),
        ("dictations_recent", json!({"limit": "5"})),
        ("dictations_recent", json!({"limit": 2.5})),
        ("dictations_recent", json!({"extra": true})),
        ("dictations_recent", json!([])),
        ("dictations_recent", json!("x")),
        (
            "dictations_search",
            json!({"query": "a".repeat(MAX_QUERY_CHARS + 1)}),
        ),
        ("dictations_search", json!({"query": 7})),
        ("conversations_list", json!({"person": "x"})),
        ("day_overview", json!({"date": "heute", "x": 1})),
    ] {
        assert!(
            go(everything(), tool, args.clone()).is_err(),
            "{tool} {args}"
        );
    }
    // Ein sehr großes Limit wird gekappt, nicht abgelehnt; null und fehlend sind gleich.
    assert!(go(
        everything(),
        "dictations_recent",
        json!({"limit": 1_000_000})
    )
    .is_ok());
    assert!(go(everything(), "dictations_recent", Value::Null).is_ok());
    assert!(go(everything(), "nicht_da", json!({})).is_err());
}

#[test]
fn text_is_cleaned_of_control_and_direction_marks() {
    let dirty = "a\u{0}b\u{202E}c\u{200B}d\n\n e\tf";
    assert_eq!(clean_line(dirty), "abcd e f");
    let mut f = fake();
    f.dictations = vec![dictation(1, now(), "vorher\u{202E}nachher\nzeile")];
    let v = run(&f, &ctx(everything()), "dictations_recent", &json!({})).unwrap();
    assert_eq!(v["dictations"][0]["text"], "vorhernachher zeile");
}

#[test]
fn a_failing_source_is_an_error_without_content() {
    struct Broken;
    impl KnowledgeSource for Broken {
        fn dictations(
            &self,
            _: Option<i64>,
            _: Option<i64>,
            _: usize,
        ) -> Result<Vec<Dictation>, String> {
            Err("Der Diktatverlauf ist gerade nicht verfügbar.".to_string())
        }
        fn conversations(&self) -> Result<Vec<ConversationInfo>, String> {
            Err("x".into())
        }
        fn conversation_entries(&self, _: usize) -> Result<Vec<EntryInfo>, String> {
            Err("x".into())
        }
        fn conversation(&self, _: &str) -> Result<ConversationView, String> {
            Err("x".into())
        }
        fn calendar(&self, _: i64, _: i64) -> Result<Vec<EventInfo>, String> {
            Err("x".into())
        }
        fn meetings(&self, _: i64, _: i64, _: usize) -> Result<Vec<MeetingInfo>, String> {
            Err("x".into())
        }
        fn pages(&self) -> Result<Vec<PageInfo>, String> {
            Err("x".into())
        }
        fn page_text(&self, _: &str) -> Result<String, String> {
            Err("x".into())
        }
    }
    let err = run(&Broken, &ctx(everything()), "dictations_recent", &json!({})).unwrap_err();
    assert!(err.contains("nicht verfügbar"));
    assert!(run(&Broken, &ctx(everything()), "day_overview", &json!({})).is_err());
}
