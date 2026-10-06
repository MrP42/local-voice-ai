//! Die Operationen je Dienst, nach der offiziellen API-Doku (Stand 06.10.2026):
//! in einen Kanal posten, Aufgabe anlegen, Seite anlegen/ergaenzen, CRM-Notiz, Datensatz.
//!
//! Jede Operation baut ihre Anfragen selbst und reicht sie an `exec` weiter: im Betrieb ist
//! das `http::execute`, in Tests eine Attrappe, die Anfragen mitschreibt und Antworten
//! vorgibt. So ist jede Anfrage (Methode, Pfad, Kopf, Koerper) ohne Netz pruefbar, auch
//! mehrstufige Ablaeufe (Kontakt suchen, dann Notiz anlegen).

use chrono::NaiveDate;
use serde_json::{json, Map, Value};

use super::http::{check_url, ApiReply, ApiRequest, Method, ServiceError};
use super::markdown;
use super::registry::{def, AuthKind, ServiceId};

/// Ausfuehrer einer Anfrage.
pub type Exec<'a> = dyn FnMut(ApiRequest) -> Result<ApiReply, ServiceError> + 'a;

/// Ein Konto: Dienst, Geheimnis (Fach `token`) und die Felder der Integration.
pub struct Account<'a> {
    pub service: ServiceId,
    pub token: &'a str,
    pub settings: &'a Map<String, Value>,
}

impl Account<'_> {
    fn field(&self, key: &str) -> Result<String, ServiceError> {
        self.settings
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ServiceError::Config(format!("In der Integration fehlt das Feld „{key}“."))
            })
    }

    fn field_opt(&self, key: &str) -> Option<String> {
        self.field(key).ok()
    }

    /// Atlassian-Site als Host (`firma.atlassian.net`), mit oder ohne `https://`.
    fn site(&self) -> Result<String, ServiceError> {
        let raw = self.field("site")?;
        let host = raw
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        Ok(host)
    }

    /// Adresse auf dem Host des Dienstes; der Host wird gegen das Register geprueft.
    fn url(&self, base: &str, path: &str) -> Result<url::Url, ServiceError> {
        let url = url::Url::parse(&format!("{base}{path}"))
            .map_err(|_| ServiceError::Config("Die Adresse des Dienstes ist ungültig.".into()))?;
        check_url(&url, def(self.service).hosts, false)?;
        Ok(url)
    }

    /// Die Webhook-Adresse (Geheimnis) gepruefte gegen die Hosts des Dienstes.
    fn webhook_url(&self) -> Result<url::Url, ServiceError> {
        let url = url::Url::parse(self.token.trim()).map_err(|_| {
            ServiceError::Config("Die Webhook-Adresse ist ungültig. Bitte neu eintragen.".into())
        })?;
        check_url(&url, def(self.service).hosts, false)?;
        Ok(url)
    }

    /// Anmeldung an die Anfrage haengen.
    fn auth(&self, req: ApiRequest) -> Result<ApiRequest, ServiceError> {
        let token = self.token.trim();
        if token.is_empty() {
            return Err(ServiceError::Config(
                "Der Schlüssel fehlt. Bitte in der Integration neu eintragen.".into(),
            ));
        }
        Ok(match def(self.service).auth {
            AuthKind::WebhookUrl => req,
            AuthKind::Bearer => req.header("Authorization", format!("Bearer {token}")),
            AuthKind::RawAuthorization => req.header("Authorization", token),
            AuthKind::BasicEmailToken => {
                use base64::Engine as _;
                let email = self.field("email")?;
                let b64 =
                    base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
                req.header("Authorization", format!("Basic {b64}"))
            }
            AuthKind::ApiTokenHeader => req.header("x-api-token", token),
            AuthKind::TrelloKeyToken => {
                let key = self.field("api_key")?;
                req.header(
                    "Authorization",
                    format!("OAuth oauth_consumer_key=\"{key}\", oauth_token=\"{token}\""),
                )
            }
        })
    }
}

/// Was ein Dienst angelegt hat (fuer Laufprotokoll und Rueckmeldung).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Created {
    pub id: String,
    pub url: Option<String>,
}

impl Created {
    fn from(id: Option<String>, url: Option<String>) -> Result<Self, ServiceError> {
        Ok(Self {
            id: id.ok_or(ServiceError::BadResponse("keine Kennung in der Antwort"))?,
            url,
        })
    }
}

fn s(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr).and_then(|x| match x {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn unsupported(acc: &Account, what: &str) -> ServiceError {
    ServiceError::Config(format!(
        "{} unterstützt „{what}“ nicht.",
        def(acc.service).label
    ))
}

// ---------------------------------------------------------------------------
// In einen Kanal posten (Slack, Teams, Discord)
// ---------------------------------------------------------------------------

/// Text in den Kanal des Webhooks. Discord teilt in Nachrichten zu 2000 Zeichen.
pub fn chat_post(acc: &Account, text: &str, exec: &mut Exec) -> Result<usize, ServiceError> {
    let url = acc.webhook_url()?;
    let text = text.trim();
    if text.is_empty() {
        return Err(ServiceError::Config("Der Text ist leer.".into()));
    }
    let bodies: Vec<Value> = match acc.service {
        // mrkdwn: Ueberschriften als fette Zeilen.
        ServiceId::Slack => vec![json!({ "text": slack_mrkdwn(text) })],
        // Workflows-Webhook: Adaptive Card (Office-365-Connectors gibt es nicht mehr).
        ServiceId::Teams => vec![json!({
            "type": "message",
            "attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "contentUrl": null,
                "content": {
                    "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                    "type": "AdaptiveCard",
                    "version": "1.4",
                    "body": [{"type": "TextBlock", "text": text, "wrap": true}]
                }
            }]
        })],
        ServiceId::Discord => markdown::chunks(text, 2000)
            .into_iter()
            .map(|c| json!({ "content": c, "allowed_mentions": {"parse": []} }))
            .collect(),
        _ => return Err(unsupported(acc, "in Kanal posten")),
    };
    let n = bodies.len();
    for (sent, body) in bodies.into_iter().enumerate() {
        if let Err(e) = exec(ApiRequest::new(Method::Post, url.clone()).json(body)) {
            return Err(if sent == 0 {
                e
            } else {
                ServiceError::Partial(format!("{sent} von {n} Nachrichten gesendet; {e}"))
            });
        }
    }
    Ok(n)
}

fn slack_mrkdwn(md: &str) -> String {
    md.lines()
        .map(|l| {
            let t = l.trim_start();
            if let Some(h) = t
                .strip_prefix("### ")
                .or_else(|| t.strip_prefix("## "))
                .or_else(|| t.strip_prefix("# "))
            {
                format!("*{}*", h.replace("**", ""))
            } else {
                l.replace("**", "*")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Aufgaben
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskInput {
    pub title: String,
    pub description: String,
    pub due: Option<NaiveDate>,
}

pub fn task_create(
    acc: &Account,
    task: &TaskInput,
    exec: &mut Exec,
) -> Result<Created, ServiceError> {
    let title: String = task.title.trim().chars().take(250).collect();
    if title.is_empty() {
        return Err(ServiceError::Config("Die Aufgabe hat keinen Titel.".into()));
    }
    let due = task.due.map(|d| d.format("%Y-%m-%d").to_string());
    match acc.service {
        ServiceId::Asana => {
            let mut data = json!({
                "name": title, "notes": task.description,
                "projects": [acc.field("project_id")?],
            });
            if let Some(d) = &due {
                data["due_on"] = json!(d);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://app.asana.com", "/api/1.0/tasks")?,
                )
                .json(json!({ "data": data })),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/data/gid"), s(&r.json, "/data/permalink_url"))
        }
        ServiceId::Clickup => {
            let list = acc.field("list_id")?;
            let mut body = json!({"name": title, "description": task.description});
            if let Some(d) = task.due {
                body["due_date"] = json!(epoch_ms(d));
                body["due_date_time"] = json!(false);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url(
                        "https://api.clickup.com",
                        &format!("/api/v2/list/{}/task", enc(&list)),
                    )?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/id"), s(&r.json, "/url"))
        }
        ServiceId::Jira => {
            let site = acc.site()?;
            let mut fields = json!({
                "project": {"key": acc.field("project_key")?},
                "summary": title,
                "issuetype": {"name": acc.field_opt("issue_type").unwrap_or_else(|| "Task".into())},
                "description": markdown::jira_adf(&task.description),
            });
            if let Some(d) = &due {
                fields["duedate"] = json!(d);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url(&format!("https://{site}"), "/rest/api/3/issue")?,
                )
                .json(json!({ "fields": fields })),
            )?;
            let r = exec(req)?;
            let key = s(&r.json, "/key");
            let url = key.as_ref().map(|k| format!("https://{site}/browse/{k}"));
            Created::from(key, url)
        }
        ServiceId::Trello => {
            let mut body = json!({
                "idList": acc.field("list_id")?, "name": title, "desc": task.description,
            });
            if let Some(d) = &due {
                body["due"] = json!(format!("{d}T12:00:00.000Z"));
            }
            let req = acc.auth(
                ApiRequest::new(Method::Post, acc.url("https://api.trello.com", "/1/cards")?)
                    .json(body),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/id"), s(&r.json, "/shortUrl"))
        }
        ServiceId::Todoist => {
            let mut body = json!({"content": title, "description": task.description});
            if let Some(p) = acc.field_opt("project_id") {
                body["project_id"] = json!(p);
            }
            if let Some(d) = &due {
                body["due_date"] = json!(d);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.todoist.com", "/api/v1/tasks")?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            let id = s(&r.json, "/id");
            let url = s(&r.json, "/url").or_else(|| {
                id.as_ref()
                    .map(|i| format!("https://app.todoist.com/app/task/{i}"))
            });
            Created::from(id, url)
        }
        ServiceId::Monday => {
            let mut vars = json!({"board": acc.field("board_id")?, "name": title});
            let mut query = "mutation ($board: ID!, $name: String!) { create_item (board_id: $board, item_name: $name) { id } }".to_string();
            if let Some(g) = acc.field_opt("group_id") {
                vars["group"] = json!(g);
                query = "mutation ($board: ID!, $group: String!, $name: String!) { create_item (board_id: $board, group_id: $group, item_name: $name) { id } }".into();
            }
            let req = acc.auth(
                ApiRequest::new(Method::Post, acc.url("https://api.monday.com", "/v2")?)
                    .header("API-Version", "2025-04")
                    .json(json!({"query": query, "variables": vars})),
            )?;
            let r = exec(req)?;
            graphql_errors(&r.json)?;
            let id = s(&r.json, "/data/create_item/id");
            // Beschreibung als Update am Eintrag (monday kennt kein Beschreibungsfeld).
            if let (Some(item), false) = (&id, task.description.trim().is_empty()) {
                let upd = acc.auth(
                    ApiRequest::new(Method::Post, acc.url("https://api.monday.com", "/v2")?)
                        .header("API-Version", "2025-04")
                        .json(json!({
                            "query": "mutation ($item: ID!, $body: String!) { create_update (item_id: $item, body: $body) { id } }",
                            "variables": {"item": item, "body": task.description}
                        })),
                )?;
                graphql_errors(&exec(upd)?.json)?;
            }
            Created::from(id, None)
        }
        ServiceId::Linear => {
            let mut input = json!({
                "teamId": acc.field("team_id")?, "title": title, "description": task.description,
            });
            if let Some(d) = &due {
                input["dueDate"] = json!(d);
            }
            let req = acc.auth(
                ApiRequest::new(Method::Post, acc.url("https://api.linear.app", "/graphql")?).json(json!({
                    "query": "mutation ($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { id identifier url } } }",
                    "variables": {"input": input}
                })),
            )?;
            let r = exec(req)?;
            graphql_errors(&r.json)?;
            Created::from(
                s(&r.json, "/data/issueCreate/issue/identifier"),
                s(&r.json, "/data/issueCreate/issue/url"),
            )
        }
        ServiceId::Github => {
            let (owner, name) = github_repo(acc)?;
            let mut body = task.description.clone();
            if let Some(d) = &due {
                body = format!("{body}\n\nFällig: {d}").trim().to_string();
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url(
                        "https://api.github.com",
                        &format!("/repos/{}/{}/issues", enc(&owner), enc(&name)),
                    )?,
                )
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .json(json!({"title": title, "body": body})),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/number"), s(&r.json, "/html_url"))
        }
        ServiceId::Hubspot => {
            let ts = task
                .due
                .map(|d| format!("{d}T12:00:00Z"))
                .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.hubapi.com", "/crm/v3/objects/tasks")?,
                )
                .json(json!({"properties": {
                    "hs_timestamp": ts, "hs_task_subject": title,
                    "hs_task_body": task.description, "hs_task_status": "NOT_STARTED",
                }})),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/id"), None)
        }
        ServiceId::Pipedrive => {
            let mut body = json!({"subject": title, "type": "task", "note": task.description});
            if let Some(d) = &due {
                body["due_date"] = json!(d);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.pipedrive.com", "/api/v2/activities")?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/data/id"), None)
        }
        _ => Err(unsupported(acc, "Aufgabe anlegen")),
    }
}

/// `besitzer/name`, beide Teile ohne weiteren Schraegstrich und ohne `.`/`..`.
fn github_repo(acc: &Account) -> Result<(String, String), ServiceError> {
    let repo = acc.field("repo")?;
    repo.split_once('/')
        .filter(|(o, n)| {
            [o, n]
                .iter()
                .all(|p| !p.is_empty() && !p.contains('/') && **p != "." && **p != "..")
        })
        .map(|(o, n)| (o.to_string(), n.to_string()))
        .ok_or_else(|| ServiceError::Config("Das Repository muss „besitzer/name“ lauten.".into()))
}

fn epoch_ms(d: NaiveDate) -> i64 {
    d.and_hms_opt(12, 0, 0)
        .expect("12 Uhr")
        .and_utc()
        .timestamp_millis()
}

/// Pfadteil maskieren (IDs und Tabellennamen aus der Konfiguration). `byte_serialize` schreibt
/// Leerzeichen als `+`, im Pfad waere das ein echtes Plus; ein echtes Plus kommt als `%2B`.
fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// GraphQL meldet Fehler mit HTTP 200 im Feld `errors`.
fn graphql_errors(json: &Value) -> Result<(), ServiceError> {
    match json.get("errors").and_then(Value::as_array) {
        Some(errs) if !errs.is_empty() => {
            let msg = super::http::reason(json);
            let auth = errs.iter().any(|e| {
                let t = e.to_string().to_ascii_lowercase();
                t.contains("auth") || t.contains("unauthorized") || t.contains("not authenticated")
            });
            if auth {
                Err(ServiceError::Auth(401))
            } else {
                Err(ServiceError::Status(200, msg))
            }
        }
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Seiten (Notion, Confluence)
// ---------------------------------------------------------------------------

const NOTION_VERSION: &str = "2025-09-03";
const NOTION_BATCH: usize = 100;

pub fn page_create(
    acc: &Account,
    title: &str,
    md: &str,
    exec: &mut Exec,
) -> Result<Created, ServiceError> {
    let title: String = title.trim().chars().take(200).collect();
    if title.is_empty() {
        return Err(ServiceError::Config("Die Seite hat keinen Titel.".into()));
    }
    match acc.service {
        ServiceId::Notion => {
            let blocks = markdown::notion_blocks(md);
            let (first, rest) = blocks.split_at(blocks.len().min(NOTION_BATCH));
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.notion.com", "/v1/pages")?,
                )
                .header("Notion-Version", NOTION_VERSION)
                .json(json!({
                    "parent": {"page_id": acc.field("parent_page_id")?},
                    "properties": {"title": {"title": [{"text": {"content": title}}]}},
                    "children": first,
                })),
            )?;
            let r = exec(req)?;
            let id = s(&r.json, "/id");
            if let Some(page) = &id {
                notion_append_blocks(acc, page, rest, exec).map_err(|e| {
                    ServiceError::Partial(format!("Seite angelegt, aber nicht vollständig; {e}"))
                })?;
            }
            Created::from(id, s(&r.json, "/url"))
        }
        ServiceId::Confluence => {
            let site = acc.site()?;
            let mut body = json!({
                "spaceId": acc.field("space_id")?, "status": "current", "title": title,
                "body": {"representation": "storage", "value": markdown::confluence_storage(md)},
            });
            if let Some(p) = acc.field_opt("parent_page_id") {
                body["parentId"] = json!(p);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url(&format!("https://{site}"), "/wiki/api/v2/pages")?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            let url = s(&r.json, "/_links/webui").map(|w| format!("https://{site}/wiki{w}"));
            Created::from(s(&r.json, "/id"), url)
        }
        _ => Err(unsupported(acc, "Seite anlegen")),
    }
}

/// Text an eine Seite anhaengen (Notion: Bloecke; Confluence: neue Version mit Zusatz).
/// Ohne `page_id` die Elternseite aus der Integration.
pub fn page_append(
    acc: &Account,
    page_id: Option<&str>,
    md: &str,
    exec: &mut Exec,
) -> Result<Created, ServiceError> {
    let page = match page_id.map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => acc.field("parent_page_id")?,
    };
    match acc.service {
        ServiceId::Notion => {
            notion_append_blocks(acc, &page, &markdown::notion_blocks(md), exec)?;
            Ok(Created {
                id: page,
                url: None,
            })
        }
        ServiceId::Confluence => {
            let site = acc.site()?;
            let base = format!("https://{site}");
            let get = acc.auth(ApiRequest::new(
                Method::Get,
                acc.url(
                    &base,
                    &format!("/wiki/api/v2/pages/{}?body-format=storage", enc(&page)),
                )?,
            ))?;
            let cur = exec(get)?.json;
            let version = cur
                .pointer("/version/number")
                .and_then(Value::as_u64)
                .ok_or(ServiceError::BadResponse("Seite ohne Versionsnummer"))?;
            let title = s(&cur, "/title").unwrap_or_default();
            let old = s(&cur, "/body/storage/value").unwrap_or_default();
            let put = acc.auth(
                ApiRequest::new(
                    Method::Put,
                    acc.url(&base, &format!("/wiki/api/v2/pages/{}", enc(&page)))?,
                )
                .json(json!({
                    "id": page, "status": "current", "title": title,
                    "body": {"representation": "storage",
                             "value": format!("{old}{}", markdown::confluence_storage(md))},
                    "version": {"number": version + 1, "message": "Local Voice AI"},
                })),
            )?;
            let r = exec(put)?;
            let url = s(&r.json, "/_links/webui").map(|w| format!("https://{site}/wiki{w}"));
            Ok(Created { id: page, url })
        }
        _ => Err(unsupported(acc, "an Seite anhängen")),
    }
}

fn notion_append_blocks(
    acc: &Account,
    page: &str,
    blocks: &[Value],
    exec: &mut Exec,
) -> Result<(), ServiceError> {
    for batch in blocks.chunks(NOTION_BATCH) {
        let req = acc.auth(
            ApiRequest::new(
                Method::Patch,
                acc.url(
                    "https://api.notion.com",
                    &format!("/v1/blocks/{}/children", enc(page)),
                )?,
            )
            .header("Notion-Version", NOTION_VERSION)
            .json(json!({ "children": batch })),
        )?;
        exec(req)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CRM-Notiz (HubSpot, Pipedrive)
// ---------------------------------------------------------------------------

/// Notiz anlegen; mit `contact_email` wird der Kontakt gesucht und die Notiz verknuepft
/// (nicht gefunden: Notiz ohne Verknuepfung, das Ergebnis sagt es).
pub fn crm_note(
    acc: &Account,
    text: &str,
    contact_email: Option<&str>,
    exec: &mut Exec,
) -> Result<(Created, bool), ServiceError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ServiceError::Config("Die Notiz ist leer.".into()));
    }
    let email = contact_email.map(str::trim).filter(|e| e.contains('@'));
    match acc.service {
        ServiceId::Hubspot => {
            let contact = match email {
                Some(e) => {
                    let req = acc.auth(
                        ApiRequest::new(
                            Method::Post,
                            acc.url("https://api.hubapi.com", "/crm/v3/objects/contacts/search")?,
                        )
                        .json(json!({"filterGroups": [{"filters": [
                                {"propertyName": "email", "operator": "EQ", "value": e}
                            ]}], "properties": ["email"], "limit": 1})),
                    )?;
                    s(&exec(req)?.json, "/results/0/id")
                }
                None => None,
            };
            let mut body = json!({"properties": {
                "hs_timestamp": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "hs_note_body": text,
            }});
            if let Some(c) = &contact {
                // 202 = Notiz zu Kontakt (HUBSPOT_DEFINED).
                body["associations"] = json!([{"to": {"id": c}, "types": [
                    {"associationCategory": "HUBSPOT_DEFINED", "associationTypeId": 202}
                ]}]);
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.hubapi.com", "/crm/v3/objects/notes")?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            Ok((Created::from(s(&r.json, "/id"), None)?, contact.is_some()))
        }
        ServiceId::Pipedrive => {
            let person = match email {
                Some(e) => {
                    let mut u = acc.url("https://api.pipedrive.com", "/api/v2/persons/search")?;
                    u.query_pairs_mut()
                        .append_pair("term", e)
                        .append_pair("fields", "email")
                        .append_pair("exact_match", "true");
                    let req = acc.auth(ApiRequest::new(Method::Get, u))?;
                    s(&exec(req)?.json, "/data/items/0/item/id")
                }
                None => None,
            };
            let mut body = json!({"content": text});
            if let Some(p) = &person {
                body["person_id"] = json!(p.parse::<i64>().unwrap_or_default());
            }
            let req = acc.auth(
                ApiRequest::new(
                    Method::Post,
                    acc.url("https://api.pipedrive.com", "/api/v1/notes")?,
                )
                .json(body),
            )?;
            let r = exec(req)?;
            Ok((
                Created::from(s(&r.json, "/data/id"), None)?,
                person.is_some(),
            ))
        }
        _ => Err(unsupported(acc, "CRM-Notiz")),
    }
}

// ---------------------------------------------------------------------------
// Datensatz (Airtable)
// ---------------------------------------------------------------------------

pub fn record_append(
    acc: &Account,
    fields: &Map<String, Value>,
    exec: &mut Exec,
) -> Result<Created, ServiceError> {
    if fields.is_empty() {
        return Err(ServiceError::Config(
            "Der Datensatz hat keine Felder.".into(),
        ));
    }
    match acc.service {
        ServiceId::Airtable => {
            let path = format!(
                "/v0/{}/{}",
                enc(&acc.field("base_id")?),
                enc(&acc.field("table")?)
            );
            let req = acc.auth(
                ApiRequest::new(Method::Post, acc.url("https://api.airtable.com", &path)?)
                    .json(json!({"records": [{"fields": fields}], "typecast": true})),
            )?;
            let r = exec(req)?;
            Created::from(s(&r.json, "/records/0/id"), None)
        }
        _ => Err(unsupported(acc, "Datensatz anlegen")),
    }
}

// ---------------------------------------------------------------------------
// Verbindung pruefen (Knopf „Testen“ im Dialog)
// ---------------------------------------------------------------------------

/// Ein lesender Aufruf, der Schluessel und Ziel prueft, ohne etwas anzulegen. Bei
/// Webhook-Diensten gibt es keinen lesenden Aufruf: es wird nur die Adresse geprueft.
pub fn check(acc: &Account, exec: &mut Exec) -> Result<String, ServiceError> {
    let get = |base: &str, path: &str| -> Result<ApiRequest, ServiceError> {
        acc.auth(ApiRequest::new(Method::Get, acc.url(base, path)?))
    };
    let r = match acc.service {
        ServiceId::Slack | ServiceId::Teams | ServiceId::Discord => {
            acc.webhook_url()?;
            return Ok("Adresse geprüft (ohne Testnachricht).".into());
        }
        ServiceId::Notion => exec(
            get(
                "https://api.notion.com",
                &format!("/v1/pages/{}", enc(&acc.field("parent_page_id")?)),
            )?
            .header("Notion-Version", NOTION_VERSION),
        )?,
        ServiceId::Confluence => exec(get(
            &format!("https://{}", acc.site()?),
            &format!("/wiki/api/v2/spaces/{}", enc(&acc.field("space_id")?)),
        )?)?,
        ServiceId::Asana => exec(get(
            "https://app.asana.com",
            &format!("/api/1.0/projects/{}", enc(&acc.field("project_id")?)),
        )?)?,
        ServiceId::Clickup => exec(get(
            "https://api.clickup.com",
            &format!("/api/v2/list/{}", enc(&acc.field("list_id")?)),
        )?)?,
        ServiceId::Jira => exec(get(
            &format!("https://{}", acc.site()?),
            &format!("/rest/api/3/project/{}", enc(&acc.field("project_key")?)),
        )?)?,
        ServiceId::Trello => exec(get(
            "https://api.trello.com",
            &format!("/1/lists/{}", enc(&acc.field("list_id")?)),
        )?)?,
        ServiceId::Todoist => exec(get("https://api.todoist.com", "/api/v1/projects")?)?,
        ServiceId::Monday => {
            let r = exec(
                acc.auth(
                    ApiRequest::new(Method::Post, acc.url("https://api.monday.com", "/v2")?)
                        .header("API-Version", "2025-04")
                        .json(
                            json!({"query": "query ($b: [ID!]) { boards (ids: $b) { id name } }",
                                     "variables": {"b": [acc.field("board_id")?]}}),
                        ),
                )?,
            )?;
            graphql_errors(&r.json)?;
            r
        }
        ServiceId::Linear => {
            let r = exec(acc.auth(
                ApiRequest::new(Method::Post, acc.url("https://api.linear.app", "/graphql")?).json(
                    json!({"query": "query ($id: String!) { team(id: $id) { id name } }",
                                 "variables": {"id": acc.field("team_id")?}}),
                ),
            )?)?;
            graphql_errors(&r.json)?;
            r
        }
        ServiceId::Github => exec({
            let (owner, name) = github_repo(acc)?;
            get(
                "https://api.github.com",
                &format!("/repos/{}/{}", enc(&owner), enc(&name)),
            )?
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
        })?,
        ServiceId::Hubspot => exec(get(
            "https://api.hubapi.com",
            "/crm/v3/objects/notes?limit=1",
        )?)?,
        ServiceId::Pipedrive => exec(get("https://api.pipedrive.com", "/api/v1/users/me")?)?,
        ServiceId::Airtable => exec(get(
            "https://api.airtable.com",
            &format!(
                "/v0/{}/{}?maxRecords=1",
                enc(&acc.field("base_id")?),
                enc(&acc.field("table")?)
            ),
        )?)?,
    };
    let name = [
        "/name",
        "/data/name",
        "/title",
        "/properties/title/title/0/plain_text",
        "/full_name",
        "/data/team/name",
    ]
    .iter()
    .find_map(|p| s(&r.json, p));
    Ok(match name {
        Some(n) => format!("Verbunden ({n})."),
        None => "Verbunden.".into(),
    })
}

#[cfg(test)]
mod tests;
