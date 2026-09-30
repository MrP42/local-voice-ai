# Recherche Workflow-Automation — Engine, Trigger, Word, OneDrive

Stand 30.09.2026. Belegstufen: **belegt** / **sekundär** / **Vermutung**. Web-Quellen abgerufen am 30.09.2026.

## 1. Eigene Engine oder vorhandene einbinden?

| Kandidat | Lizenz | Laufzeitbedarf | Einbettbar in eine verteilte Desktop-App? | Beleg |
|---|---|---|---|---|
| **n8n CE** (läuft bei Patrick: `C:\Users\wolff\AI-OS\n8n\docker-compose.yml`, `n8nio/n8n:latest` + Postgres 16, Port `127.0.0.1:5678`, Austauschordner `n8n/austausch`) | Sustainable Use License: „internal business purposes“ frei; **Einbetten in ein Produkt nicht erlaubt**, dafür „n8n Embed“ (kostenpflichtig) | Docker, Node, Postgres | **Nein** (Lizenz + Docker-Pflicht) | belegt: docs.n8n.io/privacy-and-security/sustainable-use-license; sekundär: nordflux.de, fatcamel.ai; Compose-Datei gelesen |
| Activepieces | Community Edition MIT | Node, Postgres/Redis (Vermutung) | lizenzrechtlich ja, technisch schwer (eigene Server-Laufzeit) | sekundär: ssdnodes.com, instapods.com |
| Windmill | Community AGPLv3 | Server + Worker, Postgres | AGPL-Pflichten beim Verteilen | sekundär: openalternative.co, automationatlas.io |
| Node-RED | Apache 2.0 | Node-Laufzeit (~100 MB Leerlauf) | technisch möglich, aber zweite Laufzeit und fremde UI | sekundär: ssdnodes.com |
| Temporal | MIT | Cluster, Datenbank | für eine Desktop-App überdimensioniert | Vermutung (Auftrag nennt es bereits „zu groß“) |

**Folgerung**: Eine lokale, leichte Engine in Rust im App-Prozess — sie kennt die App-Funktionen direkt (Aufnahme,
STT, KI-Notizen, Protokoll, Export, TTS), teilt RAM-Gate/`process_guard`, braucht kein Abo und keine zweite
Laufzeit. n8n bleibt **Brücke** für Patricks eigene Automationen (interne Nutzung ist durch die Lizenz gedeckt):
- App → n8n: Aktion „Webhook senden“ an `http://127.0.0.1:5678/webhook/…` (Compose setzt `WEBHOOK_URL` so; belegt).
- n8n → App: (a) Datei in einen Eingangsordner legen, den ein Ordner-Trigger der App beobachtet — `n8n/austausch`
  ist schon gemountet (Vermutung: als Bind-Mount; im Bau prüfen) — oder (b) später ein Loopback-Endpunkt der App.
  Ob ein Container `127.0.0.1` des Windows-Hosts über `host.docker.internal` erreicht, ist **nicht geprüft**
  (Vermutung: bei Docker Desktop ja).

## 2. Engine-Muster (Synthese, Vermutung wo nicht anders vermerkt)

- **Definition als JSON** (`lva-workflow@1`): Trigger, lineare Schritte mit optionaler Bedingung je Schritt,
  Fehlerregel je Schritt. Kein freier Graph im ersten Wurf (Editor bleibt formularbasiert, testbar).
- **Warteschlange in SQLite** (Läufe, Schritte, Zustand), damit Neustart/Absturz nicht zu Doppel- oder Fehlläufen
  führt: Idempotenzschlüssel je Auslöser (z. B. `workflow_id + event_key` aus `calendar/model.rs::event_key`).
- **Serielle Ausführung schwerer Schritte**: STT, LLM, TTS teilen GPU/RAM; die App startet heute LLM/Embedding
  ohnehin nacheinander (`llm/server.rs`: „die App stellt ohnehin eine Anfrage nach der anderen“, belegt).
- **Wiederholung** nur für vorübergehende Fehler (Netz, 429/5xx, gesperrte DB), exponentiell, gedeckelt.
- **Freigabe-Zustand** (`wartet_auf_freigabe`) für Schritte mit Außenwirkung — Rechte kommen aus dem Register (Goal A).
- **Trockenlauf**: jeder Schritt meldet, was er täte, ohne Wirkung (wichtig für Vertrauen und Tests).

## 3. Trigger „Termin beginnt“

- Vorhanden (belegt): `managers/calendar/reminder.rs` — fällig, wenn `start - lead <= jetzt < start + 2 min`,
  nie ganztägig/abgesagt, nie während laufender Aufnahme, Besprechungsregel (≥ 2 Teilnehmende oder Beitrittslink);
  `service.rs` füttert mit Cache und Uhr; Hinweisfenster `meeting_prompt` + „Start aus Termin“ (P5b).
- Die Engine hängt sich an denselben Takt (kein zweiter Kalender-Poller). Ein Termin wird erst nach dem nächsten
  Sync sichtbar → kurzfristig angelegte Termine können den Trigger verpassen (Risiko, Sync-Intervall prüfen).
- **Einwilligung** (§ 201 StGB) ist in der App Pflicht: „Ohne bestätigte Einwilligung startet keine Aufnahme“
  (i18n, belegt). Automatisches Aufzeichnen braucht deshalb eine Bestätigung zur Startzeit oder eine ausdrückliche,
  protokollierte Vorab-Bestätigung — Owner-Entscheidung.

## 4. Ordner-Trigger und OneDrive

- **Lokal synchronisierter OneDrive-Ordner** beobachten: kein Zusatz-Scope, offline robust. Fallen: Dateien werden
  in Teilen geschrieben (erst verarbeiten, wenn Größe/Änderungszeit einige Sekunden stabil sind), Platzhalter bei
  „Dateien bei Bedarf“ (Lesen löst Download aus), Doppelereignisse beim Umbenennen, Konfliktkopien. (Vermutung,
  Standardwissen; im Bau mit echten OneDrive-Dateien prüfen.) Rust: Crate `notify` (Dateisystem-Ereignisse,
  CC0/Artistic-2.0 laut crates.io — **Lizenz vor Einbau prüfen**, Vermutung).
- **Graph delta** für `driveItem` liefert nur Änderungen seit dem letzten Token (belegt:
  learn.microsoft.com/graph/delta-query-overview; learn.microsoft.com/onedrive/developer/rest-api/concepts/scan-guidance).
  **Webhooks** (Change Notifications) brauchen einen öffentlich erreichbaren HTTPS-Endpunkt (belegt:
  learn.microsoft.com/graph/change-notifications-delivery-webhooks) → für eine Desktop-App ohne Server nur
  Delta-Polling. Empfehlung: Sync-Ordner zuerst, Graph-delta als Option für Rechner ohne OneDrive-Client.

## 5. Word-Dokument (docx) in Rust

- **Schon vorhanden** (belegt): `managers/meetings/export.rs` schreibt `.docx` selbst über das `zip`-Crate
  (`ExportFormat::Docx`, `write_docx`). Für „Protokoll als Word ablegen“ ist kein neues Crate nötig.
- Falls später Firmenvorlagen (.dotx, Kopf/Fuß, Tabellen) gebraucht werden: `docx-rs` (MIT, meistgenutztes
  Rust-docx-Crate, sekundär: github.com/PoiScript/docx-rs, lib.rs/crates/docx-rs).
- PDF ebenfalls vorhanden (`pdf.rs`, WebView2 PrintToPdf, P6b).

## 6. Mail „an mich / Teilnehmende / alle“

- Empfängerliste aus Teilnehmenden gibt es (P6c, `mail.rs`). Kalender-Attendees tragen `is_self`, `organizer`
  (`calendar/model.rs`, belegt) → Regeln „nur ich“, „Teilnehmende ohne mich“, „alle“ sind deterministisch.
- Versand über Register (Goal A): Graph `Mail.Send` (M365) oder SMTP (App-Passwort). Fallback ohne Konto:
  `.eml`-Entwurf in Ordner / `mailto:` (vorhanden).

## 7. Externe Agenten

- Werkzeuge `list_workflows`, `run_workflow`, `get_run` über den MCP/CLI-Kanal aus Goal A; Rechte je Werkzeug.
- MCP-Spec: Tools SHOULD Mensch-in-der-Schleife, Server MUST Eingaben validieren und Aufrufe begrenzen (belegt,
  siehe `koordination/integrationen/recherche/muster-und-quellen.md` §7).

## Quellen (Abruf 30.09.2026)
- https://docs.n8n.io/privacy-and-security/sustainable-use-license ; https://nordflux.de/en/guides/the-n8n-sustainable-use-license-explained ; https://www.fatcamel.ai/blog/n8n-licensing-101-understanding-commercial-embed-and-sustainable-use-licenses
- https://www.ssdnodes.com/learn/self-hosted-n8n-alternatives ; https://instapods.com/blog/n8n-alternatives/ ; https://openalternative.co/compare/activepieces/vs/windmill ; https://automationatlas.io/answers/windmill-pricing-explained-2026/
- https://learn.microsoft.com/en-us/graph/delta-query-overview ; https://learn.microsoft.com/en-us/onedrive/developer/rest-api/concepts/scan-guidance?view=odsp-graph-online ; https://learn.microsoft.com/en-us/graph/change-notifications-delivery-webhooks
- https://github.com/PoiScript/docx-rs ; https://lib.rs/crates/docx-rs
- Lokal: `C:\Users\wolff\AI-OS\n8n\docker-compose.yml`; Repo-Fundstellen wie angegeben.
