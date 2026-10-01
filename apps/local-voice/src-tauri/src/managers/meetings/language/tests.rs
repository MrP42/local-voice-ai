use super::*;

const DE: &str = "Wir sprechen heute über den Lastgang der Stadtwerke und danach über den Speicher, \
den wir im nächsten Jahr beschaffen wollen, damit die Spitzen am Montagmorgen flacher werden.";
const EN: &str = "Today we are going to talk about the load profile of the municipal utility and \
afterwards about the storage system that we want to buy next year so that the peaks flatten out.";
const FR: &str = "Aujourd'hui nous allons parler du profil de charge de la régie municipale et ensuite \
du stockage que nous voulons acheter l'année prochaine afin d'aplatir les pics du lundi matin.";
const ES: &str = "Hoy vamos a hablar del perfil de carga de la empresa municipal y después del \
almacenamiento que queremos comprar el año que viene para que los picos del lunes se aplanen.";

fn model(id: &str, langs: &[&str], downloaded: bool, size_mb: u64, accuracy: u32) -> ModelCandidate {
    ModelCandidate {
        id: id.to_string(),
        name: id.to_string(),
        languages: langs.iter().map(|l| (*l).to_string()).collect(),
        downloaded,
        size_mb,
        accuracy,
    }
}

fn parakeet_v3(downloaded: bool) -> ModelCandidate {
    model(
        "parakeet-v3",
        &[
            "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv",
            "lt", "mt", "pl", "pt", "ro", "ru", "sk", "sl", "es", "sv", "uk",
        ],
        downloaded,
        700,
        88,
    )
}

fn qwen(downloaded: bool) -> ModelCandidate {
    model(
        "qwen3-asr-1.7b",
        &[
            "zh", "en", "yue", "ar", "de", "fr", "es", "pt", "id", "it", "ko", "ru", "th", "vi",
            "ja", "tr", "hi", "ms", "nl", "sv", "da", "fi", "pl", "cs", "fil", "fa", "el", "ro",
            "hu", "mk",
        ],
        downloaded,
        1500,
        90,
    )
}

fn english_only(downloaded: bool) -> ModelCandidate {
    model("parakeet-v2-en", &["en"], downloaded, 600, 91)
}

fn whisper_turbo(downloaded: bool) -> ModelCandidate {
    let langs: Vec<&str> = LANGUAGES.iter().map(|(c, _, _)| *c).collect();
    model("whisper-large-v3-turbo", &langs, downloaded, 880, 88)
}

// ---- Codes und Namen ------------------------------------------------------------

#[test]
fn codes_are_reduced_to_the_base_language() {
    assert_eq!(normalize_code("en-US").as_deref(), Some("en"));
    assert_eq!(normalize_code(" DE ").as_deref(), Some("de"));
    assert_eq!(normalize_code("zh_Hans").as_deref(), Some("zh"));
    assert_eq!(normalize_code("auto"), None);
    assert_eq!(normalize_code(""), None);
    assert_eq!(normalize_code("english"), None);
    assert_eq!(normalize_code("e1"), None);
}

#[test]
fn language_names_come_in_german_and_english() {
    assert_eq!(name_de("en"), "Englisch");
    assert_eq!(name_en("en"), "English");
    assert_eq!(name_de("de-DE"), "Deutsch");
    assert_eq!(name_en("fr"), "French");
    // Unbekannt: der Code, nie ein Fehler.
    assert_eq!(name_de("xx"), "xx");
}

// ---- Stufe 2: Text ----------------------------------------------------------------

#[test]
fn a_text_gets_its_language() {
    for (text, code) in [(DE, "de"), (EN, "en"), (FR, "fr"), (ES, "es")] {
        let detection = detect_text(text).unwrap_or_else(|| panic!("keine Sprache fuer {code}"));
        assert_eq!(detection.code, code);
        assert!(detection.confidence >= MIN_TEXT_CONFIDENCE);
    }
}

#[test]
fn a_short_text_gets_no_language_instead_of_a_guess() {
    assert_eq!(detect_text("Okay, gut."), None);
    assert_eq!(detect_text(""), None);
    assert_eq!(detect_text("12345 67890 12345 67890 12345 67890 12345 67890 12345"), None);
}

// ---- Stufe 1: Probenfenster --------------------------------------------------------

fn tone(ms: usize, amplitude: i16) -> Vec<i16> {
    (0..ms * 16)
        .map(|i| {
            // Rechteck mit 200 Hz: gleichmaessig laut, ohne Zufall.
            if (i / 40) % 2 == 0 {
                amplitude
            } else {
                -amplitude
            }
        })
        .collect()
}

#[test]
fn silence_gives_no_probe_window() {
    assert!(sample_windows(&vec![0i16; 16_000 * 60]).is_empty());
    assert!(sample_windows(&[]).is_empty());
}

#[test]
fn a_short_recording_is_one_window() {
    let windows = sample_windows(&tone(8_000, 3_000));
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].len(), 8_000 * 16);
}

#[test]
fn a_long_recording_gives_one_loud_window_per_third() {
    // 90 s: leise, ausser einem lauten Stueck mitten in jedem Drittel.
    let mut samples = vec![200i16; 16_000 * 90];
    for third in 0..3usize {
        let start = (third * 30 + 8) * 16_000;
        let loud = tone(PROBE_WINDOW_MS, 9_000);
        samples[start..start + loud.len()].copy_from_slice(&loud);
    }
    let windows = sample_windows(&samples);
    assert_eq!(windows.len(), PROBE_WINDOWS);
    for window in &windows {
        assert_eq!(window.len(), PROBE_WINDOW_MS * 16);
        let loudness = window.iter().map(|s| f64::from(s.abs())).sum::<f64>() / window.len() as f64;
        assert!(loudness > 0.1, "ein leises Fenster wurde gewaehlt: {loudness}");
    }
}

#[test]
fn silent_thirds_are_left_out() {
    let mut samples = vec![0i16; 16_000 * 90];
    let loud = tone(PROBE_WINDOW_MS, 9_000);
    samples[5 * 16_000..5 * 16_000 + loud.len()].copy_from_slice(&loud);
    assert_eq!(sample_windows(&samples).len(), 1);
}

// ---- Entscheidung der Probe -----------------------------------------------------------

#[test]
fn the_models_own_detection_wins_by_majority() {
    let result = ProbeResult {
        native: vec![Some("en".into()), Some("en-US".into()), Some("de".into())],
        text: DE.to_string(),
    };
    let (code, source, confidence) = decide_probe(&result).unwrap();
    assert_eq!((code.as_str(), source), ("en", SOURCE_PROBE));
    assert!((confidence - 2.0 / 3.0).abs() < 1e-9);
}

#[test]
fn without_a_native_detection_the_probe_text_decides() {
    let result = ProbeResult {
        native: vec![None, None],
        text: EN.to_string(),
    };
    let (code, source, _) = decide_probe(&result).unwrap();
    assert_eq!((code.as_str(), source), ("en", SOURCE_TEXT));
}

#[test]
fn an_empty_probe_decides_nothing() {
    assert_eq!(decide_probe(&ProbeResult::default()), None);
    let unclear = ProbeResult {
        native: vec![None],
        text: "ja".to_string(),
    };
    assert_eq!(decide_probe(&unclear), None);
}

// ---- Modellwahl ---------------------------------------------------------------------------

#[test]
fn german_keeps_the_default_model() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let choice = choose_model(Some("de"), &catalog[0], false, &catalog);
    assert_eq!(choice.id, "parakeet-v3");
    assert_eq!(choice.reason, ModelReason::Covers);
    assert_eq!(choice.covers, Some(true));
}

#[test]
fn another_language_keeps_a_default_that_covers_it() {
    // Parakeet v3 kann 25 europaeische Sprachen: kein Modellwechsel fuer Englisch.
    let catalog = [parakeet_v3(true), qwen(true)];
    let choice = choose_model(Some("en"), &catalog[0], false, &catalog);
    assert_eq!(choice.id, "parakeet-v3");
    assert_eq!(choice.reason, ModelReason::Covers);
}

#[test]
fn an_english_only_default_is_replaced_by_the_best_installed_multilingual_model() {
    // Standard = reines Englisch-Modell, Aufnahme Deutsch: das installierte
    // mehrsprachige Modell, nicht das nur-englische (obwohl dessen Katalogwert hoeher ist).
    let catalog = [english_only(true), parakeet_v3(true), qwen(true), whisper_turbo(false)];
    let choice = choose_model(Some("de"), &catalog[0], false, &catalog);
    assert_eq!(choice.reason, ModelReason::Switched);
    assert_eq!(choice.id, "qwen3-asr-1.7b", "mehrsprachig und genauer");
    assert_eq!(choice.covers, Some(true));
}

#[test]
fn a_language_the_default_lacks_goes_to_an_installed_model_that_has_it() {
    // Japanisch: Parakeet v3 kennt es nicht, Qwen3-ASR schon.
    let catalog = [parakeet_v3(true), qwen(true)];
    let choice = choose_model(Some("ja"), &catalog[0], false, &catalog);
    assert_eq!(choice.id, "qwen3-asr-1.7b");
    assert_eq!(choice.reason, ModelReason::Switched);
}

#[test]
fn a_not_installed_model_is_never_chosen_only_suggested() {
    let catalog = [parakeet_v3(true), qwen(false), whisper_turbo(false)];
    let choice = choose_model(Some("ja"), &catalog[0], false, &catalog);
    assert_eq!(choice.id, "parakeet-v3", "das geladene bleibt: nichts wird still geladen");
    assert_eq!(choice.reason, ModelReason::NoneInstalled);
    assert_eq!(choice.covers, Some(false));
    // Qwen3-ASR (90) schlaegt Whisper (88) bei gleich mehrsprachiger Abdeckung.
    assert_eq!(choice.suggestion.as_deref(), Some("qwen3-asr-1.7b"));
}

#[test]
fn the_users_model_always_wins() {
    // Auch wenn es die Sprache nicht kann: die Wahl bleibt, der Vorschlag steht dabei.
    let user = english_only(true);
    let catalog = [user.clone(), qwen(true), parakeet_v3(true)];
    let choice = choose_model(Some("de"), &user, true, &catalog);
    assert_eq!(choice.id, "parakeet-v2-en");
    assert_eq!(choice.reason, ModelReason::UserChoice);
    assert_eq!(choice.covers, Some(false));
    assert_eq!(choice.suggestion.as_deref(), Some("qwen3-asr-1.7b"));
}

#[test]
fn the_users_model_that_covers_the_language_has_no_suggestion() {
    let user = parakeet_v3(true);
    let catalog = [user.clone(), qwen(true)];
    let choice = choose_model(Some("en"), &user, true, &catalog);
    assert_eq!(choice.reason, ModelReason::UserChoice);
    assert_eq!(choice.covers, Some(true));
    assert_eq!(choice.suggestion, None);
}

#[test]
fn an_unknown_language_changes_nothing() {
    let catalog = [parakeet_v3(true), qwen(true)];
    for language in [None, Some("auto"), Some("")] {
        let choice = choose_model(language, &catalog[0], false, &catalog);
        assert_eq!(choice.id, "parakeet-v3");
        assert_eq!(choice.reason, ModelReason::Unknown);
    }
}

#[test]
fn a_model_without_known_languages_is_assumed_to_fit() {
    let unknown = model("custom", &[], true, 500, 50);
    let catalog = [unknown.clone(), qwen(true)];
    let choice = choose_model(Some("ja"), &unknown, false, &catalog);
    assert_eq!(choice.id, "custom");
    assert_eq!(choice.reason, ModelReason::Covers);
    assert_eq!(choice.covers, None);
}

#[test]
fn regional_codes_in_the_catalog_still_match() {
    let regional = model("nemotron", &["en-US", "de-DE", "fr-FR"], true, 600, 80);
    assert_eq!(regional.covers("en"), Some(true));
    assert_eq!(regional.covers("ja"), Some(false));
}

// ---- Gueltige Sprache nach dem Lauf ---------------------------------------------------------

fn detection(code: &str, confidence: f64) -> TextDetection {
    TextDetection {
        code: code.to_string(),
        confidence,
    }
}

#[test]
fn a_user_choice_stays_even_when_the_text_disagrees() {
    let stored = settle(
        Some(("de".to_string(), SOURCE_USER, 1.0)),
        Some(detection("en", 0.99)),
        Some("m".into()),
    )
    .unwrap();
    assert_eq!(stored.code, "de");
    assert_eq!(stored.source, SOURCE_USER);
    assert_eq!(stored.mismatch.as_deref(), Some("en"), "der Widerspruch wird gemeldet");
    assert_eq!(stored.forced, None);
}

#[test]
fn a_fixed_setting_is_corrected_by_a_strong_contradicting_text() {
    // Einstellung "de", Parakeet hat Englisch geliefert: das Transkript IST Englisch.
    let stored = settle(
        Some(("de".to_string(), SOURCE_SETTING, 1.0)),
        Some(detection("en", 0.95)),
        None,
    )
    .unwrap();
    assert_eq!(stored.code, "en");
    assert_eq!(stored.source, SOURCE_TEXT);
    assert_eq!(stored.forced.as_deref(), Some("de"));
}

#[test]
fn a_weak_contradiction_does_not_override_the_setting() {
    let stored = settle(
        Some(("de".to_string(), SOURCE_SETTING, 1.0)),
        Some(detection("nl", 0.6)),
        None,
    )
    .unwrap();
    assert_eq!(stored.code, "de");
    assert_eq!(stored.source, SOURCE_SETTING);
    assert_eq!(stored.mismatch, None);
}

#[test]
fn a_probe_result_stays_and_a_disagreement_is_noted() {
    let stored = settle(
        Some(("en".to_string(), SOURCE_PROBE, 1.0)),
        Some(detection("de", 0.9)),
        None,
    )
    .unwrap();
    assert_eq!(stored.code, "en");
    assert_eq!(stored.mismatch.as_deref(), Some("de"));
}

#[test]
fn without_a_plan_the_text_decides_or_nothing() {
    let stored = settle(None, Some(detection("fr", 0.8)), None).unwrap();
    assert_eq!((stored.code.as_str(), stored.source.as_str()), ("fr", SOURCE_TEXT));
    assert_eq!(settle(None, None, None), None);
}

#[test]
fn the_stored_language_round_trips_through_json_without_empty_fields() {
    let stored = StoredLanguage {
        code: "en".into(),
        source: SOURCE_PROBE.into(),
        confidence: Some(1.0),
        forced: None,
        mismatch: None,
        model_id: Some("m".into()),
    };
    let json = serde_json::to_string(&stored).unwrap();
    assert!(!json.contains("forced") && !json.contains("mismatch"));
    assert_eq!(serde_json::from_str::<StoredLanguage>(&json).unwrap(), stored);
}

// ---- Entscheidung eines Laufs ------------------------------------------------------------------

fn input<'a>(
    user_language: Option<&'a str>,
    setting: &'a str,
    current: &'a ModelCandidate,
    user_model: bool,
    catalog: &'a [ModelCandidate],
) -> RunInput<'a> {
    RunInput {
        user_language,
        setting_language: setting,
        current,
        user_model,
        catalog,
    }
}

fn english_probe() -> ProbeResult {
    ProbeResult {
        native: vec![Some("en".into()), Some("en".into())],
        text: EN.to_string(),
    }
}

#[test]
fn the_probe_runs_only_when_nothing_fixes_the_language() {
    assert!(needs_probe(None, "auto"));
    assert!(needs_probe(None, ""));
    assert!(!needs_probe(Some("en"), "auto"), "Wahl im Dialog");
    assert!(!needs_probe(None, "de"), "feste Einstellung");
    assert!(needs_probe(Some("auto"), "de"), "'auto' im Dialog heisst erkennen");
}

#[test]
fn a_probe_result_decides_the_language_and_the_model_covers_it() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let decision = decide_run(
        &input(None, "auto", &catalog[0], false, &catalog),
        Some(&english_probe()),
    );
    let (code, source, _) = decision.planned.clone().unwrap();
    assert_eq!((code.as_str(), source), ("en", SOURCE_PROBE));
    assert_eq!(decision.choice.id, "parakeet-v3", "Parakeet v3 deckt Englisch ab: kein Wechsel");
    assert_eq!(decision.force.as_deref(), Some("en"), "die erkannte Sprache wird fuer den Lauf festgelegt");
}

#[test]
fn a_detected_language_the_default_lacks_switches_to_an_installed_multilingual_model() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let japanese = ProbeResult {
        native: vec![Some("ja".into())],
        text: String::new(),
    };
    let decision = decide_run(&input(None, "auto", &catalog[0], false, &catalog), Some(&japanese));
    assert_eq!(decision.choice.reason, ModelReason::Switched);
    assert_eq!(decision.choice.id, "qwen3-asr-1.7b");
    assert_eq!(decision.force.as_deref(), Some("ja"));
}

#[test]
fn a_fixed_setting_is_used_without_a_probe_and_marked_as_setting() {
    let catalog = [parakeet_v3(true)];
    let decision = decide_run(&input(None, "de", &catalog[0], false, &catalog), None);
    let (code, source, _) = decision.planned.unwrap();
    assert_eq!((code.as_str(), source), ("de", SOURCE_SETTING));
    assert_eq!(decision.force.as_deref(), Some("de"));
}

#[test]
fn the_choice_in_the_dialog_beats_the_setting_and_the_probe() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let decision = decide_run(
        &input(Some("fr"), "de", &catalog[0], false, &catalog),
        Some(&english_probe()),
    );
    let (code, source, _) = decision.planned.unwrap();
    assert_eq!((code.as_str(), source), ("fr", SOURCE_USER));
}

#[test]
fn the_users_model_stays_even_for_a_language_it_lacks_and_the_language_is_not_forced() {
    let user = english_only(true);
    let catalog = [user.clone(), qwen(true)];
    let decision = decide_run(&input(Some("de"), "auto", &user, true, &catalog), None);
    assert_eq!(decision.choice.id, "parakeet-v2-en");
    assert_eq!(decision.choice.reason, ModelReason::UserChoice);
    assert_eq!(decision.choice.covers, Some(false));
    assert_eq!(decision.force, None, "ein Modell ohne die Sprache bekommt sie nicht aufgezwungen");
}

#[test]
fn without_a_result_nothing_is_forced_and_nothing_switches() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let decision = decide_run(&input(None, "auto", &catalog[0], false, &catalog), None);
    assert_eq!(decision.planned, None);
    assert_eq!(decision.choice.reason, ModelReason::Unknown);
    assert_eq!(decision.force, None);
    // Eine Probe ohne Ergebnis (zu kurz, Stille) ist dasselbe.
    let decision = decide_run(
        &input(None, "auto", &catalog[0], false, &catalog),
        Some(&ProbeResult::default()),
    );
    assert_eq!(decision.planned, None);
}

// ---- Anzeige im Kopf ------------------------------------------------------------------------------

#[test]
fn the_info_for_the_chip_names_language_origin_and_a_better_model() {
    let catalog = [english_only(true), qwen(true)];
    let stored = StoredLanguage {
        code: "de".into(),
        source: SOURCE_PROBE.into(),
        confidence: Some(1.0),
        forced: None,
        mismatch: None,
        model_id: Some("parakeet-v2-en".into()),
    };
    let info = build_info(Some(&stored), None, Some("parakeet-v2-en"), &catalog);
    assert_eq!(info.code.as_deref(), Some("de"));
    assert_eq!(info.source.as_deref(), Some(SOURCE_PROBE));
    assert_eq!(info.model_covers, Some(false));
    let suggestion = info.suggestion.unwrap();
    assert_eq!(suggestion.model_id, "qwen3-asr-1.7b");
    assert!(suggestion.downloaded);
}

#[test]
fn the_info_suggests_a_not_installed_model_when_none_installed_fits() {
    let catalog = [english_only(true), qwen(false)];
    let info = build_info(None, Some("ja"), Some("parakeet-v2-en"), &catalog);
    assert_eq!(info.code.as_deref(), Some("ja"));
    assert_eq!(info.source, None, "ohne gespeicherte Herkunft keine Behauptung");
    let suggestion = info.suggestion.unwrap();
    assert_eq!(suggestion.model_id, "qwen3-asr-1.7b");
    assert!(!suggestion.downloaded);
}

#[test]
fn the_info_without_a_language_or_with_a_fitting_model_has_no_suggestion() {
    let catalog = [parakeet_v3(true), qwen(true)];
    let none = build_info(None, None, Some("parakeet-v3"), &catalog);
    assert_eq!((none.code, none.suggestion), (None, None));
    let fitting = build_info(None, Some("en"), Some("parakeet-v3"), &catalog);
    assert_eq!(fitting.model_covers, Some(true));
    assert_eq!(fitting.suggestion, None);
}

#[test]
fn the_info_takes_the_meeting_column_when_nothing_was_stored() {
    let catalog = [parakeet_v3(true)];
    let info = build_info(None, Some("en-US"), None, &catalog);
    assert_eq!(info.code.as_deref(), Some("en"), "Regionalcode auf die Sprache gekuerzt");
    assert_eq!(info.model_covers, None, "kein Modell bekannt: keine Behauptung");
}
