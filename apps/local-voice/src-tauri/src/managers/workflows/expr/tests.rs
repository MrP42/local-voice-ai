use serde_json::{json, Value};

use super::*;

fn ctx() -> Value {
    json!({
        "trigger": {
            "calendar": "cal-1",
            "title": "Kundentermin Müller",
            "attendees": [
                {"email": "ich@firma.de", "is_self": true},
                {"email": "kunde@extern.de", "is_self": false}
            ],
            "minutes": 45,
            "tags": ["kunde", "angebot"]
        },
        "vars": {"vorlage": "kunde", "limit": 3},
        "steps": {"doc": {"path": "C:/Ablage/Protokoll.docx", "pages": 4}},
        "meeting": {"title": "Wochenrunde"}
    })
}

fn cond(src: &str) -> bool {
    parse_condition(src).unwrap().eval_bool(&ctx()).unwrap()
}

fn render(src: &str) -> Result<Value, ExprError> {
    parse_template(src).unwrap().render(&ctx())
}

#[test]
fn comparisons_and_the_wrapped_form_of_the_design_sketch_work() {
    assert!(cond("{{trigger.calendar}} == 'cal-1'"));
    assert!(!cond("{{trigger.calendar}} == 'cal-2'"));
    assert!(cond("trigger.minutes >= 45 and trigger.minutes < 60"));
    assert!(cond("trigger.minutes != 30"));
    assert!(cond("vars.limit == 3.0"), "Zahlen werden als Zahl verglichen");
    assert!(cond("steps.doc.pages > 3"));
}

#[test]
fn text_tests_and_lists_work() {
    assert!(cond("trigger.title contains 'Müller'"));
    assert!(cond("trigger.title startswith 'Kunden'"));
    assert!(cond("trigger.title endswith 'Müller'"));
    assert!(cond("'kunde' in trigger.tags"));
    assert!(cond("trigger.calendar in ['cal-1', 'cal-9']"));
    assert!(!cond("trigger.calendar in ['cal-2']"));
    assert!(cond("trigger.tags contains 'angebot'"));
    assert!(cond("trigger.attendees[1].email endswith '@extern.de'"));
}

#[test]
fn boolean_operators_short_circuit_and_nest() {
    assert!(cond("not (trigger.minutes < 10) && true"));
    assert!(cond("false or trigger.minutes == 45"));
    assert!(cond("!false and !(1 == 2)"));
    // Das Rechte wird nach einem entschiedenen Linken nicht mehr ausgewertet:
    // 5 > 'x' waere ein Typfehler.
    assert!(cond("true or 5 > 'x'"));
    assert!(!cond("false and 5 > 'x'"));
}

#[test]
fn a_missing_path_is_null_in_a_condition_and_exists_says_so() {
    assert!(!cond("trigger.nothing == 'x'"));
    assert!(cond("trigger.nothing == null"));
    assert!(!cond("exists(trigger.nothing)"));
    assert!(cond("exists(trigger.title)"));
    assert!(!cond("trigger.nothing > 3"), "Vergleich ohne Wert ist falsch, kein Fehler");
    assert!(cond("not exists(steps.other.path)"));
    assert!(cond("default(trigger.nothing, 'x') == 'x'"));
}

#[test]
fn functions_are_a_closed_list_of_six() {
    assert!(cond("len(trigger.attendees) == 2"));
    assert!(cond("len(trigger.title) == 19"));
    assert!(cond("lower(trigger.title) contains 'müller'"));
    assert!(cond("upper('ab') == 'AB'"));
    assert!(cond("trim('  x ') == 'x'"));
    let err = parse_condition("system('calc')").unwrap_err();
    assert!(matches!(err, ExprError::Syntax { .. }), "{err}");
    let err = parse_condition("len()").unwrap_err();
    assert!(err.to_string().contains("Argument"), "{err}");
}

#[test]
fn nothing_outside_the_grammar_parses() {
    for bad in [
        "1 = 1",
        "a = 2",
        "trigger.minutes == ",
        "x ==== y",
        "trigger.title; drop",
        "`ls`",
        "$(ls)",
        "trigger.a == 1 == 2",
        "len(trigger.tags",
        "'offener text",
        "a.b[x]",
        "not",
        "and true",
        "trigger..x",
        "a ? b : c",
        "1 + 2",
        "import os",
    ] {
        let r = parse_condition(bad);
        assert!(r.is_err(), "{bad:?} darf nicht gültig sein");
    }
}

#[test]
fn type_mismatches_are_errors_not_guesses() {
    let c = ctx();
    let e = parse_condition("trigger.title > 3").unwrap().eval_bool(&c);
    assert!(matches!(e, Err(ExprError::Type(_))), "{e:?}");
    let e = parse_condition("trigger.title").unwrap().eval_bool(&c);
    assert!(matches!(e, Err(ExprError::Type(_))), "ein Text ist keine Bedingung: {e:?}");
    let e = parse_condition("len(5)").unwrap().eval_bool(&c);
    assert!(matches!(e, Err(ExprError::Type(_))), "{e:?}");
}

#[test]
fn a_template_keeps_the_type_of_a_single_expression() {
    assert_eq!(render("{{trigger.minutes}}").unwrap(), json!(45));
    assert_eq!(render("{{trigger.tags}}").unwrap(), json!(["kunde", "angebot"]));
    assert_eq!(
        render("{{ steps.doc }}").unwrap(),
        json!({"path": "C:/Ablage/Protokoll.docx", "pages": 4})
    );
    assert_eq!(render("{{trigger.minutes > 10}}").unwrap(), json!(true));
}

#[test]
fn a_template_with_text_around_it_produces_text() {
    assert_eq!(
        render("Protokoll: {{meeting.title}} ({{trigger.minutes}} Min., {{trigger.minutes >= 45}})")
            .unwrap(),
        json!("Protokoll: Wochenrunde (45 Min., true)")
    );
    assert_eq!(
        render("{{trigger.title}}.docx").unwrap(),
        json!("Kundentermin Müller.docx")
    );
    assert_eq!(render("ohne Stelle").unwrap(), json!("ohne Stelle"));
}

#[test]
fn a_missing_value_in_a_template_is_an_error_never_an_empty_text() {
    let e = render("An: {{trigger.recipient}}").unwrap_err();
    assert_eq!(e, ExprError::Unresolved("trigger.recipient".to_string()));
    assert!(e.to_string().contains("trigger.recipient"));
    // Mit Ausweichwert ist es ausdruecklich in Ordnung.
    assert_eq!(
        render("An: {{default(trigger.recipient, 'ich')}}").unwrap(),
        json!("An: ich")
    );
    // Auch ein vorhandenes `null` wird nicht zu Text.
    let c = json!({"trigger": {"x": null}});
    let e = parse_template("a{{trigger.x}}b").unwrap().render(&c).unwrap_err();
    assert!(matches!(e, ExprError::Type(_)), "{e:?}");
}

#[test]
fn lists_and_objects_do_not_slip_into_a_text() {
    let e = render("Teilnehmende: {{trigger.attendees}}").unwrap_err();
    assert!(matches!(e, ExprError::Type(_)), "{e:?}");
}

#[test]
fn injected_template_syntax_in_data_is_never_evaluated_again() {
    // Ein Trigger-Wert, der selbst wie eine Vorlage aussieht, bleibt Text.
    let c = json!({
        "trigger": {"title": "{{vars.geheim}}"},
        "vars": {"geheim": "TOPSECRET"}
    });
    let out = parse_template("Betreff: {{trigger.title}}")
        .unwrap()
        .render(&c)
        .unwrap();
    assert_eq!(out, json!("Betreff: {{vars.geheim}}"));
    let params = json!({"subject": "{{trigger.title}}", "list": ["{{trigger.title}}"]});
    let out = render_value(&params, &c).unwrap();
    assert_eq!(out["subject"], json!("{{vars.geheim}}"));
    assert_eq!(out["list"][0], json!("{{vars.geheim}}"));
}

#[test]
fn braces_inside_quotes_do_not_close_a_placeholder_and_stray_braces_fail() {
    assert_eq!(
        render("{{default(trigger.nothing, '}} x')}}").unwrap(),
        json!("}} x")
    );
    assert!(parse_template("offen {{trigger.title").is_err());
    assert!(parse_template("zu }} viel").is_err());
    assert!(parse_template("{{}}").is_err());
}

#[test]
fn render_value_walks_objects_and_lists_but_not_keys() {
    let params = json!({
        "subject": "Protokoll {{meeting.title}}",
        "to": ["{{trigger.attendees[1].email}}", "fest@firma.de"],
        "n": 3,
        "{{key}}": "bleibt"
    });
    let out = render_value(&params, &ctx()).unwrap();
    assert_eq!(out["subject"], json!("Protokoll Wochenrunde"));
    assert_eq!(out["to"], json!(["kunde@extern.de", "fest@firma.de"]));
    assert_eq!(out["n"], json!(3));
    assert_eq!(out["{{key}}"], json!("bleibt"));
}

#[test]
fn size_and_depth_limits_stop_hostile_input() {
    let long = format!("'{}' == 'x'", "a".repeat(MAX_EXPR_CHARS));
    assert!(matches!(
        parse_condition(&long),
        Err(ExprError::TooComplex(_))
    ));
    let deep = format!("{}true{}", "(".repeat(40), ")".repeat(40));
    assert!(matches!(
        parse_condition(&deep),
        Err(ExprError::TooComplex(_))
    ));
    let wide = vec!["true"; 300].join(" and ");
    assert!(matches!(
        parse_condition(&wide),
        Err(ExprError::TooComplex(_))
    ));
    let huge = "x".repeat(MAX_TEMPLATE_CHARS + 1);
    assert!(matches!(
        parse_template(&huge),
        Err(ExprError::TooComplex(_))
    ));
    // Ein Ergebnis wird nicht beliebig gross: 40 Einsetzungen eines 8-KiB-Werts.
    let c = json!({"v": "y".repeat(8 * 1024)});
    let t = parse_template(&"{{v}}".repeat(40)).unwrap();
    assert!(matches!(t.render(&c), Err(ExprError::TooComplex(_))));
}

#[test]
fn refs_are_collected_for_the_static_check() {
    let e = parse_condition("{{trigger.calendar}} == 'x' and exists(steps.doc.path)").unwrap();
    let refs: Vec<String> = e.refs().iter().map(|p| p.to_string()).collect();
    assert_eq!(refs, vec!["trigger.calendar", "steps.doc.path"]);
    let v = json!({"a": "{{vars.x}} und {{steps.s1.out[2]}}", "b": [1, "{{trigger.t}}"]});
    let refs: Vec<String> = collect_value_refs(&v)
        .unwrap()
        .iter()
        .map(|p| p.to_string())
        .collect();
    assert_eq!(refs, vec!["vars.x", "steps.s1.out[2]", "trigger.t"]);
    let bad = json!({"a": {"b": "{{vars.}}"}});
    let (pointer, err) = collect_value_refs(&bad).unwrap_err();
    assert_eq!(pointer, "/a/b");
    assert!(matches!(err, ExprError::Syntax { .. }));
}
