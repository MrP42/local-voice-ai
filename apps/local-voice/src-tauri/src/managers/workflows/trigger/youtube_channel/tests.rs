//! Kanal-Ausloeser (B3, AK11 Ausloeserteil): Feed, Ledger, erster Abruf, Intervall, Ausfall.
//!
//! Der Abruf ist eine Attrappe (`FakeFetcher`); der echte HTTP-Weg wird gegen einen lokalen
//! Server geprueft. Nichts geht ins Netz.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::managers::integrations::model::NewIntegration;
use crate::managers::integrations::store as integrations;
use crate::managers::workflows::engine::Engine;
use crate::managers::workflows::store::{self, RunFilter};
use crate::managers::workflows::test_support::{
    armed_workflow, def, engine_with, note, roomy_gate, FakeClock, Fx, T0,
};
use crate::managers::workflows::trigger::test_sink::FlakySink;
use crate::managers::youtube::test_support::{serve, status};

const FIXTURE: &str = include_str!("fixtures/feed3.xml");
const CHANNEL: &str = "UCabcdefghijklmnopqrstuv";
const MIN: i64 = 60_000;

/// Ein Feed aus Videos `(id, titel)`; neueste zuerst.
fn feed_xml(channel: &str, videos: &[(&str, &str)]) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\"?>\n<feed xmlns:yt=\"http://www.youtube.com/xml/schemas/2015\" xmlns=\"http://www.w3.org/2005/Atom\">\n <yt:channelId>{channel}</yt:channelId>\n <title>Testkanal</title>\n"
    );
    for (id, title) in videos {
        s.push_str(&format!(
            " <entry>\n  <yt:videoId>{id}</yt:videoId>\n  <title>{title}</title>\n  <published>2026-09-30T08:00:00+00:00</published>\n </entry>\n"
        ));
    }
    s.push_str("</feed>\n");
    s
}

/// Attrappe: liefert die vorbereiteten Antworten der Reihe nach (die letzte wiederholt sich).
struct FakeFetcher {
    replies: Mutex<VecDeque<Result<String, FeedError>>>,
    last: Mutex<Option<Result<String, FeedError>>>,
    calls: AtomicUsize,
}

impl FakeFetcher {
    fn new() -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            last: Mutex::new(None),
            calls: AtomicUsize::new(0),
        }
    }
    fn push(&self, r: Result<String, FeedError>) {
        self.replies.lock().unwrap().push_back(r);
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl FeedFetcher for FakeFetcher {
    fn fetch(&self, _channel_id: &str) -> Result<String, FeedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let next = self.replies.lock().unwrap().pop_front();
        let mut last = self.last.lock().unwrap();
        match next {
            Some(r) => {
                *last = Some(r.clone());
                r
            }
            None => last.clone().unwrap_or(Err(FeedError::Timeout)),
        }
    }
}

struct World {
    fx: Fx,
    clock: Arc<FakeClock>,
    engine: Engine,
    wf: String,
    state: State,
    fetcher: FakeFetcher,
    now: i64,
}

fn world_with(extra: Value) -> World {
    let fx = Fx::new();
    let clock = FakeClock::new();
    let engine = engine_with(&fx, &clock, roomy_gate());
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({"type": KIND, "channel_id": CHANNEL});
    for (k, v) in extra.as_object().unwrap() {
        d["trigger"][k] = v.clone();
    }
    let wf = armed_workflow(&engine, &d);
    World {
        fx,
        clock,
        engine,
        wf,
        state: State::default(),
        fetcher: FakeFetcher::new(),
        now: T0 + 1_000 * MIN,
    }
}

fn world() -> World {
    world_with(json!({}))
}

impl World {
    /// Ein Takt zur aktuellen Zeit.
    fn tick(&self) -> TickReport {
        on_tick(
            &self.engine,
            &self.state,
            &self.fetcher,
            &self.fx.db_path,
            self.now,
        )
    }

    /// Die Zeit um `minutes` weiter, dann ein Takt.
    fn tick_after(&mut self, minutes: i64) -> TickReport {
        self.now += minutes * MIN;
        self.clock.set(self.now);
        self.tick()
    }

    fn runs(&self) -> Vec<store::RunRow> {
        runs_of(&self.fx, &self.wf)
    }
}

fn runs_of(fx: &Fx, wf: &str) -> Vec<store::RunRow> {
    store::list_runs(
        &fx.conn(),
        &RunFilter {
            workflow_id: Some(wf.to_string()),
            state: None,
        },
        100,
    )
    .unwrap()
}

fn trigger_of(run: &store::RunRow) -> Value {
    let ctx: Value = serde_json::from_str(&run.context_json).unwrap();
    ctx["trigger"].clone()
}

fn three() -> String {
    feed_xml(
        CHANNEL,
        &[("Vid00000003", "Drei"), ("Vid00000002", "Zwei"), ("Vid00000001", "Eins")],
    )
}

// ---------------------------------------------------------------------------
// Der Feed
// ---------------------------------------------------------------------------

#[test]
fn the_real_feed_format_is_parsed_with_entities_umlauts_and_times() {
    let feed = parse_feed(FIXTURE).unwrap();
    assert_eq!(feed.channel_id.as_deref(), Some(CHANNEL));
    assert_eq!(
        feed.channel_title.as_deref(),
        Some("Wolff Applied AI & Friends"),
        "der Titel des Kanals, nicht der eines Videos"
    );
    let ids: Vec<&str> = feed.videos.iter().map(|v| v.video_id.as_str()).collect();
    assert_eq!(ids, vec!["Vid00000003", "Vid00000002", "Vid00000001"], "neueste zuerst");
    assert_eq!(
        feed.videos[0].title,
        "Lokale KI im Mittelstand: Was \"datenschutzfreundlich\" wirklich heißt",
        "nicht der Titel aus media:group, sondern der des Eintrags, Entitaeten aufgeloest"
    );
    assert_eq!(feed.videos[1].title, "Agenten & MCP in 10 Minuten");
    assert_eq!(feed.videos[2].title, "Warum Transkripte nie fertig sind äN", "Zeichenverweise");
    assert_eq!(feed.videos[0].published.as_deref(), Some("2026-09-30T08:00:00Z"));
    assert_eq!(
        feed.videos[1].published.as_deref(),
        Some("2026-09-28T15:30:00Z"),
        "+02:00 wird nach UTC umgerechnet"
    );
}

#[test]
fn hostile_or_broken_feeds_are_rejected_or_cleaned() {
    // Keine Dokumenttyp- und Entitaetsdefinitionen (XML-Bombe, externe Entitaet).
    let bomb = "<?xml version=\"1.0\"?>\n<!DOCTYPE feed [<!ENTITY a \"aaaa\">]>\n<feed><entry><yt:videoId>Vid00000001</yt:videoId></entry></feed>";
    assert_eq!(parse_feed(bomb).unwrap_err().code(), "feed_bad");
    // Kein Feed.
    assert_eq!(parse_feed("<html>nope</html>").unwrap_err().code(), "feed_bad");
    assert_eq!(parse_feed("").unwrap_err().code(), "feed_bad");
    // Ein leerer Feed ist gueltig (ein Kanal ohne Videos).
    let empty = parse_feed(&feed_xml(CHANNEL, &[])).unwrap();
    assert!(empty.videos.is_empty());
    // Ungueltige Kennungen werden uebergangen, Steuerzeichen und Laenge bereinigt.
    let long = "x".repeat(900);
    let xml = feed_xml(
        CHANNEL,
        &[
            ("kurz", "zu kurz"),
            ("Vid0000000!", "Sonderzeichen"),
            ("Vid00000001", &format!("Zeile\u{0007}eins\nZeile zwei {long}")),
        ],
    );
    let feed = parse_feed(&xml).unwrap();
    assert_eq!(feed.videos.len(), 1, "{:?}", feed.videos);
    let title = &feed.videos[0].title;
    assert!(title.starts_with("Zeile eins Zeile zwei x"), "{title}");
    assert!(title.chars().count() <= 300 && title.ends_with('…'));
    // CDATA und Attribute am Tag.
    let cdata = "<feed><yt:channelId>UCabcdefghijklmnopqrstuv</yt:channelId><entry id=\"1\"><yt:videoId>Vid00000001</yt:videoId><title lang=\"de\"><![CDATA[Fett & <b>roh</b>]]></title></entry></feed>";
    let feed = parse_feed(cdata).unwrap();
    assert_eq!(feed.videos[0].title, "Fett & <b>roh</b>");
    // Eine Zeit, die keine ist, bleibt leer statt zu raten.
    let xml = feed_xml(CHANNEL, &[("Vid00000001", "T")]).replace("2026-09-30T08:00:00+00:00", "gestern");
    assert_eq!(parse_feed(&xml).unwrap().videos[0].published, None);
}

#[test]
fn channel_ids_have_a_strict_shape() {
    assert!(valid_channel_id(CHANNEL));
    assert!(valid_channel_id("UC_-AZaz09_-AZaz09_-AZaz"));
    for bad in ["", "@wolff", "UCtooshort", "ucabcdefghijklmnopqrstuv", "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv", "UCabcdefghijklmnopqrstu!", "UCabcdefghijklmnopqrstuvw"] {
        assert!(!valid_channel_id(bad), "{bad}");
    }
}

#[test]
fn the_definition_check_rejects_handles_and_links_with_a_sentence() {
    let w = world();
    let mut d = def(vec![note("a")]);
    d["trigger"] = json!({"type": KIND, "channel_id": "@wolffappliedai"});
    let err = w.engine.save_workflow(None, &d).unwrap_err();
    let store::WorkflowError::Invalid(issues) = err else {
        panic!("erwartet Invalid");
    };
    let issue = issues
        .iter()
        .find(|i| i.path == "/trigger/channel_id")
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert!(issue.message.contains("UC") && issue.message.contains("24"), "{}", issue.message);
    d["trigger"] = json!({"type": KIND, "channel_id": CHANNEL, "poll_minutes": 30, "backfill": 2});
    w.engine.save_workflow(None, &d).unwrap();
}

// ---------------------------------------------------------------------------
// AK11: Fixture mit 3 Videos -> genau 3 Laeufe, zweiter Abruf 0
// ---------------------------------------------------------------------------

#[test]
fn ak11_a_feed_with_three_videos_makes_exactly_three_runs_and_the_second_fetch_none() {
    // Der erste Abruf gilt dem Kanal ohne Videos (Anfangsmarke), dann erscheinen drei.
    let mut w = world();
    w.fetcher.push(Ok(feed_xml(CHANNEL, &[])));
    assert!(w.tick().started.is_empty());
    w.fetcher.push(Ok(FIXTURE.to_string()));
    let second = w.tick_after(60);
    assert_eq!(second.started.len(), 3, "{second:?}");
    assert_eq!(w.runs().len(), 3);
    // Zweiter Abruf desselben Feeds: 0 neue Laeufe, und es wird nicht einmal eingereiht.
    let again = w.tick_after(60);
    assert!(again.started.is_empty(), "{again:?}");
    assert_eq!(again.duplicates, 0, "das Ledger kennt sie: kein Einreihversuch");
    assert_eq!(w.runs().len(), 3);
    assert_eq!(w.fetcher.calls(), 3);

    // Die Daten jedes Laufs; entstanden in der Reihenfolge der Veroeffentlichung (aelteste zuerst).
    let runs = w.runs();
    let keys: Vec<String> = w
        .fx
        .conn()
        .prepare("SELECT trigger_key FROM workflow_runs ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        keys,
        vec![
            "yt:UCabcdefghijklmnopqrstuv:Vid00000001",
            "yt:UCabcdefghijklmnopqrstuv:Vid00000002",
            "yt:UCabcdefghijklmnopqrstuv:Vid00000003"
        ]
    );
    let t = trigger_of(runs.iter().find(|r| r.trigger_key.ends_with("Vid00000003")).unwrap());
    assert_eq!(t["channel_id"], CHANNEL);
    assert_eq!(t["channel_title"], "Wolff Applied AI & Friends");
    assert_eq!(t["video_id"], "Vid00000003");
    assert_eq!(t["url"], "https://www.youtube.com/watch?v=Vid00000003");
    assert_eq!(t["published"], "2026-09-30T08:00:00Z");
    assert!(t["title"].as_str().unwrap().contains("heißt"));
}

#[test]
fn ak11_with_backfill_three_the_first_fetch_itself_makes_three_runs() {
    let w = world_with(json!({"backfill": 3}));
    w.fetcher.push(Ok(FIXTURE.to_string()));
    let first = w.tick();
    assert_eq!(first.started.len(), 3, "{first:?}");
    let mut w = w;
    let again = w.tick_after(60);
    assert!(again.started.is_empty(), "{again:?}");
    assert_eq!(w.runs().len(), 3);
}

#[test]
fn the_first_fetch_marks_existing_videos_as_known_without_a_flood() {
    let mut w = world();
    w.fetcher.push(Ok(FIXTURE.to_string()));
    let first = w.tick();
    assert!(first.started.is_empty(), "keine Flut beim ersten Abruf: {first:?}");
    assert_eq!(w.runs().len(), 0);
    let conn = w.fx.conn();
    assert_eq!(ledger::count(&conn, "yt:").unwrap(), 3, "drei bekannte Videos");
    assert!(ledger::exists(&conn, &ledger::channel_init_key(&w.wf, CHANNEL)).unwrap());
    // Ein neues Video: genau ein Lauf, mit seinen Daten.
    w.fetcher.push(Ok(feed_xml(
        CHANNEL,
        &[
            ("Vid00000004", "Neu"),
            ("Vid00000003", "Drei"),
            ("Vid00000002", "Zwei"),
            ("Vid00000001", "Eins"),
        ],
    )));
    let r = w.tick_after(60);
    assert_eq!(r.started.len(), 1, "{r:?}");
    assert_eq!(trigger_of(&w.runs()[0])["video_id"], "Vid00000004");
}

#[test]
fn backfill_takes_the_newest_n_and_marks_the_rest_known() {
    let w = world_with(json!({"backfill": 2}));
    w.fetcher.push(Ok(FIXTURE.to_string()));
    let r = w.tick();
    assert_eq!(r.started.len(), 2, "{r:?}");
    let mut ids: Vec<String> = w
        .runs()
        .iter()
        .map(|r| trigger_of(r)["video_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["Vid00000002", "Vid00000003"], "die zwei neuesten");
    assert!(
        ledger::exists(&w.fx.conn(), &ledger::video_key(&w.wf, CHANNEL, "Vid00000001")).unwrap(),
        "das aelteste ist bekannt"
    );
}

#[test]
fn a_restart_repeats_no_run_and_fetches_at_once_again() {
    let mut w = world();
    w.fetcher.push(Ok(feed_xml(CHANNEL, &[])));
    w.tick();
    w.fetcher.push(Ok(three()));
    assert_eq!(w.tick_after(60).started.len(), 3);
    // Neustart: neuer Zustand, neue Engine; der Takt ruft sofort ab (Nachholen), startet nichts.
    w.state = State::default();
    w.engine = engine_with(&w.fx, &w.clock, roomy_gate());
    let calls = w.fetcher.calls();
    let r = w.tick();
    assert_eq!(w.fetcher.calls(), calls + 1, "sofort nach dem Start");
    assert!(r.started.is_empty() && r.duplicates == 0, "{r:?}");
    assert_eq!(w.runs().len(), 3);
}

#[test]
fn a_video_published_while_the_app_was_closed_is_caught_up() {
    let mut w = world();
    w.fetcher.push(Ok(three()));
    w.tick(); // erster Abruf: alle bekannt
    // Die App war aus; inzwischen kam Nr. 4.
    w.state = State::default();
    w.fetcher.push(Ok(feed_xml(
        CHANNEL,
        &[("Vid00000004", "Neu"), ("Vid00000003", "Drei"), ("Vid00000002", "Zwei")],
    )));
    let r = w.tick_after(24 * 60);
    assert_eq!(r.started.len(), 1, "{r:?}");
}

// ---------------------------------------------------------------------------
// Intervall
// ---------------------------------------------------------------------------

#[test]
fn the_poll_interval_decides_when_a_channel_is_fetched_again() {
    let mut w = world_with(json!({"poll_minutes": 30}));
    w.fetcher.push(Ok(three()));
    w.tick();
    assert_eq!(w.fetcher.calls(), 1);
    for minutes in [1, 10, 10, 8] {
        w.tick_after(minutes);
        assert_eq!(w.fetcher.calls(), 1, "nach {minutes} min noch nicht");
    }
    w.tick_after(1); // 30 min nach dem ersten Abruf
    assert_eq!(w.fetcher.calls(), 2);
    let st = w.state.status();
    assert_eq!(st.len(), 1);
    assert_eq!(st[0].next_fetch_ms, w.now + 30 * MIN);
}

#[test]
fn the_interval_is_clamped_to_fifteen_minutes_and_a_day() {
    let mk = |v: Value| {
        let t = TriggerDef {
            kind: KIND.to_string(),
            params: json!({"channel_id": CHANNEL, "poll_minutes": v})
                .as_object()
                .unwrap()
                .clone(),
        };
        Spec::from_def(&t).unwrap().poll_ms
    };
    assert_eq!(mk(json!(1)), 15 * MIN);
    assert_eq!(mk(json!(60)), 60 * MIN);
    assert_eq!(mk(json!(99_999)), 1440 * MIN);
    let t = TriggerDef {
        kind: KIND.to_string(),
        params: json!({"channel_id": CHANNEL}).as_object().unwrap().clone(),
    };
    assert_eq!(Spec::from_def(&t).unwrap().poll_ms, DEFAULT_POLL_MINUTES * MIN);
}

#[test]
fn at_most_three_channels_are_fetched_per_tick() {
    let w = world();
    // Fuenf weitere Ablaeufe auf anderen Kanaelen.
    for i in 0..5 {
        let mut d = def(vec![note("a")]);
        d["name"] = json!(format!("Kanal {i}"));
        d["trigger"] = json!({"type": KIND, "channel_id": format!("UCabcdefghijklmnopqrstu{i}")});
        armed_workflow(&w.engine, &d);
    }
    w.fetcher.push(Ok(feed_xml("", &[])));
    let _ = w.tick();
    assert_eq!(w.fetcher.calls(), MAX_FETCH_PER_TICK);
    let _ = w.tick();
    assert_eq!(w.fetcher.calls(), 6, "der Rest folgt im naechsten Takt, nicht zweimal");
}

// ---------------------------------------------------------------------------
// Ausfall
// ---------------------------------------------------------------------------

#[test]
fn repeated_failures_make_one_outage_report_one_audit_entry_and_a_recovery() {
    let mut w = world();
    w.fetcher.push(Ok(three()));
    w.tick(); // erster Abruf: bekannt
    w.fetcher.push(Err(FeedError::NotFound));
    // Fehlschlag 1 und 2: nur Log, kein Bericht.
    for n in 1..OUTAGE_AFTER {
        let r = w.tick_after(60);
        assert!(r.errors.is_empty(), "Fehlschlag {n}: {r:?}");
    }
    assert_eq!(w.state.status()[0].failures, OUTAGE_AFTER - 1);
    assert!(!w.state.status()[0].outage);
    // Der dritte ruft den Ausfall aus.
    let r = w.tick_after(60);
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert!(r.errors[0].contains("nicht erreichbar") && r.errors[0].contains("404"));
    let st = w.state.status().remove(0);
    assert!(st.outage && st.failures == OUTAGE_AFTER);
    assert!(st.last_error.unwrap().contains("404"));
    // Weitere Fehlschlaege: nichts Neues im Bericht und im Audit (einmal je Ausfall).
    for _ in 0..3 {
        let r = w.tick_after(60);
        assert!(r.errors.is_empty(), "{r:?}");
    }
    let conn = w.fx.conn();
    let audits: Vec<_> = crate::managers::integrations::audit::list(&conn, &Default::default(), 50)
        .unwrap()
        .into_iter()
        .filter(|a| a.target.as_deref() == Some(&format!("youtube:channel:{CHANNEL}")))
        .collect();
    let errors = audits.iter().filter(|a| a.outcome == "error").count();
    assert_eq!(errors, 1, "{audits:?}");
    // Der Kanal ist wieder da: Rueckkehr gemeldet, Zaehler zurueck, Audit ok.
    w.fetcher.push(Ok(three()));
    let r = w.tick_after(60);
    assert!(r.skipped.iter().any(|s| s.contains("wieder erreichbar")), "{r:?}");
    let st = w.state.status().remove(0);
    assert!(!st.outage && st.failures == 0 && st.last_error.is_none());
    assert!(st.last_ok_ms.is_some());
}

#[test]
fn after_a_failure_the_retry_comes_sooner_than_the_interval_but_never_later() {
    let mut w = world_with(json!({"poll_minutes": 120}));
    w.fetcher.push(Err(FeedError::Timeout));
    w.tick();
    let st = w.state.status().remove(0);
    assert_eq!(st.next_fetch_ms, w.now + 5 * MIN, "nach einem Fehlschlag in 5 min");
    w.tick_after(5);
    let st = w.state.status().remove(0);
    assert_eq!(st.next_fetch_ms, w.now + 10 * MIN, "5 min mal Fehlerzahl");
    let w15 = world_with(json!({"poll_minutes": 15}));
    w15.fetcher.push(Err(FeedError::Timeout));
    w15.tick();
    for _ in 0..5 {
        w15.fetcher.push(Err(FeedError::Timeout));
    }
    let mut w15 = w15;
    for _ in 0..5 {
        w15.tick_after(15);
    }
    let st = w15.state.status().remove(0);
    assert!(st.next_fetch_ms - w15.now <= 15 * MIN, "nie seltener als das Intervall");
}

#[test]
fn a_feed_of_another_channel_or_garbage_counts_as_a_failure_not_as_videos() {
    let mut w = world();
    w.fetcher.push(Ok(feed_xml("UCzzzzzzzzzzzzzzzzzzzzzz", &[("Vid00000001", "Fremd")])));
    let r = w.tick();
    assert!(r.started.is_empty());
    assert_eq!(w.state.status()[0].failures, 1);
    assert!(w.state.status()[0].last_error.as_ref().unwrap().contains("anderen Kanal"));
    w.fetcher.push(Ok("<html>Bitte Cookies akzeptieren</html>".to_string()));
    w.tick_after(5);
    assert_eq!(w.state.status()[0].failures, 2);
    assert_eq!(ledger::count(&w.fx.conn(), "yt:").unwrap(), 0, "nichts als bekannt vermerkt");
}

// ---------------------------------------------------------------------------
// Einreihen, Integration, Ablaeufe
// ---------------------------------------------------------------------------

#[test]
fn a_failed_enqueue_leaves_the_video_unknown_and_the_retry_comes_within_five_minutes() {
    let mut w = world();
    w.fetcher.push(Ok(feed_xml(CHANNEL, &[])));
    w.tick(); // Anfangsmarke
    w.fetcher.push(Ok(feed_xml(CHANNEL, &[("Vid00000001", "Eins")])));
    let sink = FlakySink::new(&w.engine, 1); // Platte voll beim ersten Einreihen
    w.now += 60 * MIN;
    let r = on_tick(&sink, &w.state, &w.fetcher, &w.fx.db_path, w.now);
    assert_eq!(r.errors.len(), 1, "{r:?}");
    assert!(r.started.is_empty());
    assert_eq!(
        ledger::count(&w.fx.conn(), "yt:").unwrap(),
        0,
        "unbekannt: nichts im Ledger"
    );
    assert_eq!(w.state.status()[0].next_fetch_ms, w.now + 5 * MIN);
    w.now += 5 * MIN;
    let r = on_tick(&sink, &w.state, &w.fetcher, &w.fx.db_path, w.now);
    assert_eq!(r.started.len(), 1, "der naechste Abruf holt es nach: {r:?}");
    assert_eq!(w.runs().len(), 1);
}

#[test]
fn a_disabled_workflow_never_goes_to_the_network() {
    let mut w = world();
    w.engine.set_enabled(&w.wf, false).unwrap();
    let r = w.tick_after(120);
    assert!(r.is_empty(), "{r:?}");
    assert_eq!(w.fetcher.calls(), 0);
    assert!(w.state.status().is_empty());
}

#[test]
fn a_disabled_or_foreign_youtube_integration_stops_the_fetch_and_says_why_once() {
    let mut w = world();
    let conn = w.fx.conn();
    let mut n = NewIntegration::new(Kind::Youtube, "YouTube");
    n.id = Some("youtube".to_string());
    integrations::create(&conn, &n, T0).unwrap();
    integrations::update(
        &conn,
        "youtube",
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(false),
            ..Default::default()
        },
        T0,
    )
    .unwrap();
    w.fetcher.push(Ok(three()));
    let r = w.tick();
    assert_eq!(w.fetcher.calls(), 0, "ausgeschaltet: kein Byte ins Netz");
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert!(r.skipped[0].contains("ausgeschaltet"));
    let more = w.tick_after(30);
    assert!(more.skipped.is_empty(), "einmal gemeldet: {more:?}");
    // Wieder eingeschaltet: es geht los.
    integrations::update(
        &conn,
        "youtube",
        &crate::managers::integrations::model::IntegrationPatch {
            enabled: Some(true),
            ..Default::default()
        },
        T0,
    )
    .unwrap();
    w.tick_after(1);
    assert_eq!(w.fetcher.calls(), 1);

    // Ohne Integration (Standard) wird abgefragt; eine ausdruecklich genannte, fehlende ruht.
    let mut d = def(vec![note("a")]);
    d["name"] = json!("Mit Integration");
    d["trigger"] = json!({"type": KIND, "channel_id": "UCabcdefghijklmnopqrstu9", "integration": "gibtsnicht"});
    armed_workflow(&w.engine, &d);
    let r = w.tick_after(1);
    assert!(r.skipped.iter().any(|s| s.contains("gibt es nicht")), "{r:?}");
}

#[test]
fn two_workflows_on_one_channel_each_get_their_own_runs() {
    let mut w = world();
    let mut d = def(vec![note("b")]);
    d["name"] = json!("Zweiter");
    d["trigger"] = json!({"type": KIND, "channel_id": CHANNEL, "backfill": 3});
    let second = armed_workflow(&w.engine, &d);
    w.fetcher.push(Ok(three()));
    let r = w.tick();
    assert_eq!(r.started.len(), 3, "nur der Ablauf mit backfill: {r:?}");
    assert_eq!(runs_of(&w.fx, &second).len(), 3);
    assert_eq!(w.runs().len(), 0, "der erste Ablauf hat sie als bekannt vermerkt");
    // Ein neues Video: beide Ablaeufe bekommen ihren Lauf.
    w.fetcher.push(Ok(feed_xml(
        CHANNEL,
        &[("Vid00000004", "Neu"), ("Vid00000003", "Drei"), ("Vid00000002", "Zwei"), ("Vid00000001", "Eins")],
    )));
    let r = w.tick_after(60);
    assert_eq!(r.started.len(), 2, "{r:?}");
    assert_eq!(w.runs().len(), 1);
    assert_eq!(runs_of(&w.fx, &second).len(), 4);
}

#[test]
fn a_second_round_at_the_same_time_is_refused() {
    let w = world();
    let guard = w.state.try_begin().expect("frei");
    let r = w.tick();
    assert_eq!(r.skipped.len(), 1, "{r:?}");
    assert_eq!(w.fetcher.calls(), 0);
    drop(guard);
    assert!(w.state.try_begin().is_some());
}

// ---------------------------------------------------------------------------
// Der echte HTTP-Weg gegen einen lokalen Server
// ---------------------------------------------------------------------------

fn http_ok(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/atom+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn fetcher_for(base: String, max_bytes: usize) -> HttpFeedFetcher {
    HttpFeedFetcher {
        base,
        timeout: Duration::from_secs(5),
        max_bytes,
        use_env_proxy: false, // ein gesetzter HTTP_PROXY darf den Testserver nicht umleiten
    }
}

#[tokio::test]
async fn the_http_fetcher_asks_for_the_feed_with_only_the_channel_id() {
    let (base, seen) = serve(vec![http_ok(FIXTURE)]).await;
    let body = fetcher_for(base, MAX_BODY_BYTES)
        .fetch_async(CHANNEL)
        .await
        .unwrap();
    assert_eq!(parse_feed(&body).unwrap().videos.len(), 3);
    let req = seen.lock().unwrap()[0].clone();
    let line = req.lines().next().unwrap();
    assert!(line.starts_with(&format!("GET /oembed?channel_id={CHANNEL} ")), "{line}");
    let lower = req.to_ascii_lowercase();
    assert!(!lower.contains("cookie:") && !lower.contains("referer:") && !lower.contains("authorization:"));
    assert!(lower.contains("user-agent: localvoiceai/"));
}

#[tokio::test]
async fn the_http_fetcher_maps_statuses_redirects_and_oversize_to_codes() {
    for (reply, code) in [
        (status(404, "Not Found"), "feed_not_found"),
        (status(410, "Gone"), "feed_not_found"),
        (status(429, "Too Many Requests"), "feed_rate_limited"),
        (status(503, "Service Unavailable"), "feed_http"),
        (status(302, "Found"), "feed_bad"),
    ] {
        let (base, _) = serve(vec![reply]).await;
        let err = fetcher_for(base, MAX_BODY_BYTES)
            .fetch_async(CHANNEL)
            .await
            .unwrap_err();
        assert_eq!(err.code(), code, "{err}");
    }
    // Zu gross: schon am Anfang abgewiesen, wenn Content-Length es verraet ...
    let (base, _) = serve(vec![http_ok(&"x".repeat(5_000))]).await;
    let err = fetcher_for(base, 1_000).fetch_async(CHANNEL).await.unwrap_err();
    assert_eq!(err, FeedError::TooBig);
    // ... und ungueltige Kennungen gehen gar nicht erst ins Netz.
    let (base, seen) = serve(vec![http_ok(FIXTURE)]).await;
    let err = fetcher_for(base, MAX_BODY_BYTES)
        .fetch_async("@name")
        .await
        .unwrap_err();
    assert_eq!(err.code(), "feed_bad");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_http_fetcher_gives_up_after_the_timeout() {
    let (base, _) = serve(vec![Vec::<u8>::new()]).await; // der Server antwortet nie
    let f = HttpFeedFetcher {
        timeout: Duration::from_millis(300),
        ..fetcher_for(base, MAX_BODY_BYTES)
    };
    let start = std::time::Instant::now();
    assert_eq!(f.fetch_async(CHANNEL).await.unwrap_err(), FeedError::Timeout);
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn an_override_address_counts_only_inside_the_sandbox() {
    assert_eq!(
        feed_base_from(Some("http://127.0.0.1:9/f"), Some("C:/sandbox")),
        "http://127.0.0.1:9/f"
    );
    assert_eq!(feed_base_from(Some("http://127.0.0.1:9/f"), None), FEED_URL);
    assert_eq!(feed_base_from(Some("http://127.0.0.1:9/f"), Some("  ")), FEED_URL);
    assert_eq!(feed_base_from(Some("ftp://x/y"), Some("C:/s")), FEED_URL);
    assert_eq!(feed_base_from(None, Some("C:/s")), FEED_URL);
}
