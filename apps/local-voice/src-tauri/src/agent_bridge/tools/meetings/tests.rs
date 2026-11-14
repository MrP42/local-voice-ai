//! Tests der lesenden Besprechungs-Werkzeuge: echte Datenbank, nur synthetische Besprechungen.

use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::catalog::{self, CallContext, ToolHandler};
use crate::agent_bridge::testkit::NOW;
use crate::agent_bridge::tools::{Host, PageRef, QueuedImport, Rendered};
use crate::managers::integrations::model::{Access, Capability};
use crate::managers::meetings::search::index::tests::{ready_meeting, tmp_store};
use crate::managers::meetings::store::{MeetingStore, StoredSegment, TranscriptDelta};
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::youtube::source::AddOptions;

/// Ein Host, der nur die Freigaben des Transkripts kennt: fuer externe Agenten und fuers Sprachgespraech.
struct MeetHost {
    transcript: bool,
    voice_transcript: bool,
    /// `true`: echte Freigabeprüfung (eigene Domäne `example.org`) statt `Offen`.
    echte_sperre: bool,
}

impl Host for MeetHost {
    fn enqueue_import(&self, _t: &str, _p: &str) -> Result<QueuedImport, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn create_page(&self, _t: &str, _x: &str) -> Result<PageRef, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn page_text(&self, _id: &str) -> Result<String, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn render_audio(&self, _p: &str, _t: &str, _f: &str) -> Result<Rendered, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        None
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions::production()
    }

    fn transcript_shared(&self) -> bool {
        self.transcript
    }

    fn voice_transcript_shared(&self) -> bool {
        self.voice_transcript
    }

    fn ki_sperre(&self) -> crate::managers::meetings::ki_freigabe::KiSperre {
        use crate::managers::meetings::ki_freigabe::KiSperre;
        if self.echte_sperre {
            return KiSperre::Pruefen {
                kundenprofil_nur_lokal: false,
                cloud_bei_externen: false,
                eigene_domaenen: vec!["example.org".into()],
            };
        }
        KiSperre::Offen
    }
}

fn segment(index: u32, start_ms: u64, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: index,
        text: text.to_string(),
        start_ms,
        end_ms: start_ms + 1_000,
        channel: 0,
        speaker_index: None,
        words: None,
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    tools: AppTools,
    meeting: String,
}

fn rig(transcript: bool) -> Rig {
    rig_with(transcript, false, 2)
}

/// `lines` Zeilen Transkript zu je rund 100 Zeichen (mehr als 40 000 Zeichen ab 400).
fn rig_with(transcript: bool, voice_transcript: bool, lines: u32) -> Rig {
    let (dir, store) = tmp_store();
    let meeting = ready_meeting(&store, "Jour fixe Vertrieb", 1_790_000_000).id;
    store
        .append_delta(
            &meeting,
            &TranscriptDelta {
                new_segments: if lines <= 2 {
                    vec![
                        segment(0, 0, "Wir starten mit dem Quartalsbericht."),
                        segment(1, 5_000, "Der Umsatz liegt bei einer Million."),
                    ]
                } else {
                    (0..lines)
                        .map(|i| {
                            let text =
                                format!("Satz {i:04} über Umsatz und Planung. {}", "x".repeat(60));
                            segment(i, u64::from(i) * 5_000, &text)
                        })
                        .collect()
                },
            },
        )
        .unwrap();
    let store: Arc<MeetingStore> = Arc::new(store);
    Rig {
        _dir: dir,
        tools: AppTools::new(
            store,
            Arc::new(MeetHost {
                transcript,
                voice_transcript,
                echte_sperre: false,
            }),
        ),
        meeting,
    }
}

fn ctx_of(client_id: &str) -> CallContext {
    CallContext {
        client_id: client_id.to_string(),
        client_label: "Sprachgespraech".to_string(),
        approved: false,
        now_ms: NOW,
    }
}

impl Rig {
    /// Ein externer Agent ueber die Bruecke.
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx_of("C1"), tool, &args)
    }

    /// Das Sprachgespraech (Zugang `voice_orchestrator`).
    fn call_voice(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx_of(CALLER_TAG), tool, &args)
    }
}

#[test]
fn the_meeting_tools_are_read_only_and_in_the_catalog() {
    let names = ["meetings_list", "meetings_search", "meeting_transcript"];
    let offered: Vec<String> = specs().into_iter().map(|s| s.name).collect();
    for name in names {
        let entry = catalog::find(name).expect("im Katalog");
        assert_eq!(entry.capability, Capability::MeetingRead, "{name}");
        assert_eq!(entry.capability.access(), Access::Read, "{name}");
        assert!(offered.iter().any(|n| n == name), "{name}");
    }
    assert_eq!(offered.len(), names.len());
    // Kein Name gehoert dem lokalen MCP-Server (sonst leitet der Proxy dessen Aufruf um).
    for s in specs() {
        assert!(!mcp_tools::is_known(&s.name), "{}", s.name);
        assert_eq!(s.input_schema["additionalProperties"], json!(false));
    }
}

#[test]
fn a_meeting_is_listed_and_searched() {
    let r = rig(true);
    let list = r.call("meetings_list", json!({})).unwrap();
    assert_eq!(list["returned"], json!(1));
    assert_eq!(list["meetings"][0]["title"], json!("Jour fixe Vertrieb"));
    let list = r
        .call("meetings_list", json!({"from": "2030-01-01"}))
        .unwrap();
    assert_eq!(list["returned"], json!(0));
    let found = r
        .call("meetings_search", json!({"query": "Jour fixe"}))
        .unwrap();
    // Ob ein Treffer kommt, haengt vom Such-Index ab (den baut die App im Hintergrund); die Form gilt immer.
    assert!(found["meetings"].is_array(), "{found}");
}

#[test]
fn arguments_are_checked_strictly() {
    let r = rig(true);
    for (tool, args) in [
        ("meetings_list", json!({"person": "Anna"})),
        ("meetings_list", json!({"folder": "x"})),
        ("meetings_list", json!({"limit": 0})),
        ("meetings_list", json!({"limit": 51})),
        ("meetings_list", json!({"limit": "zehn"})),
        ("meetings_list", json!({"from": 5})),
        ("meetings_search", json!({})),
        ("meetings_search", json!({"query": "   "})),
        ("meetings_search", json!({"query": "a".repeat(501)})),
        ("meetings_search", json!({"query": "x", "limit": 21})),
        ("meeting_transcript", json!({})),
        ("meeting_transcript", json!({"meeting_id": "../../etc"})),
        ("meeting_transcript", json!({"meeting_id": "a b"})),
        ("meeting_transcript", json!({"meeting_id": 7})),
        (
            "meeting_transcript",
            json!({"meeting_id": "x", "cursor": -1}),
        ),
        (
            "meeting_transcript",
            json!({"meeting_id": "x", "path": "C:\\a"}),
        ),
    ] {
        assert!(r.call(tool, args.clone()).is_err(), "{tool} {args}");
    }
    assert!(r.call("meetings_list", json!("kein Objekt")).is_err());
}

#[test]
fn the_transcript_is_read_page_wise_with_the_release() {
    let r = rig(true);
    let page = r
        .call("meeting_transcript", json!({"meeting_id": r.meeting}))
        .unwrap();
    let text = page["text"].as_str().unwrap();
    assert!(text.contains("Quartalsbericht"), "{page}");
    assert!(page["next_cursor"].is_null());
    let unknown = r.call("meeting_transcript", json!({"meeting_id": "gibt-es-nicht"}));
    assert!(unknown.unwrap_err().contains("nicht gefunden"));
}

#[test]
fn the_transcript_needs_the_users_release() {
    let r = rig(false);
    let err = r
        .call("meeting_transcript", json!({"meeting_id": r.meeting}))
        .unwrap_err();
    assert!(err.contains("nicht freigegeben"), "{err}");
    // Die Suche zeigt dann keinen Treffer, der nur im Transkript steht (die Filterung selbst
    // sichern die Tests des MCP-Servers mit gebautem Index).
    let found = r
        .call("meetings_search", json!({"query": "Quartalsbericht"}))
        .unwrap();
    assert_eq!(found["returned"], json!(0), "{found}");
    // Titel und Liste bleiben lesbar.
    assert_eq!(
        r.call("meetings_list", json!({})).unwrap()["returned"],
        json!(1)
    );
}

#[test]
fn bridge_and_voice_releases_are_independent() {
    // (Freigabe der Bruecke, Freigabe des Sprachgespraechs)
    for (external, voice) in [(false, false), (true, false), (false, true), (true, true)] {
        let r = rig_with(external, voice, 2);
        let args = json!({"meeting_id": r.meeting});
        assert_eq!(
            r.call("meeting_transcript", args.clone()).is_ok(),
            external,
            "extern bei ({external}, {voice})"
        );
        assert_eq!(
            r.call_voice("meeting_transcript", args).is_ok(),
            voice,
            "Sprachgespräch bei ({external}, {voice})"
        );
        // Ohne die Freigabe liefert auch die Suche des Sprachgespraechs nichts aus dem Transkript.
        if !voice {
            let found = r
                .call_voice("meetings_search", json!({"query": "Quartalsbericht"}))
                .unwrap();
            assert_eq!(found["returned"], json!(0), "{found}");
        }
        // Die Liste ist immer lesbar.
        assert_eq!(
            r.call_voice("meetings_list", json!({})).unwrap()["returned"],
            json!(1)
        );
    }
}

#[test]
fn the_voice_reads_a_long_transcript_in_small_complete_pages() {
    let r = rig_with(true, true, 450);
    let id = json!({"meeting_id": r.meeting});
    // Extern: eine grosse Seite (bis 40 000 Zeichen).
    let big = r.call("meeting_transcript", id.clone()).unwrap();
    assert!(big["total_chars"].as_u64().unwrap() > 40_000, "{big}");
    assert!(big["text"].as_str().unwrap().chars().count() > 30_000);
    // Sprachgespraech: Seiten bis 4000 Zeichen; die Seiten ergeben zusammen genau das Transkript.
    let mut cursor = 0u64;
    let mut collected = 0usize;
    let mut pages = 0;
    loop {
        let page = r
            .call_voice(
                "meeting_transcript",
                json!({"meeting_id": r.meeting, "cursor": cursor}),
            )
            .unwrap();
        let text = page["text"].as_str().unwrap();
        assert!(text.chars().count() <= crate::agent_bridge::voice::VOICE_PAGE_CHARS);
        assert!(
            page.to_string().chars().count() < crate::agent_bridge::voice::MAX_RESULT_CHARS,
            "eine Seite samt JSON passt in die Antwortgrenze"
        );
        assert_eq!(page["cursor"], json!(cursor));
        collected += text.chars().count();
        pages += 1;
        match page["next_cursor"].as_u64() {
            Some(next) => {
                assert_eq!(
                    next as usize, collected,
                    "next_cursor zeigt hinter die Seite"
                );
                cursor = next;
            }
            None => {
                assert_eq!(collected as u64, page["total_chars"].as_u64().unwrap());
                break;
            }
        }
        assert!(pages < 100, "kein Fortschritt");
    }
    assert!(pages >= 10, "{pages} Seiten");
}

/// Codex #583: Mit echter Sperre sieht die Brücke eine unbestätigte Besprechung nicht.
#[test]
fn with_the_real_gate_an_unconfirmed_meeting_stays_hidden() {
    let r = rig(true);
    let tools = AppTools::new(
        Arc::clone(&r.tools.store),
        Arc::new(MeetHost {
            transcript: true,
            voice_transcript: true,
            echte_sperre: true,
        }),
    );
    // Richtlinienspiegel wie in der App (sonst gibt der Server gar nichts aus).
    r.tools
        .store
        .ki_richtlinie_setzen(&crate::managers::meetings::ki_freigabe::Richtlinie {
            nur_lokal: false,
            cloud_bei_externen: false,
            eigene_domaenen: vec!["example.org".into()],
        })
        .unwrap();
    let ctx = ctx_of("agent-1");
    let list = tools.call(&ctx, "meetings_list", &json!({})).unwrap();
    assert_eq!(list["meetings"], json!([]));
    assert_eq!(list["total"], 0);
    let e = tools
        .call(
            &ctx,
            "meeting_transcript",
            &json!({ "meeting_id": r.meeting }),
        )
        .unwrap_err();
    assert!(e.contains("nicht für KI-Clients freigegeben"), "{e}");
    assert!(!e.contains("Umsatz"), "{e}");
}

/// Codex #583, Schreibpfade: Kein schreibendes Werkzeug der Brücke nimmt die Kennung einer
/// vorhandenen Besprechung entgegen; ihre Inhalte erreicht nur das gesperrte Lesen.
#[test]
fn no_writing_tool_takes_an_existing_meeting() {
    // Rekursiv über alle Schemas (auch verschachtelte Objekte und Listen): kein schreibendes
    // Werkzeug hat ein Feld, das eine Besprechung benennt. `run_workflow` nimmt freie `vars`;
    // dass daraus kein Besprechungsbezug wird, belegt
    // `workflows::trigger::manual::tests::an_agent_start_never_carries_a_meeting_into_the_run`.
    fn felder(schema: &Value, pfad: &str, out: &mut Vec<String>) {
        if let Some(props) = schema.get("properties").and_then(Value::as_object) {
            for (key, sub) in props {
                out.push(format!("{pfad}{key}"));
                felder(sub, &format!("{pfad}{key}."), out);
            }
        }
        if let Some(items) = schema.get("items") {
            felder(items, &format!("{pfad}[]."), out);
        }
        for k in ["anyOf", "oneOf", "allOf"] {
            for sub in schema
                .get(k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                felder(sub, pfad, out);
            }
        }
    }
    let mut geprueft = 0;
    for spec in crate::agent_bridge::tools::all_specs() {
        let entry = catalog::find(&spec.name).expect("im Katalog");
        if entry.capability.access() == Access::Read {
            continue;
        }
        let mut alle = Vec::new();
        felder(&spec.input_schema, "", &mut alle);
        for feld in &alle {
            let letzter = feld.rsplit('.').next().unwrap_or(feld).to_lowercase();
            assert!(
                !letzter.contains("meeting") && !letzter.contains("besprechung"),
                "{}: {feld}",
                spec.name
            );
        }
        geprueft += 1;
    }
    assert!(geprueft >= 5, "die schreibenden Werkzeuge wurden geprüft");
}
