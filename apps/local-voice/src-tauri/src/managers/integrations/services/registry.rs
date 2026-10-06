//! Register der Dienste (Top 20, Recherche 06.10.2026): Anmeldeart, erlaubte Hosts,
//! Faehigkeiten und die Felder, die der Dialog „Integration hinzufuegen“ abfragt.
//!
//! Geheim ist nur das Fach `token` (Webhook-Adresse, API-Schluessel, App-Passwort); alles in
//! `fields` ist Konfiguration (Projekt-ID, Site, E-Mail) und darf angezeigt werden.

use serde::Serialize;
use specta::Type;

use super::http::HostRule;
use crate::managers::integrations::model::Capability;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ServiceId {
    Slack,
    Teams,
    Discord,
    Notion,
    Confluence,
    Asana,
    Clickup,
    Jira,
    Trello,
    Todoist,
    Monday,
    Linear,
    Github,
    Hubspot,
    Pipedrive,
    Airtable,
    /// iCloud-Kalender per CalDAV (Welle 3): Folgetermine schreiben.
    Icloud,
}

impl ServiceId {
    pub const ALL: [ServiceId; 17] = [
        ServiceId::Slack,
        ServiceId::Teams,
        ServiceId::Discord,
        ServiceId::Notion,
        ServiceId::Confluence,
        ServiceId::Asana,
        ServiceId::Clickup,
        ServiceId::Jira,
        ServiceId::Trello,
        ServiceId::Todoist,
        ServiceId::Monday,
        ServiceId::Linear,
        ServiceId::Github,
        ServiceId::Hubspot,
        ServiceId::Pipedrive,
        ServiceId::Airtable,
        ServiceId::Icloud,
    ];

    pub fn as_str(self) -> &'static str {
        def(self).id
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == s)
    }
}

/// Wie sich der Dienst anmeldet. Das Geheimnis steht immer im Fach `token`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Das Geheimnis ist eine Webhook-Adresse (Schluessel im Pfad).
    WebhookUrl,
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// `Authorization: <token>` ohne Vorsilbe (ClickUp, monday, Linear-API-Key).
    RawAuthorization,
    /// HTTP Basic mit E-Mail (Feld `email`) und Token (Atlassian; iCloud: Apple-ID und
    /// app-spezifisches Passwort).
    BasicEmailToken,
    /// `x-api-token: <token>` (Pipedrive).
    ApiTokenHeader,
    /// Trello: oeffentlicher API-Key (Feld `api_key`) und Nutzer-Token, als OAuth-Kopfzeile.
    TrelloKeyToken,
}

/// Ein Feld im Dialog (nicht geheim).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDef {
    pub key: &'static str,
    pub required: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ServiceDef {
    pub id: &'static str,
    pub label: &'static str,
    pub auth: AuthKind,
    pub hosts: &'static [HostRule],
    pub capabilities: &'static [Capability],
    /// Faehigkeiten fuer die Oberflaeche (Punktschreibweise).
    pub capability_ids: &'static [&'static str],
    pub fields: &'static [FieldDef],
    /// Wo man den Schluessel bekommt (Doku des Anbieters).
    pub token_help_url: &'static str,
}

const fn req(key: &'static str) -> FieldDef {
    FieldDef {
        key,
        required: true,
    }
}
const fn opt(key: &'static str) -> FieldDef {
    FieldDef {
        key,
        required: false,
    }
}

use Capability::{CalendarWrite, ChatPost, CrmWrite, PageWrite, RecordWrite, TaskCreate};

static DEFS: [ServiceDef; 17] = [
    ServiceDef {
        id: "slack",
        label: "Slack",
        auth: AuthKind::WebhookUrl,
        hosts: &[HostRule::Exact("hooks.slack.com")],
        capabilities: &[ChatPost],
        capability_ids: &["chat.post"],
        fields: &[],
        token_help_url: "https://docs.slack.dev/messaging/sending-messages-using-incoming-webhooks",
    },
    ServiceDef {
        id: "teams",
        label: "Microsoft Teams",
        auth: AuthKind::WebhookUrl,
        // Workflows-Webhooks (Power Automate) seit dem Ende der Office-365-Connectors (05/2026).
        hosts: &[
            HostRule::Suffix("logic.azure.com"),
            HostRule::Suffix("powerplatform.com"),
            HostRule::Suffix("environment.api.powerplatform.com"),
        ],
        capabilities: &[ChatPost],
        capability_ids: &["chat.post"],
        fields: &[],
        token_help_url: "https://support.microsoft.com/en-us/office/create-incoming-webhooks-with-workflows-for-microsoft-teams-8ae491c7-0394-4861-ba59-055e33f75498",
    },
    ServiceDef {
        id: "discord",
        label: "Discord",
        auth: AuthKind::WebhookUrl,
        hosts: &[
            HostRule::Exact("discord.com"),
            HostRule::Exact("discordapp.com"),
            HostRule::Exact("ptb.discord.com"),
            HostRule::Exact("canary.discord.com"),
        ],
        capabilities: &[ChatPost],
        capability_ids: &["chat.post"],
        fields: &[],
        token_help_url: "https://support.discord.com/hc/en-us/articles/228383668",
    },
    ServiceDef {
        id: "notion",
        label: "Notion",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("api.notion.com")],
        capabilities: &[PageWrite],
        capability_ids: &["page.write"],
        // Elternseite, unter der Protokolle entstehen (der Integration freigegeben).
        fields: &[req("parent_page_id")],
        token_help_url: "https://www.notion.com/help/create-integrations-with-the-notion-api",
    },
    ServiceDef {
        id: "confluence",
        label: "Confluence",
        auth: AuthKind::BasicEmailToken,
        hosts: &[HostRule::Suffix("atlassian.net")],
        capabilities: &[PageWrite],
        capability_ids: &["page.write"],
        fields: &[req("site"), req("email"), req("space_id"), opt("parent_page_id")],
        token_help_url: "https://id.atlassian.com/manage-profile/security/api-tokens",
    },
    ServiceDef {
        id: "asana",
        label: "Asana",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("app.asana.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("project_id")],
        token_help_url: "https://developers.asana.com/docs/personal-access-token",
    },
    ServiceDef {
        id: "clickup",
        label: "ClickUp",
        auth: AuthKind::RawAuthorization,
        hosts: &[HostRule::Exact("api.clickup.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("list_id")],
        token_help_url: "https://developer.clickup.com/docs/authentication",
    },
    ServiceDef {
        id: "jira",
        label: "Jira",
        auth: AuthKind::BasicEmailToken,
        hosts: &[HostRule::Suffix("atlassian.net")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("site"), req("email"), req("project_key"), opt("issue_type")],
        token_help_url: "https://id.atlassian.com/manage-profile/security/api-tokens",
    },
    ServiceDef {
        id: "trello",
        label: "Trello",
        auth: AuthKind::TrelloKeyToken,
        hosts: &[HostRule::Exact("api.trello.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("api_key"), req("list_id")],
        token_help_url: "https://developer.atlassian.com/cloud/trello/guides/rest-api/api-introduction/",
    },
    ServiceDef {
        id: "todoist",
        label: "Todoist",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("api.todoist.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[opt("project_id")],
        token_help_url: "https://todoist.com/help/articles/find-your-api-token-Jpzx9IIlB",
    },
    ServiceDef {
        id: "monday",
        label: "monday.com",
        auth: AuthKind::RawAuthorization,
        hosts: &[HostRule::Exact("api.monday.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("board_id"), opt("group_id")],
        token_help_url: "https://developer.monday.com/api-reference/docs/authentication",
    },
    ServiceDef {
        id: "linear",
        label: "Linear",
        auth: AuthKind::RawAuthorization,
        hosts: &[HostRule::Exact("api.linear.app")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("team_id")],
        token_help_url: "https://linear.app/developers/graphql",
    },
    ServiceDef {
        id: "github",
        label: "GitHub",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("api.github.com")],
        capabilities: &[TaskCreate],
        capability_ids: &["task.create"],
        fields: &[req("repo")],
        token_help_url: "https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens",
    },
    ServiceDef {
        id: "hubspot",
        label: "HubSpot",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("api.hubapi.com")],
        capabilities: &[CrmWrite, TaskCreate],
        capability_ids: &["crm.write", "task.create"],
        fields: &[],
        token_help_url: "https://developers.hubspot.com/blog/hubspot-service-keys-the-right-api-credential-for-data-integrations",
    },
    ServiceDef {
        id: "pipedrive",
        label: "Pipedrive",
        auth: AuthKind::ApiTokenHeader,
        hosts: &[HostRule::Exact("api.pipedrive.com")],
        capabilities: &[CrmWrite, TaskCreate],
        capability_ids: &["crm.write", "task.create"],
        fields: &[],
        token_help_url: "https://pipedrive.readme.io/docs/how-to-find-the-api-token",
    },
    ServiceDef {
        id: "airtable",
        label: "Airtable",
        auth: AuthKind::Bearer,
        hosts: &[HostRule::Exact("api.airtable.com")],
        capabilities: &[RecordWrite],
        capability_ids: &["record.write"],
        fields: &[req("base_id"), req("table")],
        token_help_url: "https://airtable.com/developers/web/guides/personal-access-tokens",
    },
    ServiceDef {
        id: "icloud",
        label: "iCloud-Kalender",
        auth: AuthKind::BasicEmailToken,
        // caldav.icloud.com und die Kalender-Server pNN-caldav.icloud.com.
        hosts: &[HostRule::Suffix("icloud.com")],
        capabilities: &[CalendarWrite],
        capability_ids: &["calendar.write"],
        // Apple-ID; Kalender nach Namen (ohne Angabe: der erste mit Terminen).
        fields: &[req("email"), opt("calendar")],
        token_help_url: "https://support.apple.com/de-de/102654",
    },
];

pub fn def(id: ServiceId) -> &'static ServiceDef {
    let i = ServiceId::ALL
        .iter()
        .position(|x| *x == id)
        .expect("im Register");
    &DEFS[i]
}

pub fn all() -> &'static [ServiceDef] {
    &DEFS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_service_has_hosts_capabilities_and_matching_ids() {
        for id in ServiceId::ALL {
            let d = def(id);
            assert_eq!(ServiceId::parse(d.id), Some(id));
            assert!(!d.hosts.is_empty(), "{}", d.id);
            assert!(!d.capabilities.is_empty(), "{}", d.id);
            assert_eq!(d.capabilities.len(), d.capability_ids.len(), "{}", d.id);
            for (c, s) in d.capabilities.iter().zip(d.capability_ids) {
                assert_eq!(c.as_str(), *s, "{}", d.id);
            }
            assert!(d.token_help_url.starts_with("https://"), "{}", d.id);
        }
    }

    #[test]
    fn host_rules_accept_only_the_service() {
        let slack = def(ServiceId::Slack).hosts;
        assert!(slack.iter().any(|r| r.matches("hooks.slack.com")));
        assert!(!slack.iter().any(|r| r.matches("hooks.slack.com.evil.net")));
        let jira = def(ServiceId::Jira).hosts;
        assert!(jira.iter().any(|r| r.matches("firma.atlassian.net")));
        assert!(!jira.iter().any(|r| r.matches("atlassian.net.evil.org")));
        assert!(!jira.iter().any(|r| r.matches("evilatlassian.net")));
        let teams = def(ServiceId::Teams).hosts;
        assert!(teams
            .iter()
            .any(|r| r.matches("prod-12.westeurope.logic.azure.com")));
    }
}
