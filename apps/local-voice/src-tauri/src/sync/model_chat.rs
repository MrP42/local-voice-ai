//! Gemeinsames Modell-Relay (#801 T4c, #804). Transport und Kopplung wie BookTalk.
//! Eigene Objekt-Namen in derselben verschluesselten Sammlung; keine Inhalte im Log.
use super::*;

pub const FUNCTION: &str = "model_chat";
const MAX_WIRE_BYTES: usize = 180_000;
const ERR_CONFIGURATION_CHANGED: &str = "configuration_changed";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Purpose {
    #[serde(rename = "meetingMinutes")]
    MeetingMinutes,
    #[serde(rename = "meetingSummary")]
    MeetingSummary,
    #[serde(rename = "talk")]
    Talk,
    #[serde(rename = "textPrep")]
    TextPrep,
}
impl Purpose {
    fn limit(&self) -> u32 {
        match self { Self::MeetingMinutes => 4000, Self::MeetingSummary => 1500,
            Self::Talk => 800, Self::TextPrep => 2000 }
    }
    fn ledger(&self) -> crate::managers::usage::Purpose {
        use crate::managers::usage::Purpose;
        match self { Self::MeetingMinutes => Purpose::Minutes, Self::MeetingSummary => Purpose::Summary,
            Self::Talk => Purpose::Chat, Self::TextPrep => Purpose::PostProcess }
    }
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role { System, User, Assistant }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message { pub role: Role, pub text: String }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Input {
    pub provider: String,
    /// Stabile PC-Konfigurationskennung, kein Alias und keine implizite Modellwahl.
    pub model: String,
    pub configuration_fingerprint: String,
    pub purpose: Purpose,
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    /// Ganzzahlig, damit Swift und Rust denselben MAC bilden.
    pub temperature_milli: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Model {
    pub provider: String, pub model: String, pub label: String, pub vendor: String,
    pub configuration_fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub context_tokens: u32, pub supports_streaming: bool, pub external: bool,
}
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Chunk { pub seq: usize, pub text: String }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Error { NotAllowed, ModelUnavailable, ConfigurationChanged, Limit, Clock, Timeout, Cancelled, Provider }
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WireKind { Models, Req, Resp }

/// Ein Vertrag fuer models/req/resp. Alle Felder einschliesslich usage sind signiert.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Wire {
    pub v: u32, pub function: String, pub kind: WireKind,
    pub request_id: String, pub device_id: String, pub key_id: String,
    pub created_at: i64, pub expires_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<Model>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<Input>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks: Option<Vec<Chunk>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    pub mac: String,
}

fn canonical(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::String(s) => json_quote(s),
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(",")),
        Value::Object(o) => {
            let sorted: BTreeMap<_, _> = o.iter().collect();
            format!("{{{}}}", sorted.iter().map(|(k, v)| format!("{}:{}", json_quote(k), canonical(v)))
                .collect::<Vec<_>>().join(","))
        }
        _ => value.to_string(),
    }
}
impl Wire {
    fn new(kind: WireKind, id: &str, device: &str, key_id: &str, now: i64) -> Self {
        Self { v: 1, function: FUNCTION.into(), kind, request_id: id.into(), device_id: device.into(),
            key_id: key_id.into(), created_at: now, expires_at: now.saturating_add(REQUEST_WINDOW_S),
            desktop_device_id: None, models: None, request: None, chunks: None, done: None,
            error: None, usage: None, mac: String::new() }
    }
    pub fn canonical(&self) -> String {
        let mut value = serde_json::to_value(self).expect("Wire ist JSON");
        value.as_object_mut().expect("Wire ist Objekt").remove("mac");
        canonical(&value)
    }
    pub fn sign(&mut self, key: &[u8]) { self.mac = mac_hex(key, self.canonical().as_bytes()); }
    pub fn authentic(&self, key: &[u8]) -> bool { mac_ok(key, self.canonical().as_bytes(), &self.mac) }
}

fn oid(kind: &str, id: &str) -> String { format!("model_{kind}_{id}") }
fn list_id(desktop: &str, device: &str) -> String { oid("models", &format!("{desktop}_{}", device_key(device))) }
fn permitted(grant: &DeviceGrant, model: &str) -> bool {
    grant.functions.iter().any(|f| f == FUNCTION)
        && grant.model_chat_models.iter().any(|id| id == model)
}

/// Nur eingeschaltete, am PC konfigurierte Modelle; nie einen Ersatz auswaehlen.
fn available(settings: &AppSettings) -> Vec<Model> {
    settings.llm_models.iter().filter(|m| m.enabled).filter_map(|m| {
        let c = settings.llm_connections.iter().find(|c| c.id == m.connection_id && c.enabled)?;
        let facts = crate::managers::compliance::facts_for(&c.kind, &c.base_url);
        let vendor = match Cli::from_base_url(&c.base_url) {
            Some(Cli::Claude) => "Anthropic".into(), Some(Cli::Codex) => "OpenAI".into(),
            None => match facts.map(|f| f.provider) {
                Some("openai") => "OpenAI".into(), Some("anthropic") => "Anthropic".into(),
                Some("mistral") => "Mistral".into(), Some(provider) => provider.into(),
                None => url::Url::parse(&c.base_url).ok().and_then(|u| u.host_str().map(str::to_owned))
                    .unwrap_or_else(|| "PC".into()),
            },
        };
        // Keine Zugangsdaten: Ziel, Remote-Modell, Optionen und zustimmungsrelevante
        // Anbieterdaten binden. Aenderungen verlangen einen neuen Katalog/Zustimmung.
        let template = settings.llm_template(c).cloned().map(|mut t| {
            t.base_url = c.base_url.clone(); t
        });
        let binding = serde_json::json!({ "connection": c, "model": m,
            "default_effort": settings.llm_default_effort, "template": template,
            "vendor": vendor, "region": facts.map(|f| f.countries.join(", ")) });
        let configuration_fingerprint = hex(&Sha256::digest(canonical(&binding).as_bytes()));
        Some(Model { provider: c.id.clone(), model: m.id.clone(), label: m.label.clone(), configuration_fingerprint,
            vendor, region: facts.map(|f| f.countries.join(", ")), context_tokens: m.context_limit.unwrap_or(DEFAULT_CONTEXT_TOKENS),
            supports_streaming: true, external: !crate::llm_client::is_loopback_url(&c.base_url) })
    }).collect()
}

fn check_request(req: &Wire, id: &str, sender: &str, store: &RelayStore,
                 models: &[Model], now: i64, busy: bool) -> Verdict {
    if req.v != 1 || req.kind != WireKind::Req || req.request_id != id || !valid_request_id(id)
        || req.device_id != sender { return Verdict::Ignore("kennung"); }
    let Some(grant) = store.grant_for(sender, &req.key_id, now) else { return Verdict::Ignore("keine_freigabe"); };
    let Some(key) = grant.key() else { return Verdict::Ignore("schluessel"); };
    if !req.authentic(&key) { return Verdict::Ignore("mac"); }
    if store.has_seen(id) { return Verdict::Ignore("replay"); }
    let time = now.div_euclid(1000);
    if req.created_at.checked_add(REQUEST_WINDOW_S) != Some(req.expires_at)
        || req.created_at.abs_diff(time) > CLOCK_SKEW_S as u64
        || req.expires_at.saturating_sub(time) < MIN_REMAINING_S { return Verdict::Reject(ERR_CLOCK); }
    let Some(input) = &req.request else { return Verdict::Reject(ERR_NOT_ALLOWED); };
    if req.function != FUNCTION || !permitted(grant, &input.model) { return Verdict::Reject(ERR_NOT_ALLOWED); }
    if let Err(error) = matching_model(input, models) {
        return Verdict::Reject(match error { Error::ConfigurationChanged => ERR_CONFIGURATION_CHANGED,
            _ => ERR_MODEL_UNAVAILABLE });
    }
    let bytes: usize = input.messages.iter().map(|m| m.text.len()).sum::<usize>()
        + input.json_schema.as_ref().map_or(0, String::len);
    let system_bytes = input.messages.iter().filter(|m| m.role == Role::System).map(|m| m.text.len()).sum::<usize>()
        + input.json_schema.as_ref().map_or(0, String::len);
    if bytes > MAX_PROMPT_CHARS || input.messages.is_empty() || input.messages.len() > 128
        || system_bytes > MAX_SYSTEM_CHARS
        || input.max_tokens == 0 || input.max_tokens > input.purpose.limit() || input.temperature_milli > 2000
        || serde_json::to_vec(req).map_or(true, |b| b.len() > MAX_WIRE_BYTES) { return Verdict::Reject(ERR_LIMIT); }
    let dkey = device_key(sender);
    if busy || store.accepted_since(&dkey, now - 3_600_000) >= MAX_PER_HOUR
        || store.accepted_since(&dkey, now - DAY_MS) >= MAX_PER_DAY { return Verdict::Reject(ERR_LIMIT); }
    Verdict::Accept
}

fn matching_model(input: &Input, models: &[Model]) -> Result<(), Error> {
    let model = models.iter().find(|m| m.model == input.model && m.provider == input.provider)
        .ok_or(Error::ModelUnavailable)?;
    if input.configuration_fingerprint != model.configuration_fingerprint {
        return Err(Error::ConfigurationChanged);
    }
    Ok(())
}

/// Kopplung bleibt dieselbe; diese Funktion ist je Kopplung zunaechst AUS.
#[tauri::command]
#[specta::specta]
pub fn model_chat_relay_authorize(app: AppHandle, device_key: String, model_ids: Vec<String>) -> Result<(), String> {
    let settings = crate::settings::get_settings(&app);
    let offered = available(&settings);
    if model_ids.iter().any(|id| !offered.iter().any(|m| &m.model == id)) {
        return Err("model_unavailable".into());
    }
    update_store(&store_path(&app)?, |store| {
        let grant = store.grants.iter_mut().find(|g| g.device_key == device_key && g.active(now_ms()))
            .ok_or("not_paired")?;
        grant.functions.retain(|f| f != FUNCTION);
        if !model_ids.is_empty() { grant.functions.push(FUNCTION.into()); }
        grant.model_chat_models = model_ids;
        Ok::<_, &str>(())
    })?.map_err(str::to_string)
}

/// Neue Kopplung auch fuer PCs ohne Abo-CLI; Einrichtungscode wie bei BookTalk.
#[tauri::command]
#[specta::specta]
pub async fn model_chat_relay_pair(app: AppHandle, device_key: String, name: String,
                                   model_ids: Vec<String>) -> Result<RelayPairing, String> {
    if !valid_device_key(&device_key) || name.trim().is_empty() { return Err("invalid_pairing".into()); }
    let offered = available(&crate::settings::get_settings(&app));
    if model_ids.is_empty() || model_ids.iter().any(|id| !offered.iter().any(|m| &m.model == id)) {
        return Err("model_unavailable".into());
    }
    let cfg = account::load(&app).ok_or("not_paired")?;
    let device_id = resolve_device_id(&HubClient::new(&cfg.url, &cfg.token), &device_key).await
        .map_err(|_| "unavailable")?.ok_or("not_paired")?;
    let mut grant = new_grant(&device_id, name.trim(), Vec::new(), now_ms());
    grant.functions = vec![FUNCTION.into()];
    grant.model_chat_models = model_ids;
    let pairing = pairing_of(&cfg, &grant)?;
    update_store(&store_path(&app)?, |store| store.add_grant(grant))?;
    Ok(pairing)
}

fn current_grant(ctx: &Ctx, req: &Wire) -> bool {
    load_store(&ctx.store_path).grant_for(&req.device_id, &req.key_id, now_ms())
        .is_some_and(|g| req.request.as_ref().is_some_and(|i| permitted(g, &i.model)))
}

fn cancel_owned(ctx: &Ctx, id: &str, sender: &str) {
    if let Ok(shared) = ctx.shared.lock() {
        if shared.busy.running.get(sender).is_some_and(|running| running == id) { shared.cancel(id); }
    }
}

/// Eigener Cursor, gleicher Hub und dieselbe verschluesselte Freigabeablage.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let slots = Arc::new(tokio::sync::Semaphore::new(2));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut tag = String::new();
        let mut cursor = 0;
        let mut lists = BTreeSet::<String>::new();
        let mut published_at = 0;
        let mut scanned = false;
        loop {
            tokio::time::sleep(FAST_TICK).await;
            let Some(cfg) = account::load(&app) else {
                if let Ok(s) = shared.lock() { for c in s.cancels.values() { c.store(true, Ordering::SeqCst); } }
                continue;
            };
            let Ok(key) = cfg.key() else { continue; };
            let Ok(path) = store_path(&app) else { continue; };
            let next_tag = account_tag(&cfg);
            if next_tag != tag {
                if let Ok(mut s) = shared.lock() {
                    for c in s.cancels.values() { c.store(true, Ordering::SeqCst); }
                    *s = Shared::default();
                }
                cursor = 0; lists.clear(); published_at = 0; scanned = false; tag = next_tag;
            }
            let switch_app = app.clone();
            let ctx = Arc::new(Ctx::new(cfg, key, path, shared.clone()).with_switch(
                crate::managers::llm::preview::PreviewSwitch::new(move || crate::settings::book_talk_preview_switch_on(&switch_app))));
            let settings = crate::settings::get_settings(&app);
            let models = available(&settings);
            let now = now_ms();
            let store = load_store(&ctx.store_path);
            let active: Vec<_> = store.grants.iter().filter(|g| g.active(now) && g.functions.iter().any(|f| f == FUNCTION)).collect();
            let wanted: BTreeSet<_> = active.iter().map(|g| list_id(&ctx.cfg.device_id, &g.device_id)).collect();
            let idle = wanted.is_empty() && lists.is_empty()
                && shared.lock().is_ok_and(|s| s.cancels.is_empty());
            if idle && scanned { continue; }
            for id in lists.difference(&wanted) { let _ = ctx.put(id, None).await; }
            if now - published_at >= 30_000 || lists != wanted {
                for grant in active {
                    let Some(key) = grant.key() else { continue; };
                    let mut wire = Wire::new(WireKind::Models, "catalog", &grant.device_id, &grant.key_id, now / 1000);
                    wire.desktop_device_id = Some(ctx.cfg.device_id.clone());
                    wire.models = Some(models.iter().filter(|m| permitted(grant, &m.model)).cloned().collect());
                    wire.sign(&key);
                    let _ = ctx.put_json(&list_id(&ctx.cfg.device_id, &grant.device_id), &wire).await;
                }
                published_at = now; lists = wanted;
            }
            // Auch ohne Freigabe weiter lesen: Loeschen muss laufende Transporte abbrechen.
            let Ok(status) = ctx.client.status().await else { continue; };
            for _ in 0..20 {
                let page = match ctx.client.pull_shared(cursor, 100).await {
                    Ok(p) => p,
                    Err(HubError::CursorExpired) => { cursor = 0; break; }
                    Err(_) => break,
                };
                for remote in &page.objects {
                    if remote.collection != COLLECTION || !remote.object_id.starts_with("model_") { continue; }
                    ctx.remember(&remote.object_id, remote.revision);
                    if remote.device_id == ctx.cfg.device_id {
                        if !remote.deleted && remote.object_id.starts_with("model_models_") && !lists.contains(&remote.object_id) {
                            let _ = ctx.put(&remote.object_id, None).await;
                        }
                        continue;
                    }
                    if remote.deleted {
                        if let Some(id) = remote.object_id.strip_prefix("model_req_").or_else(|| remote.object_id.strip_prefix("model_resp_")) {
                            cancel_owned(&ctx, id, &remote.device_id);
                        }
                        continue;
                    }
                    let Some(id) = remote.object_id.strip_prefix("model_req_") else { continue; };
                    if !status.devices.iter().any(|d| d.device == device_key(&remote.device_id)) { continue; }
                    let Some(req) = ctx.decrypt::<Wire>(remote) else { continue; };
                    let outcome = update_store(&ctx.store_path, |store| {
                        store.prune(now);
                        let busy = shared.lock().map_or(true, |s| s.busy.is_busy(&remote.device_id));
                        let mut verdict = check_request(&req, id, &remote.device_id, store, &models, now, busy);
                        let permit = if verdict == Verdict::Accept { slots.clone().try_acquire_owned().ok() } else { None };
                        if verdict == Verdict::Accept && permit.is_none() { verdict = Verdict::Reject(ERR_LIMIT); }
                        if matches!(verdict, Verdict::Accept | Verdict::Reject(_)) {
                            store.activate(&req.device_id, &req.key_id);
                            store.record(id, &device_key(&req.device_id), verdict == Verdict::Accept, now + REQUEST_WINDOW_S * 1000, now);
                        }
                        let key = store.grant_for(&req.device_id, &req.key_id, now).and_then(DeviceGrant::key);
                        (verdict, key, permit)
                    });
                    let Ok((verdict, Some(key), permit)) = outcome else { continue; };
                    match verdict {
                        Verdict::Accept => {
                            let cancel = Arc::new(AtomicBool::new(false));
                            if let Ok(mut s) = shared.lock() {
                                s.busy.try_begin(&req.device_id, &req.request_id);
                                s.cancels.insert(req.request_id.clone(), cancel.clone());
                            }
                            let run_ctx = ctx.clone(); let run_app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                let _permit = permit;
                                let _on_exit = CancelOnDrop(cancel.clone());
                                process(&run_ctx, &run_app, &req, &key, cancel).await;
                                end_busy(&run_ctx, &req.device_id, &req.request_id);
                            });
                        }
                        Verdict::Reject(code) => {
                            let mut response = response(&req);
                            response.done = Some(true);
                            response.error = Some(match code { ERR_CLOCK => Error::Clock, ERR_LIMIT => Error::Limit,
                                ERR_CONFIGURATION_CHANGED => Error::ConfigurationChanged,
                                ERR_MODEL_UNAVAILABLE => Error::ModelUnavailable, _ => Error::NotAllowed });
                            response.sign(&key);
                            let _ = ctx.put_json(&oid("resp", id), &response).await;
                            ctx.schedule_cleanup(oid("resp", id), RESP_KEEP_MS);
                            ctx.schedule_cleanup(oid("req", id), RESP_KEEP_MS);
                        }
                        _ => {}
                    }
                }
                if page.cursor <= cursor { scanned = !page.more; break; }
                cursor = page.cursor;
                if !page.more { scanned = true; break; }
            }
        }
    });
}

fn response(req: &Wire) -> Wire {
    let mut wire = Wire::new(WireKind::Resp, &req.request_id, &req.device_id, &req.key_id, req.created_at);
    wire.chunks = Some(vec![]); wire.done = Some(false); wire
}

fn provider_error(error: &str) -> Error {
    match map_error(error) {
        ERR_NOT_ALLOWED => Error::NotAllowed,
        ERR_MODEL_UNAVAILABLE => Error::ModelUnavailable,
        ERR_TIMEOUT => Error::Timeout,
        _ => Error::Provider,
    }
}

#[allow(deprecated)]
async fn process(ctx: &Arc<Ctx>, app: &AppHandle, req: &Wire, key: &[u8], cancel: Arc<AtomicBool>) {
    use crate::llm_client::{StreamLimits, StreamMessage, CLI_CANCEL, CLI_MAX_TOKENS, CLI_REQUIRE_USAGE, CLI_TIMEOUT, CLI_PREVIEW, CLI_PREVIEW_SWITCH, CLI_META, CliMeta};
    let Some(input) = &req.request else { return; };
    let mut resp = response(req);
    resp.sign(key);
    if matches!(tokio::time::timeout(Duration::from_secs(3), ctx.put_json(&oid("resp", &req.request_id), &resp)).await,
        Ok(Ok(Written::Gone))) { return; }
    ctx.schedule_cleanup(oid("resp", &req.request_id), RESP_KEEP_MS);
    ctx.schedule_cleanup(oid("req", &req.request_id), RESP_KEEP_MS);
    let meta = Arc::new(Mutex::new(CliMeta::default()));
    let settings = crate::settings::get_settings(app);
    let started = Instant::now();
    let work = async {
        if cancel.load(Ordering::SeqCst) { return Err(Error::Cancelled); }
        if !current_grant(ctx, req) {
            return Err(Error::NotAllowed);
        }
        matching_model(input, &available(&settings))?;
        let call = settings_for_model(&settings, &input.model);
        let (connection, config) = call.chosen_llm_model().ok_or(Error::ModelUnavailable)?;
        // Die Legacy-Aufloesung darf fuer eine explizite Relay-Wahl keinen Ersatz nutzen.
        if connection.id != input.provider
            || call.active_llm_model().is_none_or(|(c, m)| c.id != connection.id || m.id != config.id) {
            return Err(Error::ModelUnavailable);
        }
        let (provider, model, api_key) = crate::managers::meetings::llm_call::resolve_provider_coded(&call)
            .map_err(|_| Error::ModelUnavailable)?;
        let mut messages: Vec<_> = input.messages.iter().map(|m| StreamMessage::new(match m.role {
            Role::System => "system", Role::User => "user", Role::Assistant => "assistant",
        }, m.text.clone())).collect();
        if let Some(schema) = &input.json_schema {
            messages.insert(0, StreamMessage::new("system", format!("JSON schema:\n{schema}")));
        }
        let buffer = Arc::new(Mutex::new(String::new()));
        let sink = buffer.clone();
        let overflow = Arc::new(AtomicBool::new(false));
        let overflow_sink = overflow.clone();
        let on_delta = move |text: &str| { if let Ok(mut b) = sink.lock() {
            if b.len().saturating_add(text.len()) <= 100_000 { b.push_str(text); }
            else { overflow_sink.store(true, Ordering::SeqCst); }
        } };
        let limits = StreamLimits { max_tokens: Some(input.max_tokens), num_ctx: None, strict: true };
        let llm = crate::llm_versand::uebergang::send_chat_completion_stream_limited(
            input.purpose.ledger(), &provider, api_key, &model, messages, &on_delta, &limits);
        let llm = crate::managers::usage::with_model_selection(connection.clone(), config.clone(), llm);
        let llm = crate::llm_client::CHAT_TEMPERATURE.scope(input.temperature_milli as f64 / 1000.0,
            CLI_META.scope(meta.clone(), CLI_TIMEOUT.scope(CALL_TIMEOUT, CLI_CANCEL.scope(cancel.clone(),
                CLI_MAX_TOKENS.scope(input.max_tokens, CLI_REQUIRE_USAGE.scope(true, llm))))));
        // Derselbe Abo-Vorschau-Nachweis wie BookTalk; ohne Beleg kein CLI-Aufruf.
        let preview = if let Some(cli) = Cli::from_base_url(&provider.base_url) {
            if !cli_ready(cli).await { return Err(Error::ModelUnavailable); }
            let id = model.clone();
            let binding = tokio::task::spawn_blocking(move || crate::managers::llm::preview::current_binding(cli, &id))
                .await.ok().and_then(Result::ok);
            Some(crate::book_ai::talk::preview_check(&call, cli, binding, input.max_tokens, &ctx.evidence())
                .ok_or(Error::NotAllowed)?)
        } else { None };
        // Nach wartender CLI-Vorbereitung direkt vor dem ersten Transport nochmals lesen.
        matching_model(input, &available(&crate::settings::get_settings(app)))?;
        let guarded = async {
            match preview {
                Some(check) => CLI_PREVIEW_SWITCH.scope(ctx.preview_switch.clone(), CLI_PREVIEW.scope(check, llm)).await,
                None => llm.await,
            }
        };
        tokio::pin!(guarded);
        let mut ticker = tokio::time::interval(RESP_INTERVAL);
        let mut emitted = 0;
        loop {
            tokio::select! {
                outcome = &mut guarded => {
                    let text = outcome.map_err(|error| provider_error(&error))?;
                    if text.len() > 100_000 || overflow.load(Ordering::SeqCst) { return Err(Error::Limit); }
                    let partial = buffer.lock().map(|b| b.clone()).unwrap_or_default();
                    if !text.is_char_boundary(emitted) || !text.starts_with(&partial[..emitted]) { return Err(Error::Provider); }
                    if emitted < text.len() {
                        let chunks = resp.chunks.as_mut().unwrap();
                        chunks.push(Chunk { seq: chunks.len() + 1, text: text[emitted..].into() });
                    }
                    return Ok(());
                }
                _ = ticker.tick() => {
                    let fresh = crate::settings::get_settings(app);
                    let stop = if cancel.load(Ordering::SeqCst) { Some(Error::Cancelled) }
                        else if started.elapsed() >= CALL_TIMEOUT { Some(Error::Timeout) }
                        else if overflow.load(Ordering::SeqCst) { Some(Error::Limit) }
                        else if !current_grant(ctx, req) { Some(Error::NotAllowed) }
                        else if let Err(error) = matching_model(input, &available(&fresh)) { Some(error) }
                        else { None };
                    if let Some(error) = stop {
                        cancel.store(true, Ordering::SeqCst);
                        // Versand beendet den Transport und bucht, bevor seine Future verlassen wird.
                        let _ = (&mut guarded).await;
                        return Err(error);
                    }
                    let text = buffer.lock().map(|b| b.clone()).unwrap_or_default();
                    if text.len() > emitted {
                        let chunks = resp.chunks.as_mut().unwrap();
                        chunks.push(Chunk { seq: chunks.len() + 1, text: text[emitted..].into() });
                        emitted = text.len();
                    }
                    resp.sign(key);
                    if serde_json::to_vec(&resp).map_or(true, |b| b.len() > MAX_WIRE_BYTES) {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = (&mut guarded).await;
                        return Err(Error::Limit);
                    }
                    if matches!(tokio::time::timeout(Duration::from_secs(2),
                        ctx.put_json(&oid("resp", &req.request_id), &resp)).await, Ok(Ok(Written::Gone))) {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = (&mut guarded).await;
                        return Err(Error::Cancelled);
                    }
                }
            }
        }
    };
    // Die innere Zeitgrenze signalisiert Abbruch und wartet den CLI-Prozess ab.
    // Eine aeussere Timeout-Future wuerde den Blocking-Aufruf wieder abhaengen.
    let mut result = work.await;
    let fresh = crate::settings::get_settings(app);
    if matches!(matching_model(input, &available(&fresh)), Err(Error::ConfigurationChanged)) {
        result = Err(Error::ConfigurationChanged);
    }
    if (cancel.load(Ordering::SeqCst) && !matches!(&result, Err(Error::Limit | Error::Timeout | Error::ConfigurationChanged)))
        || !current_grant(ctx, req)
        || (matching_model(input, &available(&fresh)).is_err() && result != Err(Error::ConfigurationChanged))
        || account::load(app).is_none_or(|cfg| account_tag(&cfg) != account_tag(&ctx.cfg)) { return; }
    cancel.store(true, Ordering::SeqCst);
    resp.done = Some(true);
    resp.error = result.err();
    if let Ok(meta) = meta.lock() {
        if meta.prompt_tokens + meta.completion_tokens > 0 {
            resp.usage = Some(Usage { input_tokens: meta.prompt_tokens, output_tokens: meta.completion_tokens });
        }
    }
    if resp.error.is_some() || serde_json::to_vec(&resp).map_or(true, |b| b.len() > MAX_WIRE_BYTES) {
        resp.chunks = Some(vec![]);
        if resp.error.is_none() { resp.error = Some(Error::Limit); }
    }
    resp.sign(key);
    let _ = tokio::time::timeout(Duration::from_secs(3), ctx.put_json(&oid("resp", &req.request_id), &resp)).await;
}

#[cfg(test)]
#[path = "model_chat_tests.rs"]
mod tests;
