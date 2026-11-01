use super::diff::*;
use super::setup::*;
use super::store::*;

fn pair(heard: &str, corrected: &str) -> Pair {
    Pair {
        heard: heard.to_string(),
        corrected: corrected.to_string(),
    }
}

fn learn(heard: &str, corrected: &str) -> Analysis {
    analyze(heard, corrected, Mode::Correction)
}

fn store() -> SpeechLearningStore {
    SpeechLearningStore::open_in_memory().unwrap()
}

fn no_words() -> Vec<String> {
    Vec::new()
}

/// Ein Verhörer, den es sicher gibt: „Grund Steuer“ statt „Grundsteuer“.
const HEARD: &str = "Ich zahle die Grund Steuer morgen";
const FIXED: &str = "Ich zahle die Grundsteuer morgen";

// ---------------------------------------------------------------------------
// Diff: Verhörer
// ---------------------------------------------------------------------------

#[test]
fn a_split_compound_is_a_pair() {
    let a = learn(HEARD, FIXED);
    assert_eq!(a.verdict, Verdict::Pairs);
    assert_eq!(a.pairs, vec![pair("Grund Steuer", "Grundsteuer")]);
}

#[test]
fn a_single_misspelled_word_is_a_pair() {
    let a = learn(
        "Er legt Wiederspruch gegen den Bescheid ein",
        "Er legt Widerspruch gegen den Bescheid ein",
    );
    assert_eq!(a.pairs, vec![pair("Wiederspruch", "Widerspruch")]);
}

#[test]
fn umlauts_are_ordinary_letters() {
    let a = learn(
        "Das Bürgeramt ist für die Mülabfuhr zuständig",
        "Das Bürgeramt ist für die Müllabfuhr zuständig",
    );
    assert_eq!(a.pairs, vec![pair("Mülabfuhr", "Müllabfuhr")]);
    let a = learn("Die Strasse ist gesperrt", "Die Straße ist gesperrt");
    assert_eq!(a.pairs, vec![pair("Strasse", "Straße")]);
}

#[test]
fn uppercase_umlaut_words_compare_by_lowercase() {
    let a = learn("ÖFFENTLICHE Bekanntmachung", "Öffentliche Bekanntmachung");
    assert_eq!(a.verdict, Verdict::Nothing);
}

#[test]
fn a_joined_word_is_a_pair() {
    let a = learn(
        "Der Gemeinde Rat tagt heute Abend",
        "Der Gemeinderat tagt heute Abend",
    );
    assert_eq!(a.pairs, vec![pair("Gemeinde Rat", "Gemeinderat")]);
}

#[test]
fn up_to_three_misheard_words_in_one_correction() {
    let a = learn(
        "Die Hunt und die Katse sitzen im Gaten vor dem Haus",
        "Die Hund und die Katze sitzen im Garten vor dem Haus",
    );
    assert_eq!(a.verdict, Verdict::Pairs);
    assert_eq!(
        a.pairs,
        vec![
            pair("Hunt", "Hund"),
            pair("Katse", "Katze"),
            pair("Gaten", "Garten")
        ]
    );
}

#[test]
fn the_same_pair_twice_in_one_text_counts_once() {
    let a = learn(
        "Grund Steuer und noch einmal Grund Steuer bitte",
        "Grundsteuer und noch einmal Grundsteuer bitte",
    );
    assert_eq!(a.pairs, vec![pair("Grund Steuer", "Grundsteuer")]);
}

#[test]
fn punctuation_next_to_a_word_is_not_part_of_the_pair() {
    let a = learn(
        "Ich zahle die Grund Steuer, morgen.",
        "Ich zahle die Grundsteuer, morgen.",
    );
    assert_eq!(a.pairs, vec![pair("Grund Steuer", "Grundsteuer")]);
}

#[test]
fn a_hyphen_inside_a_word_stays() {
    let a = learn(
        "Schick mir die Email Adresse bitte",
        "Schick mir die E-Mail-Adresse bitte",
    );
    assert_eq!(a.verdict, Verdict::Pairs);
    assert_eq!(a.pairs, vec![pair("Email Adresse", "E-Mail-Adresse")]);
}

// ---------------------------------------------------------------------------
// Diff: Umformulierung und Nichtigkeiten
// ---------------------------------------------------------------------------

#[test]
fn a_rewording_is_rejected() {
    let a = learn(
        "Bitte schick mir den Bescheid heute noch",
        "Ich brauche den Antrag sofort bitte",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
    assert!(a.pairs.is_empty());
}

#[test]
fn a_replaced_word_with_a_different_meaning_is_a_rewording() {
    let a = learn(
        "Ich zahle den Bescheid morgen",
        "Ich zahle den Antrag morgen",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
}

#[test]
fn a_completely_different_sentence_is_rejected() {
    let a = learn("Das Wetter ist heute schön", "Der Antrag wurde abgelehnt");
    assert_eq!(a.verdict, Verdict::Rewrite);
}

#[test]
fn one_real_rewording_next_to_a_misheard_word_rejects_everything() {
    let a = learn(
        "Ich zahle die Grund Steuer an das Amt",
        "Ich zahle die Grundsteuer an die Kasse",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
    assert!(a.pairs.is_empty());
}

#[test]
fn more_than_three_blocks_is_a_rewording() {
    let a = learn(
        "Die Hunt und die Katse sitzen im Gaten und fressen Brod vor dem Haus",
        "Die Hund und die Katze sitzen im Garten und fressen Brot vor dem Haus",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
}

#[test]
fn four_blocks_are_fine_while_reading() {
    let a = analyze(
        "Die Hunt und die Katse sitzen im Gaten und fressen Brod vor dem Haus",
        "Die Hund und die Katze sitzen im Garten und fressen Brot vor dem Haus",
        Mode::Reading,
    );
    assert_eq!(a.verdict, Verdict::Pairs);
    assert_eq!(a.pairs.len(), 4);
}

#[test]
fn a_block_of_four_words_is_a_rewording() {
    let a = learn(
        "Heute gilt die Auf Wands Kosten Verordnung für alle Bürger",
        "Heute gilt die Aufwandsverordnung für alle Bürger",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
}

#[test]
fn a_block_of_four_words_is_skipped_while_reading() {
    let a = analyze(
        "Heute gilt die Auf Wands Kosten Verordnung für alle Bürger",
        "Heute gilt die Aufwandsverordnung für alle Bürger",
        Mode::Reading,
    );
    assert_eq!(a.verdict, Verdict::Nothing);
}

#[test]
fn a_side_longer_than_64_characters_is_a_rewording() {
    let long = "a".repeat(65);
    let longer = format!("{long}b");
    let a = learn(
        &format!("Das ist {long} hier"),
        &format!("Das ist {longer} hier"),
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
    let ok = "a".repeat(64);
    let a = learn(
        &format!("Das ist {ok} hier"),
        &format!("Das ist {ok}b hier"),
    );
    assert_eq!(
        a.verdict,
        Verdict::Rewrite,
        "65 Zeichen auf der korrigierten Seite"
    );
    let a = learn(
        &format!("Das ist {}b hier", "a".repeat(62)),
        &format!("Das ist {}c hier", "a".repeat(63)),
    );
    assert_eq!(a.verdict, Verdict::Pairs, "63 und 64 Zeichen sind erlaubt");
}

#[test]
fn only_case_or_punctuation_changes_are_nothing() {
    for (a, b) in [
        ("hallo welt", "Hallo, Welt!"),
        ("Das ist gut.", "das ist gut"),
        ("Ja, bitte - sofort", "Ja bitte sofort"),
        ("«Antrag» genehmigt", "Antrag genehmigt"),
        ("ARBEITSAMT Mitte", "Arbeitsamt mitte"),
    ] {
        let r = learn(a, b);
        assert_eq!(r.verdict, Verdict::Nothing, "{a} / {b}");
        assert!(r.pairs.is_empty());
    }
}

#[test]
fn identical_and_empty_texts_are_nothing() {
    assert_eq!(learn(HEARD, HEARD).verdict, Verdict::Nothing);
    assert_eq!(learn("", "").verdict, Verdict::Nothing);
    assert_eq!(learn("   ", "...").verdict, Verdict::Nothing);
}

#[test]
fn only_insertions_or_deletions_are_nothing() {
    assert_eq!(
        learn("Ich brauche Antrag", "Ich brauche den Antrag").verdict,
        Verdict::Nothing
    );
    assert_eq!(
        learn("Ich brauche den den Antrag", "Ich brauche den Antrag").verdict,
        Verdict::Nothing
    );
}

#[test]
fn numbers_are_not_learned() {
    // Zahl gegen Zahl und Zahlwort gegen Ziffern sind keine Verhörer für den Wortschatz.
    assert_eq!(
        learn("Paragraf 35 gilt hier", "Paragraf 53 gilt hier").verdict,
        Verdict::Nothing
    );
    assert_eq!(
        learn("Es kommen zehn Anträge heute", "Es kommen 10 Anträge heute").verdict,
        Verdict::Nothing
    );
    assert_eq!(
        learn("Es kostet 1500 Euro", "Es kostet 1.500 Euro").verdict,
        Verdict::Nothing
    );
}

#[test]
fn numbers_next_to_a_misheard_word_do_not_reject_it() {
    let a = learn(
        "Paragraf 35 regelt die Grund Steuer",
        "Paragraf 53 regelt die Grundsteuer",
    );
    assert_eq!(a.verdict, Verdict::Pairs);
    assert_eq!(a.pairs, vec![pair("Grund Steuer", "Grundsteuer")]);
}

#[test]
fn a_word_with_letters_and_digits_is_learnable() {
    let a = learn(
        "Das ist Gewerbe Steuer Form 2b",
        "Das ist Gewerbesteuer Form 2b",
    );
    assert_eq!(a.pairs, vec![pair("Gewerbe Steuer", "Gewerbesteuer")]);
}

#[test]
fn very_short_corrections_are_skipped() {
    // „ist“ → „is“: ähnlich, aber die Korrektur hat nur zwei Buchstaben.
    let a = learn("Das ist gut so", "Das is gut so");
    assert_eq!(a.verdict, Verdict::Nothing);
}

#[test]
fn characters_the_word_list_refuses_are_skipped() {
    let a = learn("Heute ist Goethes Tag", "Heute ist Goethe's Tag");
    assert_eq!(a.verdict, Verdict::Nothing);
    let a = learn("Das Tag ist offen", "Das <Tag> ist offen");
    assert_eq!(a.verdict, Verdict::Nothing);
}

#[test]
fn texts_with_little_in_common_are_rewritten_even_when_short() {
    let a = learn(
        "Guten Morgen liebe Gäste heute",
        "Auf Wiedersehen bis bald zusammen",
    );
    assert_eq!(a.verdict, Verdict::Rewrite);
    assert_eq!(a.matched, 0);
}

#[test]
fn matched_and_total_count_words() {
    let a = learn(HEARD, FIXED);
    assert_eq!(a.total, 6);
    assert_eq!(a.matched, 4);
}

#[test]
fn similarity_is_symmetric_and_bounded() {
    assert_eq!(similarity("abc", "abc"), 1.0);
    assert_eq!(similarity("", ""), 1.0);
    assert_eq!(similarity("abc", ""), 0.0);
    assert!((similarity("grund steuer", "grundsteuer") - 1.0).abs() < 1e-9);
    assert!(similarity("bescheid", "antrag") < MIN_SIMILARITY);
    assert!(similarity("wiederspruch", "widerspruch") > 0.9);
    assert_eq!(similarity("kater", "katze"), similarity("katze", "kater"));
}

#[test]
fn normalize_key_ignores_case_and_edge_punctuation() {
    assert_eq!(normalize_key("  Grund   STEUER, "), "grund steuer");
    assert_eq!(normalize_key("Müll!"), "müll");
    assert_eq!(normalize_key("..."), "");
}

// ---------------------------------------------------------------------------
// Grenzen
// ---------------------------------------------------------------------------

#[test]
fn text_length_limit_counts_characters() {
    assert!(check_text(&"ä".repeat(MAX_TEXT_CHARS)).is_ok());
    let err = check_text(&"ä".repeat(MAX_TEXT_CHARS + 1)).unwrap_err();
    assert!(!err.contains('ä'), "keine Inhalte in der Meldung");
}

#[test]
fn bereich_pattern() {
    for ok in ["standard", "a", "a-b_c9", &"x".repeat(32)] {
        assert!(check_bereich(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "Standard",
        "ä",
        "a b",
        "a/b",
        "a.b",
        &"x".repeat(33),
        "a\n",
    ] {
        assert!(check_bereich(bad).is_err(), "{bad:?}");
    }
    let msg = check_bereich("Böse Eingabe").unwrap_err();
    assert!(!msg.contains("Böse"));
}

// ---------------------------------------------------------------------------
// Speicher
// ---------------------------------------------------------------------------

fn feed(s: &SpeechLearningStore, auto: bool, words: &[String]) -> (FeedbackOutcome, WordsDelta) {
    s.feedback("standard", HEARD, FIXED, auto, words).unwrap()
}

#[test]
fn first_time_is_only_observed() {
    let s = store();
    let (out, delta) = feed(&s, false, &no_words());
    assert_eq!(out.reason, "ok");
    assert_eq!((out.learned, out.suggestions, out.adopted), (1, 0, 0));
    assert!(delta.is_empty());
    let list = s.list("standard", &[]).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].gehoert, "Grund Steuer");
    assert_eq!(list[0].korrigiert, "Grundsteuer");
    assert_eq!(list[0].anzahl, 1);
    assert_eq!(list[0].status, STATUS_OBSERVED);
}

#[test]
fn the_second_time_it_becomes_a_suggestion() {
    let s = store();
    feed(&s, false, &no_words());
    let (out, delta) = feed(&s, false, &no_words());
    assert_eq!((out.learned, out.suggestions, out.adopted), (1, 1, 0));
    assert!(
        delta.is_empty(),
        "ohne automatisch übernehmen bleibt der Wortschatz unberührt"
    );
    let list = s.list("standard", &[]).unwrap();
    assert_eq!(list[0].anzahl, 2);
    assert_eq!(list[0].status, STATUS_SUGGESTED);
    let (out, _) = feed(&s, false, &no_words());
    assert_eq!(out.suggestions, 0, "schon Vorschlag, nicht neu");
    assert_eq!(s.list("standard", &[]).unwrap()[0].anzahl, 3);
}

#[test]
fn case_variants_are_the_same_pair() {
    let s = store();
    s.feedback("standard", HEARD, FIXED, false, &[]).unwrap();
    s.feedback(
        "standard",
        "ich zahle die GRUND STEUER morgen",
        "ich zahle die Grundsteuer morgen",
        false,
        &[],
    )
    .unwrap();
    let list = s.list("standard", &[]).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].anzahl, 2);
}

#[test]
fn auto_apply_adopts_on_the_second_occurrence() {
    let s = store();
    let (_, d1) = feed(&s, true, &no_words());
    assert!(d1.is_empty(), "ein Vorkommen genügt nicht");
    assert_eq!(s.list("standard", &[]).unwrap()[0].status, STATUS_OBSERVED);
    let (out, d2) = feed(&s, true, &no_words());
    assert_eq!((out.suggestions, out.adopted), (1, 1));
    assert_eq!(d2.add, vec!["Grundsteuer".to_string()]);
    assert!(d2.remove.is_empty());
    assert_eq!(s.list("standard", &[]).unwrap()[0].status, STATUS_ADOPTED);
    let mut words = vec!["Amtsblatt".to_string()];
    assert!(d2.apply(&mut words));
    assert_eq!(words, vec!["Amtsblatt", "Grundsteuer"]);
    assert!(!d2.apply(&mut words), "zweites Anwenden ändert nichts");
}

#[test]
fn adoption_into_custom_words_is_skipped_for_known_words_and_left_alone_on_delete() {
    let s = store();
    let words = vec!["grundsteuer".to_string()];
    feed(&s, true, &words);
    let (_, delta) = feed(&s, true, &words);
    assert!(
        delta.is_empty(),
        "der Begriff stand schon (Groß-/Kleinschreibung egal) im Wortschatz"
    );
    let id = s.list("standard", &words).unwrap()[0].id;
    assert_eq!(
        s.list("standard", &words).unwrap()[0].status,
        STATUS_ADOPTED
    );
    assert!(
        s.delete(id, &words).unwrap().is_empty(),
        "ein selbst eingetragener Begriff bleibt beim Löschen"
    );
}

#[test]
fn a_rejected_pair_stays_rejected_and_never_adopts() {
    let s = store();
    feed(&s, false, &no_words());
    feed(&s, false, &no_words());
    let id = s.list("standard", &[]).unwrap()[0].id;
    let (row, delta) = s.set_status(id, STATUS_REJECTED, &no_words()).unwrap();
    assert_eq!(row.status, STATUS_REJECTED);
    assert!(delta.is_empty());
    let (out, delta) = feed(&s, true, &no_words());
    assert_eq!((out.suggestions, out.adopted), (0, 0));
    assert!(delta.is_empty());
    let row = &s.list("standard", &[]).unwrap()[0];
    assert_eq!(row.status, STATUS_REJECTED);
    assert_eq!(row.anzahl, 3);
}

#[test]
fn set_status_adopts_and_takes_back() {
    let s = store();
    feed(&s, false, &no_words());
    feed(&s, false, &no_words());
    let id = s.list("standard", &[]).unwrap()[0].id;
    let (row, delta) = s.set_status(id, STATUS_ADOPTED, &no_words()).unwrap();
    assert_eq!(row.status, STATUS_ADOPTED);
    assert_eq!(delta.add, vec!["Grundsteuer".to_string()]);
    let (_, again) = s
        .set_status(id, STATUS_ADOPTED, &["Grundsteuer".into()])
        .unwrap();
    assert!(again.is_empty(), "schon übernommen: nichts zu tun");
    let (row, delta) = s
        .set_status(id, STATUS_REJECTED, &["Grundsteuer".into()])
        .unwrap();
    assert_eq!(row.status, STATUS_REJECTED);
    assert_eq!(delta.remove, vec!["Grundsteuer".to_string()]);
    let (row, delta) = s.set_status(id, STATUS_SUGGESTED, &no_words()).unwrap();
    assert_eq!(row.status, STATUS_SUGGESTED);
    assert!(delta.is_empty());
}

#[test]
fn set_status_validates_its_input() {
    let s = store();
    feed(&s, false, &no_words());
    let id = s.list("standard", &[]).unwrap()[0].id;
    assert!(s.set_status(id, "beobachtet", &[]).is_err());
    assert!(s.set_status(id, "egal", &[]).is_err());
    assert!(s.set_status(id + 99, STATUS_ADOPTED, &[]).is_err());
}

#[test]
fn a_term_that_does_not_fit_the_word_list_cannot_be_adopted() {
    let s = store();
    let heard = "a".repeat(54);
    let corrected = "a".repeat(55);
    for _ in 0..2 {
        s.feedback(
            "standard",
            &format!("Das {heard} hier"),
            &format!("Das {corrected} hier"),
            true,
            &[],
        )
        .unwrap();
    }
    let row = &s.list("standard", &[]).unwrap()[0];
    assert_eq!(
        row.status, STATUS_SUGGESTED,
        "auto_apply übernimmt ihn nicht"
    );
    let err = s
        .set_status(row.id, STATUS_ADOPTED, &[])
        .unwrap_err()
        .to_string();
    assert!(!err.contains("aaaa"));
    assert_eq!(s.list("standard", &[]).unwrap()[0].status, STATUS_SUGGESTED);
}

#[test]
fn two_pairs_with_one_term_keep_it_until_the_last_is_gone() {
    let s = store();
    for _ in 0..2 {
        s.feedback(
            "standard",
            "Die Grund Steuer ist da",
            "Die Grundsteuer ist da",
            false,
            &[],
        )
        .unwrap();
    }
    // zweites Paar mit demselben Ziel
    for _ in 0..2 {
        s.feedback(
            "standard",
            "Die Grund Steuern ist da",
            "Die Grundsteuer ist da",
            false,
            &[],
        )
        .unwrap();
    }
    let list = s.list("standard", &[]).unwrap();
    assert_eq!(list.len(), 2, "{list:?}");
    let mut words: Vec<String> = Vec::new();
    for p in &list {
        let (_, d) = s.set_status(p.id, STATUS_ADOPTED, &words).unwrap();
        d.apply(&mut words);
    }
    assert_eq!(words, vec!["Grundsteuer"]);
    let d1 = s.delete(list[0].id, &words).unwrap();
    assert!(d1.is_empty(), "das andere Paar hält den Begriff noch");
    let d2 = s.delete(list[1].id, &words).unwrap();
    assert_eq!(d2.remove, vec!["Grundsteuer".to_string()]);
}

#[test]
fn delete_is_real_deletion() {
    let s = store();
    feed(&s, false, &no_words());
    let id = s.list("standard", &[]).unwrap()[0].id;
    s.delete(id, &[]).unwrap();
    assert!(s.list("standard", &[]).unwrap().is_empty());
    assert!(s.delete(id, &[]).is_err(), "gibt es nicht mehr");
    // Erneutes Lernen beginnt bei 1.
    feed(&s, false, &no_words());
    assert_eq!(s.list("standard", &[]).unwrap()[0].anzahl, 1);
}

#[test]
fn clear_removes_everything_in_one_bereich_and_what_it_added() {
    let s = store();
    for _ in 0..2 {
        feed(&s, true, &no_words());
    }
    let words = vec!["Grundsteuer".to_string(), "Eigenwort".to_string()];
    s.feedback("kasse", HEARD, FIXED, false, &words).unwrap();
    s.feedback(
        "standard",
        "Es gibt Wiederspruch hier",
        "Es gibt Widerspruch hier",
        false,
        &words,
    )
    .unwrap();
    let (count, delta) = s.clear("standard", &words).unwrap();
    assert_eq!(count, 2);
    assert_eq!(delta.remove, vec!["Grundsteuer".to_string()]);
    assert!(s.list("standard", &[]).unwrap().is_empty());
    assert_eq!(
        s.list("kasse", &[]).unwrap().len(),
        1,
        "andere Bereiche bleiben"
    );
    let mut w = words;
    delta.apply(&mut w);
    assert_eq!(w, vec!["Eigenwort"]);
}

#[test]
fn bereiche_are_kept_apart() {
    let s = store();
    s.feedback("standard", HEARD, FIXED, false, &[]).unwrap();
    s.feedback("standard", HEARD, FIXED, false, &[]).unwrap();
    s.feedback("bauamt", HEARD, FIXED, false, &[]).unwrap();
    assert_eq!(s.list("standard", &[]).unwrap()[0].anzahl, 2);
    assert_eq!(s.list("bauamt", &[]).unwrap()[0].anzahl, 1);
    assert!(s.list("leer", &[]).unwrap().is_empty());
}

#[test]
fn feedback_validates_bereich_and_length() {
    let s = store();
    assert!(s.feedback("Standard", HEARD, FIXED, false, &[]).is_err());
    assert!(s.feedback("", HEARD, FIXED, false, &[]).is_err());
    assert!(s.list("a b", &[]).is_err());
    assert!(s.clear("ä", &[]).is_err());
    let long = "wort ".repeat(500);
    assert!(long.chars().count() > MAX_TEXT_CHARS);
    let err = s
        .feedback("standard", &long, FIXED, false, &[])
        .unwrap_err()
        .to_string();
    assert!(!err.contains("wort"));
    assert!(s.feedback("standard", FIXED, &long, false, &[]).is_err());
    let at_limit = format!("{} ", "a").repeat(MAX_TEXT_CHARS / 2);
    assert!(at_limit.chars().count() <= MAX_TEXT_CHARS);
    assert!(s
        .feedback("standard", &at_limit, &at_limit, false, &[])
        .is_ok());
    assert!(s.list("standard", &[]).unwrap().is_empty());
}

#[test]
fn rewordings_and_nothing_store_nothing() {
    let s = store();
    let (out, d) = s
        .feedback(
            "standard",
            "Das Wetter ist heute schön",
            "Der Antrag wurde abgelehnt",
            true,
            &[],
        )
        .unwrap();
    assert_eq!((out.reason.as_str(), out.learned), ("rewrite", 0));
    assert!(d.is_empty());
    let (out, _) = s
        .feedback("standard", "hallo welt", "Hallo, Welt!", true, &[])
        .unwrap();
    assert_eq!((out.reason.as_str(), out.learned), ("nothing", 0));
    assert!(s.list("standard", &[]).unwrap().is_empty());
}

fn name(mut n: u32) -> String {
    let mut out = String::new();
    for _ in 0..6 {
        out.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    out
}

fn learn_n(s: &SpeechLearningStore, n: u32) -> FeedbackOutcome {
    let w = name(n);
    s.feedback(
        "standard",
        &format!("x {w}q y"),
        &format!("x {w} y"),
        false,
        &[],
    )
    .unwrap()
    .0
}

#[test]
fn an_area_holds_at_most_500_pairs_and_drops_the_oldest_suggestions() {
    let s = store();
    for n in 0..MAX_PAIRS {
        assert_eq!(learn_n(&s, n).learned, 1);
    }
    assert_eq!(s.list("standard", &[]).unwrap().len(), MAX_PAIRS as usize);
    assert_eq!(learn_n(&s, MAX_PAIRS).learned, 1);
    let list = s.list("standard", &[]).unwrap();
    assert_eq!(list.len(), MAX_PAIRS as usize, "nie mehr als 500");
    assert!(list.iter().any(|p| p.korrigiert == name(MAX_PAIRS)));
    assert!(
        !list.iter().any(|p| p.korrigiert == name(0)),
        "das älteste ist weggefallen"
    );
    // Ein anderer Bereich hat sein eigenes Fach.
    s.feedback("zweiter", "x abcdefq y", "x abcdef y", false, &[])
        .unwrap();
    assert_eq!(s.list("zweiter", &[]).unwrap().len(), 1);
    assert_eq!(s.list("standard", &[]).unwrap().len(), MAX_PAIRS as usize);
}

#[test]
fn a_full_area_of_kept_pairs_takes_no_more() {
    let s = store();
    let mut words: Vec<String> = Vec::new();
    for n in 0..MAX_PAIRS {
        learn_n(&s, n);
    }
    for p in s.list("standard", &[]).unwrap() {
        let (_, d) = s.set_status(p.id, STATUS_ADOPTED, &words).unwrap();
        d.apply(&mut words);
    }
    assert_eq!(learn_n(&s, MAX_PAIRS + 7).learned, 0, "nichts darf weichen");
    assert_eq!(s.list("standard", &[]).unwrap().len(), MAX_PAIRS as usize);
    assert_eq!(words.len(), MAX_PAIRS as usize);
}

#[test]
fn the_data_survives_reopening_the_file() {
    let dir = std::env::temp_dir().join(format!(
        "lva-speech-learning-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("speech_learning.db");
    {
        let s = SpeechLearningStore::open(&path).unwrap();
        feed(&s, false, &no_words());
    }
    {
        let s = SpeechLearningStore::open(&path).unwrap();
        let list = s.list("standard", &[]).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].gehoert, "Grund Steuer");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Einrichtung durch Vorlesen
// ---------------------------------------------------------------------------

#[test]
fn setup_text_has_about_six_sentences_and_is_synthetic() {
    let t = setup_text(&[]);
    assert_eq!(t.sentences.len(), 6);
    assert_eq!(t.text, t.sentences.join(" "));
    assert!(t.text.chars().count() < MAX_TEXT_CHARS);
    assert!(t.sentences.iter().all(|s| s.ends_with('.')));
}

#[test]
fn setup_text_adds_the_custom_words_once_and_cleaned() {
    let words: Vec<String> = [
        "ChargeBee",
        "chargebee",
        " Amtsblatt ",
        "",
        "bad<word>",
        "it's",
        &"x".repeat(51),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let t = setup_text(&words);
    assert_eq!(t.sentences.len(), 7);
    let last = t.sentences.last().unwrap();
    assert_eq!(last, "Außerdem noch einige Begriffe: ChargeBee, Amtsblatt.");
}

#[test]
fn setup_text_takes_at_most_twelve_words_and_stays_short() {
    let words: Vec<String> = (0..40)
        .map(|n| format!("{}{}", name(n), "z".repeat(40)))
        .collect();
    let t = setup_text(&words);
    let last = t.sentences.last().unwrap();
    assert_eq!(last.matches(", ").count(), MAX_WORDS_IN_TEXT - 1);
    assert!(t.text.chars().count() <= MAX_TEXT_CHARS);
}

fn aw() -> Vec<String> {
    vec!["Amtsblatt".to_string()]
}

fn reference() -> String {
    setup_text(&aw()).text
}

#[test]
fn reading_that_is_recognised_perfectly_yields_nothing() {
    let s = store();
    let r = reference();
    let out = s
        .setup_evaluate("standard", &r, &r.to_lowercase(), &aw())
        .unwrap();
    assert_eq!(out.reason, "nothing");
    assert_eq!(out.matched_percent, 100);
    assert!(out.pairs.is_empty());
}

#[test]
fn misrecognised_words_become_suggestions_at_once() {
    let s = store();
    let r = reference();
    let heard = r
        .replace("Grundsteuer", "Grund Steuer")
        .replace("Widerspruchsbelehrung", "Wiederspruchsbelehrung")
        .replace("Gemeinderats", "Gemeinde Rats")
        .replace("Amtsblatt", "Amtsplatt");
    let out = s.setup_evaluate("standard", &r, &heard, &aw()).unwrap();
    assert_eq!(out.reason, "ok");
    assert!(out.matched_percent >= 80, "{}", out.matched_percent);
    let mut got: Vec<(String, String)> = out
        .pairs
        .iter()
        .map(|p| (p.gehoert.clone(), p.korrigiert.clone()))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("Amtsplatt".to_string(), "Amtsblatt".to_string()),
            ("Gemeinde Rats".to_string(), "Gemeinderats".to_string()),
            ("Grund Steuer".to_string(), "Grundsteuer".to_string()),
            (
                "Wiederspruchsbelehrung".to_string(),
                "Widerspruchsbelehrung".to_string()
            ),
        ]
    );
    assert!(out
        .pairs
        .iter()
        .all(|p| p.status == STATUS_SUGGESTED && p.anzahl == MIN_COUNT));
    assert_eq!(s.list("standard", &[]).unwrap().len(), 4);
}

#[test]
fn reading_twice_does_not_adopt_anything_by_itself() {
    let s = store();
    let r = reference();
    let heard = r.replace("Grundsteuer", "Grund Steuer");
    s.setup_evaluate("standard", &r, &heard, &aw()).unwrap();
    let out = s.setup_evaluate("standard", &r, &heard, &aw()).unwrap();
    assert_eq!(out.pairs[0].anzahl, 3);
    assert_eq!(out.pairs[0].status, STATUS_SUGGESTED);
}

#[test]
fn reading_something_else_is_a_rewrite() {
    let s = store();
    let out = s
        .setup_evaluate(
            "standard",
            &reference(),
            "Das ist ganz etwas anderes als der Text",
            &aw(),
        )
        .unwrap();
    assert_eq!(out.reason, "rewrite");
    assert!(out.pairs.is_empty());
    let out = s
        .setup_evaluate("standard", &reference(), "", &aw())
        .unwrap();
    assert_eq!(out.reason, "rewrite");
    assert_eq!(out.matched_percent, 0);
    assert!(s.list("standard", &[]).unwrap().is_empty());
}

#[test]
fn setup_evaluate_validates_its_input() {
    let s = store();
    let long = "a ".repeat(1500);
    assert!(s.setup_evaluate("Standard", "a", "b", &[]).is_err());
    assert!(s.setup_evaluate("standard", &long, "b", &[]).is_err());
    assert!(s.setup_evaluate("standard", "a", &long, &[]).is_err());
}

// ---------------------------------------------------------------------------
// Wortschatz
// ---------------------------------------------------------------------------

#[test]
fn words_delta_adds_ignoring_case_and_removes_exactly() {
    let mut words = vec!["Eins".to_string(), "Zwei".to_string()];
    let delta = WordsDelta {
        add: vec!["zwei".to_string(), "Drei".to_string()],
        remove: vec!["Eins".to_string()],
    };
    assert!(delta.apply(&mut words));
    assert_eq!(words, vec!["Zwei", "Drei"]);
    let other_case = WordsDelta {
        add: vec![],
        remove: vec!["ZWEI".to_string()],
    };
    assert!(
        !other_case.apply(&mut words),
        "entfernt wird nur in genau dieser Schreibweise"
    );
    assert_eq!(words, vec!["Zwei", "Drei"]);
    assert!(!WordsDelta::default().apply(&mut words));
}

#[test]
fn custom_word_filter() {
    assert_eq!(
        custom_word("  Grundsteuer "),
        Some("Grundsteuer".to_string())
    );
    assert_eq!(
        custom_word("Gemeinde Rat"),
        Some("Gemeinde Rat".to_string())
    );
    assert_eq!(custom_word(""), None);
    assert_eq!(custom_word("a\"b"), None);
    assert_eq!(
        custom_word(&"x".repeat(MAX_WORD_CHARS)).map(|w| w.len()),
        Some(MAX_WORD_CHARS)
    );
    assert_eq!(custom_word(&"x".repeat(MAX_WORD_CHARS + 1)), None);
}

// ---------------------------------------------------------------------------
// Echtes Löschen
// ---------------------------------------------------------------------------

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lva-sl-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Alle Rohbytes der Datenbank und ihres Journals.
fn raw_bytes(dir: &std::path::Path) -> Vec<u8> {
    let mut all = Vec::new();
    for name in ["speech_learning.db", "speech_learning.db-wal"] {
        if let Ok(b) = std::fs::read(dir.join(name)) {
            all.extend(b);
        }
    }
    all
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    let n = needle.as_bytes();
    haystack.windows(n.len()).any(|w| w == n)
}

fn learn_twice(s: &SpeechLearningStore, heard: &str, fixed: &str) {
    for _ in 0..2 {
        s.feedback(
            "standard",
            &format!("Wir sehen {heard} heute"),
            &format!("Wir sehen {fixed} heute"),
            false,
            &[],
        )
        .unwrap();
    }
}

#[test]
fn clear_leaves_no_trace_in_the_files() {
    let dir = temp_dir("clear");
    let s = SpeechLearningStore::open(&dir.join("speech_learning.db")).unwrap();
    learn_twice(&s, "Zebrastreifenwort", "Zebrastreifenwortt");
    assert!(
        contains(&raw_bytes(&dir), "Zebrastreifenwort"),
        "Gegenprobe: vor dem Löschen steht das Wort in den Dateien"
    );
    s.clear("standard", &[]).unwrap();
    assert!(!contains(&raw_bytes(&dir), "Zebrastreifenwort"));
    drop(s);
    assert!(!contains(&raw_bytes(&dir), "Zebrastreifenwort"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_leaves_no_trace_in_the_files() {
    let dir = temp_dir("delete");
    let s = SpeechLearningStore::open(&dir.join("speech_learning.db")).unwrap();
    learn_twice(&s, "Quallenfrachtwort", "Quallenfrachtwortt");
    learn_twice(&s, "Stehengebliebenes", "Stehengebliebeness");
    let id = s
        .list("standard", &[])
        .unwrap()
        .into_iter()
        .find(|p| p.gehoert == "Quallenfrachtwort")
        .unwrap()
        .id;
    s.delete(id, &[]).unwrap();
    let raw = raw_bytes(&dir);
    assert!(!contains(&raw, "Quallenfrachtwort"));
    assert!(contains(&raw, "Stehengebliebenes"), "das andere bleibt");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_pair_dropped_by_the_limit_leaves_no_trace_either() {
    let dir = temp_dir("limit");
    let s = SpeechLearningStore::open(&dir.join("speech_learning.db")).unwrap();
    s.feedback(
        "standard",
        "x Aelteswortzz y",
        "x Aeltesworttt y",
        false,
        &[],
    )
    .unwrap();
    for n in 0..MAX_PAIRS - 1 {
        learn_n(&s, n);
    }
    assert!(contains(&raw_bytes(&dir), "Aelteswortzz"));
    learn_n(&s, MAX_PAIRS + 1);
    assert!(!s
        .list("standard", &[])
        .unwrap()
        .iter()
        .any(|p| p.gehoert == "Aelteswortzz"));
    assert!(!contains(&raw_bytes(&dir), "Aelteswortzz"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Rücknahme gegen den echten Wortschatz
// ---------------------------------------------------------------------------

fn adopted_store() -> (SpeechLearningStore, u32, Vec<String>) {
    let s = store();
    feed(&s, false, &no_words());
    feed(&s, false, &no_words());
    let id = s.list("standard", &[]).unwrap()[0].id;
    let mut words = vec!["Eigenwort".to_string()];
    let (_, d) = s.set_status(id, STATUS_ADOPTED, &words).unwrap();
    d.apply(&mut words);
    assert_eq!(words, vec!["Eigenwort", "Grundsteuer"]);
    (s, id, words)
}

#[test]
fn a_term_the_user_removed_is_forgotten_and_a_new_entry_is_never_taken_back() {
    let (s, id, _) = adopted_store();
    // Der Nutzer hat „Grundsteuer“ entfernt und später selbst neu eingetragen.
    let words = vec!["Eigenwort".to_string(), "Grundsteuer".to_string()];
    let without = vec!["Eigenwort".to_string()];
    s.list("standard", &without).unwrap();
    let d = s.set_status(id, STATUS_REJECTED, &words).unwrap().1;
    assert!(d.is_empty(), "das neu eingetragene Wort bleibt: {d:?}");
}

#[test]
fn listing_reconciles_before_any_takeback() {
    let (s, id, _) = adopted_store();
    s.list("standard", &["Eigenwort".to_string()]).unwrap();
    let d = s
        .delete(id, &["Eigenwort".to_string(), "Grundsteuer".to_string()])
        .unwrap();
    assert!(d.is_empty());
}

#[test]
fn takeback_removes_only_the_exact_spelling() {
    let (s, id, _) = adopted_store();
    let changed_case = vec!["Eigenwort".to_string(), "grundsteuer".to_string()];
    let d = s.set_status(id, STATUS_REJECTED, &changed_case).unwrap().1;
    assert!(
        d.is_empty(),
        "„grundsteuer“ ist ein anderes Wort als „Grundsteuer“"
    );
    let (s, id, words) = adopted_store();
    let d = s.set_status(id, STATUS_REJECTED, &words).unwrap().1;
    assert_eq!(d.remove, vec!["Grundsteuer".to_string()]);
    let mut w = words;
    assert!(d.apply(&mut w));
    assert_eq!(w, vec!["Eigenwort"]);
}

#[test]
fn clear_does_not_touch_a_term_the_user_removed_and_re_entered() {
    let (s, _, _) = adopted_store();
    s.list("standard", &["Eigenwort".to_string()]).unwrap();
    let (_, d) = s
        .clear(
            "standard",
            &["Eigenwort".to_string(), "Grundsteuer".to_string()],
        )
        .unwrap();
    assert!(d.is_empty());
}

// ---------------------------------------------------------------------------
// Vorlesen: Referenz und Menge
// ---------------------------------------------------------------------------

#[test]
fn a_reference_that_is_not_the_server_text_is_refused() {
    let s = store();
    let err = s
        .setup_evaluate(
            "standard",
            "Ein ganz anderer Vorlesetext",
            "irgendwas",
            &aw(),
        )
        .unwrap_err()
        .to_string();
    assert!(!err.contains("anderer"));
    let r = reference();
    assert!(
        s.setup_evaluate("standard", &r, &r, &[]).is_err(),
        "andere Begriffe, anderer Text"
    );
    assert!(s.setup_evaluate("standard", &r, &r, &aw()).is_ok());
}

#[test]
fn one_reading_creates_at_most_twelve_pairs() {
    let s = store();
    let words: Vec<String> = Vec::new();
    let base = setup_text(&words).text;
    let mut heard = base.clone();
    for w in [
        "Bürgermeister",
        "Verwaltung",
        "Bauantrag",
        "Kindertagesstätte",
        "Freitag",
        "Haushaltsplan",
        "Sanierung",
        "Grundschule",
        "Erneuerung",
        "Straßenbeleuchtung",
        "Ordnungsamt",
        "Gebührenbescheid",
        "Grundsteuer",
        "Widerspruchsbelehrung",
        "Gemeinderats",
        "Tagesordnung",
    ] {
        let broken = format!("{}x", &w[..w.len() - 1]);
        heard = heard.replace(w, &broken);
    }
    let out = s.setup_evaluate("standard", &base, &heard, &words).unwrap();
    assert_eq!(out.pairs.len(), MAX_SETUP_PAIRS);
}

// ---------------------------------------------------------------------------
// Normalisierung und Funktionswörter
// ---------------------------------------------------------------------------

#[test]
fn decomposed_umlauts_equal_composed_ones() {
    assert_eq!(compose("Mu\u{0308}ll Ao\u{0308}"), "Müll Aö");
    assert_eq!(compose("e\u{0308}"), "e\u{0308}", "nur deutsche Umlaute");
    assert_eq!(
        normalize_key("Mu\u{0308}llabfuhr"),
        normalize_key("Müllabfuhr")
    );
    let a = learn(
        "Die Mu\u{0308}labfuhr kommt heute",
        "Die Müllabfuhr kommt heute",
    );
    assert_eq!(a.pairs, vec![pair("Mülabfuhr", "Müllabfuhr")]);
    assert_eq!(
        learn("Die Mu\u{0308}llabfuhr kommt", "Die Müllabfuhr kommt").verdict,
        Verdict::Nothing
    );
}

#[test]
fn function_words_are_never_learned() {
    for (a, b) in [
        ("Das ist der Antrag hier", "Das ist die Antrag hier"),
        ("Er geht in den Park heute", "Er geht im Park heute"),
        ("Wir sehen ein Haus dort", "Wir sehen eine Haus dort"),
        ("Das gilt für mich sehr", "Das gilt für mir sehr"),
    ] {
        let r = learn(a, b);
        assert!(r.pairs.is_empty(), "{a} / {b}: {:?}", r.pairs);
    }
}

#[test]
fn function_words_at_the_edge_of_a_block_are_cut_off() {
    let a = learn(
        "Es geht um den Grund Steuer Fall",
        "Es geht um die Grundsteuer Fall",
    );
    assert_eq!(a.pairs, vec![pair("Grund Steuer", "Grundsteuer")]);
}

#[test]
fn a_term_with_a_function_word_cannot_be_adopted() {
    assert_eq!(custom_word("Grundsteuer"), Some("Grundsteuer".to_string()));
    assert_eq!(
        custom_word("Max Mustermann"),
        Some("Max Mustermann".to_string())
    );
    assert_eq!(custom_word("Haus und Hof"), None);
    assert_eq!(custom_word("der"), None);
}
