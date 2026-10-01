use serde_json::Value;

use super::*;
use crate::managers::workflows::{catalog, templates};

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/managers/workflows/schema/lva-workflow-1.schema.json")
}

#[test]
fn the_checked_in_schema_file_equals_the_generated_one() {
    let generated = serde_json::to_string_pretty(&json_schema()).unwrap() + "\n";
    if std::env::var("LVA_UPDATE_SCHEMA").is_ok() {
        std::fs::write(golden_path(), &generated).unwrap();
    }
    let on_disk = std::fs::read_to_string(golden_path()).expect(
        "schema/lva-workflow-1.schema.json fehlt: LVA_UPDATE_SCHEMA=1 setzen und neu testen",
    );
    assert_eq!(
        on_disk.replace("\r\n", "\n"),
        generated,
        "Das Schema weicht ab: LVA_UPDATE_SCHEMA=1 cargo test --lib workflows::jsonschema"
    );
}

#[test]
fn the_schema_names_every_trigger_and_every_action_of_the_catalog() {
    let schema = json_schema();
    let triggers: Vec<String> = schema["properties"]["trigger"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b["properties"]["type"]["const"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    let expected: Vec<String> = catalog::triggers()
        .iter()
        .map(|t| t.id.to_string())
        .collect();
    assert_eq!(triggers, expected);

    let actions: Vec<String> = schema["properties"]["steps"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b["properties"]["action"]["const"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    let expected: Vec<String> = catalog::actions()
        .iter()
        .map(|a| a.id.to_string())
        .collect();
    assert_eq!(actions, expected);
    assert_eq!(schema["properties"]["schema"]["const"], "lva-workflow@1");
    assert_eq!(schema["additionalProperties"], false);
}

#[test]
fn required_fields_and_literal_fields_follow_the_catalog() {
    let schema = json_schema();
    let mail = schema["properties"]["steps"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["properties"]["action"]["const"] == "mail.send")
        .unwrap();
    let params = &mail["properties"]["params"];
    let required: Vec<&str> = params["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, vec!["via", "to", "subject"]);
    // Der Empfaenger ist eine feste Auswahl OHNE Vorlagen-Ausweg.
    assert_eq!(
        params["properties"]["to"],
        serde_json::json!({"enum": ["me", "participants", "all", "list"]})
    );
    assert_eq!(params["additionalProperties"], false);
    // Eine nicht feste Zahl darf dagegen eine Vorlage sein.
    let wait = schema["properties"]["steps"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["properties"]["action"]["const"] == "wait")
        .unwrap();
    assert!(wait["properties"]["params"]["properties"]["minutes"]["anyOf"].is_array());
}

#[test]
fn every_shipped_template_has_only_fields_the_schema_knows() {
    // Kein Schema-Validator im Baum: Die Namen der Felder jedes Schritts muessen im Zweig
    // seines Bausteins stehen, die Felder des Ausloesers im Zweig seiner Art.
    let schema = json_schema();
    for (id, text) in templates::all() {
        let v: Value = serde_json::from_str(text).unwrap();
        let trig = v["trigger"]["type"].as_str().unwrap();
        let branch = schema["properties"]["trigger"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["properties"]["type"]["const"] == trig)
            .unwrap_or_else(|| panic!("{id}: Ausloeser {trig} fehlt im Schema"));
        for key in v["trigger"].as_object().unwrap().keys() {
            assert!(
                branch["properties"].get(key).is_some(),
                "{id}: Ausloeserfeld {key} fehlt im Schema"
            );
        }
        for step in v["steps"].as_array().unwrap() {
            let action = step["action"].as_str().unwrap();
            let branch = schema["properties"]["steps"]["items"]["oneOf"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["properties"]["action"]["const"] == action)
                .unwrap();
            for key in step["params"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.keys())
            {
                assert!(
                    branch["properties"]["params"]["properties"]
                        .get(key)
                        .is_some(),
                    "{id}: Parameter {key} von {action} fehlt im Schema"
                );
            }
        }
    }
}
