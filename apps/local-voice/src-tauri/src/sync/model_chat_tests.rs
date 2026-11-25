use super::*;

fn vector(kind: &str) -> Wire {
    let root: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tooling/fixtures/model-chat/golden.json"
    )).unwrap();
    serde_json::from_value(root["vectors"].as_array().unwrap().iter()
        .find(|v| v["wire"]["kind"] == kind).unwrap()["wire"].clone()).unwrap()
}

#[test]
fn shared_vectors_and_wrong_mac() {
    let root: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../tooling/fixtures/model-chat/golden.json"
    )).unwrap();
    for entry in root["vectors"].as_array().unwrap() {
        let wire: Wire = serde_json::from_value(entry["wire"].clone()).unwrap();
        assert_eq!(wire.canonical(), entry["canonical"].as_str().unwrap());
        assert!(wire.authentic(&[7; 32]));
        assert!(!wire.authentic(&[8; 32]));
        let mut changed = wire.clone();
        changed.expires_at += 1;
        assert!(!changed.authentic(&[7; 32]));
    }
}

#[test]
fn grant_replay_deadline_model_size_and_token_limits() {
    let req = vector("req");
    let mut grant = new_grant(&req.device_id, "synthetic", vec![], req.created_at * 1000);
    grant.key_b64 = B64.encode([7; 32]);
    grant.key_id = req.key_id.clone();
    grant.functions.push(FUNCTION.into());
    grant.model_chat_models.push(req.request.as_ref().unwrap().model.clone());
    let mut store = RelayStore::default();
    store.add_grant(grant);
    let offered = vector("models").models.unwrap();
    let now = req.created_at * 1000;
    let check = |r: &Wire, s: &RelayStore, time| check_request(r, &r.request_id, &r.device_id, s, &offered, time, false);
    assert_eq!(check(&req, &store, now), Verdict::Accept);
    assert_eq!(check(&req, &store, now + 90_000), Verdict::Reject(ERR_CLOCK));
    let mut changed = req.clone();
    changed.mac = "00".repeat(32);
    assert_eq!(check(&changed, &store, now), Verdict::Ignore("mac"));
    let mut denied = store.clone();
    denied.grants[0].model_chat_models.clear();
    assert_eq!(check(&req, &denied, now), Verdict::Reject(ERR_NOT_ALLOWED));
    assert_eq!(check_request(&req, &req.request_id, "other-device", &store, &offered, now, false), Verdict::Ignore("kennung"));
    assert_eq!(check_request(&req, &req.request_id, &req.device_id, &store, &offered, now, true), Verdict::Reject(ERR_LIMIT));
    denied = store.clone();
    denied.grants[0].functions.retain(|f| f != FUNCTION);
    assert_eq!(check(&req, &denied, now), Verdict::Reject(ERR_NOT_ALLOWED));
    for (purpose, tokens, accepted) in [(Purpose::Talk, 800, true), (Purpose::Talk, 801, false),
        (Purpose::MeetingMinutes, 4000, true), (Purpose::MeetingMinutes, 4001, false)] {
        changed = req.clone();
        let input = changed.request.as_mut().unwrap();
        input.purpose = purpose; input.max_tokens = tokens;
        changed.sign(&[7; 32]);
        assert_eq!(check(&changed, &store, now) == Verdict::Accept, accepted);
    }
    changed = req.clone();
    changed.request.as_mut().unwrap().messages[0].text = "x".repeat(MAX_PROMPT_CHARS + 1);
    changed.sign(&[7; 32]);
    assert_eq!(check(&changed, &store, now), Verdict::Reject(ERR_LIMIT));
    changed = req.clone();
    let input = changed.request.as_mut().unwrap();
    input.messages[0].role = Role::System;
    input.messages[0].text = "x".repeat(4001);
    input.json_schema = Some("x".repeat(4000));
    changed.sign(&[7; 32]);
    assert_eq!(check(&changed, &store, now), Verdict::Reject(ERR_LIMIT));
    changed.request.as_mut().unwrap().messages[0].text = "x".repeat(4000);
    changed.sign(&[7; 32]);
    assert_eq!(check(&changed, &store, now), Verdict::Accept);
    let mut rate_limited = store.clone();
    for n in 0..MAX_PER_HOUR {
        rate_limited.mark_seen(&format!("synthetic-{n}"), &device_key(&req.device_id), true,
            req.expires_at * 1000, now);
    }
    assert_eq!(check(&req, &rate_limited, now), Verdict::Reject(ERR_LIMIT));
    store.mark_seen(&req.request_id, &device_key(&req.device_id), true, req.expires_at * 1000, now);
    assert_eq!(check(&req, &store, now), Verdict::Ignore("replay"));
}

#[test]
fn cancellation_and_concurrency_are_bounded() {
    let mut shared = Shared::default();
    let flag = Arc::new(AtomicBool::new(false));
    shared.cancels.insert("synthetic".into(), flag.clone());
    assert!(shared.cancel("synthetic"));
    assert!(flag.load(Ordering::SeqCst));
    let semaphore = tokio::sync::Semaphore::new(2);
    let _one = semaphore.try_acquire().unwrap();
    let _two = semaphore.try_acquire().unwrap();
    assert!(semaphore.try_acquire().is_err());
}

#[test]
fn provider_errors_remain_typed_and_content_free() {
    assert_eq!(provider_error("compliance_blocked: synthetic-secret"), Error::NotAllowed);
    assert_eq!(provider_error("cli_missing: synthetic-secret"), Error::ModelUnavailable);
    assert_eq!(provider_error("cli_timeout: synthetic-secret"), Error::Timeout);
    assert_eq!(provider_error("synthetic-secret"), Error::Provider);
}

#[test]
fn booktalk_model_names_cannot_grant_model_chat() {
    let mut grant = new_grant("synthetic-phone", "synthetic", vec!["model_chat:synthetic-model".into()], 1);
    grant.functions.push(FUNCTION.into());
    assert!(!permitted(&grant, "synthetic-model"));
    grant.model_chat_models.push("synthetic-model".into());
    assert!(permitted(&grant, "synthetic-model"));
}

#[test]
fn catalog_uses_the_actual_recipient_instead_of_a_connection_label() {
    let mut settings = crate::settings::get_default_settings();
    settings.llm_connections = vec![serde_json::from_value(serde_json::json!({
        "id":"synthetic", "kind":"openai", "label":"misleading label",
        "base_url":"https://synthetic.invalid/v1"
    })).unwrap()];
    settings.llm_models = vec![serde_json::from_value(serde_json::json!({
        "id":"synthetic-model", "connection_id":"synthetic", "remote_id":"m", "label":"Model"
    })).unwrap()];
    assert_eq!(available(&settings)[0].vendor, "synthetic.invalid");
    settings.llm_connections[0].base_url = "https://api.openai.com/v1".into();
    assert_eq!(available(&settings)[0].vendor, "OpenAI");
    settings.llm_models[0].enabled = false;
    assert!(available(&settings).is_empty());
}

#[tokio::test]
async fn relay_temperature_and_http_output_limit_reach_the_body() {
    use crate::llm_client::{apply_limits, StreamLimits, CHAT_TEMPERATURE};
    let limits = StreamLimits { max_tokens: Some(4000), num_ctx: None, strict: true };
    let mut baseline = serde_json::json!({});
    apply_limits(&mut baseline, false, &limits);
    assert_eq!(baseline["max_tokens"], 4000);
    assert!(baseline.get("temperature").is_none());
    let selected = CHAT_TEMPERATURE.scope(0.2, async {
        let mut body = serde_json::json!({});
        apply_limits(&mut body, false, &limits);
        body
    }).await;
    assert_eq!(selected["temperature"], 0.2);
    assert_eq!(selected["max_tokens"], 4000);
}
