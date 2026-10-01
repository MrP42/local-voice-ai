use serde_json::{json, Value};

use super::*;
use crate::managers::workflows::templates;

fn base() -> Value {
    json!({
        "schema": "lva-workflow@1",
        "name": "Test",
        "trigger": {"type": "manual"},
        "steps": [
            {"id": "a", "action": "notify.local", "params": {"title": "Hallo"}}
        ]
    })
}

fn issues(v: &Value) -> Vec<Issue> {
    check(v)
}

fn has(issues: &[Issue], path: &str, needle: &str) -> bool {
    issues
        .iter()
        .any(|i| i.path == path && i.message.contains(needle))
}

#[test]
fn every_shipped_template_is_valid() {
    for (id, text) in templates::all() {
        let r = parse_definition_str(text);
        assert!(r.is_ok(), "Vorlage {id}: {:?}", r.err());
    }
    let def = parse_definition_str(templates::MEETING_SAMPLE).unwrap();
    assert_eq!(def.steps.len(), 5);
    assert!(def.name.contains("Termin"));
}

#[test]
fn a_minimal_definition_is_valid_and_round_trips() {
    let v = base();
    let def = parse_definition(&v).unwrap();
    assert_eq!(def.schema, "lva-workflow@1");
    // Typ -> JSON -> Typ ergibt dieselbe Definition (Export/Import, AK9).
    let again = parse_definition(&serde_json::to_value(&def).unwrap()).unwrap();
    assert_eq!(def, again);
}

#[test]
fn schema_name_and_unknown_top_level_keys_are_reported_with_paths() {
    let v = json!({"schema": "lva-workflow@2", "name": "  ", "extra": 1,
                   "trigger": {"type": "manual"}, "steps": [{"id": "a", "action": "wait", "params": {"minutes": 1}}]});
    let found = issues(&v);
    assert!(has(&found, "/schema", "lva-workflow@1"), "{found:?}");
    assert!(has(&found, "/name", "leer"), "{found:?}");
    assert!(has(&found, "/extra", "Unbekanntes Feld"), "{found:?}");
    // Kein JSON-Objekt, kein Text.
    assert_eq!(issues(&json!([1, 2])).len(), 1);
    assert!(parse_definition_str("{kaputt").unwrap_err()[0]
        .message
        .contains("Zeile"));
}

#[test]
fn trigger_type_fields_and_required_values_are_checked_against_the_catalog() {
    let mut v = base();
    v["trigger"] = json!({"type": "ordner.neu"});
    assert!(has(&issues(&v), "/trigger/type", "Unbekannter Auslöser"));

    v["trigger"] = json!({"type": "calendar.event_starting", "lead_min": 500, "foo": true});
    let found = issues(&v);
    assert!(
        has(&found, "/trigger/integration", "Pflichtfeld"),
        "{found:?}"
    );
    assert!(
        has(&found, "/trigger/lead_min", "zwischen 0 und 120"),
        "{found:?}"
    );
    assert!(has(&found, "/trigger/foo", "Unbekanntes Feld"), "{found:?}");

    // Ein Ausloeser ist feste Konfiguration, keine Vorlage.
    v["trigger"] = json!({"type": "calendar.event_starting", "integration": "{{vars.x}}"});
    assert!(has(&issues(&v), "/trigger/integration", "Kennung erwartet"));
}

#[test]
fn step_names_must_be_unique_and_well_formed() {
    let mut v = base();
    v["steps"] = json!([
        {"id": "a", "action": "notify.local", "params": {"title": "x"}},
        {"id": "a", "action": "notify.local", "params": {"title": "y"}},
        {"id": "Gross", "action": "notify.local", "params": {"title": "z"}},
        {"action": "notify.local", "params": {"title": "z"}}
    ]);
    let found = issues(&v);
    assert!(has(&found, "/steps/1/id", "mehrfach"), "{found:?}");
    assert!(has(&found, "/steps/2/id", "Kleinbuchstaben"), "{found:?}");
    assert!(has(&found, "/steps/3/id", "Pflichtfeld"), "{found:?}");

    v["steps"] = json!([]);
    assert!(has(&issues(&v), "/steps", "mindestens einen"));
    let many: Vec<Value> = (0..51)
        .map(|i| json!({"id": format!("s{i}"), "action": "notify.local", "params": {"title": "x"}}))
        .collect();
    v["steps"] = Value::Array(many);
    assert!(has(&issues(&v), "/steps", "Zu viele Schritte"));
}

#[test]
fn actions_and_their_parameters_are_checked_against_the_catalog() {
    let mut v = base();
    v["steps"] = json!([
        {"id": "a", "action": "dateien.loeschen", "params": {}},
        {"id": "b", "action": "mail.send", "params": {"to": "irgendwer", "subject": 5, "zusatz": 1}},
        {"id": "c", "action": "wait", "params": {"minutes": 0}}
    ]);
    let found = issues(&v);
    assert!(
        has(&found, "/steps/0/action", "Unbekannter Baustein"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/1/params/via", "Pflichtparameter"),
        "{found:?}"
    );
    assert!(has(&found, "/steps/1/params/to", "eines von"), "{found:?}");
    assert!(
        has(&found, "/steps/1/params/subject", "Text erwartet"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/1/params/zusatz", "Unbekannter Parameter"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/2/params/minutes", "zwischen 1 und 10080"),
        "{found:?}"
    );
}

#[test]
fn recipients_can_only_come_from_the_rule_never_from_data() {
    let mut v = base();
    v["steps"] = json!([{
        "id": "m", "action": "mail.send",
        "params": {"via": "m365-1", "to": "{{trigger.attendees}}", "subject": "x"}
    }]);
    let found = issues(&v);
    assert!(
        has(&found, "/steps/0/params/to", "nur feste Werte"),
        "{found:?}"
    );

    v["steps"] = json!([{
        "id": "m", "action": "mail.send",
        "params": {"via": "m365-1", "to": "list", "list": ["{{vars.adresse}}"], "subject": "x"}
    }]);
    let found = issues(&v);
    assert!(
        has(&found, "/steps/0/params/list", "nur feste Werte"),
        "{found:?}"
    );

    // Ein Webhook-Ziel ist ebenfalls fest.
    v["steps"] = json!([{
        "id": "w", "action": "webhook.post", "params": {"url": "http://127.0.0.1:5678/{{vars.pfad}}"}
    }]);
    assert!(has(&issues(&v), "/steps/0/params/url", "nur feste Werte"));
}

#[test]
fn references_must_point_to_something_that_can_exist() {
    let mut v = base();
    v["variables"] = json!({"x": {"type": "string", "default": "a"}});
    v["steps"] = json!([
        {"id": "a", "action": "notify.local", "params": {"title": "{{steps.b.out}}"}},
        {"id": "b", "action": "notify.local", "params": {"title": "{{steps.a.ok}} {{steps.b.ok}}"}},
        {"id": "c", "action": "notify.local", "params": {"title": "{{steps.nix.x}}"}},
        {"id": "d", "action": "notify.local", "params": {"title": "{{vars.y}} {{vars.x}}"}},
        {"id": "e", "action": "notify.local", "params": {"title": "{{secrets.token}}"}},
        {"id": "f", "action": "notify.local", "when": "steps.f.ok", "params": {"title": "x"}},
        {"id": "g", "action": "notify.local", "params": {"title": "{{trigger.anything}}"}},
        {"id": "h", "action": "notify.local", "params": {"title": "{{run.geheim}} {{workflow.name}}"}}
    ]);
    let found = issues(&v);
    assert!(
        has(&found, "/steps/0/params", "läuft erst später"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/1/params", "dieser Schritt selbst"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/2/params", "keinen Schritt „nix“"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/3/params", "„y“ ist nicht deklariert"),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|i| i.message.contains("„x“")),
        "x ist deklariert"
    );
    assert!(
        has(&found, "/steps/4/params", "Unbekannter Bereich „secrets“"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/5/when", "dieser Schritt selbst"),
        "{found:?}"
    );
    // `trigger.<feld>` bei einem freien Ausloeser (manual) ist erlaubt.
    assert!(!has(&found, "/steps/6/params", "trigger"), "{found:?}");
    assert!(
        has(&found, "/steps/7/params", "„run“ kennt nur"),
        "{found:?}"
    );

    // Ein Ausloeser mit festem Feldsatz: ein Tippfehler faellt auf.
    let mut v2 = base();
    v2["trigger"] = json!({"type": "calendar.event_starting", "integration": "cal-1"});
    v2["steps"] =
        json!([{"id": "a", "action": "notify.local", "params": {"title": "{{trigger.titel}}"}}]);
    assert!(has(
        &issues(&v2),
        "/steps/0/params",
        "liefert kein Feld „titel“"
    ));
}

#[test]
fn conditions_and_templates_must_parse() {
    let mut v = base();
    v["steps"] = json!([
        {"id": "a", "action": "notify.local", "when": "trigger.x ==", "params": {"title": "x"}},
        {"id": "b", "action": "notify.local", "params": {"title": "{{vars.}}"}},
        {"id": "c", "action": "notify.local", "params": {"title": "{{ system('rm') }}"}}
    ]);
    let found = issues(&v);
    assert!(
        has(&found, "/steps/0/when", "Ungültiger Ausdruck"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/1/params/title", "Ungültiger Ausdruck"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/2/params/title", "unbekannte Funktion"),
        "{found:?}"
    );
}

#[test]
fn variables_retry_and_on_error_are_checked() {
    let mut v = base();
    v["variables"] = json!({
        "Gross": {"type": "string"},
        "n": {"type": "number", "default": "text"},
        "t": {"type": "datum"},
        "ok": {"type": "list", "default": ["a", 1, true]}
    });
    v["steps"] = json!([{
        "id": "a", "action": "notify.local", "params": {"title": "x"},
        "on_error": "ignore", "retry": {"max_attempts": 99, "backoff_ms": 5, "x": 1}
    }]);
    let found = issues(&v);
    assert!(
        has(&found, "/variables/Gross", "Kleinbuchstaben"),
        "{found:?}"
    );
    assert!(
        has(&found, "/variables/n/default", "passt nicht zur Art"),
        "{found:?}"
    );
    assert!(
        has(
            &found,
            "/variables/t/type",
            "string, number, bool oder list"
        ),
        "{found:?}"
    );
    assert!(
        !found.iter().any(|i| i.path.starts_with("/variables/ok")),
        "{found:?}"
    );
    assert!(has(&found, "/steps/0/on_error", "fail"), "{found:?}");
    assert!(
        has(&found, "/steps/0/retry/max_attempts", "1 bis 10"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/0/retry/backoff_ms", "100 bis"),
        "{found:?}"
    );
    assert!(
        has(&found, "/steps/0/retry/x", "Unbekanntes Feld"),
        "{found:?}"
    );
}

#[test]
fn size_limits_stop_an_oversized_definition() {
    let mut v = base();
    let big = "x".repeat(70 * 1024);
    v["steps"][0]["params"]["body"] = json!(big);
    let found = issues(&v);
    assert!(
        found
            .iter()
            .any(|i| i.message.contains("zu groß") || i.message.contains("Zu groß")),
        "{found:?}"
    );
    let mut huge = base();
    huge["description"] = json!("y".repeat(300 * 1024));
    assert!(issues(&huge)[0].message.contains("zu groß"));
}
