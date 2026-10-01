use serde_json::json;

use super::*;

fn show(target: Option<&str>, args: serde_json::Value) -> String {
    build(target, Some(&args)).unwrap()
}

#[test]
fn the_target_comes_first_and_safety_fields_follow_before_everything_else() {
    let p = show(
        Some("Angebot.docx"),
        json!({ "body": "Text", "subject": "Betreff", "to": "a@example.invalid", "zzz": 1 }),
    );
    let lines: Vec<&str> = p.lines().collect();
    assert_eq!(lines[0], "Ziel: Angebot.docx");
    assert_eq!(lines[1], "• to: a@example.invalid");
    // Danach die uebrigen Felder in fester Reihenfolge.
    assert_eq!(lines[2], "• body: Text");
    assert!(p.contains("• subject: Betreff"));
    assert!(p.contains("• zzz: 1"));
}

#[test]
fn a_long_body_is_clipped_with_a_visible_marker_and_the_recipients_stay_complete() {
    let long = "Text ".repeat(2_000);
    let p = show(
        Some("Mail"),
        json!({
            "body": long,
            "to": ["a@example.invalid", "b@example.invalid"],
            "cc": "c@example.invalid",
            "bcc": "d@example.invalid",
            "attachments": [{ "path": "C:/Daten/Bilanz.xlsx" }],
        }),
    );
    for must in [
        "a@example.invalid",
        "b@example.invalid",
        "c@example.invalid",
        "d@example.invalid",
        "C:/Daten/Bilanz.xlsx",
    ] {
        assert!(p.contains(must), "{must} fehlt:\n{p}");
    }
    assert!(p.contains("gekürzt, insgesamt 10000 Zeichen"), "{p}");
    assert!(p.chars().count() <= 4_000);
    // Die Sicherheitsfelder stehen vor dem Text, nicht dahinter.
    assert!(p.find("d@example.invalid").unwrap() < p.find("• body").unwrap());
}

#[test]
fn safety_fields_are_found_in_nested_objects_and_arrays_by_any_key_in_the_path() {
    let p = show(
        None,
        json!({
            "message": { "to": ["x@example.invalid"], "text": "Hallo" },
            "Empfänger": "y@example.invalid",
            "files": [{ "Dateiname": "z.txt" }],
            "target_url": "https://h.example/a",
        }),
    );
    let safety_end = p.find("• message.text").unwrap();
    for must in [
        "x@example.invalid",
        "y@example.invalid",
        "z.txt",
        "https://h.example/a",
    ] {
        let at = p.find(must).unwrap_or_else(|| panic!("{must} fehlt:\n{p}"));
        assert!(
            at < safety_end,
            "{must} gehoert vor die uebrigen Felder:\n{p}"
        );
    }
}

#[test]
fn a_safety_field_that_cannot_be_shown_in_full_refuses_the_request() {
    let too_long = "a".repeat(MAX_SAFETY_FIELD_CHARS + 1);
    let err = build(None, Some(&json!({ "to": too_long }))).unwrap_err();
    assert!(matches!(err, PreviewError::FieldTooLong { .. }), "{err:?}");
    // Genau an der Grenze geht es noch.
    let exact = "a".repeat(MAX_SAFETY_FIELD_CHARS);
    assert!(build(None, Some(&json!({ "to": exact }))).is_ok());
    // Zu viele Empfaenger sprengen das Gesamtbudget der Sicherheitsfelder.
    let many: Vec<String> = (0..80)
        .map(|i| format!("r{i:03}@example.invalid"))
        .collect();
    assert_eq!(
        build(None, Some(&json!({ "bcc": many }))).unwrap_err(),
        PreviewError::SafetyFieldsTooLarge
    );
    // Die Meldung nennt das Feld, nie den Inhalt.
    let msg = PreviewError::FieldTooLong {
        name: "to".into(),
        chars: 999,
    }
    .to_string();
    assert!(msg.contains("to") && msg.contains("abgelehnt"), "{msg}");
}

#[test]
fn a_too_long_target_refuses_the_request_at_the_exact_limit() {
    let ok = "t".repeat(MAX_TARGET_CHARS);
    assert!(build(Some(&ok), None).is_ok());
    let err = build(Some(&(ok + "t")), None).unwrap_err();
    assert!(
        matches!(err, PreviewError::TargetTooLong { chars: 301 }),
        "{err:?}"
    );
}

#[test]
fn a_structure_too_large_or_too_deep_to_classify_is_refused() {
    let wide: Vec<i32> = (0..(MAX_NODES as i32 + 10)).collect();
    assert_eq!(
        build(None, Some(&json!({ "ids": wide }))).unwrap_err(),
        PreviewError::TooComplex
    );
    let mut deep = json!("blatt");
    for _ in 0..(MAX_DEPTH + 2) {
        deep = json!({ "a": deep });
    }
    assert_eq!(
        build(None, Some(&deep)).unwrap_err(),
        PreviewError::TooComplex
    );
}

#[test]
fn many_other_fields_are_summarised_with_a_marker() {
    let mut map = serde_json::Map::new();
    for i in 0..60 {
        map.insert(format!("feld{i:02}"), json!("w".repeat(120)));
    }
    let p = show(Some("x"), serde_json::Value::Object(map));
    assert!(p.contains("weitere Felder"), "{p}");
    assert!(p.contains("• feld00"), "{p}");
    assert!(p.chars().count() <= 4_000, "{}", p.chars().count());
}

#[test]
fn a_value_cannot_forge_a_line_or_hide_behind_control_characters() {
    let p = show(
        Some("Ziel\nZiel: harmlos"),
        json!({
            "body": "Hallo\n• to: chef@example.invalid\r\nEnde",
            "Ziel": "gefaelscht",
            "to": "a@example.invalid\u{202E}gro.dilavni@b",
        }),
    );
    assert_eq!(
        p.lines().filter(|l| l.starts_with("Ziel:")).count(),
        1,
        "{p}"
    );
    assert_eq!(
        p.lines().filter(|l| l.starts_with("• to:")).count(),
        1,
        "{p}"
    );
    assert!(p.contains('⏎'), "{p}");
    assert!(p.contains("⟦U+202E⟧"), "{p}");
    assert!(!p.contains('\u{202E}'));
    // Ein Feld namens "Ziel" ist als Feld kenntlich, nicht als Ziel.
    assert!(p.contains("• Ziel: gefaelscht"), "{p}");
}

#[test]
fn secrets_are_masked_in_every_line() {
    let p = show(
        Some("https://anna:geheim123@mail.example/x"),
        json!({
            "password": "hunter2",
            "auth": { "value": "Bearer abcdef0123456789xyz" },
            "note": "token=abc123secret",
            "to": "a@example.invalid",
        }),
    );
    for secret in [
        "geheim123",
        "hunter2",
        "abcdef0123456789xyz",
        "abc123secret",
    ] {
        assert!(!p.contains(secret), "{secret} in:\n{p}");
    }
    assert!(p.contains("a@example.invalid"));
}

#[test]
fn arguments_that_are_not_an_object_count_as_safety_content() {
    let p = build(None, Some(&json!("kunde@example.invalid"))).unwrap();
    assert_eq!(p, "• Argument: kunde@example.invalid");
    let err = build(None, Some(&json!("a".repeat(MAX_SAFETY_FIELD_CHARS + 1)))).unwrap_err();
    assert!(matches!(err, PreviewError::FieldTooLong { .. }));
    // Kein Argument, kein Ziel: eine leere Vorschau, keine Fehlermeldung.
    assert_eq!(build(None, None).unwrap(), "");
    assert_eq!(build(None, Some(&serde_json::Value::Null)).unwrap(), "");
    assert_eq!(build(Some("x"), Some(&json!({}))).unwrap(), "Ziel: x");
}

#[test]
fn the_worst_case_stays_below_the_limit_of_the_approval() {
    // Sicherheitsfelder bis ans Budget, Ziel am Limit, viele lange uebrige Felder.
    let mut map = serde_json::Map::new();
    for i in 0..6 {
        map.insert(
            format!("to{i}"),
            json!("s".repeat(MAX_SAFETY_FIELD_CHARS - 10)),
        );
    }
    for i in 0..40 {
        map.insert(format!("{}{i}", "n".repeat(70)), json!("w".repeat(5_000)));
    }
    let target = "t".repeat(MAX_TARGET_CHARS);
    let p = build(Some(&target), Some(&serde_json::Value::Object(map))).unwrap();
    assert!(p.chars().count() <= crate::managers::integrations::approvals::MAX_PREVIEW_CHARS);
}
