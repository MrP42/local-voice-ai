# Top-20-Verbindungen und -Aktionen – Entwurf

Stand 06.10.2026 · Auftrag Patrick („recherchiere, analysiere und implementiere die Top 20
Verbindungen sowie Top 20 Aktionen voll funktionsfähig“) · Zweig `feat/verbindungen-aktionen`

## Entscheidungen (06.10.)
- Umfang: alle drei Wellen. Google entfällt (Patrick nutzt **iCloud-Kalender und Outlook**) →
  Welle 3 = iCloud (CalDAV). Gmail, Google Drive, Salesforce zurückgestellt.
- Echte Tests: Slack, Teams, Discord (Patrick liefert Webhook-Adressen). Alles andere gegen
  Attrappen nach offizieller API-Doku; Abnahme durch Patrick mit eigenen Schlüsseln.

## Bestand (Explore 06.10.)
- `managers/integrations`: `Kind` (model.rs), Rechte je Fähigkeit × Aufrufer (`grants.rs`),
  Schranke (`gate.rs`: off/ask/allow, Audit, Freigabe), Geheimnisse DPAPI
  (`SecretRef::integration`), Anlegen über `targets.rs` + `integration_create_with_settings`.
- `webhook.rs::post`: nur HTTPS (oder Loopback), keine Weiterleitungen, Größen-/Zeitgrenzen.
- Automationen: `workflows/catalog.rs` (ActionSpec, NeedsSpec::Cap), `integration_actions.rs`
  (MailSend, WebhookPost als Vorlage), Ausgaben früherer Schritte über `steps.<id>`;
  `agent_notes/items.rs::read(context, from)` liest To-dos/Fristen/Entscheidungen aus `agent.extract`.

## Architektur
**Eine Integrationsart `service`** (statt 17 Arten): Konfiguration `{service, settings}`, das
Geheimnis im Fach `token` (bzw. `url` bei Webhook-Diensten). Ein Dienstregister
(`integrations/services/registry.rs`) beschreibt je Dienst:
- Anmeldeart: Webhook-Adresse · Bearer-Token · E-Mail+API-Token (Atlassian, Basic) ·
  Header-Token (Pipedrive) · Key+Token (Trello) · App-Passwort (iCloud, Basic).
- Erlaubte Hosts (exakt oder Suffix, z. B. `hooks.slack.com`, `*.atlassian.net`,
  `*.logic.azure.com`/`*.powerplatform.com` für Teams-Workflows, `discord.com`).
- Fähigkeiten und Zielfelder (Projekt-/Listen-/Board-ID, Notion-Elternseite, …).
- Der Dialog „Integration hinzufügen“ wird aus dem Register erzeugt (Backend liefert Schema).

**HTTP**: ein JSON-Client auf Basis der Webhook-Regeln (HTTPS, keine Weiterleitung, Host
gegen das Register, Größen-/Zeitgrenzen, Fehlerklassen 401/403 → `auth`, 404 → `target`,
429 → `rate_limited` mit Retry-After), Geheimnisse nie im Log.

**Neue Fähigkeiten**: `chat.post`, `task.create`, `page.write`, `crm.write`, `record.write`
(+ vorhandene `calendar.write`, `mail.send`, `files.write`). Alle schreibend → Vorgabe „fragen“.

## Dienste (Wellen)
1. **Welle 1 – Schlüssel/Webhook (16)**: Slack, Teams (Workflows), Discord · Notion,
   Confluence · Asana, ClickUp, Jira, Trello, Todoist, monday, Linear, GitHub · HubSpot
   (Service Key), Pipedrive · Airtable. Webhook (n8n/Make/Zapier) besteht schon.
2. **Welle 2 – Microsoft** (vorhandene Entra-App, M365-Integration): Outlook-Entwurf,
   Outlook-Folgetermin, OneDrive-Ablage, Teams-Kanal per Graph (optional).
3. **Welle 3 – iCloud-Kalender** (CalDAV, app-spezifisches Passwort): Termine lesen,
   Folgetermin anlegen.

## Aktionen (Automationen)
| Aktion | Dienste | Quelle |
|---|---|---|
| `chat.post` – Zusammenfassung/Aufgaben in Kanal | Slack, Teams, Discord | Text oder Schritt |
| `task.create_from` – Aufgabe je To-do/Frist | Asana, ClickUp, Jira, Trello, Todoist, monday, Linear, GitHub | `agent.extract` |
| `task.create` – einzelne Aufgabe | dieselben | Felder |
| `page.create` – Protokollseite | Notion, Confluence | Protokoll/Text |
| `page.append` – an Seite anhängen (Entscheidungen) | Notion, Confluence | Text/Schritt |
| `crm.note` – Notiz zu Kontakt/Deal | HubSpot, Pipedrive | Text, Teilnehmer-E-Mail |
| `crm.tasks_from` – CRM-Aufgabe je To-do | HubSpot, Pipedrive | `agent.extract` |
| `record.append` – Zeile im Meeting-Log | Airtable | Felder |
| `calendar.followup` – Folgetermin | Outlook, iCloud | Felder |
| `mail.draft` – Follow-up-Entwurf | Outlook | Felder |
| `files.save` – Protokoll ablegen | OneDrive | Protokoll |
Dazu bestehend: `webhook.post`, `mail.send`, `obsidian.note`/`agent.note`, `export.document`.

**Umgesetzt (Stand 06.10.)** – Abweichungen vom Entwurf:
- `crm.tasks_from` entfällt als eigener Baustein: `task.create_from` arbeitet auch mit HubSpot
  und Pipedrive (beide haben `task.create`).
- `mail.draft` hat eine eigene Fähigkeit `mail.draft` (Scope `Mail.ReadWrite`, beim Einschalten
  „Zustimmung erweitern“); ein Entwurf erreicht niemanden, E3 greift nicht.
- `files.save` = `export.document` mit einem Microsoft-365-Konto als Ziel (OneDrive, nie
  überschreiben: OneDrive vergibt bei gleichem Namen einen freien).
- `calendar.followup`: Outlook (mit `invite` werden Teilnehmende eingeladen, dann immer
  Freigabe) und iCloud als Dienst `icloud` (CalDAV-`PUT` mit `If-None-Match`, UID aus
  Lauf+Schritt, ohne Einladungen). Lesen von iCloud bleibt beim ICS-Freigabelink.
- Teams-Kanal per Graph: nicht umgesetzt (der Teams-Workflows-Webhook deckt Posten ab).

## Tests
Je Dienst: Attrappe (Loopback-HTTP) prüft Methode, Pfad, Kopf (Auth), Körper nach API-Doku,
Fehlerklassen; Host-Regel; Geheimnis nie im Fehlertext. Slack/Teams/Discord zusätzlich echt
(`#[ignore]`, Adressen per Umgebungsvariable). UI: Playwright für Katalog/Dialog/Aktionen.

## Risiken (Recherche)
Teams nur noch per Workflows (Office-Connectors seit 05/2026 aus); HubSpot nur Service Keys
(seit 28.09.2026); Todoist nur API v1; Atlassian-Tokens laufen ab (Ablauf anzeigen); Raten:
Slack 1/s/Kanal, Discord 30/min, Airtable 5/s; Microsoft-Rechte z. T. Admin-Zustimmung.

## Aufwand
~1,2–1,5 Mio. Token: Welle 1 ~0,9, Welle 2 ~0,15, Welle 3 ~0,15. Zwischenstand bei 50/80 %.
