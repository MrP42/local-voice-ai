use serde_json::json;

use super::*;

fn att(email: &str, is_self: bool) -> Attendee {
    Attendee {
        email: email.to_string(),
        is_self,
    }
}

fn team() -> Vec<Attendee> {
    vec![
        att("ich@wolff.de", true),
        att("anna@kunde.de", false),
        att("bert@wolff.de", false),
        att("Anna@Kunde.de", false), // Dublette in anderer Schreibweise
        att("cora@partner.eu", false),
    ]
}

fn src<'a>(
    attendees: &'a [Attendee],
    self_emails: &'a [String],
    own: Option<&'a str>,
    list: &'a [String],
) -> Sources<'a> {
    Sources {
        attendees,
        self_emails,
        own_address: own,
        list,
    }
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn me_is_the_calendars_own_address_first_then_the_setting_then_the_integration() {
    let none: Vec<String> = vec![];
    // Kalender kennzeichnet die eigene Adresse.
    let a = team();
    assert_eq!(
        resolve(
            Rule::Me,
            &src(&a, &strs(&["privat@home.de"]), Some("smtp@wolff.de"), &none)
        ),
        Ok(vec!["ich@wolff.de".to_string()])
    );
    // Ohne Kennzeichen: die Einstellung.
    let b = vec![att("anna@kunde.de", false)];
    assert_eq!(
        resolve(
            Rule::Me,
            &src(
                &b,
                &strs(&["privat@home.de", "zweit@home.de"]),
                Some("smtp@wolff.de"),
                &none
            )
        ),
        Ok(vec!["privat@home.de".to_string()]),
        "genau eine Adresse, nie alle"
    );
    // Ohne beides: die Adresse der Integration (SMTP: Absender).
    assert_eq!(
        resolve(Rule::Me, &src(&b, &[], Some("smtp@wolff.de"), &none)),
        Ok(vec!["smtp@wolff.de".to_string()])
    );
    // Gar nichts bekannt: Fehler statt Raten.
    assert_eq!(
        resolve(Rule::Me, &src(&b, &[], None, &none)),
        Err(RecipientError::NoSelf)
    );
}

#[test]
fn participants_are_everybody_but_me_without_duplicates_in_the_order_of_the_invitation() {
    let none: Vec<String> = vec![];
    let a = team();
    let got = resolve(Rule::Participants, &src(&a, &[], None, &none)).unwrap();
    assert_eq!(
        got,
        strs(&["anna@kunde.de", "bert@wolff.de", "cora@partner.eu"])
    );
    assert!(!got.iter().any(|x| x == "ich@wolff.de"));
}

#[test]
fn an_attendee_with_my_address_is_me_even_when_the_calendar_does_not_say_so() {
    let none: Vec<String> = vec![];
    // ICS kennt `is_self` nicht: die Einstellung entscheidet.
    let a = vec![att("Ich@Wolff.de", false), att("anna@kunde.de", false)];
    let me = strs(&["ich@wolff.de"]);
    assert_eq!(
        resolve(Rule::Participants, &src(&a, &me, None, &none)),
        Ok(strs(&["anna@kunde.de"]))
    );
    assert_eq!(
        resolve(Rule::Me, &src(&a, &me, None, &none)),
        Ok(strs(&["ich@wolff.de"]))
    );
    // Die Absenderadresse der Integration ist auch „ich“.
    let b = vec![att("smtp@wolff.de", false), att("anna@kunde.de", false)];
    assert_eq!(
        resolve(
            Rule::Participants,
            &src(&b, &[], Some("smtp@wolff.de"), &none)
        ),
        Ok(strs(&["anna@kunde.de"]))
    );
}

#[test]
fn all_is_me_first_then_the_others() {
    let none: Vec<String> = vec![];
    let a = team();
    assert_eq!(
        resolve(Rule::All, &src(&a, &[], None, &none)).unwrap(),
        strs(&[
            "ich@wolff.de",
            "anna@kunde.de",
            "bert@wolff.de",
            "cora@partner.eu"
        ])
    );
    // Ist niemand sonst da, ist „alle“ kein Auftrag: Fehler statt einer Mail an mich allein.
    let solo = vec![att("ich@wolff.de", true)];
    assert_eq!(
        resolve(Rule::All, &src(&solo, &[], None, &none)),
        Err(RecipientError::NoAttendees)
    );
    assert_eq!(
        resolve(Rule::Participants, &src(&solo, &[], None, &none)),
        Err(RecipientError::NoAttendees)
    );
}

#[test]
fn internal_is_the_participants_from_the_domain_of_my_addresses() {
    let none: Vec<String> = vec![];
    let a = team();
    assert_eq!(
        resolve(Rule::Internal, &src(&a, &[], None, &none)).unwrap(),
        strs(&["bert@wolff.de"])
    );
    // Die Domaene kann auch aus der Einstellung oder der Integration kommen.
    let b = vec![att("anna@kunde.de", false), att("x@home.de", false)];
    assert_eq!(
        resolve(
            Rule::Internal,
            &src(&b, &strs(&["privat@home.de"]), None, &none)
        )
        .unwrap(),
        strs(&["x@home.de"])
    );
    // Niemand intern / keine eigene Domaene bekannt.
    assert_eq!(
        resolve(
            Rule::Internal,
            &src(&b, &strs(&["a@wolff.de"]), None, &none)
        ),
        Err(RecipientError::NoInternal)
    );
    assert_eq!(
        resolve(Rule::Internal, &src(&b, &[], None, &none)),
        Err(RecipientError::NoOwnDomain)
    );
}

#[test]
fn the_fixed_list_is_cleaned_but_a_bad_address_stops_the_mail() {
    let none: Vec<String> = vec![];
    let nobody: Vec<Attendee> = vec![];
    let list = strs(&["  a@x.de ", "A@X.de", "b@y.de", ""]);
    assert_eq!(
        resolve(Rule::List, &src(&nobody, &[], None, &list)),
        Ok(strs(&["a@x.de", "b@y.de"]))
    );
    let bad = strs(&["a@x.de", "kein-mensch"]);
    assert_eq!(
        resolve(Rule::List, &src(&nobody, &[], None, &bad)),
        Err(RecipientError::BadAddress("kein-mensch".to_string()))
    );
    assert_eq!(
        resolve(Rule::List, &src(&nobody, &[], None, &none)),
        Err(RecipientError::EmptyList)
    );
}

#[test]
fn a_bad_attendee_address_is_never_dropped_silently() {
    let none: Vec<String> = vec![];
    let a = vec![
        att("ich@wolff.de", true),
        att("anna@kunde.de", false),
        att("a@b.de, c@d.de", false), // zwei Adressen in einem Feld: ein Einschleusungsversuch
    ];
    let e = resolve(Rule::Participants, &src(&a, &[], None, &none)).unwrap_err();
    assert!(matches!(e, RecipientError::BadAddress(_)), "{e:?}");
    let a = vec![
        att("ich@wolff.de", true),
        att("evil@x.de\r\nBcc: x@y.de", false),
    ];
    assert!(matches!(
        resolve(Rule::All, &src(&a, &[], None, &none)),
        Err(RecipientError::BadAddress(_))
    ));
}

#[test]
fn more_than_the_limit_is_an_error_not_a_cut() {
    let none: Vec<String> = vec![];
    let many: Vec<Attendee> = (0..=MAX_RECIPIENTS)
        .map(|i| att(&format!("p{i}@kunde.de"), false))
        .collect();
    assert_eq!(
        resolve(Rule::Participants, &src(&many, &[], None, &none)),
        Err(RecipientError::TooMany(MAX_RECIPIENTS + 1))
    );
    let fits: Vec<Attendee> = (0..MAX_RECIPIENTS)
        .map(|i| att(&format!("p{i}@kunde.de"), false))
        .collect();
    assert_eq!(
        resolve(Rule::Participants, &src(&fits, &[], None, &none))
            .unwrap()
            .len(),
        MAX_RECIPIENTS
    );
}

#[test]
fn attendees_come_out_of_the_trigger_data_and_garbage_gives_an_empty_list() {
    let ctx = json!({"trigger": {"attendees": [
        {"email": "a@x.de", "name": "A", "is_self": true},
        {"email": null, "name": "Ohne"},
        {"email": "  ", "is_self": false},
        {"email": "b@y.de"},
        "kein Objekt"
    ]}});
    assert_eq!(
        attendees_of(&ctx),
        vec![att("a@x.de", true), att("b@y.de", false)]
    );
    assert!(attendees_of(&json!({})).is_empty());
    assert!(attendees_of(&json!({"trigger": {"attendees": "alle"}})).is_empty());
}

#[test]
fn a_rule_is_one_of_five_fixed_words_and_nothing_else() {
    for r in ["me", "participants", "all", "internal", "list"] {
        assert_eq!(Rule::parse(r).unwrap().as_str(), r);
    }
    for bad in [
        "",
        "ME",
        "alle",
        "{{trigger.attendees}}",
        "anna@kunde.de",
        "me,all",
    ] {
        assert!(Rule::parse(bad).is_none(), "{bad}");
    }
    assert!(!Rule::Me.reaches_others());
    assert!(Rule::Participants.reaches_others() && Rule::All.reaches_others());
    assert!(Rule::Internal.reaches_others() && Rule::List.reaches_others());
}

#[test]
fn the_target_names_the_first_recipient_and_the_count() {
    assert_eq!(target_of(&[]), "(kein Empfänger)");
    assert_eq!(target_of(&strs(&["a@x.de"])), "a@x.de");
    assert_eq!(
        target_of(&strs(&["a@x.de", "b@x.de", "c@x.de"])),
        "a@x.de (+2)"
    );
}
