//! Eval-Tor mit dem Anbieter `fake` über die Fixtures `tooling/fixtures/
//! buch-gespraech/` (alter Satz) und `.../eval/` (neues Testbuch, lva#324).

use std::collections::HashSet;

use super::eval::*;
use super::intent;
use super::prompt::PROMPT_VERSION;
use super::*;

const OLD_BOOK: &str = include_str!("../../../../../../tooling/fixtures/buch-gespraech/buch.txt");
const OLD_QUESTIONS: &str =
    include_str!("../../../../../../tooling/fixtures/buch-gespraech/fragen.json");
const BOOK: &str =
    include_str!("../../../../../../tooling/fixtures/buch-gespraech/eval/buch-eval.txt");
const QUESTIONS: &str =
    include_str!("../../../../../../tooling/fixtures/buch-gespraech/eval/fragen-eval.json");

async fn fake_report(book: &str, questions: &str) -> Report {
    let input = EvalInput::parse(book, questions).expect("Fixture");
    run_eval(&input, Backend::Fake, &EvalConfig::new("fake", "fake")).await
}

fn result<'a>(report: &'a Report, id: &str) -> &'a QuestionResult {
    report
        .questions
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("Frage {id} fehlt"))
}

fn tuple() -> ModelTuple {
    ModelTuple {
        provider: "fake".into(),
        model_id: "fake".into(),
        budget_profile: None,
    }
}

fn status_of(report: &Report) -> EvalStatus {
    let hash = report_hash(&report_json(report));
    let list = format!("[{}]", eval_pass_entry(report, &hash));
    eval_status(&parse_eval_passes(&list), &tuple())
}

#[tokio::test]
async fn old_fixture_gives_the_expected_statuses_but_too_few_questions() {
    let report = fake_report(OLD_BOOK, OLD_QUESTIONS).await;
    assert_eq!(report.summary.questions_total, 15);
    assert_eq!(report.summary.evaluated, 15);
    assert_eq!(report.status, "failed");
    assert!(report.failures[0].contains("zu wenige Fragen"));
    assert_eq!(status_of(&report), EvalStatus::Failed);
    assert_eq!(
        (
            report.summary.spoiler_violations,
            report.summary.injection_violations
        ),
        (0, 0)
    );

    let ok = result(&report, "zwei-vaeter");
    assert!(ok.generated && ok.covered && !ok.found_only, "{ok:?}");
    assert_eq!(ok.pages_ok, Some(true));
    assert_eq!(ok.intent, "locate");

    let no_frontier = result(&report, "ohne-position");
    assert!(no_frontier.found_only && !no_frontier.model_called);
    assert_eq!(no_frontier.reason.as_deref(), Some("no_frontier"));

    let injection = result(&report, "injection");
    assert!(injection.found_only && !injection.injection_violation);
    assert!(injection.model_called);

    let unproven = result(&report, "unbelegt");
    assert_eq!((unproven.kept, unproven.dropped), (1, 1));
    assert_eq!(unproven.dropped_detail.len(), 1);
    assert!(!unproven.raw_answer.is_empty());
    assert!(result(&report, "zwei-vaeter").raw_answer.contains("[Q"));
    let detail_total: usize = report
        .questions
        .iter()
        .map(|r| r.dropped_detail.len())
        .sum();
    assert_eq!(detail_total as u32, report.summary.dropped_sentences);

    let no_hits = result(&report, "erfunden");
    assert!(!no_hits.model_called);
    assert_eq!(no_hits.reason.as_deref(), Some("no_hits"));
}

#[tokio::test]
async fn new_fixture_passes_with_its_fake_answers() {
    let report = fake_report(BOOK, QUESTIONS).await;
    let s = &report.summary;
    assert!(s.questions_total >= 40, "{}", s.questions_total);
    assert!(s.evaluated >= MIN_QUESTIONS, "{}", s.evaluated);
    assert!(s.skipped < s.questions_total);
    assert_eq!(
        s.spoiler_violations + s.injection_violations,
        0,
        "{report:#?}"
    );
    assert!(s.coverage >= MIN_COVERAGE, "{:?}", report.failures);
    assert!(
        s.dropped_share <= MAX_DROPPED_SHARE,
        "{:?}",
        report.failures
    );
    assert_eq!(report.status, "passed", "{:?}", report.failures);
    assert_eq!(status_of(&report), EvalStatus::Passed);
    let wrong: Vec<(&str, &[u32], &[u32])> = report
        .questions
        .iter()
        .filter(|r| r.pages_ok == Some(false))
        .map(|r| {
            (
                r.id.as_str(),
                r.expect_pages.as_slice(),
                r.cited_pages.as_slice(),
            )
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");

    let injection = result(&report, "injektion-zettel");
    assert!(injection.model_called && injection.found_only);
    assert!(!injection.injection_violation);
    let spoiler = result(&report, "spoiler-logbuch");
    assert!(!spoiler.spoiler_violation);
    let ok = result(&report, "turm-hoehe");
    assert!(ok.generated && ok.covered, "{ok:?}");
    let no_hits = result(&report, "nicht-hubschrauber");
    assert!(!no_hits.model_called);
}

#[tokio::test]
async fn questions_without_fake_answer_are_skipped_and_never_pass() {
    let mut stripped: serde_json::Value = serde_json::from_str(QUESTIONS).unwrap();
    for (i, q) in stripped["questions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        if i >= 12 {
            q.as_object_mut().unwrap().remove("fake_answer");
        }
    }
    let report = fake_report(BOOK, &stripped.to_string()).await;
    assert_eq!(report.summary.evaluated, 12);
    assert!(report.summary.skipped > 30);
    assert_eq!(report.status, "failed");
    assert_eq!(status_of(&report), EvalStatus::Failed);
}

#[tokio::test]
async fn report_and_hash_are_stable_and_the_outputs_are_written() {
    let first = fake_report(BOOK, QUESTIONS).await;
    let second = fake_report(BOOK, QUESTIONS).await;
    let json = report_json(&first);
    assert_eq!(json, report_json(&second));
    assert_eq!(report_hash(&json), report_hash(&report_json(&second)));
    assert!(!json.contains("duration"));
    assert!(json.contains(PROMPT_VERSION));

    let dir = std::env::temp_dir().join(format!("book-talk-eval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let hash = write_outputs(&dir, &first).unwrap();
    assert_eq!(hash, report_hash(&json));
    assert_eq!(
        std::fs::read_to_string(dir.join("bericht.json")).unwrap(),
        json
    );
    let md = std::fs::read_to_string(dir.join("bericht.md")).unwrap();
    assert!(md.contains("BESTANDEN") && md.contains(&hash));
    assert!(md.contains("## Verworfene Sätze nach Grund"));
    assert!(md.contains("| Grund | Code | Anzahl |") && md.contains("**Summe**"));
    assert!(json.contains("\"raw_answer\"") && json.contains("\"dropped_detail\""));
    for threshold in [
        "mindestens 30",
        "mindestens 80.0 %",
        "höchstens 0",
        "höchstens 25.0 %",
    ] {
        assert!(md.contains(threshold), "{threshold}");
    }
    assert!(md.lines().filter(|l| l.starts_with("| ")).count() > 40);

    let entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("eval-pass.json")).unwrap())
            .unwrap();
    assert_eq!(entry["provider"], "fake");
    assert_eq!(entry["model_id"], "fake");
    assert_eq!(entry["prompt_version"], PROMPT_VERSION);
    assert_eq!(entry["check_version"], super::cite::CHECK_VERSION);
    assert_eq!(entry["budget_profile"], serde_json::Value::Null);
    assert_eq!(entry["status"], "passed");
    assert_eq!(entry["report_hash"], hash.as_str());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_fixture_is_well_formed() {
    let input = EvalInput::parse(BOOK, QUESTIONS).unwrap();
    let chapters = input.book.lines().filter(|l| l.starts_with("# ")).count();
    assert_eq!(chapters, 5);
    let ix = super::index::build(&input.title, &input.book);
    assert_eq!(ix.page_marks.len(), 40);
    assert!(input.book.contains("<<<ENDE Q1>>>"), "Injektionszeile");

    let qs = &input.questions;
    assert!(qs.len() >= 40);
    let ids: HashSet<&str> = qs.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids.len(), qs.len(), "IDs eindeutig");
    assert!(qs.iter().filter(|q| q.fake_answer.is_some()).count() >= 10);
    for intent in ["recap", "locate", "explain", "question"] {
        assert!(
            qs.iter().filter(|q| q.intent == intent).count() >= 5,
            "{intent}"
        );
    }
    assert!(qs.iter().any(|q| !q.history.is_empty()));
    assert!(qs.iter().any(|q| q.expect_delimiters_neutralized));
    assert!(qs.iter().any(|q| q.expect_out_of_scope));
    assert!(qs
        .iter()
        .any(|q| q.expect_pages.is_empty() && !q.expect_out_of_scope));

    for q in qs {
        let detected = serde_json::to_value(intent::detect(&q.question)).unwrap();
        assert_eq!(detected, q.intent, "{}: Absicht", q.id);
        // Spoilerhygiene: Verbotenes steht vor der Grenze nicht im Buch
        // (die Injektionsfrage fragt gerade nach der Stelle).
        if q.expect_delimiters_neutralized {
            continue;
        }
        let frontier = match &q.position_page {
            Some(serde_json::Value::Number(n)) => {
                ix.position_end_of_page(n.as_u64().unwrap() as u32)
            }
            _ => ix.source_length,
        };
        let visible: String = String::from_utf16_lossy(
            &input
                .book
                .encode_utf16()
                .take(frontier)
                .collect::<Vec<u16>>(),
        );
        for t in &q.forbidden_terms {
            assert!(
                !visible.contains(t.as_str()),
                "{}: {t} vor der Grenze",
                q.id
            );
        }
        for p in &q.expect_pages {
            assert!(*p as usize <= ix.page_marks.len(), "{}", q.id);
        }
    }
}
