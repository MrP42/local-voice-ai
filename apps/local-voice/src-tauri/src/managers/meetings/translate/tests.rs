use std::cell::RefCell;

use super::*;

fn seg(i: u32, start_s: u64, text: &str) -> StoredSegment {
    StoredSegment {
        segment_index: i,
        text: text.to_string(),
        start_ms: start_s * 1000,
        end_ms: start_s * 1000 + 900,
        channel: if i % 2 == 0 { 0 } else { 1 },
        speaker_index: Some(i % 3),
        words: None,
    }
}

fn english() -> Vec<StoredSegment> {
    vec![
        seg(0, 1, "Good morning, this is Anna from Siemens."),
        seg(1, 4, "We pay 1,200.50 euros every month."),
        seg(2, 8, "Thomas will send the offer on Monday."),
    ]
}

fn params<'a>(known: &'a [String]) -> Params<'a> {
    Params {
        source_language: Some("en"),
        target_language: "de",
        known_names: known,
    }
}

fn reply(lines: &[(u32, &str)]) -> TranslateReply {
    TranslateReply {
        lines: lines
            .iter()
            .map(|(n, t)| ReplyLine {
                n: *n,
                text: (*t).to_string(),
            })
            .collect(),
    }
}

/// Die Zeilen "[n] Text" eines Anfragetextes.
fn parse_lines(user: &str) -> Vec<(u32, String)> {
    user.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('[')?;
            let (n, text) = rest.split_once("] ")?;
            Some((n.parse().ok()?, text.to_string()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Zahlen
// ---------------------------------------------------------------------------

#[test]
fn numbers_are_compared_independent_of_the_number_format() {
    assert_eq!(numbers_in("We pay 1,200.50 euros."), numbers_in("Wir zahlen 1.200,50 Euro."));
    assert_eq!(numbers_in("3.5 percent"), numbers_in("3,5 Prozent"));
    assert_eq!(numbers_in("on 05 May"), numbers_in("am 5. Mai"));
    assert_eq!(numbers_in("2,000 people"), numbers_in("2.000 Menschen"));
}

#[test]
fn a_space_before_three_digits_is_a_thousands_separator() {
    // Franzoesisch: "1 200,50" (auch mit geschuetztem Leerzeichen), spanisch "5 000".
    assert_eq!(numbers_in("1 200,50 euros"), numbers_in("1.200,50 Euro"));
    assert_eq!(numbers_in("1\u{a0}200,50"), numbers_in("1,200.50"));
    assert_eq!(numbers_in("5 000 personas"), numbers_in("5,000 people"));
    // Zwei getrennte Zahlen bleiben zwei.
    assert_eq!(numbers_in("12 and 34"), vec!["12".to_string(), "34".to_string()]);
    assert_eq!(numbers_in("page 12 340 times"), numbers_in("page 12,340 times"));
}

#[test]
fn different_numbers_are_different() {
    assert_ne!(numbers_in("We pay 120 euros."), numbers_in("Wir zahlen 12 Euro."));
    assert_ne!(numbers_in("3.5"), numbers_in("35"));
    assert_ne!(numbers_in("one of 120 and 15"), numbers_in("eins von 120"));
    assert!(numbers_in("no digits here").is_empty());
}

#[test]
fn a_time_keeps_its_two_parts() {
    assert_eq!(numbers_in("at 12:30"), numbers_in("um 12:30 Uhr"));
    assert_ne!(numbers_in("at 12:30"), numbers_in("um 12:45 Uhr"));
}

// ---------------------------------------------------------------------------
// Satzanzahl
// ---------------------------------------------------------------------------

#[test]
fn sentences_are_counted_by_their_end_marks() {
    assert_eq!(sentence_count("We pay 120 euros."), 1);
    assert_eq!(sentence_count("We pay 3.5 euros. Anna agrees."), 2);
    assert_eq!(sentence_count("Wait... what?"), 2);
    assert_eq!(sentence_count("Really?! Yes."), 2);
    assert_eq!(sentence_count("no end mark"), 1);
    assert_eq!(sentence_count("   "), 0);
}

#[test]
fn abbreviations_and_decimals_do_not_end_a_sentence() {
    assert_eq!(sentence_count("Dr. Müller kommt. Er sagt z. B. ja."), 2);
    assert_eq!(sentence_count("Mr. Smith pays 3.5 million."), 1);
    assert_eq!(sentence_count("See e.g. the offer."), 1);
}

// ---------------------------------------------------------------------------
// Namen
// ---------------------------------------------------------------------------

#[test]
fn names_are_taken_from_capitals_inside_sentences_and_known_people() {
    let doc = document_names(&english(), Some("en"));
    assert!(doc.contains("Siemens"), "Grossschreibung mitten im Satz: {doc:?}");
    assert!(!doc.contains("Monday"), "ein Wochentag ist kein Name: {doc:?}");
    assert!(!doc.contains("Good"), "Satzanfang zaehlt nicht: {doc:?}");
}

#[test]
fn a_sentence_initial_name_counts_when_it_appears_as_a_name_elsewhere() {
    let segments = vec![
        seg(0, 1, "Thomas will send the offer."),
        seg(1, 4, "Please ask Thomas about the price."),
    ];
    let doc = document_names(&segments, Some("en"));
    assert!(doc.contains("Thomas"));
    let required = required_names("Thomas will send the offer.", &doc, &[], Some("en"));
    assert_eq!(required, vec!["Thomas".to_string()]);
}

#[test]
fn known_people_are_always_required_even_at_the_start_of_a_sentence() {
    let known = vec!["Anna Berg".to_string()];
    let required = required_names("Anna Berg agrees.", &HashSet::new(), &known, Some("en"));
    assert_eq!(required, vec!["Anna Berg".to_string()]);
    assert!(required_names("Nobody agrees.", &HashSet::new(), &known, Some("en")).is_empty());
}

#[test]
fn a_german_source_only_requires_acronyms_and_known_people() {
    // Im Deutschen sind alle Nomen gross: "Lastgang" ist kein Name.
    let segments = vec![seg(0, 1, "Wir sprechen ueber den Lastgang der Stadtwerke und die BEW.")];
    let doc = document_names(&segments, Some("de"));
    assert!(!doc.contains("Lastgang") && !doc.contains("Stadtwerke"), "{doc:?}");
    assert!(doc.contains("BEW"), "Abkuerzungen schon: {doc:?}");
}

// ---------------------------------------------------------------------------
// Pruefung eines Satzes
// ---------------------------------------------------------------------------

fn ctx<'a>(segments: &'a [StoredSegment], known: &'a [String]) -> Checker {
    Checker::new(segments, &params(known))
}

#[test]
fn a_faithful_translation_has_no_flag() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    assert!(c
        .check(&source[0].text, "Guten Morgen, hier ist Anna von Siemens.")
        .is_empty());
    assert!(c
        .check(&source[1].text, "Wir zahlen jeden Monat 1.200,50 Euro.")
        .is_empty());
}

#[test]
fn a_changed_number_is_flagged_but_the_text_stays() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    assert_eq!(
        c.check(&source[1].text, "Wir zahlen jeden Monat 120,50 Euro."),
        vec![REASON_NUMBERS]
    );
}

#[test]
fn a_lost_name_is_flagged() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    assert_eq!(
        c.check(&source[0].text, "Guten Morgen, hier ist Anna von der Firma."),
        vec![REASON_NAMES]
    );
}

#[test]
fn a_changed_sentence_count_is_flagged() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    assert_eq!(
        c.check(&source[0].text, "Guten Morgen. Hier ist Anna von Siemens."),
        vec![REASON_SENTENCES]
    );
}

#[test]
fn several_problems_are_all_named() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    let reasons = c.check(&source[1].text, "Wir zahlen 5 Euro. Jeden Monat.");
    assert!(reasons.contains(&REASON_NUMBERS) && reasons.contains(&REASON_SENTENCES), "{reasons:?}");
}

#[test]
fn an_empty_or_unchanged_translation_is_flagged_as_not_translated() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    assert_eq!(c.check(&source[2].text, "  "), vec![REASON_EMPTY]);
    assert!(c
        .check(&source[2].text, &source[2].text)
        .contains(&REASON_NOT_TRANSLATED));
}

#[test]
fn an_absurd_length_is_flagged() {
    let source = english();
    let known = vec![];
    let c = ctx(&source, &known);
    let long = "Guten Morgen, hier ist Anna von Siemens. ".repeat(12);
    assert!(c.check(&source[0].text, &long).contains(&REASON_LENGTH));
}

// ---------------------------------------------------------------------------
// Prompt und Schema
// ---------------------------------------------------------------------------

#[test]
fn the_system_prompt_names_both_languages_and_the_rules() {
    let prompt = system_prompt(Some("en"), "de");
    for needle in ["English", "German", "numbers", "names", "one line", "same numbers"] {
        assert!(prompt.contains(needle), "{needle} fehlt in: {prompt}");
    }
    // Ohne bekannte Ausgangssprache steht keine falsche Angabe im Prompt.
    assert!(system_prompt(None, "de").contains("German"));
    assert!(!system_prompt(None, "de").contains("from English"));
}

#[test]
fn the_user_prompt_numbers_every_line_from_the_given_start() {
    let source = english();
    let user = user_prompt(&source[1..], 2);
    assert_eq!(parse_lines(&user).iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![2, 3]);
}

#[test]
fn the_shape_check_asks_again_when_numbers_are_missing_or_wrong() {
    assert!(shape_problem(&reply(&[(1, "a"), (2, "b")]), &[1, 2]).is_none());
    assert!(shape_problem(&reply(&[(1, "a")]), &[1, 2]).is_some());
    assert!(shape_problem(&reply(&[(1, "a"), (3, "b")]), &[1, 2]).is_some());
}

// ---------------------------------------------------------------------------
// Bloecke
// ---------------------------------------------------------------------------

#[test]
fn a_long_transcript_is_cut_into_blocks_of_whole_lines_covering_every_line_once() {
    let source: Vec<StoredSegment> = (0..200)
        .map(|i| seg(i, u64::from(i) * 3, &format!("This is sentence number {i} of a rather long meeting.")))
        .collect();
    let blocks = plan_blocks(&source);
    assert!(blocks.len() > 3);
    let mut next = 0;
    for range in &blocks {
        assert_eq!(range.start, next, "luecken- und ueberlappungsfrei");
        next = range.end;
        let chars: usize = source[range.clone()].iter().map(|s| s.text.chars().count() + 8).sum();
        let longest = source[range.clone()].iter().map(|s| s.text.chars().count() + 8).max().unwrap();
        assert!(chars <= BLOCK_CHARS + longest, "Block zu gross: {chars}");
    }
    assert_eq!(next, source.len());
}

// ---------------------------------------------------------------------------
// Der ganze Lauf
// ---------------------------------------------------------------------------

/// Antwortet auf jede Anfrage mit den Zeilen aus `table` (nach Nummer).
fn translator<'a>(
    table: Vec<(u32, &'static str)>,
    calls: &'a RefCell<Vec<Vec<u32>>>,
) -> impl FnMut(BlockRequest) -> std::future::Ready<Result<TranslateReply, String>> + 'a {
    move |request: BlockRequest| {
        let wanted: Vec<u32> = parse_lines(&request.user).iter().map(|(n, _)| *n).collect();
        calls.borrow_mut().push(wanted.clone());
        let lines = wanted
            .iter()
            .filter_map(|n| {
                table
                    .iter()
                    .find(|(m, _)| m == n)
                    .map(|(n, t)| ReplyLine { n: *n, text: (*t).to_string() })
            })
            .collect();
        std::future::ready(Ok(TranslateReply { lines }))
    }
}

fn good_table() -> Vec<(u32, &'static str)> {
    vec![
        (1, "Guten Morgen, hier ist Anna von Siemens."),
        (2, "Wir zahlen jeden Monat 1.200,50 Euro."),
        (3, "Thomas schickt das Angebot am Montag."),
    ]
}

fn go() -> std::future::Ready<Control> {
    std::future::ready(Control::Go)
}

fn block<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn every_source_line_gets_exactly_one_translated_line_with_the_same_time_channel_and_speaker() {
    let source = english();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let outcome = block(run(&source, &params(&known), translator(good_table(), &calls), go, |_, _| {})).unwrap();
    assert_eq!(outcome.segments.len(), source.len());
    for (translated, original) in outcome.segments.iter().zip(&source) {
        assert_eq!(
            (translated.segment_index, translated.start_ms, translated.end_ms, translated.channel, translated.speaker_index),
            (original.segment_index, original.start_ms, original.end_ms, original.channel, original.speaker_index),
            "1:1 mit Zeit, Kanal und Sprecher"
        );
        assert!(translated.words.is_none(), "Wortzeiten der Quellsprache gelten fuer den Text nicht");
    }
    assert_eq!(outcome.segments[1].text, "Wir zahlen jeden Monat 1.200,50 Euro.");
    assert!(outcome.flagged.is_empty());
}

#[test]
fn the_source_segments_are_never_changed() {
    let source = english();
    let before = source.clone();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let _ = block(run(&source, &params(&known), translator(good_table(), &calls), go, |_, _| {})).unwrap();
    assert_eq!(source, before);
}

#[test]
fn a_deviating_sentence_is_flagged_and_keeps_the_models_text() {
    let source = english();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let mut table = good_table();
    table[1] = (2, "Wir zahlen jeden Monat 120 Euro.");
    let outcome = block(run(&source, &params(&known), translator(table, &calls), go, |_, _| {})).unwrap();
    assert_eq!(outcome.segments[1].text, "Wir zahlen jeden Monat 120 Euro.", "nicht still verworfen");
    assert_eq!(outcome.flagged.len(), 1);
    assert_eq!(outcome.flagged[0].segment_index, 1);
    assert_eq!(outcome.flagged[0].reasons, vec![REASON_NUMBERS.to_string()]);
}

#[test]
fn a_line_the_model_left_out_keeps_the_source_text_and_is_flagged() {
    let source = english();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let table = vec![good_table()[0], good_table()[2]];
    let outcome = block(run(&source, &params(&known), translator(table, &calls), go, |_, _| {})).unwrap();
    assert_eq!(outcome.segments[1].text, source[1].text, "kein Loch im Transkript");
    let flag = outcome.flagged.iter().find(|f| f.segment_index == 1).unwrap();
    assert!(flag.reasons.contains(&REASON_NOT_TRANSLATED.to_string()));
}

#[test]
fn the_translation_runs_block_by_block_and_reports_progress() {
    let source: Vec<StoredSegment> = (0..120)
        .map(|i| seg(i, u64::from(i) * 3, &format!("This is sentence number {i} of a rather long meeting.")))
        .collect();
    let known = vec![];
    let expected_blocks = plan_blocks(&source).len();
    assert!(expected_blocks > 2);
    let mut seen = Vec::new();
    let ask = |request: BlockRequest| {
        let lines = parse_lines(&request.user)
            .into_iter()
            .map(|(n, t)| ReplyLine { n, text: format!("Das ist Satz {n} ({})", t.len()) })
            .collect();
        std::future::ready(Ok::<_, String>(TranslateReply { lines }))
    };
    let outcome = block(run(&source, &params(&known), ask, go, |done, total| seen.push((done, total)))).unwrap();
    assert_eq!(outcome.blocks, expected_blocks);
    assert_eq!(seen.first(), Some(&(0, expected_blocks)));
    assert_eq!(seen.last(), Some(&(expected_blocks, expected_blocks)));
    assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "der Fortschritt laeuft nie zurueck");
}

#[test]
fn a_stop_between_blocks_ends_the_run_without_a_result() {
    let source: Vec<StoredSegment> = (0..120)
        .map(|i| seg(i, u64::from(i) * 3, &format!("This is sentence number {i} of a rather long meeting.")))
        .collect();
    let known = vec![];
    let asked = RefCell::new(0usize);
    let ask = |request: BlockRequest| {
        *asked.borrow_mut() += 1;
        let lines = parse_lines(&request.user)
            .into_iter()
            .map(|(n, _)| ReplyLine { n, text: format!("Satz {n}") })
            .collect();
        std::future::ready(Ok::<_, String>(TranslateReply { lines }))
    };
    let calls = RefCell::new(0usize);
    let gate = || {
        *calls.borrow_mut() += 1;
        std::future::ready(if *calls.borrow() > 2 { Control::Stop } else { Control::Go })
    };
    let result = block(run(&source, &params(&known), ask, gate, |_, _| {}));
    assert_eq!(result.unwrap_err(), TranslateError::Cancelled);
    assert_eq!(*asked.borrow(), 2, "nach dem Stopp wird kein weiterer Block gefragt");
}

#[test]
fn a_model_failure_ends_the_run_without_a_result() {
    // llama-server stirbt mitten im Lauf: kein Teilergebnis, das Original bleibt.
    let source: Vec<StoredSegment> = (0..120)
        .map(|i| seg(i, u64::from(i) * 3, &format!("This is sentence number {i} of a rather long meeting.")))
        .collect();
    let known = vec![];
    let n = RefCell::new(0usize);
    let ask = |request: BlockRequest| {
        *n.borrow_mut() += 1;
        if *n.borrow() == 2 {
            return std::future::ready(Err("Uebersetzung-Erzeugung fehlgeschlagen: connection reset".to_string()));
        }
        let lines = parse_lines(&request.user)
            .into_iter()
            .map(|(n, _)| ReplyLine { n, text: format!("Satz {n}") })
            .collect();
        std::future::ready(Ok(TranslateReply { lines }))
    };
    let err = block(run(&source, &params(&known), ask, go, |_, _| {})).unwrap_err();
    assert!(matches!(err, TranslateError::Llm(m) if m.contains("connection reset")));
}

#[test]
fn when_most_blocks_come_back_unusable_there_is_no_translation() {
    let source: Vec<StoredSegment> = (0..120)
        .map(|i| seg(i, u64::from(i) * 3, &format!("This is sentence number {i} of a rather long meeting.")))
        .collect();
    let known = vec![];
    let ask = |_: BlockRequest| std::future::ready(Ok::<_, String>(TranslateReply { lines: vec![] }));
    let err = block(run(&source, &params(&known), ask, go, |_, _| {})).unwrap_err();
    assert!(matches!(err, TranslateError::Rejected { .. }), "{err:?}");
}

#[test]
fn nothing_to_translate_and_the_same_language_are_refused_before_any_model_call() {
    let known = vec![];
    let asked = RefCell::new(0usize);
    let ask = |_: BlockRequest| {
        *asked.borrow_mut() += 1;
        std::future::ready(Ok::<_, String>(TranslateReply { lines: vec![] }))
    };
    let empty = vec![seg(0, 1, "   ")];
    assert_eq!(block(run(&empty, &params(&known), ask, go, |_, _| {})).unwrap_err(), TranslateError::Empty);
    let same = Params { source_language: Some("en"), target_language: "en-US", known_names: &known };
    let source = english();
    assert_eq!(block(run(&source, &same, ask, go, |_, _| {})).unwrap_err(), TranslateError::SameLanguage);
    assert_eq!(*asked.borrow(), 0);
}

#[test]
fn the_segments_come_back_in_source_order_even_when_the_source_is_unsorted() {
    let mut source = english();
    source.reverse();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let outcome = block(run(&source, &params(&known), translator(good_table(), &calls), go, |_, _| {})).unwrap();
    // Sortiert nach Startzeit, die Nummern der Segmente bleiben die der Quelle.
    assert_eq!(outcome.segments.iter().map(|s| s.segment_index).collect::<Vec<_>>(), vec![0, 1, 2]);
}

#[test]
fn the_report_lists_flags_and_counts_for_the_variant() {
    let source = english();
    let calls = RefCell::new(Vec::new());
    let known = vec![];
    let mut table = good_table();
    table[2] = (3, "Thomas schickt das Angebot am Dienstag. Danke.");
    let outcome = block(run(&source, &params(&known), translator(table, &calls), go, |_, _| {})).unwrap();
    let report = outcome.report("vsrc", Some("llm-test"), "de");
    assert_eq!(report.source_variant_id, "vsrc");
    assert_eq!(report.target_language, "de");
    assert_eq!(report.source_language.as_deref(), Some("en"));
    assert_eq!(report.checked, 3);
    assert_eq!(report.flagged.len(), 1);
    let json = serde_json::to_string(&report).unwrap();
    assert!(json.contains("\"flagged\""), "der Name, den `variants` zaehlt");
}
