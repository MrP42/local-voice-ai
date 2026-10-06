use super::*;

fn provider(id: &str, base_url: &str) -> PostProcessProvider {
    PostProcessProvider {
        id: id.into(),
        label: id.into(),
        base_url: base_url.into(),
        allow_base_url_edit: false,
        models_endpoint: None,
        supports_structured_output: false,
    }
}

fn facts(eu: Tri, dpf: Tri, trains: Tri, dpa: Tri, checked: &'static str) -> ProviderFacts {
    ProviderFacts {
        provider: "x",
        host: None,
        countries: &["US"],
        eu_processing: eu,
        dpf_certified: dpf,
        trains_on_data: trains,
        dpa,
        training_opt_out_possible: false,
        sources: &["https://example.org"],
        checked,
    }
}

#[test]
fn local_providers_are_always_allowed_and_not_cloud() {
    for p in [
        provider("local", "http://127.0.0.1:0/v1"),
        provider("ollama", "http://localhost:11434/v1"),
        provider("vllm", "http://127.0.0.1:8000/v1"),
    ] {
        for profile in [
            ComplianceProfile::Eu,
            ComplianceProfile::LocalOnly,
            ComplianceProfile::None,
        ] {
            let a = assess(profile, &p, false, "2026-10-06");
            assert_eq!(a.verdict, Verdict::Allowed, "{} {profile:?}", p.id);
            assert!(!a.cloud);
        }
    }
}

#[test]
fn local_only_blocks_every_cloud_provider() {
    let a = assess(
        ComplianceProfile::LocalOnly,
        &provider("openai", "https://api.openai.com/v1"),
        false,
        "2026-10-06",
    );
    assert_eq!(a.verdict, Verdict::Blocked);
    assert!(a.cloud);
    assert_eq!(a.reasons, vec!["cloud_not_allowed"]);
}

#[test]
fn no_restriction_allows_cloud_but_marks_it() {
    let a = assess(
        ComplianceProfile::None,
        &provider("groq", "https://api.groq.com/openai/v1"),
        false,
        "2026-10-06",
    );
    assert_eq!(a.verdict, Verdict::Allowed);
    assert!(a.cloud);
}

#[test]
fn eu_blocks_providers_without_documented_facts() {
    let a = assess(
        ComplianceProfile::Eu,
        &provider("unbekannt", "https://llm.example.com/v1"),
        false,
        "2026-10-06",
    );
    assert_eq!(a.verdict, Verdict::Blocked);
    assert_eq!(a.reasons, vec!["facts_unknown"]);
}

#[test]
fn eu_rule_matrix() {
    use Tri::*;
    let today = "2026-10-06";
    // EU-Verarbeitung, kein Training, AVV: erlaubt.
    assert_eq!(
        eu_verdict(&facts(Yes, Unknown, No, Yes, today), false, today).0,
        Verdict::Allowed
    );
    // Drittland mit DPF: erlaubt mit Bedingung.
    let (v, r) = eu_verdict(&facts(No, Yes, No, Yes, today), false, today);
    assert_eq!(
        (v, r),
        (Verdict::Conditional, vec!["third_country_dpf".to_string()])
    );
    // Drittland ohne DPF: gesperrt.
    assert_eq!(
        eu_verdict(&facts(No, No, No, Yes, today), false, today).0,
        Verdict::Blocked
    );
    // Training mit den Daten: gesperrt, auch in der EU.
    let (v, r) = eu_verdict(&facts(Yes, Unknown, Yes, Yes, today), false, today);
    assert_eq!(v, Verdict::Blocked);
    assert!(r.contains(&"trains_on_data".to_string()));
    // Unbekanntes zaehlt nie als erfuellt.
    assert_eq!(
        eu_verdict(&facts(Yes, Unknown, Unknown, Yes, today), false, today).0,
        Verdict::Blocked
    );
    assert_eq!(
        eu_verdict(&facts(Yes, Unknown, No, Unknown, today), false, today).0,
        Verdict::Blocked
    );
    assert_eq!(
        eu_verdict(&facts(Unknown, Unknown, No, Yes, today), false, today).0,
        Verdict::Blocked
    );
}

#[test]
fn stale_facts_turn_allowed_into_conditional() {
    let f = facts(Tri::Yes, Tri::Unknown, Tri::No, Tri::Yes, "2026-01-01");
    let (v, r) = eu_verdict(&f, false, "2026-10-06");
    assert_eq!(v, Verdict::Conditional);
    assert_eq!(r, vec!["facts_stale".to_string()]);
    assert_eq!(eu_verdict(&f, false, "2026-03-01").0, Verdict::Allowed);
}

#[test]
fn every_fact_row_has_sources_and_a_readable_date() {
    for f in facts::FACTS {
        assert!(!f.sources.is_empty(), "{}: ohne Quelle", f.provider);
        assert!(
            facts_age_days(f.checked, "2026-10-06").is_some(),
            "{}: Datum",
            f.provider
        );
        assert!(!f.countries.is_empty(), "{}: ohne Standort", f.provider);
    }
}

#[test]
fn opt_out_attestation_unblocks_providers_that_train_by_default() {
    let mut f = facts(Tri::Yes, Tri::Unknown, Tri::Yes, Tri::Yes, "2026-10-06");
    f.training_opt_out_possible = true;
    let (v, r) = eu_verdict(&f, false, "2026-10-06");
    assert_eq!(
        (v, r),
        (
            Verdict::Blocked,
            vec!["training_opt_out_missing".to_string()]
        )
    );
    let (v, r) = eu_verdict(&f, true, "2026-10-06");
    assert_eq!(v, Verdict::Allowed);
    assert!(r.contains(&"training_opt_out_attested".to_string()));
}

/// Die Einstufung der Recherche vom 06.10.2026 -- so muss die Tabelle wirken.
#[test]
fn eu_profile_classifies_the_real_providers() {
    let t = "2026-10-06";
    let eu = |id: &str, url: &str, opt: bool| {
        assess(ComplianceProfile::Eu, &provider(id, url), opt, t).verdict
    };
    assert_eq!(
        eu("anthropic", "https://api.anthropic.com/v1", false),
        Verdict::Blocked
    );
    assert_eq!(eu("claude_cli", "cli://claude", true), Verdict::Blocked);
    assert_eq!(eu("codex_cli", "cli://codex", true), Verdict::Blocked);
    assert_eq!(
        eu("openai", "https://api.openai.com/v1", false),
        Verdict::Blocked
    );
    assert_eq!(
        eu("openai_eu", "https://eu.api.openai.com/v1", false),
        Verdict::Allowed
    );
    assert_eq!(
        eu("mistral", "https://api.mistral.ai/v1", false),
        Verdict::Blocked
    );
    assert_eq!(
        eu("mistral", "https://api.mistral.ai/v1", true),
        Verdict::Allowed
    );
    assert_eq!(
        eu(
            "bedrock_mantle",
            "https://bedrock-mantle.eu-central-1.api.aws/v1",
            false
        ),
        Verdict::Allowed
    );
    assert_eq!(
        eu(
            "bedrock_mantle",
            "https://bedrock-mantle.us-east-1.api.aws/v1",
            false
        ),
        Verdict::Blocked
    );
    let de = assess(
        ComplianceProfile::Eu,
        &provider(
            "bedrock_mantle",
            "https://bedrock-mantle.eu-central-1.api.aws/v1",
        ),
        false,
        t,
    );
    assert_eq!(de.countries, vec!["DE"]);
}

#[test]
fn shield_takes_the_worst_check() {
    let local = provider("local", "http://127.0.0.1:0/v1");
    let s = shield(
        ComplianceProfile::Eu,
        Some(("Gemma", &local, false)),
        &ShieldInputs::default(),
        "2026-10-06",
    );
    assert_eq!(s.level, ShieldLevel::Green);
    let s = shield(
        ComplianceProfile::Eu,
        Some(("Gemma", &local, false)),
        &ShieldInputs {
            plaintext_keys: 2,
            ..Default::default()
        },
        "2026-10-06",
    );
    assert_eq!(s.level, ShieldLevel::Yellow);
    let cloud = provider("anthropic", "https://api.anthropic.com/v1");
    let s = shield(
        ComplianceProfile::Eu,
        Some(("Claude", &cloud, false)),
        &ShieldInputs::default(),
        "2026-10-06",
    );
    assert_eq!(s.level, ShieldLevel::Red);
    assert!(s.checks.iter().any(|c| c.id == "model_blocked"));
    let s = shield(
        ComplianceProfile::Eu,
        None,
        &ShieldInputs {
            blocks_24h: vec!["Anthropic".into()],
            ..Default::default()
        },
        "2026-10-06",
    );
    assert_eq!(s.level, ShieldLevel::Red);
}
