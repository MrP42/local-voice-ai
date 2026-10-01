# Recherche Integrationen — Muster, Auth, Rechte, MCP/CLI-Steuerung

Stand 30.09.2026. Belegstufen: **belegt** (Primärquelle oder eigener Code-/Dateibefund), **sekundär**
(Fachartikel, Sekundärquelle), **Vermutung** (eigene Einschätzung, vor Bau prüfen).
Alle Web-Quellen abgerufen am 30.09.2026.

## 1. Ist-Stand im Repo (belegt, Code gelesen in `wt-plan`)

| Baustein | Fundstelle | Befund für das Register |
|---|---|---|
| Kalenderquellen ICS + Graph | `src-tauri/src/managers/calendar/model.rs` (`CalendarKind {Ics, Graph}`, `CalendarSource`), Tabelle `calendar_sources` in `meetings.db` (`managers/meetings/store.rs:187`, `CALENDAR_MIGRATION`) | Mehrere Quellen gibt es schon; das Register übernimmt sie, statt eine zweite Liste anzulegen. |
| Graph-Anmeldung | `managers/calendar/graph.rs:4` („Autorisierungscode mit PKCE (S256) über den Systembrowser“), `:84` `SCOPES = "Calendars.Read offline_access User.Read"`, Client-ID aus `settings.calendar_graph_client_id` (E14) | PKCE-Loopback-Fluss ist fertig und getestet (`graph/tests.rs`, 2107 Zeilen). Für Mail/OneDrive/Kalender-Schreiben müssen nur Scopes je Fähigkeit dazukommen. |
| Geheimnisse | `managers/calendar/secret.rs`: DPAPI Benutzerbereich, `<appdata>/secrets/<id>.bin`, Entropy = Präfix + Name, atomar, `Zeroizing`, nie Klartext in Fehlern, nicht im Sync | Taugt als allgemeiner Geheimnisspeicher; nur Namensraum/Präfix verallgemeinern. |
| MCP-Server | `src-tauri/src/mcp/*` (P6e): `--mcp`, stdio, JSON-RPC, **nur lesend**, eigener Prozess ohne Tauri, READ_ONLY-SQLite, Schalter `meeting_mcp_enabled` (Standard aus), 4 Werkzeuge `list_meetings`, `search_meetings`, `get_meeting`, `get_transcript` | Schreibende Werkzeuge können NICHT in diesem Prozess laufen (kein App-Zustand, keine Modelle); sie brauchen einen Kanal zur laufenden App. |
| CLI-Weiterreichung | `lib.rs:2108` `tauri_plugin_single_instance::init`: `--toggle-transcription`, `--toggle-post-process`, `--cancel`, `--read-file` | Einweg: kein Rückgabewert, kein Exit-Code, keine Rechteprüfung. Für Agenten zu wenig. |
| Headless-CLI | `cli.rs`: `--transcribe-file`, `--import-meeting` (druckt `MEETING_ID=`), `--export-meeting`, `--tts-test`, `--dump-meeting` | Läuft als eigene Instanz neben der App (headless). Gut für Tests, aber zweiter Modellstart neben laufender App (RAM/VRAM). |
| Follow-up-Mail | `managers/meetings/mail.rs` (P6c): Entwurf, `mailto:`, `.eml` mit `X-Unsent: 1`, keine Netzfunktion | Echter Versand fehlt; das Register liefert ihn (Graph `Mail.Send` oder SMTP). |
| Export | `managers/meetings/export.rs`: Markdown, Text, **Word (.docx, eigener Writer über `zip`)**, HTML, PDF (`pdf.rs`), SRT/VTT, JSON | docx-Erzeugung existiert; Ablage in Ordner-/OneDrive-Integration ist nur ein Ziel mehr. |
| Navigation | `src/components/Sidebar.tsx`: `home, history, meetings, models, tts, settings`; unten `["models","settings"]` | Neuer Eintrag `integrations` zwischen `models` und `settings` passt ins vorhandene Muster. |
| Einwilligung | i18n `Einwilligung erforderlich … § 201 StGB`, `consentRequired: „Ohne bestätigte Einwilligung startet keine Aufnahme.“` | Ein Agent darf die Einwilligung nicht abgeben; „Aufnahme starten“ per MCP muss in der App bestätigt werden. |

## 2. Zielsysteme WAI AI OS (belegt, Repo `C:/Users/wolff/ai-os-wissensbasis`, Commit 605d79d)

- **Wissensbasis-MCP** (`backend/waios/mcp_server/tools.py`): Werkzeuge `wissen_suchen`, `dokument_lesen`,
  `bereiche_auflisten`; Maschinen-Key per Bearer, Scopes `wissen:read` bzw. `wissen:read:<bereich>`.
- **Rechte** (`backend/waios/modules/rag/rechte.py`): Maschinenrollen `system_service`/`agent_workflow` sehen nur
  `public` und `internal`, nie `confidential`/`restricted`; ohne Wissens-Scope nie Zugriff; Bereiche kommen aus dem
  Prinzipal, nie aus dem Request; Request-Parameter dürfen nur verkleinern. Zehn Bereiche (`shared/enums.py`):
  privat, familie, beruf, wai, schule_uni, finanzen, gesundheit, behoerden, projekte, kunden.
- **Schreiben in den RAG**: `modules/rag/router.py:79` `POST /documents` ist ein **Schema-Stub („nicht aktiv“)**;
  es gibt keinen Maschinen-Endpunkt für Wissen-Schreiben. Der RAG indiziert Vault-Notizen über einen
  **Vault-Write-Hook** (`rag/indexing.py:14-20, :88-91`). `vault:write` existiert nur als Scope-Konstante.
- **Obsidian-Vault** = AI-OS-Vault `C:\Users\wolff\AI-OS\vault` (Pfad belegt über
  `~/.claude/projects/…Selbst-ndigkeit/memory/project-ki-wissensbasis.md`): Ordner `00_inbox/` (Triage),
  `10_contexts/<bereich>/`, `20_ai_os/`, `90_templates/` (nicht indiziert). Frontmatter-Vertrag (Vault-README):
  `title, tags, context_area, data_class, sensitivity, tier`; fehlt `data_class` → `restricted` (fail-closed).
  Keine Community-Plugins installiert (`.obsidian/plugins` fehlt) → **Local-REST-API-Plugin ist nicht vorhanden**.
- **Folgerung**: „In Obsidian + RAG übertragen“ heißt heute: Markdown mit gültigem Frontmatter in den Vault
  schreiben; der AI-OS-Index übernimmt es. Ein echter `wissen:write`-Endpunkt wäre Arbeit im AI-OS-Repo
  (eigenes Goal, Owner-Entscheidung).

## 3. Register-Muster am Markt

| Produkt | Muster | Übertrag | Beleg |
|---|---|---|---|
| Home Assistant | *Config Entry* je Konto, angelegt über einen *Config Flow* (Assistent); mehrere Einträge je Integration, `unique_id` gegen Doppelanlage; *Subentries* für Unterobjekte | Eine Integration = ein Eintrag je Konto/Ordner; Assistent je Art; Dublettenschutz über Konto-ID | belegt: developers.home-assistant.io/docs/config_entries_index/, …/config_entries_config_flow_handler/ |
| n8n | Credentials getrennt von Workflows, verschlüsselt mit Instanzschlüssel `N8N_ENCRYPTION_KEY`; typisiert (OAuth2, API-Key, Basic); geteilte Nutzer dürfen verwenden, aber nicht lesen | Geheimnis nie im Klartext an UI/Agent zurück; Verbindung wird *verwendet*, nicht *ausgelesen* | sekundär: docs.n8n.io/integrations/creating-nodes/build/reference/credentials-files/, netholics.com (2026) |
| Power Automate | *Connections* je Connector; DLP-Richtlinien klassifizieren Connectoren (Business / Non-Business / Blocked) und verbieten Mischung in einem Flow | Datenklassen je Integration (z. B. „geschäftlich“/„privat“) und Regel „kein Fluss von Klasse A nach B“ als spätere Ausbaustufe | belegt: learn.microsoft.com/power-platform/admin/dlp-connector-classification |
| Claude Connectors | Remote-MCP als Connector; je Werkzeug/Kategorie „Always allow“ / „Needs approval“ / „Blocked“ | Genau das Drei-Stufen-Modell je Fähigkeit: **aus / fragen / erlaubt** | sekundär: support.claude.com/en/articles/11175166 |
| Raycast, Zapier/Make | Konten („Connections“) je App, wiederverwendbar über viele Automationen; Einstellungen je Erweiterung | Register ist die einzige Stelle für Konten; Workflows (Goal B) referenzieren nur die ID | Vermutung (bekanntes Produktverhalten, nicht neu geprüft) |
| MCP-Registry | Verzeichnis von Servern mit Metadaten | „Eigener MCP-Server“ als individuelle Integration (HTTP-URL + Token) | Vermutung |

## 4. Auth je Integrationsart

- **Microsoft 365 / Outlook.com (Graph)**: PKCE-Public-Client ist gebaut. Delegierte Berechtigungen je Fähigkeit:
  `Calendars.Read` (lesen), `Calendars.ReadWrite` (Besprechungsinfo in Termin schreiben), `Mail.Send` (senden),
  `Files.ReadWrite` (OneDrive ablegen/lesen). Inkrementell anfordern, wenn eine Fähigkeit eingeschaltet wird
  (Vermutung: Microsoft-Identity-Plattform unterstützt inkrementelle Zustimmung; im Spike prüfen). Patrick muss die
  Entra-App-Registrierung (E14) um diese Berechtigungen ergänzen.
- **SMTP/IMAP**: Exchange Online schaltet SMTP-AUTH-Basic Ende Dezember 2026 für bestehende Mandanten
  standardmäßig ab, finale Abschaltung 2027 (belegt: techcommunity.microsoft.com/blog/exchange/…/4114750 und
  office365itpros.com/2026/01/29/smtp-auth-basic-retirement/). Für M365 deshalb Graph `Mail.Send`, nicht SMTP.
  Google Workspace: IMAP/SMTP mit Passwort seit 14.03.2025 aus, App-Passwörter für private Konten weiter möglich
  (sekundär: getmailbird.com, support.google.com/a/answer/14114704). → SMTP mit App-Passwort als generischer
  Weg für Gmail/Provider; Google-OAuth später.
- **Lokaler Ordner / OneDrive-Sync-Ordner**: kein Auth; Rechte = Pfad-Sandbox (kanonisieren, nichts außerhalb der
  Wurzel, keine Symlink-Ausbrüche — Vorbild AI-OS-Vault-Scan).
- **Obsidian**: als Dateiordner (Vault-Pfad) — belegt nutzbar ohne Plugin. Local-REST-API-Plugin (HTTPS
  127.0.0.1:27124, API-Key, eigenes Zertifikat, inzwischen mit MCP) wäre eine Alternative, ist aber nicht
  installiert und erzwingt laufendes Obsidian (belegt: github.com/coddingtonbear/obsidian-local-rest-api).
- **WAI-Wissensbasis**: HTTP-MCP-Client, `Authorization: Bearer <Maschinen-Key>`; Key per DPAPI.

## 5. Geheimnisablage Windows

- DPAPI-Dateien (vorhanden) vs. Credential Manager: `CRED_MAX_CREDENTIAL_BLOB_SIZE` = 5×512 = **2560 Byte**;
  OAuth-Token reißen das regelmäßig (belegt: learn.microsoft.com/windows/win32/api/wincred/ns-wincred-credentiala;
  sekundär: mehrere Issue-Berichte, z. B. github.com/Zious11/jira-cli/issues/759). → **DPAPI-Dateien behalten**,
  Credential Manager nicht einführen.

## 6. Rechte- und Richtungsmodell (Synthese)

- **Richtung** je Integration: `lesen | schreiben | beidseitig` — eine grobe Obergrenze.
- **Fähigkeiten** je Integrationsart (fein): z. B. Kalender `events.read`, `events.write_note`; Mail `send.self`,
  `send.participants`, `send.any`; Ordner `files.read`, `files.write`; Vault `notes.write`; Wissen `search`.
- **Modus** je Fähigkeit und je Aufrufer (UI / Workflow / externer Agent / lokaler Agent): `aus | fragen | erlaubt`.
  Wirksam = Minimum aus Richtung, Fähigkeit, Aufrufer-Recht.
- **Audit-Log** jeder Wirkung nach außen und jeder Ablehnung (Zeit, Aufrufer, Integration, Fähigkeit, Ziel
  gekürzt, Ergebnis). MCP-Spec verlangt von Clients „Log tool usage for audit purposes“ (belegt, s. u.).

## 7. MCP für schreibende Werkzeuge (Spec 2026-07-28, belegt)

Quelle: modelcontextprotocol.io/specification/2026-07-28 (…/server/tools, …/client/elicitation, …/basic/authorization).

- Aktuelle Spec-Version ist **2026-07-28**; jede Anfrage MUSS `_meta` mit Protokollversion/Client-Info tragen, Tools
  können `InputRequiredResult` (mehrstufige Anfrage) liefern. Der P6e-Server verhandelt ältere Versionen →
  Kompatibilität prüfen (Risiko).
- „there SHOULD always be a human in the loop with the ability to deny tool invocations“; Clients SHOULD
  „Present confirmation prompts … for operations“.
- „clients MUST consider tool annotations to be untrusted unless they come from trusted servers“ → die App
  prüft Rechte selbst; `readOnlyHint`/`destructiveHint` sind nur Hinweise.
- Server MUST: „Validate all tool inputs, Implement proper access controls, Rate limit tool invocations,
  Sanitize tool outputs“.
- `tools/list` „MAY vary by the authorization presented on the request“ → nur freigegebene Werkzeuge listen.
- Elicitation: Form-Modus nie für Geheimnisse; Nutzer kann ablehnen. **Aber**: Elicitation läuft im Client
  (Claude/Codex) — ein automatisierter Client kann „accept“ senden. Für §-201-Einwilligung und Mailversand muss
  die Bestätigung **in der App** passieren (Vermutung/Designschluss).
- Authorization: „Implementations using an STDIO transport SHOULD NOT follow this specification, and instead
  retrieve credentials from the environment.“ HTTP: OAuth 2.1 + Resource Indicators; Token-Passthrough verboten.
- Sicherheit: Confused Deputy / Token-Passthrough (sekundär: aembit.io, wiz.io); Prompt-Injection über
  Besprechungsinhalte (Aussagen Dritter) — Meta „Agents Rule of Two“ (31.10.2025): höchstens zwei von
  [A] nicht vertrauenswürdige Eingaben, [B] sensible Daten, [C] Zustand ändern/nach außen kommunizieren; sonst
  Mensch in der Schleife (belegt: ai.meta.com/blog/practical-ai-agent-security/). Ein Agent, der Transkripte
  liest (A+B) und Mails sendet (C), braucht also Freigabe.

## 8. CLI-Fernsteuerung der laufenden App

| Weg | Pro | Contra | Beleg |
|---|---|---|---|
| Single-Instance-Argumente (vorhanden) | schon da | Einweg, keine Antwort, keine Auth | belegt (Code) |
| Loopback-HTTP 127.0.0.1 + Token | von Docker/n8n erreichbar, Streamable-HTTP-MCP möglich | Port offen für alle lokalen Prozesse, Token-Verwaltung, Browser-CSRF/DNS-Rebinding beachten | Vermutung |
| **Named Pipe** mit DACL nur aktueller Benutzer + `PIPE_REJECT_REMOTE_CLIENTS` | kein Port, kein Netz, Windows-ACL; `GetNamedPipeClientProcessId` für Audit | Windows-spezifisch (macOS: Unix-Socket); PID nicht als Sicherheitsgrenze nutzen | belegt: learn.microsoft.com/windows/win32/ipc/named-pipe-security-and-access-rights; sekundär: comcomponent.com (Named-Pipe-Praxis) |

Empfehlung: Named Pipe als Standardkanal für `--mcp` (schreibend) und `local-voice-ai.exe ctl …`; zusätzlich je
Agent-Client ein Token (Rechte je Werkzeug, widerrufbar). Loopback-HTTP nur als spätere Option für n8n (Goal B).

## 9. YouTube als erste Integration (Zusatz Patrick, 30.09.2026)

Keine Rechtsberatung; Rechtslage vor Auslieferung an Dritte anwaltlich prüfen lassen.

### 9.1 Wege, ein Video in der App anzusehen

| Weg | Werbung | Grundlage fürs Transkribieren | Rechts-/ToS-Lage (DE) | Beleg |
|---|---|---|---|---|
| **A: YouTube-IFrame-Player** (Einbettung in der WebView) | ja; Werbung darf nicht blockiert, verändert oder ersetzt werden | keine Datei; nur Mitschnitt der Systemwiedergabe (Loopback) in Echtzeit | ToS-konform, ausdrücklich erlaubter Zugang („Embeddable Player“) | belegt: developers.google.com/youtube/terms/developer-policies (Werbung nicht blockieren/verändern; kein verschachteltes iframe zur Verschleierung, III.I.21); youtube.com/static?template=terms |
| **B: yt-dlp lädt Datei**, Wiedergabe lokal | nein | ja, direkt (Audio → STT) | ToS: Download nur bei YouTube-eigenem Download-Link; **OLG Hamburg 21.11.2024 (5 U 54/23): „Rolling Cipher“ ist wirksame technische Schutzmaßnahme (§ 95a UrhG)**, youtube-dl-Hosting haftet → Anbieten/Bündeln eines Umgehungswerkzeugs in einer verteilten App ist hochriskant; private Kopie (§ 53 UrhG) deckt Umgehung nicht (Vermutung, juristisch prüfen) | belegt: openjur.de/u/2498326.html; sekundär: lto.de, ferner-alsdorf.de |
| **C: Werbeblocker im eingebetteten Player** | nein | wie A | Verstoß gegen Developer Policies; BGH 31.07.2025 (I ZR 131/23): Werbeblocker **können** Urheberrecht (Programmschutz § 69a UrhG) verletzen, zurückverwiesen an OLG Hamburg | belegt: bundesgerichtshof.de Pressemitteilung 2025148; sekundär: lto.de, heise.de |
| **D: Nutzer liefert Datei** (eigene Videos aus YouTube Studio, vom Urheber bereitgestellte Datei) | nein | ja (vorhandener Import) | unkritisch bei eigenen/freigegebenen Inhalten | Vermutung (Studio-Download eigener Videos ist YouTube-Funktion) |
| **E: YouTube Premium** | im Premium-Konto keine Werbung | keine Datei | ToS-konform; ob die Anmeldung in einer eingebetteten WebView2 funktioniert, ist ungeprüft (Google sperrt Anmeldungen in eingebetteten Browsern teils) | Vermutung |

**yt-dlp technisch** (belegt: github.com/yt-dlp/yt-dlp/issues/14404, /15012): braucht seit Ende 2025 eine externe
JavaScript-Laufzeit (Deno standardmäßig; Node/Bun/QuickJS optional) für Signatur-/n-Challenges und PO-Token;
YouTube erzwingt SABR-Streaming und hält Formate ohne PO-Token zurück. Folge: häufige Updates nötig
(Wettrüsten), zweites Binary (Deno) im Paket, Brüche ohne Vorwarnung. Lizenz yt-dlp: Unlicense (sekundär, Repo).
Bündelung wäre technisch über `process_guard` (Job-Objekt, RAM-/CPU-Deckel) machbar — die Frage ist rechtlich.

**Ehrliche Einordnung**: „ohne Werbung ansehen“ und „rechtssicher“ gehen nur mit Premium (E) oder eigenen/freigegebenen
Dateien (D). Weg B erfüllt beides technisch, ist aber rechtlich der riskanteste — für eine verteilte App mehr als für
Privatnutzung.

### 9.2 Untertitel

- **Offizielle API**: `captions.download` nur für Videos, die der angemeldete Nutzer besitzt/bearbeiten darf
  (sonst 403, „third-party contributions“) — belegt: developers.google.com/youtube/v3/docs/captions/download;
  sekundär: youtube2text.org. Für fremde Kanäle also nicht nutzbar.
- **Inoffiziell** (yt-dlp `--write-subs`/`--write-auto-subs`, Timedtext-Adressen aus der Player-Antwort): manuelle
  und automatische Spuren, viele Sprachen (auto-übersetzt). ToS-Verstoß („nur über die bereitgestellte
  Funktionalität“); ob dabei eine Schutzmaßnahme umgangen wird, ist offen (Vermutung: Untertitel sind nicht
  chiffriert, PO-Token/Player-Client aber nötig).
- **Selbst transkribieren** ist unabhängig davon möglich, sobald Audio vorliegt (Weg B/D oder Loopback bei A).
- **Vergleich/Zusammenführen**: vorhandene Bausteine — Import von VTT/SRT (`managers/meetings/import.rs`,
  `SUBTITLE_EXTENSIONS`), Wort-Diff aus `--reference`-Bewertung (`cli.rs`), LLM-Aufrufe mit Schema (Goal C/M7).

### 9.3 Metadaten ohne API-Schlüssel
- oEmbed (`https://www.youtube.com/oembed?url=…&format=json`: Titel, Kanal, Vorschaubild) — Vermutung (bekannte,
  öffentliche Schnittstelle, nicht neu geprüft).
- Kanal-RSS `https://www.youtube.com/feeds/videos.xml?channel_id=UC…` liefert die 15 neuesten Uploads; ab Dezember
  2025 zeitweise 404, Stand Mai 2026 wieder funktionsfähig (sekundär: rsscribe.com, wprssaggregator.com).

## 10. Provenienz — was es heute schon gibt (belegt, Code)

| Ort | Inhalt | Lücke |
|---|---|---|
| `managers/usage.rs` → `usage.db`, Tabelle `usage_event` | je LLM-Aufruf: `ts, purpose, connection_*, model_id/label, prompt/completion_tokens, Preise, cost_micro, duration_ms, ok, error`; `record()` liefert die Zeilen-ID (`Result<i64>`) | **kein Bezug zum erzeugten Inhalt** (keine meeting_id/document_id); `Purpose` fest: PostProcess, Minutes, EnhancedNotes, Summary, Tagging, Translation, Chat |
| `meetings.db`, `meeting_documents.generation_metadata_json` | Protokoll/Notizen: u. a. `model` (Test `minutes.rs` prüft „test-model“) | freies JSON, keine Tokens/Dauer/Quellen einheitlich, nicht in der UI |
| `transcripts.provider, model, language` | STT-Modell je Transkript | keine Dauer/RTF, keine Quelle (YouTube-Untertitel vs. eigene STT) |
| `managers/meetings/notes/model.rs` `EnhancedNotes.stats: EnhanceStats` | Zahl belegter KI-Einträge (`ai_entries_sourced`), verworfene Quellen-IDs, Chunks | Qualitäts-/Belegzahlen, gute Grundlage für „Konfidenz“ |
| `action_items.source` (`ai`/`user`/`manual`) | Herkunft je Aufgabe | nur grob |

**Folgerung**: Ein kleines, einheitliches `provenance`-Fundament in `meetings.db`, das je erzeugtem Inhalt auf
`usage_event.id` verweist (Tokens/Kosten bleiben im Ledger, keine Doppelhaltung), plus Quellenliste, Konfidenz und
Auslöser (Nutzer, Automatik, Workflow, Agent). Bestehende `generation_metadata_json` bleibt und wird beim Lesen
als Rückfall genutzt.

## Quellen (Abruf 30.09.2026)
- https://developers.google.com/youtube/terms/developer-policies ; https://www.youtube.com/static?template=terms ; https://developers.google.com/youtube/v3/docs/captions/download ; https://youtube2text.org/blog/youtube-data-api-transcripts
- https://openjur.de/u/2498326.html (OLG Hamburg 5 U 54/23) ; https://www.lto.de/recht/hintergruende/h/urheberrecht-youtube-musik-videos-download-haftung-uberspace-youtube-dl ; https://www.ferner-alsdorf.de/entscheidung-des-olg-hamburg-zu-youtube-dl/
- https://www.bundesgerichtshof.de/SharedDocs/Pressemitteilungen/DE/2025/2025148.html ; https://www.lto.de/recht/nachrichten/n/bgh-izr13123-werbeblocker-urheberrechte-umarbeitungsrechte ; https://www.heise.de/en/news/Copyright-Springer-vs-Adblock-Plus-enters-another-round-10505898.html
- https://github.com/yt-dlp/yt-dlp/issues/14404 ; https://github.com/yt-dlp/yt-dlp/issues/15012
- https://rsscribe.com/blog/youtube-rss-feeds-explained ; https://www.wprssaggregator.com/youtube-rss-feed/
- https://modelcontextprotocol.io/specification/latest und …/2026-07-28/server/tools, …/client/elicitation, …/basic/authorization
- https://ai.meta.com/blog/practical-ai-agent-security/ ; https://simonwillison.net/2025/Jun/13/prompt-injection-design-patterns/
- https://developers.home-assistant.io/docs/config_entries_index/ ; https://developers.home-assistant.io/docs/config_entries_config_flow_handler/
- https://learn.microsoft.com/en-IN/power-platform/admin/dlp-connector-classification
- https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp
- https://docs.n8n.io/integrations/creating-nodes/build/reference/credentials-files/
- https://techcommunity.microsoft.com/blog/exchange/exchange-online-to-retire-basic-auth-for-client-submission-smtp-auth/4114750 ; https://office365itpros.com/2026/01/29/smtp-auth-basic-retirement/
- https://support.google.com/a/answer/14114704 ; https://www.getmailbird.com/gmail-oauth-changes-app-password-phase-out/
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentiala ; https://github.com/Zious11/jira-cli/issues/759
- https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights ; https://comcomponent.com/en/blog/windows-named-pipes-practical-guide/
- https://github.com/coddingtonbear/obsidian-local-rest-api
- https://aembit.io/blog/mcp-authentication-and-authorization-patterns/ ; https://www.wiz.io/academy/ai-security/model-context-protocol-security
