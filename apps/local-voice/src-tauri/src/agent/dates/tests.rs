use chrono::NaiveDate;

use super::*;

/// Donnerstag, der Tag der Besprechung in den meisten Faellen.
fn thu() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Jede Zeile: Angabe -> erwartetes Datum (`None`: nicht aufloesbar).
fn check(reference: NaiveDate, table: &[(&str, Option<NaiveDate>)]) {
    for (phrase, expected) in table {
        assert_eq!(
            resolve(phrase, reference).map(|r| r.date),
            *expected,
            "Angabe: {phrase:?} (Bezug {reference})"
        );
    }
}

#[test]
fn iso_dates_are_parsed_strictly_and_validated_against_the_meeting_date() {
    assert_eq!(parse_iso("2026-10-05"), Ok(day(2026, 10, 5)));
    assert_eq!(parse_iso(" 2026-10-05 "), Ok(day(2026, 10, 5)));
    for bad in ["2026-1-5", "05.10.2026", "2026-10-05T10:00", "", "morgen"] {
        assert_eq!(parse_iso(bad), Err(DateIssue::Malformed), "{bad:?}");
    }
    assert_eq!(parse_iso("2026-02-30"), Err(DateIssue::NotACalendarDate));
    assert_eq!(parse_iso("2026-13-01"), Err(DateIssue::NotACalendarDate));

    let reference = thu();
    assert_eq!(validate(day(2026, 10, 1), reference), Ok(day(2026, 10, 1)));
    assert_eq!(validate(day(2026, 9, 30), reference), Err(DateIssue::Past));
    assert_eq!(validate(day(2028, 10, 1), reference), Ok(day(2028, 10, 1)));
    assert_eq!(validate(day(2028, 10, 2), reference), Err(DateIssue::TooFar));
    assert_eq!(
        validate_iso("2026-09-01", reference),
        Err(DateIssue::Past)
    );
    assert_eq!(
        validate_iso("2026-10-15", reference),
        Ok(day(2026, 10, 15))
    );
    assert_eq!(
        validate_iso("2026-02-30", reference),
        Err(DateIssue::NotACalendarDate)
    );
}

#[test]
fn day_words_and_single_weekdays_resolve_from_the_meeting_date() {
    check(
        thu(),
        &[
            ("heute", Some(day(2026, 10, 1))),
            ("morgen", Some(day(2026, 10, 2))),
            ("bis morgen früh", Some(day(2026, 10, 2))),
            ("Übermorgen", Some(day(2026, 10, 3))),
            ("gestern", Some(day(2026, 9, 30))),
            ("vorgestern", Some(day(2026, 9, 29))),
            // Zwei Tageswoerter, die sich widersprechen: keine Vermutung.
            ("heute Morgen", None),
            ("Freitag", Some(day(2026, 10, 2))),
            ("bis Freitag", Some(day(2026, 10, 2))),
            ("nächsten Montag", Some(day(2026, 10, 5))),
            // Derselbe Wochentag: in einer Woche, nicht heute.
            ("bis Donnerstag", Some(day(2026, 10, 8))),
            ("Freitag oder Montag", None),
            // Ein zusammengesetztes Wort ist kein Wochentag.
            ("Freitagnachmittag", None),
        ],
    );
}

#[test]
fn weekdays_with_a_week_qualifier_use_the_calendar_week() {
    check(
        thu(),
        &[
            ("Freitag nächster Woche", Some(day(2026, 10, 9))),
            ("Freitag in der nächsten Woche", Some(day(2026, 10, 9))),
            ("nächste Woche Dienstag", Some(day(2026, 10, 6))),
            ("übernächste Woche Mittwoch", Some(day(2026, 10, 14))),
            ("bis spätestens Freitag nächster Woche um 14 Uhr", Some(day(2026, 10, 9))),
            // Diese Woche, aber schon vorbei: das Datum kommt zurueck, die Pruefung verwirft es.
            ("Montag dieser Woche", Some(day(2026, 9, 28))),
            // Ohne Tag ist "naechste Woche" nicht eindeutig.
            ("nächste Woche", None),
            ("Wochenende", None),
        ],
    );
    assert_eq!(
        validate(day(2026, 9, 28), thu()),
        Err(DateIssue::Past),
        "ein Datum vor der Besprechung wird verworfen"
    );
}

#[test]
fn week_month_and_year_edges() {
    check(
        thu(),
        &[
            ("Ende der Woche", Some(day(2026, 10, 2))),
            ("Ende nächster Woche", Some(day(2026, 10, 9))),
            ("Anfang nächster Woche", Some(day(2026, 10, 5))),
            ("Ende des Monats", Some(day(2026, 10, 31))),
            ("zum Monatsende", Some(day(2026, 10, 31))),
            ("am letzten Tag dieses Monats", Some(day(2026, 10, 31))),
            ("Ende nächsten Monats", Some(day(2026, 11, 30))),
            ("Ende des nächsten Monats", Some(day(2026, 11, 30))),
            ("Anfang nächsten Monats", Some(day(2026, 11, 1))),
            ("Ende Oktober", Some(day(2026, 10, 31))),
            ("Anfang November", Some(day(2026, 11, 1))),
            ("Mitte Dezember", Some(day(2026, 12, 15))),
            // Ein Monat, der im Jahr schon vorbei ist, meint das naechste Jahr.
            ("Ende September", Some(day(2027, 9, 30))),
            ("Jahresende", Some(day(2026, 12, 31))),
            ("Ende nächsten Jahres", Some(day(2027, 12, 31))),
        ],
    );
    // Am Wochenende meint "Ende der Woche" die kommende Woche.
    check(
        day(2026, 10, 3),
        &[("Ende der Woche", Some(day(2026, 10, 9)))],
    );
}

#[test]
fn in_n_units_accepts_digits_and_number_words_but_not_vague_amounts() {
    check(
        thu(),
        &[
            ("in zwei Wochen", Some(day(2026, 10, 15))),
            ("in 3 Tagen", Some(day(2026, 10, 4))),
            ("in 14 Tagen", Some(day(2026, 10, 15))),
            ("in einer Woche", Some(day(2026, 10, 8))),
            ("in einem Monat", Some(day(2026, 11, 1))),
            ("in zwölf Monaten", Some(day(2027, 10, 1))),
            ("in einigen Tagen", None),
            ("in zwei bis drei Wochen", None),
            // Ein genannter Wochentag muss zum Ergebnis passen.
            ("in zwei Wochen am Freitag", None),
            ("in zwei Wochen am Donnerstag", Some(day(2026, 10, 15))),
        ],
    );
    // Monatsende wird nicht ueberlaufen: 31.1. + 1 Monat = 28.2.
    check(
        day(2027, 1, 31),
        &[("in einem Monat", Some(day(2027, 2, 28)))],
    );
}

#[test]
fn calendar_weeks_mean_the_friday_and_roll_over_into_next_year() {
    check(
        thu(),
        &[
            ("KW 42", Some(day(2026, 10, 16))),
            ("bis Kalenderwoche 42", Some(day(2026, 10, 16))),
            ("KW 3", Some(day(2027, 1, 22))),
            ("KW 99", None),
        ],
    );
}

#[test]
fn absolute_dates_in_every_spoken_and_written_form() {
    check(
        thu(),
        &[
            ("15. Oktober", Some(day(2026, 10, 15))),
            ("bis zum 15.10.", Some(day(2026, 10, 15))),
            ("15.10.2026", Some(day(2026, 10, 15))),
            ("15.10.26", Some(day(2026, 10, 15))),
            ("15. 10. 2026", Some(day(2026, 10, 15))),
            ("2026-10-15", Some(day(2026, 10, 15))),
            ("3. November 2026", Some(day(2026, 11, 3))),
            ("am 3.11.", Some(day(2026, 11, 3))),
            ("1. Oktober", Some(day(2026, 10, 1))),
            // Ohne Jahr: das naechste Vorkommen ab dem Bezug.
            ("30. September", Some(day(2027, 9, 30))),
            ("29. Februar", Some(day(2028, 2, 29))),
            // Den Tag gibt es nie.
            ("31. Februar", None),
            ("31.2.2026", None),
            // Zwei verschiedene Daten sind keine Frist.
            ("am 3. November oder am 5. November", None),
            ("am 3. November, spätestens am 3.11.", Some(day(2026, 11, 3))),
        ],
    );
}

#[test]
fn a_date_and_a_relative_expression_must_agree_and_weekdays_are_a_cross_check() {
    check(
        thu(),
        &[
            ("Freitag, den 9. Oktober", Some(day(2026, 10, 9))),
            ("Donnerstag, den 9. Oktober", None),
            ("morgen, also der 2. Oktober", Some(day(2026, 10, 2))),
            ("morgen, also der 5. Oktober", None),
            ("Ende des Monats, spätestens aber 15. Oktober", None),
        ],
    );
}

#[test]
fn unusable_or_empty_phrases_never_resolve() {
    check(
        thu(),
        &[
            ("", None),
            ("   ", None),
            ("zeitnah", None),
            ("so bald wie möglich", None),
            ("irgendwann", None),
        ],
    );
}

#[test]
fn the_rule_that_produced_a_date_is_reported() {
    let r = resolve("Freitag nächster Woche", thu()).unwrap();
    assert_eq!(r.rule, "wochentag_in_woche");
    assert_eq!(resolve("15.10.2026", thu()).unwrap().rule, "absolut");
    assert_eq!(resolve("übermorgen", thu()).unwrap().rule, "tageswort");
}

#[test]
fn the_help_table_gives_the_model_computed_dates() {
    let table = help_table(thu());
    assert!(table.contains("heute = Donnerstag, 2026-10-01"), "{table}");
    assert!(table.contains("morgen = Freitag, 2026-10-02"), "{table}");
    assert!(table.contains("übermorgen = Samstag, 2026-10-03"), "{table}");
    assert!(table.contains("Montag 2026-10-05"), "{table}");
    assert!(table.contains("in zwei Wochen = 2026-10-15"), "{table}");
    assert!(table.contains("Ende des Monats = 2026-10-31"), "{table}");
}
