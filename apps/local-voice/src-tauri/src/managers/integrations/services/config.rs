//! Konfiguration einer Dienst-Integration (`Kind::Service`): Pruefung beim Anlegen und Aendern.
//!
//! Im Register steht `{"service": "<id>", <Felder des Dienstes>, "host": <Server der Webhook-
//! Adresse>}`; nie das Geheimnis. Unbekannte Felder werden verworfen, Pflichtfelder muessen
//! gefuellt sein, eine Atlassian-Site muss auf `atlassian.net` liegen. Die Webhook-Adresse und
//! der Schluessel werden beim Eintragen geprueft (`check_secret`), nicht erst beim ersten Lauf.

use serde::Serialize;
use serde_json::{json, Map, Value};
use specta::Type;

use super::http::{check_url, HostRule};
use super::registry::{all, def, AuthKind, ServiceId};
use crate::managers::integrations::webhook::host_of;

/// Laengster Feldwert (IDs, Site, E-Mail, Tabellenname).
pub const MAX_FIELD_CHARS: usize = 300;
/// Laengster Schluessel bzw. laengste Webhook-Adresse.
pub const MAX_SECRET_CHARS: usize = 4096;

pub const ERR_SERVICE_UNKNOWN: &str = "service_unknown";

/// Anzeigename eines Felds in Fehlermeldungen (die Oberflaeche hat eigene Beschriftungen).
pub fn field_label(key: &str) -> &'static str {
    match key {
        "parent_page_id" => "Elternseite (ID)",
        "site" => "Site",
        "email" => "E-Mail",
        "space_id" => "Bereichs-ID",
        "project_id" => "Projekt-ID",
        "list_id" => "Listen-ID",
        "project_key" => "Projektschlüssel",
        "issue_type" => "Vorgangstyp",
        "api_key" => "API-Key",
        "board_id" => "Board-ID",
        "group_id" => "Gruppen-ID",
        "team_id" => "Team-ID",
        "repo" => "Repository",
        "base_id" => "Base-ID",
        "table" => "Tabelle",
        "calendar" => "Kalender (Name)",
        _ => "Feld",
    }
}

/// Site als Host (`https://firma.atlassian.net/` -> `firma.atlassian.net`).
pub fn site_host(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn host_allowed(host: &str, rules: &[HostRule]) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && rules.iter().any(|r| r.matches(host))
}

/// Der Dienst einer rohen Konfiguration.
pub fn service_of(raw: &Value) -> Option<ServiceId> {
    ServiceId::parse(raw.get("service")?.as_str()?)
}

/// Prueft und bereinigt die Konfiguration; liefert sie samt Anzeige (`account_hint`: Site,
/// Server der Webhook-Adresse oder API-Host, nie ein Geheimnis).
pub fn normalize(raw: &Value) -> Result<(Value, Option<String>), String> {
    let id = service_of(raw).ok_or_else(|| ERR_SERVICE_UNKNOWN.to_string())?;
    let d = def(id);
    let mut out = Map::new();
    out.insert("service".into(), json!(d.id));
    for f in d.fields {
        let v = raw
            .get(f.key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        if v.is_empty() {
            if f.required {
                return Err(format!(
                    "Für {} fehlt das Feld „{}“.",
                    d.label,
                    field_label(f.key)
                ));
            }
            continue;
        }
        if v.chars().count() > MAX_FIELD_CHARS || v.chars().any(char::is_control) {
            return Err(format!(
                "Das Feld „{}“ ist zu lang oder enthält Steuerzeichen.",
                field_label(f.key)
            ));
        }
        let v = match f.key {
            "site" => {
                let host = site_host(v);
                if !host_allowed(&host, d.hosts) {
                    return Err(format!(
                        "Die Site muss auf atlassian.net liegen (z. B. firma.atlassian.net), nicht „{host}“."
                    ));
                }
                host
            }
            "email" if !v.contains('@') => {
                return Err("Die E-Mail-Adresse ist ungültig.".to_string());
            }
            "repo" => {
                let ok = v.split_once('/').is_some_and(|(o, n)| {
                    [o, n]
                        .iter()
                        .all(|p| !p.is_empty() && !p.contains('/') && *p != "." && *p != "..")
                });
                if !ok {
                    return Err("Das Repository muss „besitzer/name“ lauten.".to_string());
                }
                v.to_string()
            }
            _ => v.to_string(),
        };
        out.insert(f.key.into(), json!(v));
    }
    let hint = match d.auth {
        AuthKind::WebhookUrl => {
            let host = raw
                .get("host")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .to_ascii_lowercase();
            if host.is_empty() {
                return Err("Die Webhook-Adresse fehlt.".to_string());
            }
            if !host_allowed(&host, d.hosts) {
                return Err(format!("Die Adresse gehört nicht zu {} ({host}).", d.label));
            }
            out.insert("host".into(), json!(host));
            Some(host)
        }
        _ if id == ServiceId::Icloud => Some("caldav.icloud.com".to_string()),
        _ => out
            .get("site")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                d.hosts.iter().find_map(|r| match r {
                    HostRule::Exact(h) => Some(h.to_string()),
                    HostRule::Suffix(_) => None,
                })
            }),
    };
    Ok((Value::Object(out), hint))
}

/// Prueft das Geheimnis beim Eintragen: Webhook-Adresse gegen die Hosts des Dienstes,
/// Schluessel ohne Leer- und Steuerzeichen (sie gingen sonst in eine Kopfzeile).
pub fn check_secret(id: ServiceId, secret: &str) -> Result<(), String> {
    let s = secret.trim();
    if s.chars().count() > MAX_SECRET_CHARS {
        return Err("Der Schlüssel ist zu lang.".to_string());
    }
    match def(id).auth {
        AuthKind::WebhookUrl => {
            let url = url::Url::parse(s).map_err(|_| {
                "Die Webhook-Adresse ist ungültig (vollständig mit https:// einfügen).".to_string()
            })?;
            check_url(&url, def(id).hosts, false).map_err(|e| e.to_string())
        }
        _ => {
            if s.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(
                    "Der Schlüssel enthält Leer- oder Steuerzeichen. Bitte nur den Schlüssel einfügen."
                        .to_string(),
                );
            }
            Ok(())
        }
    }
}

/// Server einer Webhook-Adresse fuer die Konfiguration (Anzeige); `None` bei Schluesseln.
pub fn webhook_host(id: ServiceId, secret: &str) -> Option<String> {
    if def(id).auth != AuthKind::WebhookUrl {
        return None;
    }
    url::Url::parse(secret.trim()).ok().map(|u| host_of(&u))
}

/// Ein Dienst, wie die Oberflaeche ihn fuer den Dialog „Integration hinzufuegen“ braucht.
#[derive(Clone, Debug, Serialize, Type)]
pub struct ServiceInfo {
    pub id: ServiceId,
    pub label: String,
    pub auth: AuthKind,
    pub capabilities: Vec<String>,
    pub fields: Vec<ServiceField>,
    pub token_help_url: String,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct ServiceField {
    pub key: String,
    pub required: bool,
}

/// Alle Dienste fuer die Oberflaeche (Katalog, Dialogfelder, Hilfe-Link).
pub fn catalog() -> Vec<ServiceInfo> {
    all()
        .iter()
        .filter_map(|d| {
            Some(ServiceInfo {
                id: ServiceId::parse(d.id)?,
                label: d.label.to_string(),
                auth: d.auth,
                capabilities: d.capability_ids.iter().map(|c| c.to_string()).collect(),
                fields: d
                    .fields
                    .iter()
                    .map(|f| ServiceField {
                        key: f.key.to_string(),
                        required: f.required,
                    })
                    .collect(),
                token_help_url: d.token_help_url.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jira_config_is_trimmed_checked_and_shows_the_site() {
        let raw = json!({
            "service": "jira", "site": " https://Firma.atlassian.net/ ", "email": "p@x.de",
            "project_key": "LV", "fremd": "weg", "token": "nie hier"
        });
        let (cfg, hint) = normalize(&raw).unwrap();
        assert_eq!(cfg["site"], "firma.atlassian.net");
        assert!(cfg.get("fremd").is_none() && cfg.get("token").is_none());
        assert_eq!(hint.as_deref(), Some("firma.atlassian.net"));
    }

    #[test]
    fn missing_required_fields_and_foreign_sites_are_refused() {
        let e = normalize(&json!({"service": "asana"})).unwrap_err();
        assert!(e.contains("Projekt-ID"), "{e}");
        let e = normalize(
            &json!({"service": "jira", "site": "evil.example", "email": "a@b", "project_key": "X"}),
        )
        .unwrap_err();
        assert!(e.contains("atlassian.net"), "{e}");
        let e = normalize(&json!({"service": "jira", "site": "x.atlassian.net/../evil", "email": "a@b", "project_key": "X"}))
            .unwrap_err();
        assert!(e.contains("atlassian.net"), "{e}");
        assert_eq!(
            normalize(&json!({"service": "gmail"})).unwrap_err(),
            ERR_SERVICE_UNKNOWN
        );
        assert!(normalize(&json!({"service": "github", "repo": "a/../b"})).is_err());
    }

    #[test]
    fn webhook_services_keep_only_the_host_and_check_it() {
        let (cfg, hint) =
            normalize(&json!({"service": "slack", "host": "hooks.slack.com"})).unwrap();
        assert_eq!(cfg, json!({"service": "slack", "host": "hooks.slack.com"}));
        assert_eq!(hint.as_deref(), Some("hooks.slack.com"));
        assert!(normalize(&json!({"service": "slack", "host": "evil.example"})).is_err());
        assert!(normalize(&json!({"service": "slack"})).is_err());
    }

    #[test]
    fn secrets_are_checked_when_entered() {
        assert!(check_secret(ServiceId::Slack, "https://hooks.slack.com/services/T/B/X").is_ok());
        assert!(check_secret(ServiceId::Slack, "https://evil.example/services/T").is_err());
        assert!(check_secret(ServiceId::Discord, "discord.com/api/webhooks/1/x").is_err());
        assert!(check_secret(ServiceId::Asana, "2/1234/abcd").is_ok());
        assert!(check_secret(ServiceId::Asana, "abc\r\nX-Evil: 1").is_err());
        assert!(check_secret(ServiceId::Asana, "Bearer abc").is_err());
        assert_eq!(
            webhook_host(
                ServiceId::Teams,
                "https://prod-1.westeurope.logic.azure.com/x?sig=1"
            )
            .as_deref(),
            Some("prod-1.westeurope.logic.azure.com")
        );
        assert_eq!(webhook_host(ServiceId::Asana, "x"), None);
    }

    #[test]
    fn the_catalog_lists_every_service_with_its_fields() {
        let c = catalog();
        assert_eq!(c.len(), ServiceId::ALL.len());
        let jira = c.iter().find(|s| s.id == ServiceId::Jira).unwrap();
        assert!(jira.fields.iter().any(|f| f.key == "site" && f.required));
        assert!(jira
            .fields
            .iter()
            .any(|f| f.key == "issue_type" && !f.required));
        for s in &c {
            for f in &s.fields {
                assert_ne!(field_label(&f.key), "Feld", "{}: {}", s.label, f.key);
            }
        }
    }
}
