//! Jede Anfrage gegen die API-Doku (Stand 06.10.2026): Methode, Host/Pfad, Anmeldung, Koerper.
//! Die Attrappe schreibt Anfragen mit und liefert vorgegebene Antworten.

use super::*;
use crate::managers::integrations::services::http::ApiReply;

struct Fake {
    calls: Vec<ApiRequest>,
    replies: Vec<Result<ApiReply, ServiceError>>,
}

impl Fake {
    fn new(replies: Vec<Value>) -> Self {
        Self {
            calls: Vec::new(),
            replies: replies.into_iter().map(|j| Ok(ApiReply::ok(j))).collect(),
        }
    }
    fn exec(&mut self) -> impl FnMut(ApiRequest) -> Result<ApiReply, ServiceError> + '_ {
        move |req| {
            self.calls.push(req);
            if self.replies.is_empty() {
                Ok(ApiReply::ok(Value::Null))
            } else {
                self.replies.remove(0)
            }
        }
    }
}

fn settings(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

fn task() -> TaskInput {
    TaskInput {
        title: "Tests reparieren".into(),
        description: "Bis zum Release".into(),
        due: NaiveDate::from_ymd_opt(2026, 10, 9),
    }
}

fn path(r: &ApiRequest) -> String {
    format!("{}{}", r.url.host_str().unwrap_or(""), r.url.path())
}

// --- Kanal ------------------------------------------------------------------

#[test]
fn slack_posts_mrkdwn_text_to_the_webhook() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Slack,
        token: "https://hooks.slack.com/services/T/B/X",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    let n = chat_post(&acc, "## Protokoll\n- **wichtig**", &mut f.exec()).unwrap();
    assert_eq!(n, 1);
    let r = &f.calls[0];
    assert_eq!(r.method, Method::Post);
    assert_eq!(path(r), "hooks.slack.com/services/T/B/X");
    assert_eq!(r.body.as_ref().unwrap()["text"], "*Protokoll*\n- *wichtig*");
    assert!(r.header_value("Authorization").is_none());
}

#[test]
fn teams_posts_an_adaptive_card() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Teams,
        token: "https://prod-1.westeurope.logic.azure.com/workflows/abc/triggers/manual/paths/invoke?sig=x",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    chat_post(&acc, "Hallo", &mut f.exec()).unwrap();
    let b = f.calls[0].body.as_ref().unwrap();
    assert_eq!(
        b["attachments"][0]["contentType"],
        "application/vnd.microsoft.card.adaptive"
    );
    assert_eq!(b["attachments"][0]["content"]["body"][0]["text"], "Hallo");
}

#[test]
fn discord_splits_into_2000_char_messages_without_mentions() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Discord,
        token: "https://discord.com/api/webhooks/1/tok",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    let n = chat_post(&acc, &"x".repeat(4100), &mut f.exec()).unwrap();
    assert_eq!(n, 3);
    assert_eq!(
        f.calls[0].body.as_ref().unwrap()["content"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        2000
    );
    assert_eq!(
        f.calls[0].body.as_ref().unwrap()["allowed_mentions"]["parse"],
        json!([])
    );
}

#[test]
fn a_failure_after_the_first_discord_message_is_partial_never_retried() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Discord,
        token: "https://discord.com/api/webhooks/1/tok",
        settings: &st,
    };
    let mut calls = 0;
    let mut exec = |_r: ApiRequest| {
        calls += 1;
        if calls == 1 {
            Ok(ApiReply::ok(Value::Null))
        } else {
            Err(ServiceError::Connect("discord.com".into()))
        }
    };
    let e = chat_post(&acc, &"x".repeat(2500), &mut exec).unwrap_err();
    assert!(matches!(e, ServiceError::Partial(_)), "{e:?}");
    assert_eq!(e.class(), super::super::http::Class::Unknown);
}

#[test]
fn a_webhook_for_another_host_is_refused_before_sending() {
    let st = settings(&[]);
    for (svc, url) in [
        (ServiceId::Slack, "https://evil.example/services/T/B/X"),
        (ServiceId::Slack, "http://hooks.slack.com/services/x"),
        (
            ServiceId::Discord,
            "https://discord.com.evil.net/api/webhooks/1/x",
        ),
        (ServiceId::Teams, "https://user:pw@prod.logic.azure.com/x"),
    ] {
        let acc = Account {
            service: svc,
            token: url,
            settings: &st,
        };
        let mut f = Fake::new(vec![]);
        let err = chat_post(&acc, "x", &mut f.exec()).unwrap_err();
        assert!(matches!(err, ServiceError::Config(_)), "{url}: {err:?}");
        assert!(f.calls.is_empty(), "{url}: darf nichts senden");
        assert!(
            !err.to_string().contains("/services/"),
            "Pfad (Geheimnis) im Fehlertext"
        );
    }
}

// --- Aufgaben ---------------------------------------------------------------

#[test]
fn asana_creates_a_task_in_the_project() {
    let st = settings(&[("project_id", "120")]);
    let acc = Account {
        service: ServiceId::Asana,
        token: "pat",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"data": {"gid": "9", "permalink_url": "https://app.asana.com/0/120/9"}}),
    ]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(
        c,
        Created {
            id: "9".into(),
            url: Some("https://app.asana.com/0/120/9".into())
        }
    );
    let r = &f.calls[0];
    assert_eq!(path(r), "app.asana.com/api/1.0/tasks");
    assert_eq!(r.header_value("Authorization"), Some("Bearer pat"));
    let d = &r.body.as_ref().unwrap()["data"];
    assert_eq!(d["projects"], json!(["120"]));
    assert_eq!(d["due_on"], "2026-10-09");
    assert_eq!(d["name"], "Tests reparieren");
}

#[test]
fn clickup_uses_raw_token_and_epoch_due_date() {
    let st = settings(&[("list_id", "L1")]);
    let acc = Account {
        service: ServiceId::Clickup,
        token: "pk_1",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"id": "t1", "url": "https://app.clickup.com/t/t1"}),
    ]);
    task_create(&acc, &task(), &mut f.exec()).unwrap();
    let r = &f.calls[0];
    assert_eq!(path(r), "api.clickup.com/api/v2/list/L1/task");
    assert_eq!(r.header_value("Authorization"), Some("pk_1"));
    assert_eq!(
        r.body.as_ref().unwrap()["due_date"],
        json!(1_791_547_200_000_i64)
    );
}

#[test]
fn jira_uses_basic_auth_adf_and_the_site() {
    let st = settings(&[
        ("site", "https://firma.atlassian.net/"),
        ("email", "p@x.de"),
        ("project_key", "LV"),
    ]);
    let acc = Account {
        service: ServiceId::Jira,
        token: "tok",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"id": "1", "key": "LV-7"})]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(
        c.url.as_deref(),
        Some("https://firma.atlassian.net/browse/LV-7")
    );
    let r = &f.calls[0];
    assert_eq!(path(r), "firma.atlassian.net/rest/api/3/issue");
    // base64("p@x.de:tok")
    assert_eq!(
        r.header_value("Authorization"),
        Some("Basic cEB4LmRlOnRvaw==")
    );
    let fields = &r.body.as_ref().unwrap()["fields"];
    assert_eq!(fields["issuetype"]["name"], "Task");
    assert_eq!(fields["description"]["type"], "doc");
    assert_eq!(fields["duedate"], "2026-10-09");
}

#[test]
fn jira_refuses_a_site_outside_atlassian() {
    let st = settings(&[
        ("site", "evil.example"),
        ("email", "p@x.de"),
        ("project_key", "LV"),
    ]);
    let acc = Account {
        service: ServiceId::Jira,
        token: "tok",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    assert!(matches!(
        task_create(&acc, &task(), &mut f.exec()),
        Err(ServiceError::Config(_))
    ));
    assert!(f.calls.is_empty());
}

#[test]
fn trello_keeps_key_and_token_out_of_the_url() {
    let st = settings(&[("api_key", "KEY"), ("list_id", "L")]);
    let acc = Account {
        service: ServiceId::Trello,
        token: "TOK",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"id": "c", "shortUrl": "https://trello.com/c/x"}),
    ]);
    task_create(&acc, &task(), &mut f.exec()).unwrap();
    let r = &f.calls[0];
    assert_eq!(r.url.query(), None);
    assert_eq!(
        r.header_value("Authorization"),
        Some("OAuth oauth_consumer_key=\"KEY\", oauth_token=\"TOK\"")
    );
    assert_eq!(r.body.as_ref().unwrap()["idList"], "L");
}

#[test]
fn todoist_v1_with_optional_project() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Todoist,
        token: "t",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"id": "6X"})]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(
        c.url.as_deref(),
        Some("https://app.todoist.com/app/task/6X")
    );
    let r = &f.calls[0];
    assert_eq!(path(r), "api.todoist.com/api/v1/tasks");
    assert!(r.body.as_ref().unwrap().get("project_id").is_none());
    assert_eq!(r.body.as_ref().unwrap()["due_date"], "2026-10-09");
}

#[test]
fn monday_creates_item_then_update_and_reports_graphql_errors() {
    let st = settings(&[("board_id", "42"), ("group_id", "topics")]);
    let acc = Account {
        service: ServiceId::Monday,
        token: "m",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"data": {"create_item": {"id": "77"}}}),
        json!({"data": {"create_update": {"id": "1"}}}),
    ]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(c.id, "77");
    assert_eq!(f.calls.len(), 2);
    assert!(f.calls[0].body.as_ref().unwrap()["query"]
        .as_str()
        .unwrap()
        .contains("group_id"));
    assert_eq!(f.calls[1].body.as_ref().unwrap()["variables"]["item"], "77");

    let mut f = Fake::new(vec![json!({"errors": [{"message": "Not Authenticated"}]})]);
    assert!(matches!(
        task_create(&acc, &task(), &mut f.exec()),
        Err(ServiceError::Auth(_))
    ));
}

#[test]
fn linear_issue_create_returns_identifier_and_url() {
    let st = settings(&[("team_id", "T")]);
    let acc = Account {
        service: ServiceId::Linear,
        token: "lin_api",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"data": {"issueCreate": {"success": true,
        "issue": {"id": "u", "identifier": "ENG-5", "url": "https://linear.app/x/issue/ENG-5"}}}})]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(c.id, "ENG-5");
    assert_eq!(f.calls[0].header_value("Authorization"), Some("lin_api"));
    assert_eq!(
        f.calls[0].body.as_ref().unwrap()["variables"]["input"]["dueDate"],
        "2026-10-09"
    );
}

#[test]
fn github_issue_with_api_version_header_and_strict_repo() {
    let st = settings(&[("repo", "mrp/app")]);
    let acc = Account {
        service: ServiceId::Github,
        token: "ghp",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"number": 12, "html_url": "https://github.com/mrp/app/issues/12"}),
    ]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(c.id, "12");
    let r = &f.calls[0];
    assert_eq!(path(r), "api.github.com/repos/mrp/app/issues");
    assert_eq!(r.header_value("X-GitHub-Api-Version"), Some("2022-11-28"));
    assert!(r.body.as_ref().unwrap()["body"]
        .as_str()
        .unwrap()
        .contains("Fällig: 2026-10-09"));

    let bad = settings(&[("repo", "../../user")]);
    let acc = Account {
        service: ServiceId::Github,
        token: "ghp",
        settings: &bad,
    };
    let mut f = Fake::new(vec![]);
    assert!(matches!(
        task_create(&acc, &task(), &mut f.exec()),
        Err(ServiceError::Config(_))
    ));
}

#[test]
fn hubspot_and_pipedrive_tasks() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Hubspot,
        token: "svc",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"id": "5"})]);
    task_create(&acc, &task(), &mut f.exec()).unwrap();
    let p = &f.calls[0].body.as_ref().unwrap()["properties"];
    assert_eq!(p["hs_task_status"], "NOT_STARTED");
    assert_eq!(p["hs_timestamp"], "2026-10-09T12:00:00Z");

    let acc = Account {
        service: ServiceId::Pipedrive,
        token: "pd",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"data": {"id": 3}})]);
    let c = task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert_eq!(c.id, "3");
    assert_eq!(path(&f.calls[0]), "api.pipedrive.com/api/v2/activities");
    assert_eq!(f.calls[0].header_value("x-api-token"), Some("pd"));
}

#[test]
fn chat_services_do_not_create_tasks() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Slack,
        token: "https://hooks.slack.com/services/x",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    assert!(matches!(
        task_create(&acc, &task(), &mut f.exec()),
        Err(ServiceError::Config(_))
    ));
}

#[test]
fn missing_token_or_field_is_a_clear_config_error() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Asana,
        token: "pat",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    let e = task_create(&acc, &task(), &mut f.exec()).unwrap_err();
    assert!(e.to_string().contains("project_id"), "{e}");
    let st = settings(&[("project_id", "1")]);
    let acc = Account {
        service: ServiceId::Asana,
        token: "  ",
        settings: &st,
    };
    let e = task_create(&acc, &task(), &mut f.exec()).unwrap_err();
    assert!(e.to_string().contains("Schlüssel"), "{e}");
    assert!(f.calls.is_empty());
}

// --- Seiten -----------------------------------------------------------------

#[test]
fn notion_page_with_version_header_and_batched_children() {
    let st = settings(&[("parent_page_id", "P")]);
    let acc = Account {
        service: ServiceId::Notion,
        token: "ntn",
        settings: &st,
    };
    let md: String = (0..150).map(|i| format!("- Punkt {i}\n")).collect();
    let mut f = Fake::new(vec![
        json!({"id": "NEW", "url": "https://www.notion.so/NEW"}),
    ]);
    let c = page_create(&acc, "Protokoll", &md, &mut f.exec()).unwrap();
    assert_eq!(c.id, "NEW");
    assert_eq!(f.calls.len(), 2, "100 Bloecke beim Anlegen, 50 angehaengt");
    let first = &f.calls[0];
    assert_eq!(first.header_value("Notion-Version"), Some("2025-09-03"));
    assert_eq!(first.body.as_ref().unwrap()["parent"]["page_id"], "P");
    assert_eq!(
        first.body.as_ref().unwrap()["children"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    assert_eq!(
        first.body.as_ref().unwrap()["properties"]["title"]["title"][0]["text"]["content"],
        "Protokoll"
    );
    assert_eq!(f.calls[1].method, Method::Patch);
    assert_eq!(path(&f.calls[1]), "api.notion.com/v1/blocks/NEW/children");
}

#[test]
fn confluence_append_reads_version_then_puts_next_version() {
    let st = settings(&[
        ("site", "firma.atlassian.net"),
        ("email", "p@x.de"),
        ("space_id", "S"),
        ("parent_page_id", "11"),
    ]);
    let acc = Account {
        service: ServiceId::Confluence,
        token: "t",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"id": "11", "title": "Entscheidungen", "version": {"number": 4}, "body": {"storage": {"value": "<p>alt</p>"}}}),
        json!({"id": "11", "_links": {"webui": "/spaces/S/pages/11"}}),
    ]);
    let c = page_append(&acc, None, "- neu", &mut f.exec()).unwrap();
    assert_eq!(
        c.url.as_deref(),
        Some("https://firma.atlassian.net/wiki/spaces/S/pages/11")
    );
    assert_eq!(f.calls[0].method, Method::Get);
    assert_eq!(f.calls[1].method, Method::Put);
    let b = f.calls[1].body.as_ref().unwrap();
    assert_eq!(b["version"]["number"], 5);
    assert_eq!(b["body"]["value"], "<p>alt</p><ul><li>neu</li></ul>");
}

// --- CRM / Datensatz --------------------------------------------------------

#[test]
fn hubspot_note_links_to_the_contact_found_by_email() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Hubspot,
        token: "svc",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"results": [{"id": "501"}]}),
        json!({"id": "n1"}),
    ]);
    let (c, linked) = crm_note(
        &acc,
        "Gespräch: Angebot folgt.",
        Some("kunde@firma.de"),
        &mut f.exec(),
    )
    .unwrap();
    assert!(linked);
    assert_eq!(c.id, "n1");
    assert_eq!(
        path(&f.calls[0]),
        "api.hubapi.com/crm/v3/objects/contacts/search"
    );
    let note = f.calls[1].body.as_ref().unwrap();
    assert_eq!(note["associations"][0]["to"]["id"], "501");
    assert_eq!(
        note["associations"][0]["types"][0]["associationTypeId"],
        202
    );
}

#[test]
fn pipedrive_note_without_match_is_still_written_unlinked() {
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Pipedrive,
        token: "pd",
        settings: &st,
    };
    let mut f = Fake::new(vec![
        json!({"data": {"items": []}}),
        json!({"data": {"id": 9}}),
    ]);
    let (c, linked) = crm_note(&acc, "Notiz", Some("neu@firma.de"), &mut f.exec()).unwrap();
    assert!(!linked);
    assert_eq!(c.id, "9");
    assert!(f.calls[0]
        .url
        .query()
        .unwrap()
        .contains("term=neu%40firma.de"));
    assert!(f.calls[1].body.as_ref().unwrap().get("person_id").is_none());
}

#[test]
fn airtable_appends_one_record_with_typecast() {
    let st = settings(&[("base_id", "app1"), ("table", "Meeting Log")]);
    let acc = Account {
        service: ServiceId::Airtable,
        token: "pat",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"records": [{"id": "rec1"}]})]);
    let fields: Map<String, Value> = [("Titel".to_string(), json!("Case ie2s"))]
        .into_iter()
        .collect();
    let c = record_append(&acc, &fields, &mut f.exec()).unwrap();
    assert_eq!(c.id, "rec1");
    assert_eq!(path(&f.calls[0]), "api.airtable.com/v0/app1/Meeting%20Log");
    assert_eq!(f.calls[0].body.as_ref().unwrap()["typecast"], true);
}

// --- Pruefen / Sicherheit ---------------------------------------------------

#[test]
fn check_reads_without_creating_and_names_the_target() {
    let st = settings(&[("project_id", "120")]);
    let acc = Account {
        service: ServiceId::Asana,
        token: "pat",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"data": {"name": "Kundenprojekt"}})]);
    assert_eq!(
        check(&acc, &mut f.exec()).unwrap(),
        "Verbunden (Kundenprojekt)."
    );
    assert_eq!(f.calls[0].method, Method::Get);
    let st = settings(&[]);
    let acc = Account {
        service: ServiceId::Slack,
        token: "https://hooks.slack.com/services/x",
        settings: &st,
    };
    let mut f = Fake::new(vec![]);
    check(&acc, &mut f.exec()).unwrap();
    assert!(f.calls.is_empty(), "keine Testnachricht in den Kanal");
}

#[test]
fn request_debug_output_never_contains_the_secret() {
    let st = settings(&[("project_id", "1")]);
    let acc = Account {
        service: ServiceId::Asana,
        token: "GEHEIM",
        settings: &st,
    };
    let mut f = Fake::new(vec![json!({"data": {"gid": "1"}})]);
    task_create(&acc, &task(), &mut f.exec()).unwrap();
    assert!(!format!("{:?}", f.calls[0]).contains("GEHEIM"));
}
