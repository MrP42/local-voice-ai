//! Dienste mit Schluessel oder Webhook (Welle 1, Spec 2026-10-06-verbindungen-aktionen):
//! Slack, Teams, Discord, Notion, Confluence, Asana, ClickUp, Jira, Trello, Todoist, monday,
//! Linear, GitHub, HubSpot, Pipedrive, Airtable.
//!
//! - `config`: Pruefung der Konfiguration und des Geheimnisses beim Anlegen/Aendern.
//! - `registry`: welche Dienste es gibt, Anmeldung, erlaubte Hosts, Felder, Faehigkeiten.
//! - `http`: Ausfuehren einer Anfrage (nur HTTPS, keine Umleitungen, Grenzen).
//! - `markdown`: Protokoll-Markdown in Notion-Bloecke, Confluence-Storage, Jira-ADF.
//! - `ops`: die Operationen je Dienst als reine Anfrage-Bauer mit austauschbarem Ausfuehrer.

pub mod config;
pub mod http;
pub mod markdown;
pub mod ops;
pub mod registry;

#[cfg(test)]
mod live_tests;
