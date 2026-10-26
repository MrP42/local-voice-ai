//! Tests der Werkzeuge der Sprachausgabe-Anbieter (P3b): Katalog und Rechte, Auskunft, Wahl mit
//! Sperren, Benchmark-Ergebnisse (Pfade, Größe), Brücke mit Rechten.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;
use crate::agent_bridge::bridge::{Bridge, BridgeError, CallStep, ClientCtx};
use crate::agent_bridge::catalog::{self, CallContext, ToolHandler, ToolRegistry};
use crate::agent_bridge::clients;
use crate::agent_bridge::protocol::code;
use crate::agent_bridge::testkit::{fast_config, NOW};
use crate::agent_bridge::tools::{Host, PageRef, QueuedImport, Rendered};
use crate::commands::tts_providers::select_in;
use crate::managers::integrations::model::{Access, Caller, Capability, GrantMode, Kind};
use crate::managers::integrations::store as register;
use crate::managers::integrations::test_support::Fx;
use crate::managers::meetings::store::MeetingStore;
use crate::managers::tts::engine::TtsEngineKind;
use crate::managers::tts::providers::contract::{ProviderTier, SynthesisOptions};
use crate::managers::tts::providers::mock::MockProvider;
use crate::managers::tts::providers::pipeline::ProviderEngine;
use crate::managers::tts::providers::policy::{
    license_fingerprint, resolve_provider_configs, LiveApprovals, ProviderOverrides,
    TtsProvidersSettings,
};
use crate::managers::tts::providers::registry::ProviderRegistry;
use crate::managers::tts::providers::setup::{build_registry, ProviderPaths};
use crate::managers::workflows::recording::RecordingControl;
use crate::managers::youtube::source::AddOptions;
use crate::settings::{get_default_settings, AppSettings};

const TOOLS: [&str; 6] = [
    "tts_providers_list",
    "tts_provider_capabilities",
    "tts_provider_voices",
    "tts_provider_health",
    "tts_provider_select",
    "tts_benchmark_results",
];

// ---------------------------------------------------------------------------
// Attrappen
// ---------------------------------------------------------------------------

/// Anbieterschicht der Attrappe: die fünf echten Anbieter (ohne Installation) plus ein Mock
/// `mock-tts`; `select` nimmt den ECHTEN Weg (`select_in`) auf Einstellungen im Speicher.
struct FakeAccess {
    settings: Mutex<AppSettings>,
    mock: Arc<MockProvider>,
    select_calls: AtomicUsize,
    roots: Mutex<BenchRoots>,
}

impl FakeAccess {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            settings: Mutex::new(get_default_settings()),
            mock: Arc::new(MockProvider::new().with_id("mock-tts")),
            select_calls: AtomicUsize::new(0),
            roots: Mutex::new(BenchRoots::default()),
        })
    }

    fn enable(&self, id: &str) {
        self.settings
            .lock()
            .unwrap()
            .tts_providers
            .set_enabled(id, true);
    }

    fn providers(&self) -> TtsProvidersSettings {
        self.settings.lock().unwrap().tts_providers.clone()
    }

    fn engine(&self) -> String {
        self.settings.lock().unwrap().tts_engine.clone()
    }

    fn set_roots(&self, roots: BenchRoots) {
        *self.roots.lock().unwrap() = roots;
    }
}

impl TtsProviderAccess for FakeAccess {
    fn snapshot(&self) -> Result<ProviderSnapshot, String> {
        let s = self.settings.lock().unwrap().clone();
        let resolved =
            resolve_provider_configs(&s.tts_providers, &|_| None, &ProviderOverrides::default());
        let paths = ProviderPaths {
            fish_dir: PathBuf::from("C:/gibt/es/nicht"),
            ..Default::default()
        };
        let registry = build_registry(&resolved, &paths, None).map_err(|e| e.message)?;
        registry
            .register(self.mock.clone())
            .map_err(|e| e.message)?;
        let selected = crate::managers::tts::engine::provider_id_from_setting(&s.tts_engine)
            .unwrap_or_else(|| "fish".to_string());
        Ok(ProviderSnapshot {
            settings: Arc::new(s.tts_providers.clone()),
            resolved: Arc::new(resolved),
            registry: Arc::new(registry),
            selected,
            fish_voice: s.tts_voice.clone(),
            piper_voice: s.tts_piper_voice.clone(),
        })
    }

    fn select(&self, id: &str, voice: Option<String>) -> Result<(), TtsError> {
        self.select_calls.fetch_add(1, Ordering::SeqCst);
        let snapshot = self.snapshot().map_err(TtsError::unavailable)?;
        let mut settings = self.settings.lock().unwrap();
        select_in(&mut settings, &snapshot, id, voice)
    }

    fn benchmark_roots(&self) -> BenchRoots {
        self.roots.lock().unwrap().clone()
    }
}

/// Ein Host, der nur die Anbieterschicht kennt.
struct TtsHost {
    access: Option<Arc<dyn TtsProviderAccess>>,
}

impl Host for TtsHost {
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

    fn tts_providers(&self) -> Option<Arc<dyn TtsProviderAccess>> {
        self.access.clone()
    }
}

struct Rig {
    // Hält die Test-Datenbank am Leben.
    _fx: Fx,
    tools: Arc<AppTools>,
    access: Arc<FakeAccess>,
}

fn rig() -> Rig {
    let fx = Fx::new();
    let access = FakeAccess::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let tools = Arc::new(AppTools::new(
        store,
        Arc::new(TtsHost {
            access: Some(access.clone()),
        }),
    ));
    Rig {
        _fx: fx,
        tools,
        access,
    }
}

fn ctx() -> CallContext {
    CallContext {
        client_id: "C1".to_string(),
        client_label: "Claude Code".to_string(),
        approved: false,
        now_ms: NOW,
    }
}

impl Rig {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.tools.call(&ctx(), tool, &args)
    }
}

// ---------------------------------------------------------------------------
// Katalog und Rechte
// ---------------------------------------------------------------------------

#[test]
fn the_tts_tools_sit_behind_read_and_render_rights() {
    for name in TOOLS {
        let e = catalog::find(name).unwrap_or_else(|| panic!("{name} fehlt im Katalog"));
        let expected = if name == "tts_provider_select" {
            Capability::TtsRender
        } else {
            Capability::TtsRead
        };
        assert_eq!(e.capability, expected, "{name}");
        assert!(Kind::Agent.capabilities().contains(&e.capability), "{name}");
        assert!(!e.capability.never_allow(), "{name}");
        let ro = catalog::annotations(e)["readOnlyHint"].clone();
        assert_eq!(ro, json!(name != "tts_provider_select"), "{name}");
    }
    assert_eq!(Capability::TtsRead.access(), Access::Read);
    assert_eq!(Capability::TtsRender.access(), Access::Write);
    assert_eq!(Capability::parse("tts.read"), Some(Capability::TtsRead));
    assert_eq!(
        serde_json::to_value(Capability::TtsRead).unwrap(),
        json!("tts.read")
    );
}

#[test]
fn the_specs_are_strict_and_offer_no_switch_for_releases() {
    let specs = specs();
    let mut names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    let mut expected = TOOLS.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);
    for s in &specs {
        assert_eq!(
            s.input_schema["additionalProperties"],
            json!(false),
            "{}",
            s.name
        );
        let props = s.input_schema["properties"].as_object().unwrap();
        for key in props.keys() {
            assert!(
                ["id", "voice", "language", "run_dir", "source"].contains(&key.as_str()),
                "{}: unerwartetes Argument {key}",
                s.name
            );
        }
        for bad in ["enable", "allow", "license", "ack", "remote", "opt"] {
            assert!(
                !s.name.contains(bad) && !props.keys().any(|k| k.contains(bad)),
                "{} sähe nach Freigabe aus",
                s.name
            );
        }
    }
    // Alle Werkzeuge gehen auch durch die Registry (Name im Katalog, Recht von der Art angeboten).
    let mut registry = ToolRegistry::new();
    registry.register(rig().tools).unwrap();
    for name in TOOLS {
        assert!(registry.has(name), "{name}");
    }
}

// ---------------------------------------------------------------------------
// Auskunft
// ---------------------------------------------------------------------------

#[test]
fn list_shows_every_provider_with_release_state_and_health() {
    let r = rig();
    let v = r.call("tts_providers_list", json!({})).unwrap();
    let list = v["providers"].as_array().unwrap();
    assert_eq!(list.len(), 6, "fünf Anbieter und der Test-Mock");
    assert_eq!(v["count"], 6);
    assert_eq!(v["selected"], "fish");
    let by = |id: &str| {
        list.iter()
            .find(|p| p["id"] == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    for id in ["fish", "piper", "phonon", "openai-speech", "ollama-speech"] {
        let p = by(id);
        for key in [
            "label",
            "tier",
            "capabilities",
            "license",
            "health",
            "enabled",
            "license_needs_ack",
            "license_acked",
            "allow_remote",
            "blocked",
            "usable",
        ] {
            assert!(p.get(key).is_some(), "{id}: {key} fehlt");
        }
    }
    let phonon = by("phonon");
    assert_eq!(phonon["tier"], "experimental");
    assert_eq!(phonon["enabled"], false);
    assert_eq!(phonon["license_needs_ack"], true);
    assert_eq!(phonon["blocked"]["kind"], "license_blocked");
    assert_eq!(phonon["usable"], false);
    assert_eq!(phonon["health"]["available"], false);
    assert!(phonon["health"]["reason"].is_string());
    // Fish: nie gesperrt, aber hier nicht installiert -> nicht nutzbar, mit Grund.
    let fish = by("fish");
    assert!(fish["blocked"].is_null());
    assert_eq!(fish["usable"], false);
    assert!(fish["health"]["reason"]
        .as_str()
        .unwrap()
        .contains("nicht eingerichtet"));
    assert_eq!(fish["selected"], true);
    assert!(r.call("tts_providers_list", json!({"x": 1})).is_err());
}

#[test]
fn provider_texts_are_scrubbed_before_they_reach_the_agent() {
    // Ein Grund mit Zugangsdaten und Adresse: sie dürfen nicht beim Agenten ankommen.
    let health = ProviderHealth::unavailable(
        "Server https://nutzer:passwort@tts.example.org/v1/geheimer/pfad?token=abc123 antwortet nicht",
        Some("Bitte Adresse https://nutzer:passwort@tts.example.org/v1 prüfen".to_string()),
    );
    let text = health_json(&health).to_string();
    for secret in ["passwort", "abc123", "geheimer"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    assert!(text.contains("antwortet nicht"), "{text}");
    // Lange Texte werden gekürzt.
    let long = ProviderHealth::unavailable("x".repeat(5000), None);
    assert!(health_json(&long).to_string().len() < 1000);
    // Die Sperrmeldung nennt den Freigabeweg.
    let e = TtsError::new(TtsErrorKind::LicenseBlocked, "X ist nicht freigeschaltet.");
    let t = failure_text(&e);
    assert!(
        t.starts_with("Gesperrt (license_blocked)") && t.contains("nur der Nutzer"),
        "{t}"
    );
    assert_eq!(failure_text(&TtsError::backend("kaputt")), "kaputt");
}

#[test]
fn capabilities_and_health_answer_for_one_provider_and_unknown_ids_fail() {
    let r = rig();
    let c = r
        .call("tts_provider_capabilities", json!({"id": "piper"}))
        .unwrap();
    assert_eq!(c["id"], "piper");
    assert_eq!(c["tier"], "supported");
    assert!(c["capabilities"]["languages"].as_array().unwrap().len() >= 1);
    assert!(c["license"]["code"].is_string());
    assert_eq!(c["enabled"], true, "Piper braucht kein Opt-in");
    assert!(c["blocked"].is_null());
    let c = r
        .call("tts_provider_capabilities", json!({"id": "phonon"}))
        .unwrap();
    assert_eq!(c["blocked"]["kind"], "license_blocked");
    assert_eq!(c["enabled"], false);

    let h = r
        .call("tts_provider_health", json!({"id": "phonon"}))
        .unwrap();
    assert_eq!(h["id"], "phonon");
    assert_eq!(h["health"]["available"], false);
    assert!(h["health"]["reason"].is_string());
    let h = r
        .call("tts_provider_health", json!({"id": "mock-tts"}))
        .unwrap();
    assert_eq!(h["health"]["available"], true);
    assert_eq!(h["health"]["version"], "mock-1");

    for tool in ["tts_provider_capabilities", "tts_provider_health"] {
        let e = r.call(tool, json!({"id": "gibt-es-nicht"})).unwrap_err();
        assert!(e.contains("Unbekannter Anbieter"), "{tool}: {e}");
        assert!(
            r.call(tool, json!({})).unwrap_err().contains("fehlt"),
            "{tool}"
        );
        assert!(r.call(tool, json!({"id": "../x"})).is_err(), "{tool}");
        assert!(
            r.call(tool, json!({"id": "piper", "extra": 1})).is_err(),
            "{tool}"
        );
        assert!(r.call(tool, json!({"id": 5})).is_err(), "{tool}");
    }
}

#[test]
fn voices_need_a_released_provider_and_filter_by_language() {
    let r = rig();
    // Gesperrt: der Sperrgrund statt einer Liste.
    let e = r
        .call("tts_provider_voices", json!({"id": "phonon"}))
        .unwrap_err();
    assert!(e.contains("Gesperrt (license_blocked)"), "{e}");
    assert!(e.contains("nur der Nutzer"), "{e}");
    // Freigegeben: die Liste, auf Wunsch gefiltert.
    r.access.enable("mock-tts");
    let v = r
        .call("tts_provider_voices", json!({"id": "mock-tts"}))
        .unwrap();
    assert_eq!(v["count"], 2);
    assert_eq!(v["total"], 2);
    assert_eq!(v["provider"], "mock-tts");
    let v = r
        .call(
            "tts_provider_voices",
            json!({"id": "mock-tts", "language": "de"}),
        )
        .unwrap();
    assert_eq!(v["count"], 1);
    assert_eq!(v["voices"][0]["id"], "mock-de");
    assert_eq!(v["voices"][0]["language"], "de");
    // Eingaben
    for bad in ["d", "de/../x", "de de", "1234", &"x".repeat(13)] {
        assert!(
            r.call(
                "tts_provider_voices",
                json!({"id": "mock-tts", "language": bad})
            )
            .is_err(),
            "{bad}"
        );
    }
    assert!(r
        .call("tts_provider_voices", json!({"language": "de"}))
        .is_err());
    // Piper ist nie gesperrt (ohne Installation: leere Liste).
    let v = r
        .call("tts_provider_voices", json!({"id": "piper"}))
        .unwrap();
    assert_eq!(v["count"], 0);
}

// ---------------------------------------------------------------------------
// Wahl und Sperren
// ---------------------------------------------------------------------------

#[test]
fn select_refuses_a_blocked_provider_and_changes_nothing() {
    let r = rig();
    let before = r.access.providers();
    let engine_before = r.access.engine();
    // Kein Opt-in.
    let e = r
        .call("tts_provider_select", json!({"id": "phonon"}))
        .unwrap_err();
    assert!(e.contains("Gesperrt (license_blocked)"), "{e}");
    assert!(e.contains("nicht freigeschaltet"), "{e}");
    // Mit Opt-in, aber ohne Lizenz-Zustimmung: weiter gesperrt.
    r.access.enable("phonon");
    let e = r
        .call("tts_provider_select", json!({"id": "phonon"}))
        .unwrap_err();
    assert!(e.contains("Gesperrt (license_blocked)"), "{e}");
    assert!(e.contains("Lizenz"), "{e}");
    // Ein unbekannter Anbieter.
    let e = r
        .call("tts_provider_select", json!({"id": "gibt-es-nicht"}))
        .unwrap_err();
    assert!(e.contains("Unbekannter Anbieter"), "{e}");
    assert_eq!(r.access.engine(), engine_before, "die Engine blieb");
    assert_eq!(
        r.access.select_calls.load(Ordering::SeqCst),
        0,
        "die erste Prüfung hielt auf"
    );
    let mut after = r.access.providers();
    after.set_enabled("phonon", false);
    assert_eq!(after, before, "nur der Opt-in des Tests, sonst nichts");
}

#[test]
fn select_sets_engine_and_voice_for_a_released_provider() {
    let r = rig();
    r.access.enable("mock-tts");
    let before = r.access.providers();
    let v = r
        .call(
            "tts_provider_select",
            json!({"id": "mock-tts", "voice": "mock-en"}),
        )
        .unwrap();
    assert_eq!(v["selected"], "mock-tts");
    assert_eq!(v["voice"], "mock-en");
    assert_eq!(v["engine"], "provider:mock-tts");
    assert_eq!(r.access.engine(), "provider:mock-tts");
    let after = r.access.providers();
    assert_eq!(
        after.voices.get("mock-tts").map(String::as_str),
        Some("mock-en")
    );
    // Freigaben: unverändert (nur die Stimme kam hinzu).
    assert_eq!(after.enabled, before.enabled);
    assert_eq!(after.license_acks, before.license_acks);
    assert_eq!(after.allow_remote, before.allow_remote);
    // Zurück zu einem eingebauten Anbieter: `piper` bleibt `piper`.
    r.call("tts_provider_select", json!({"id": "piper"}))
        .unwrap();
    assert_eq!(r.access.engine(), "piper");
    assert_eq!(
        TtsEngineKind::from_setting(&r.access.engine()),
        TtsEngineKind::Piper
    );
}

#[test]
fn select_keeps_the_voice_without_one_and_rejects_unlisted_or_odd_voices() {
    let r = rig();
    r.access.enable("mock-tts");
    r.call(
        "tts_provider_select",
        json!({"id": "mock-tts", "voice": "mock-de"}),
    )
    .unwrap();
    // Ohne Stimme bleibt die bisherige (die Auswahl in der App würde sie zurücksetzen):
    // Piper und Fish tragen ihre Stimme in `tts_piper_voice` bzw. `tts_voice`.
    {
        let mut s = r.access.settings.lock().unwrap();
        s.tts_piper_voice = Some("de_DE-thorsten-high".to_string());
        s.tts_voice = Some("patrick".to_string());
    }
    let v = r
        .call("tts_provider_select", json!({"id": "piper"}))
        .unwrap();
    assert_eq!(v["voice"], "de_DE-thorsten-high");
    assert_eq!(
        r.access.settings.lock().unwrap().tts_piper_voice.as_deref(),
        Some("de_DE-thorsten-high")
    );
    let v = r
        .call("tts_provider_select", json!({"id": "fish"}))
        .unwrap();
    assert_eq!(v["voice"], "patrick");
    assert_eq!(
        r.access.settings.lock().unwrap().tts_voice.as_deref(),
        Some("patrick")
    );
    // Zurück zum Mock für die Prüfungen unten.
    r.call(
        "tts_provider_select",
        json!({"id": "mock-tts", "voice": "mock-de"}),
    )
    .unwrap();
    let calls = r.access.select_calls.load(Ordering::SeqCst);
    // Eine Stimme, die der Anbieter nicht nennt.
    let e = r
        .call(
            "tts_provider_select",
            json!({"id": "mock-tts", "voice": "gibt-es-nicht"}),
        )
        .unwrap_err();
    assert!(e.contains("gibt es bei diesem Anbieter nicht"), "{e}");
    // Gestalt der Kennung: Pfad, Schalter, Steuerzeichen, zu lang.
    for bad in [
        "-x",
        "../etc",
        "a/b",
        "a\\b",
        "C:\\x",
        "mock-de\n--evil",
        "a..b",
        &"x".repeat(101),
    ] {
        assert!(
            r.call(
                "tts_provider_select",
                json!({"id": "mock-tts", "voice": bad})
            )
            .is_err(),
            "{bad:?}"
        );
    }
    assert!(r
        .call("tts_provider_select", json!({"id": "mock-tts", "voice": 7}))
        .is_err());
    assert_eq!(
        r.access.select_calls.load(Ordering::SeqCst),
        calls,
        "keine der abgelehnten Wahlen schrieb"
    );
    assert_eq!(
        r.access
            .providers()
            .voices
            .get("mock-tts")
            .map(String::as_str),
        Some("mock-de")
    );
}

#[test]
fn select_cannot_lift_the_privacy_block() {
    let r = rig();
    {
        let mut s = r.access.settings.lock().unwrap();
        s.tts_providers.openai_speech.base_url = "https://tts.example.org/v1".to_string();
        s.tts_providers.openai_speech.model = "kokoro".to_string();
        s.tts_providers.set_enabled("openai-speech", true);
    }
    let snap = r.access.snapshot().unwrap();
    let desc = snap.descriptor("openai-speech").unwrap();
    let fp = license_fingerprint(&desc, &snap.resolved.subject("openai-speech"));
    r.access
        .settings
        .lock()
        .unwrap()
        .tts_providers
        .record_ack("openai-speech", &fp, "t");
    let before = r.access.providers();
    // Opt-in und Zustimmung liegen vor, der Fernzugriff nicht: gesperrt.
    let e = r
        .call("tts_provider_select", json!({"id": "openai-speech"}))
        .unwrap_err();
    assert!(e.contains("Gesperrt (privacy_blocked)"), "{e}");
    let e = r
        .call("tts_provider_voices", json!({"id": "openai-speech"}))
        .unwrap_err();
    assert!(e.contains("privacy_blocked"), "{e}");
    assert_eq!(r.access.providers(), before);
    assert_eq!(r.access.engine(), get_default_settings().tts_engine);
    // Erst die Erlaubnis des Nutzers (hier: direkt in den Einstellungen) macht es möglich.
    r.access
        .settings
        .lock()
        .unwrap()
        .tts_providers
        .set_remote_allowed("openai-speech", true);
    r.call("tts_provider_select", json!({"id": "openai-speech"}))
        .unwrap();
    assert_eq!(r.access.engine(), "provider:openai-speech");
}

#[test]
fn no_tool_can_change_enabled_acks_or_remote() {
    let r = rig();
    let before = r.access.providers();
    let engine_before = r.access.engine();
    let attacks = [
        json!({}),
        json!({"id": "phonon"}),
        json!({"id": "phonon", "enabled": true}),
        json!({"id": "phonon", "allow_remote": true, "license_ack": true, "opt_in": true}),
        json!({"id": "openai-speech", "voice": "x", "allow_remote": ["openai-speech"]}),
        json!({"id": "ollama-speech", "enabled": ["ollama-speech"], "license_acks": []}),
        json!({"id": "mock-tts", "voice": "mock-de"}),
        json!({"id": ["phonon"]}),
        json!({"id": "phonon", "config": {"enabled": ["phonon"]}}),
        json!({"run_dir": "x", "enabled": true}),
    ];
    for tool in TOOLS {
        for args in &attacks {
            let _ = r.call(tool, args.clone());
        }
    }
    assert_eq!(
        r.access.providers(),
        before,
        "Freigaben und Stimmen unverändert"
    );
    assert_eq!(r.access.engine(), engine_before);
    // Und die Zusatzfelder werden ausdrücklich als „gibt es nicht“ abgelehnt.
    let e = r
        .call(
            "tts_provider_select",
            json!({"id": "phonon", "enabled": true}),
        )
        .unwrap_err();
    assert!(e.contains("gibt es bei diesem Werkzeug nicht"), "{e}");
}

#[test]
fn without_the_host_access_the_tools_say_so() {
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let tools = AppTools::new(store, Arc::new(TtsHost { access: None }));
    for (tool, args) in [
        ("tts_providers_list", json!({})),
        ("tts_provider_capabilities", json!({"id": "piper"})),
        ("tts_provider_voices", json!({"id": "piper"})),
        ("tts_provider_health", json!({"id": "piper"})),
        ("tts_provider_select", json!({"id": "piper"})),
        ("tts_benchmark_results", json!({})),
    ] {
        let e = tools.call(&ctx(), tool, &args).unwrap_err();
        assert!(
            e.contains("in dieser Umgebung nicht verfügbar"),
            "{tool}: {e}"
        );
    }
}

// ---------------------------------------------------------------------------
// tts_render_audio hält die Sperren ein
// ---------------------------------------------------------------------------

/// Host, dessen Audio-Erzeugung denselben Weg nimmt wie `TtsManager::speak_to_file` bei
/// `tts_engine = provider:<id>`: erst `ensure_ready`, dann je Satz `synthesize` — beides durch die
/// Guardrails der `ProviderEngine`.
struct RenderHost {
    engine: ProviderEngine,
    dir: PathBuf,
}

impl Host for RenderHost {
    fn enqueue_import(&self, _t: &str, _p: &str) -> Result<QueuedImport, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn create_page(&self, _t: &str, _x: &str) -> Result<PageRef, String> {
        Err("nicht in diesem Test".to_string())
    }

    fn page_text(&self, _id: &str) -> Result<String, String> {
        Ok("Dies ist ein Satz zum Vorlesen.".to_string())
    }

    fn render_audio(&self, _p: &str, text: &str, file_name: &str) -> Result<Rendered, String> {
        tauri::async_runtime::block_on(async {
            self.engine.ensure_ready().await?;
            let wav = self.engine.synthesize(text, None).await?;
            let path = self.dir.join(format!("{file_name}.wav"));
            std::fs::write(&path, &wav).map_err(|e| e.to_string())?;
            Ok(Rendered {
                file_name: format!("{file_name}.wav"),
                path: path.to_string_lossy().into_owned(),
                bytes: wav.len() as u64,
                format: "wav".to_string(),
            })
        })
    }

    fn recording(&self) -> Option<Arc<dyn RecordingControl>> {
        None
    }

    fn youtube_options(&self) -> AddOptions {
        AddOptions::production()
    }
}

#[test]
fn render_audio_honours_the_locks_of_a_provider_engine() {
    assert_eq!(
        TtsEngineKind::from_setting("provider:zzz"),
        TtsEngineKind::Provider,
        "die Engine-Wahl, um die es hier geht"
    );
    let mock = Arc::new(MockProvider::new().with_id("zzz"));
    let registry = Arc::new(ProviderRegistry::new());
    registry.register(mock.clone()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let call = |enabled: bool| {
        let mut settings = TtsProvidersSettings::default();
        if enabled {
            settings.set_enabled("zzz", true);
        }
        let engine =
            ProviderEngine::new(registry.clone(), "zzz", None, LiveApprovals::new(settings));
        let tools = AppTools::new(
            store.clone(),
            Arc::new(RenderHost {
                engine,
                dir: dir.path().to_path_buf(),
            }),
        );
        tools.call(
            &ctx(),
            "tts_render_audio",
            &json!({"page_id": "page_1", "file_name": "probe"}),
        )
    };
    // Ohne Opt-in: kein Aufruf des Anbieters, keine Datei.
    let e = call(false).unwrap_err();
    assert!(e.contains("nicht freigeschaltet"), "{e}");
    assert_eq!(
        mock.attempts(),
        0,
        "der gesperrte Anbieter wurde nie gefragt"
    );
    assert!(!dir.path().join("probe.wav").exists());
    // Mit Opt-in läuft derselbe Weg.
    let v = call(true).unwrap();
    assert_eq!(v["file"], "probe.wav");
    assert_eq!(mock.attempts(), 1);
    assert!(dir.path().join("probe.wav").exists());
    // Nichtkommerziell ohne Zustimmung: ebenfalls gesperrt (Lizenz-Sperre gilt hier wie überall).
    let nc = Arc::new(NonCommercial(MockProvider::new().with_id("nc")));
    let reg = Arc::new(ProviderRegistry::new());
    reg.register(nc.clone()).unwrap();
    let mut settings = TtsProvidersSettings::default();
    settings.set_enabled("nc", true);
    let engine = ProviderEngine::new(reg, "nc", None, LiveApprovals::new(settings));
    let tools = AppTools::new(
        store,
        Arc::new(RenderHost {
            engine,
            dir: dir.path().to_path_buf(),
        }),
    );
    let e = tools
        .call(
            &ctx(),
            "tts_render_audio",
            &json!({"page_id": "page_1", "file_name": "nc"}),
        )
        .unwrap_err();
    assert!(e.contains("Lizenz"), "{e}");
    assert_eq!(nc.0.attempts(), 0);
}

/// Anbieter mit nichtkommerzieller Gewichts-Lizenz.
struct NonCommercial(MockProvider);

impl crate::managers::tts::providers::contract::TtsProvider for NonCommercial {
    fn descriptor(&self) -> crate::managers::tts::providers::contract::ProviderDescriptor {
        let mut d = self.0.descriptor();
        d.tier = ProviderTier::Experimental;
        d.license.non_commercial = true;
        d.license.verified = false;
        d
    }
    fn health_check(
        &self,
    ) -> futures_util::future::BoxFuture<
        '_,
        crate::managers::tts::providers::contract::ProviderHealth,
    > {
        self.0.health_check()
    }
    fn list_voices<'a>(
        &'a self,
        language: Option<&'a str>,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<Vec<crate::managers::tts::providers::contract::VoiceDescriptor>, TtsError>,
    > {
        self.0.list_voices(language)
    }
    fn synthesize<'a>(
        &'a self,
        text: &'a str,
        opts: &'a SynthesisOptions,
        cancel: &'a crate::managers::tts::providers::contract::CancelToken,
    ) -> futures_util::future::BoxFuture<
        'a,
        Result<crate::managers::tts::providers::contract::AudioClip, TtsError>,
    > {
        self.0.synthesize(text, opts, cancel)
    }
    fn cache_tag(&self, voice: Option<&str>) -> String {
        self.0.cache_tag(voice)
    }
}

// ---------------------------------------------------------------------------
// Benchmark-Ergebnisse
// ---------------------------------------------------------------------------

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Ordner mit zwei veröffentlichten und einem lokalen Lauf.
fn roots_fixture() -> (tempfile::TempDir, BenchRoots) {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    let bench = dir.path().join("bench");
    write(
        &docs.join("2026-10-27/results.json"),
        r#"{"configs":[{"id":"piper"}]}"#,
    );
    write(&docs.join("2026-10-20/results.json"), r#"{"configs":[]}"#);
    write(&docs.join("2026-10-20/report.md"), "# Bericht");
    write(&docs.join("ohne-ergebnis/readme.txt"), "x");
    write(
        &bench.join("20261026-120000-app/report/results.json"),
        r#"{"configs":[{"id":"fish"}]}"#,
    );
    write(&bench.join("20261026-120000-app/rows.jsonl"), "{}");
    (
        dir,
        BenchRoots {
            bench: Some(bench),
            docs: Some(docs),
        },
    )
}

#[test]
fn benchmark_results_list_and_read() {
    let (_dir, roots) = roots_fixture();
    let r = rig();
    r.access.set_roots(roots);
    let v = r.call("tts_benchmark_results", json!({})).unwrap();
    assert_eq!(v["count"], 3);
    let runs = v["runs"].as_array().unwrap();
    // Veröffentlichte zuerst, je Quelle die neueste Kennung zuerst; Ordner ohne results.json fehlen.
    let ids: Vec<&str> = runs
        .iter()
        .map(|x| x["run_dir"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["2026-10-27", "2026-10-20", "20261026-120000-app"]);
    assert_eq!(runs[0]["source"], "docs");
    assert_eq!(runs[2]["source"], "bench");
    assert!(runs[0]["bytes"].as_u64().unwrap() > 0);
    assert!(runs[0]["modified"].as_str().unwrap().ends_with('Z'));
    assert_eq!(v["roots"]["docs"], true);
    let text = v.to_string();
    assert!(
        !text.contains("bench\\") && !text.contains("docs\\") && !text.contains("Temp"),
        "keine Pfade: {text}"
    );

    let one = r
        .call("tts_benchmark_results", json!({"run_dir": "2026-10-27"}))
        .unwrap();
    assert_eq!(one["source"], "docs");
    assert_eq!(one["results"]["configs"][0]["id"], "piper");
    let local = r
        .call(
            "tts_benchmark_results",
            json!({"run_dir": "20261026-120000-app"}),
        )
        .unwrap();
    assert_eq!(local["source"], "bench");
    assert_eq!(local["results"]["configs"][0]["id"], "fish");
    let docs_only = r
        .call(
            "tts_benchmark_results",
            json!({"run_dir": "20261026-120000-app", "source": "docs"}),
        )
        .unwrap_err();
    assert!(docs_only.contains("keine results.json"), "{docs_only}");
    assert!(r
        .call(
            "tts_benchmark_results",
            json!({"run_dir": "2026-10-27", "source": "etc"})
        )
        .is_err());
    assert!(r
        .call("tts_benchmark_results", json!({"run_dir": "gibt-es-nicht"}))
        .unwrap_err()
        .contains("keine results.json"));
    // Nichts eingerichtet: leere Liste, kein Fehler.
    r.access.set_roots(BenchRoots::default());
    let v = r.call("tts_benchmark_results", json!({})).unwrap();
    assert_eq!(v["count"], 0);
    assert_eq!(v["roots"]["docs"], false);
}

#[test]
fn benchmark_results_refuse_path_traversal() {
    let (dir, roots) = roots_fixture();
    // Ein Geheimnis NEBEN den Ergebnisordnern.
    write(
        &dir.path().join("geheim/results.json"),
        r#"{"geheim":true}"#,
    );
    write(&dir.path().join("docs/geheim.json"), r#"{"geheim":true}"#);
    let r = rig();
    r.access.set_roots(roots);
    for bad in [
        "..",
        "../geheim",
        "..\\geheim",
        "2026-10-27/..",
        "2026-10-27/../2026-10-20",
        "a/b",
        "a\\b",
        "C:\\Windows",
        "C:",
        "/etc/passwd",
        "\\\\server\\share",
        ".versteckt",
        "a..b",
        "NUL",
        "con.txt",
        "",
        " ",
        "2026-10-27:strom",
        &"x".repeat(65),
    ] {
        let e = r.call("tts_benchmark_results", json!({"run_dir": bad}));
        match e {
            Err(m) => assert!(
                m.contains("keine gültige Kennung") || m.contains("fehlt") || m.contains("zu lang"),
                "{bad:?}: {m}"
            ),
            Ok(v) => assert!(
                bad.trim().is_empty() && v.get("runs").is_some(),
                "{bad:?} lieferte {v}"
            ),
        }
        assert!(!r.access.roots.lock().unwrap().docs.is_none());
    }
    // Direkt gegen die reine Funktion (ohne Bereinigung der Argumente).
    let roots = r.access.benchmark_roots();
    for bad in [
        "../geheim",
        "..\\geheim",
        "docs/../geheim",
        "%2e%2e/geheim",
        "2026-10-27\0",
    ] {
        assert!(benchmark_results(&roots, Some(bad), None).is_err(), "{bad}");
    }
    // Über das Werkzeug fallen Steuerzeichen schon bei der Bereinigung der Argumente weg.
    assert!(r
        .call(
            "tts_benchmark_results",
            json!({"run_dir": "2026-10-27\u{0}"})
        )
        .is_ok());
    assert!(!valid_run_id("../x") && !valid_run_id("a/b") && valid_run_id("2026-10-27"));
}

#[test]
fn benchmark_results_refuse_a_link_that_leaves_the_folder() {
    let (dir, roots) = roots_fixture();
    let outside = dir.path().join("draussen");
    write(&outside.join("results.json"), r#"{"geheim":true}"#);
    let docs = roots.docs.clone().unwrap();
    let link = docs.join("verknuepft");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&outside, &link);
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(&outside, &link);
    if made.is_err() {
        // Ohne Recht für Verknüpfungen (Windows ohne Entwicklermodus) lässt sich der Fall nicht bauen.
        eprintln!("Verknüpfung nicht anlegbar - Teil übersprungen");
        return;
    }
    let r = rig();
    r.access.set_roots(roots.clone());
    let e = r
        .call("tts_benchmark_results", json!({"run_dir": "verknuepft"}))
        .unwrap_err();
    assert!(e.contains("außerhalb"), "{e}");
    // In der Liste taucht die Verknüpfung nicht auf.
    let v = r.call("tts_benchmark_results", json!({})).unwrap();
    assert!(!v.to_string().contains("verknuepft"));
}

#[test]
fn benchmark_results_bound_the_size_and_check_the_content() {
    let (_dir, roots) = roots_fixture();
    let docs = roots.docs.clone().unwrap();
    // Genau an der Grenze: geht; ein Byte mehr: Fehler.
    let pad = |n: usize| format!("{{\"x\":\"{}\"}}", "a".repeat(n - 8));
    write(
        &docs.join("grenze/results.json"),
        &pad(MAX_RESULTS_BYTES as usize),
    );
    write(
        &docs.join("zugross/results.json"),
        &pad(MAX_RESULTS_BYTES as usize + 1),
    );
    write(&docs.join("kaputt/results.json"), "{ das ist kein json");
    let r = rig();
    r.access.set_roots(roots);
    let v = r
        .call("tts_benchmark_results", json!({"run_dir": "grenze"}))
        .unwrap();
    assert_eq!(v["bytes"], MAX_RESULTS_BYTES);
    let e = r
        .call("tts_benchmark_results", json!({"run_dir": "zugross"}))
        .unwrap_err();
    assert!(e.contains("zu groß"), "{e}");
    let e = r
        .call("tts_benchmark_results", json!({"run_dir": "kaputt"}))
        .unwrap_err();
    assert!(e.contains("kein gültiges JSON"), "{e}");
    let list = r.call("tts_benchmark_results", json!({})).unwrap();
    let too_big = list["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["run_dir"] == "zugross")
        .unwrap();
    assert_eq!(too_big["readable"], false);
}

#[test]
fn bench_roots_follow_the_environment_then_the_program_path() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    };
    let roots = bench_roots_from(&env(&[("LOCALAPPDATA", "C:/Users/x/AppData/Local")]), None);
    assert_eq!(
        roots.bench,
        Some(PathBuf::from("C:/Users/x/AppData/Local/lva-bench/tts"))
    );
    assert_eq!(roots.docs, None);
    let roots = bench_roots_from(
        &env(&[
            ("LOCALAPPDATA", "C:/Users/x/AppData/Local"),
            ("LVA_BENCH_DIR", "D:/bench"),
            ("LVA_TTS_BENCH_DOCS", "D:/repo/docs/tts/benchmarks"),
        ]),
        None,
    );
    assert_eq!(roots.bench, Some(PathBuf::from("D:/bench/tts")));
    assert_eq!(
        roots.docs,
        Some(PathBuf::from("D:/repo/docs/tts/benchmarks"))
    );
    // Leere Werte zählen nicht.
    let roots = bench_roots_from(
        &env(&[("LVA_BENCH_DIR", "  "), ("LOCALAPPDATA", "E:/L")]),
        None,
    );
    assert_eq!(roots.bench, Some(PathBuf::from("E:/L/lva-bench/tts")));
    // Repo-Build: `docs/tts/benchmarks` oberhalb des Programms.
    let dir = tempfile::tempdir().unwrap();
    let benchmarks = dir.path().join("docs/tts/benchmarks");
    std::fs::create_dir_all(&benchmarks).unwrap();
    let exe = dir
        .path()
        .join("apps/local-voice/src-tauri/target/debug/local-voice-ai.exe");
    let roots = bench_roots_from(&env(&[]), Some(&exe));
    assert_eq!(roots.docs, Some(benchmarks));
    assert_eq!(roots.bench, None);
}

/// Befund 9: eine installierte App sucht NICHT oberhalb ihres Programms nach
/// `docs/tts/benchmarks` (sonst fände sie `C:\docs\tts\benchmarks` eines
/// beliebigen Ordners, auf den sie dann liest). Die Suche gibt es nur im
/// Entwicklungsbau; ausdrücklich gesetzte Umgebungsvariablen gelten immer.
#[test]
fn an_installed_app_does_not_search_above_its_program_for_docs() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    };
    let dir = tempfile::tempdir().unwrap();
    // Ein fremder `docs/tts/benchmarks`-Ordner oberhalb der Installation.
    let foreign = dir.path().join("docs/tts/benchmarks");
    std::fs::create_dir_all(&foreign).unwrap();
    let exe = dir.path().join("Programme/LocalVoice/local-voice-ai.exe");

    let installed = bench_roots_with(&env(&[]), Some(&exe), false);
    assert_eq!(installed.docs, None, "kein Suchen oberhalb des Programms");
    assert_eq!(installed.bench, None);

    // Im Entwicklungsbau (Repo): dieselbe Anordnung wird gefunden.
    let dev = bench_roots_with(&env(&[]), Some(&exe), true);
    assert_eq!(dev.docs, Some(foreign.clone()));

    // Der ausdrückliche Weg gilt in beiden Fällen.
    let explicit = bench_roots_with(
        &env(&[("LVA_TTS_BENCH_DOCS", "D:/repo/docs/tts/benchmarks")]),
        Some(&exe),
        false,
    );
    assert_eq!(
        explicit.docs,
        Some(PathBuf::from("D:/repo/docs/tts/benchmarks"))
    );

    // Der Standardweg folgt dem Bautyp: Entwicklungsbau sucht, Release nie.
    let default_docs = bench_roots_from(&env(&[]), Some(&exe)).docs;
    assert_eq!(default_docs.is_some(), cfg!(debug_assertions));
}

// ---------------------------------------------------------------------------
// Über die echte Brücke: Rechte
// ---------------------------------------------------------------------------

struct BridgeRig {
    fx: Fx,
    bridge: Bridge,
    client: ClientCtx,
    access: Arc<FakeAccess>,
}

fn bridge_rig() -> BridgeRig {
    let fx = Fx::new();
    let store = Arc::new(MeetingStore::open_at(&fx.db_path).unwrap());
    let access = FakeAccess::new();
    let tools = Arc::new(AppTools::new(
        store.clone(),
        Arc::new(TtsHost {
            access: Some(access.clone()),
        }),
    ));
    let mut registry = ToolRegistry::new();
    registry.register(tools).unwrap();
    let bridge = Bridge::new(store, registry, fast_config()).with_clock(Arc::new(|| NOW));
    let (client, _) = clients::create(&fx.conn(), "Claude Code", None, NOW).unwrap();
    BridgeRig {
        fx,
        bridge,
        client: ClientCtx::from(&client),
        access,
    }
}

impl BridgeRig {
    fn grant(&self, tool: &str, mode: GrantMode) {
        let conn = self.fx.conn();
        let entry = catalog::find(tool).unwrap();
        register::set_grant(
            &conn,
            "agents",
            entry.capability,
            Caller::AgentExternal,
            GrantMode::Allow,
        )
        .unwrap();
        clients::set_tool_mode(&conn, &self.client.id, tool, mode).unwrap();
    }

    fn call(&self, tool: &str, args: Value) -> Result<CallStep, BridgeError> {
        self.bridge.call(&self.client, tool, &args, None)
    }

    fn names(&self) -> Vec<String> {
        self.bridge
            .tools_list(&self.client)
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }
}

#[test]
fn through_the_bridge_the_tts_tools_are_off_until_granted_and_read_does_not_grant_write() {
    let r = bridge_rig();
    // Ein neuer Zugang: keines der Werkzeuge ist sichtbar oder aufrufbar.
    for name in TOOLS {
        assert!(!r.names().contains(&name.to_string()), "{name}");
        let e = r.call(name, json!({})).unwrap_err();
        assert_eq!(e.code, code::TOOL_OFF, "{name}");
    }
    // Lesen freigegeben: Auskunft geht, Wählen bleibt aus (anderes Recht).
    for name in [
        "tts_providers_list",
        "tts_provider_health",
        "tts_benchmark_results",
    ] {
        r.grant(name, GrantMode::Allow);
    }
    let CallStep::Done(v) = r.call("tts_providers_list", json!({})).unwrap() else {
        panic!("erwartet: Done");
    };
    assert_eq!(v["count"], 6);
    assert!(r.names().contains(&"tts_provider_health".to_string()));
    let e = r
        .call("tts_provider_select", json!({"id": "piper"}))
        .unwrap_err();
    assert_eq!(e.code, code::TOOL_OFF);
    assert_eq!(r.access.engine(), "fish");
    // Wählen freigegeben, aber der Anbieter ist gesperrt: Fehler mit Grund, nichts ändert sich.
    r.grant("tts_provider_select", GrantMode::Allow);
    let before = r.access.providers();
    let e = r
        .call("tts_provider_select", json!({"id": "phonon"}))
        .unwrap_err();
    assert_eq!(e.code, code::FAILED);
    assert!(e.message.contains("license_blocked"), "{}", e.message);
    assert_eq!(r.access.providers(), before);
    assert_eq!(r.access.engine(), "fish");
    // Ein freigegebener Anbieter lässt sich wählen — und die Freigaben bleiben, wie sie waren.
    let CallStep::Done(v) = r
        .call("tts_provider_select", json!({"id": "piper"}))
        .unwrap()
    else {
        panic!("erwartet: Done");
    };
    assert_eq!(v["engine"], "piper");
    assert_eq!(r.access.providers().enabled, before.enabled);
    assert_eq!(r.access.providers().allow_remote, before.allow_remote);
    // Das Recht auf „aus“ zurücksetzen: weg.
    r.grant("tts_provider_select", GrantMode::Off);
    assert_eq!(
        r.call("tts_provider_select", json!({"id": "fish"}))
            .unwrap_err()
            .code,
        code::TOOL_OFF
    );
    assert_eq!(r.access.engine(), "piper");
}
