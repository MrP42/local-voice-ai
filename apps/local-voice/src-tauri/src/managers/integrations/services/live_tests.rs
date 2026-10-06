//! Echte Aufrufe gegen Slack, Teams und Discord (Welle 1). Laufen nur auf Anforderung:
//!
//! ```text
//! LV_TEST_SLACK_WEBHOOK=https://hooks.slack.com/services/...
//! LV_TEST_TEAMS_WEBHOOK=https://...logic.azure.com/...
//! LV_TEST_DISCORD_WEBHOOK=https://discord.com/api/webhooks/...
//! cargo test --lib services::live_tests -- --ignored --nocapture
//! ```
//!
//! Ohne Variable wird der jeweilige Test uebersprungen (Meldung, kein Fehler). Die Adresse wird
//! nie ausgegeben, nur der Server.

use serde_json::Map;

use super::http::{execute, ApiRequest};
use super::ops::{chat_post, Account};
use super::registry::ServiceId;
use crate::managers::integrations::webhook::{host_of, HttpOpts};

const TEXT: &str = "## Testnachricht von Local Voice AI\n\
- Verbindung geprüft (Welle 1, Dienste)\n\
- **Bitte ignorieren**: kein Protokoll, keine Aufgabe";

fn post(service: ServiceId, var: &str) {
    let Ok(url) = std::env::var(var) else {
        eprintln!("{var} nicht gesetzt: übersprungen");
        return;
    };
    let host = url::Url::parse(url.trim())
        .map(|u| host_of(&u))
        .unwrap_or_else(|_| "?".into());
    let settings = Map::new();
    let acc = Account {
        service,
        token: &url,
        settings: &settings,
    };
    let opts = HttpOpts::default();
    let mut exec = |r: ApiRequest| execute(&r, &opts);
    match chat_post(&acc, TEXT, &mut exec) {
        Ok(n) => eprintln!("{service:?}: {n} Nachricht(en) an {host} gesendet"),
        Err(e) => panic!("{service:?} an {host}: {e} ({})", e.code()),
    }
}

#[test]
#[ignore = "echter Aufruf; LV_TEST_SLACK_WEBHOOK setzen"]
fn slack_receives_a_test_message() {
    post(ServiceId::Slack, "LV_TEST_SLACK_WEBHOOK");
}

#[test]
#[ignore = "echter Aufruf; LV_TEST_TEAMS_WEBHOOK setzen"]
fn teams_receives_a_test_message() {
    post(ServiceId::Teams, "LV_TEST_TEAMS_WEBHOOK");
}

#[test]
#[ignore = "echter Aufruf; LV_TEST_DISCORD_WEBHOOK setzen"]
fn discord_receives_a_test_message() {
    post(ServiceId::Discord, "LV_TEST_DISCORD_WEBHOOK");
}
