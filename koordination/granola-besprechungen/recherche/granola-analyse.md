# Granola (granola.ai) - technische Tiefenanalyse für einen Nachbau

Stand der Recherche: 29.09.2026. Auftrag: Besprechungsfunktionen von Granola so verstehen, dass sie in einer lokal laufenden Desktop-App (Tauri/Rust/React, Windows zuerst, macOS später) nachgebaut und übertroffen werden können.

## 0. Methode, Quellenqualität, Legende

- Quellen: Herstellerdokumentation (docs.granola.ai inkl. `llms.txt`-Index, ca. 120 Seiten), granola.ai (Updates, Blog, Pricing, Security), Interviews der Gründer, Fachpresse, Reverse-Engineering-Repos, Reviews, Stellenanzeigen, Datenschutz-Audits Dritter.
- Abrufweise: Seiten wurden über WebFetch gelesen (ein kleines Modell fasst die Seite zusammen). Wörtliche Zitate sind daher selten; Zahlen und Aussagen wurden nach Möglichkeit an zwei Stellen gegengeprüft. Nicht direkt lesbar war die Trust-Center-Seite `trust.granola.ai/subprocessors` (JavaScript-Seite), deshalb kommt die Unterauftragsverarbeiter-Liste aus zwei Zweitquellen (siehe 3.8).
- Legende der Belegstufe:
  - **belegt** = Aussage des Herstellers (Doku, Blog, Update, Changelog) oder eindeutiger Primärbefund (Netzwerk-Allowlist, Post-Mortem, API-Doku).
  - **sekundär** = Reverse Engineering, Presse oder Interview-Zusammenfassung Dritter.
  - **Vermutung** = eigener Schluss aus Indizien, ausdrücklich nicht bestätigt.
- Konkurrenten-Blogs (sally.io, meetergo, hedy.ai, tl;dv, anarlog/Hyprnote, coffee.ai) haben Eigeninteresse; ihre Aussagen sind entsprechend gekennzeichnet und nur dort verwendet, wo sie durch Herstellerdoku oder eine zweite Quelle gestützt werden.

## 1. Kurzfassung (Management-Blick)

1. Granola ist ein **Electron-Desktop-Notizblock** (TypeScript, React, Node, AWS), der Mikrofon und Systemaudio lokal abgreift, **die Audioströme live an Cloud-STT-Anbieter (Deepgram, AssemblyAI) streamt** und danach mit LLMs (OpenAI, Anthropic, weitere) aus Nutzerstichpunkten + Transkript + Kalenderkontext "verbesserte Notizen" erzeugt. Audio wird nicht gespeichert, nur Transkript und Notizen.
2. Der Kern-Trick ist **UX, nicht Modell**: ein Notizblock statt Bot, Nutzertext schwarz, KI-Text grau, ein Klick von der Notiz zur Transkriptstelle. Die Gründer sagen das selbst (Pedregal: "Alle App-Layer-Produkte laufen auf denselben Basismodellen").
3. Seit 2025/2026 wandelt sich Granola von der Einzelplatz-App zur **Kontext-Plattform** für Teams und Agenten: Chat über alle Meetings, Spaces, People/Companies, Recipes, Briefs, MCP-Server, öffentliche API, Webhooks, CRM-Sync, Workflows (Mail/Slack/Kalender). Laut Pedregal ist MCP das am schnellsten wachsende Nutzungsdiagramm.
4. Die **Schwächen liegen genau dort, wo ein lokales System überlegen sein kann**: keine Audio-Wiedergabe zur Verifikation, schwache Sprecherzuordnung (Desktop: nur Ich/Andere, Namen nur über Auslesen der Meeting-App-Oberfläche), stille Aufnahmeausfälle, US-only-Cloud mit Trainings-Default, Deutsch/gemischte Sprachen nur mäßig, Bezahlmauer nach 30 Tagen.
5. Business: Series C 125 Mio. USD bei 1,5 Mrd. USD Bewertung (25.03.2026), gesamt ca. 192 Mio. USD, 70-80 Mitarbeiter (Platformer, Juli 2026). Sammelklage wegen heimlicher Aufnahme (Chamberlain v. Granola, N.D. Cal., 30.07.2026).

## 2. Feature-Inventar

Abo-Stufen: **Basic** (gratis), **Business** (14 USD/Nutzer/Monat), **Enterprise** (ab 35 USD/Nutzer/Monat). Details in Abschnitt 5.

### 2.1 Aufnahme, Transkription, Sprecher

| Feature | Beschreibung | Technik (belegt/vermutet) | Abo-Stufe | Quelle |
|---|---|---|---|---|
| Bot-lose Aufnahme | Kein Teilnehmer "Granola" im Call; läuft lokal, funktioniert für Zoom, Meet, Teams, Webex, Slack-Huddles, FaceTime, Präsenztermine. Start per Klick (Benachrichtigung oder "New Note"), kein Auto-Start. | belegt: Audio von Mikrofon + Systemaudio des Geräts | alle | https://www.granola.ai/blog/why-granola-doesnt-use-a-bot ; https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md |
| Live-Transkript | Sprechblasen: "Me" (grün, rechts = Mikrofon), "Them" (grau, links = Systemaudio). Einzelne Transkript-Abschnitte lassen sich während/nach dem Meeting suchen, kopieren, löschen. Anzeige "grüne tanzende Balken" bei aktiver Aufnahme. | belegt: zwei getrennte Kanäle; Reverse Engineering: jedes Segment trägt `source: microphone/system`, Start/Ende-Zeitstempel, `confidence` (sekundär) | alle | https://docs.granola.ai/help-center/taking-notes/transcription.md ; https://github.com/getprobo/reverse-engineering-granola-api |
| Transkript teilweise löschen | Sensible Passagen (Telefonnummern, Passwörter) entfernen, Notizen neu erzeugen | belegt | alle | https://www.granola.ai/updates (28.01.2026) |
| Sprecher-Tags | Klarnamen statt "Me/Them" für Google Meet, Zoom (Workplace-Desktop-App, nicht Web-Client), Teams (Desktop) auf macOS/Windows. Nicht rückwirkend. Scheitert bei überlappendem Sprechen, Raumgeräten, geteilten Geräten. | belegt: liest Teilnehmernamen und "aktiver Sprecher" aus der Oberfläche der Meeting-App über die Accessibility-API (macOS: Berechtigung nötig; Windows: keine Extra-Berechtigung) bzw. Chrome-Erweiterung für Meet. Das ist **keine akustische Diarisierung**. | alle (Admin-Guide für Teams) | https://docs.granola.ai/help-center/taking-notes/speaker-attribution.md ; .../speaker-attribution-google-meet.md ; .../speaker-attribution-zoom.md |
| Mobile: Diarisierung | Mobile Transkripte nennen "Speaker A, Speaker B ..." | belegt (Doku); Vermutung: nachträgliche Batch-Diarisierung, Beschriftung entspricht der AssemblyAI-Konvention | alle | https://docs.granola.ai/help-center/ios/transcription.md |
| Sprachen | 32 Sprachen (Update 02.09.2026, u. a. Deutsch, Französisch, Spanisch, Japanisch, Mandarin, Arabisch, Hindi). Desktop: "Multi-language"-Modus wechselt Sprachen im laufenden Meeting. Mobile: pro Meeting eine Sprache, danach fix. Alte Transkripte nicht übersetzbar (nur per Chat). | belegt | alle | https://releasebot.io/updates/granola ; https://docs.granola.ai/help-center/customising-granola/multi-language.md |
| Eigenes Vokabular | Bis zu 30 Begriffe pro Nutzer, 50 pro Workspace (Admin); ersetzt keine Wörter, löst keine Abkürzungen auf | belegt (Vermutung: Keyterm-Boosting beim STT-Anbieter) | alle | https://docs.granola.ai/help-center/customising-granola/customising-transcription.md |
| Telefonate (iOS) | Ausgehende Anrufe über die App, Nummernverifikation per 6-stelligem Code (Selbstanruf), 40+ Länder, keine eingehenden Anrufe, nur iOS | belegt; Vermutung: serverseitige Anruf-Brücke (Telefonie-Anbieter), Anbieter nicht genannt | alle | https://docs.granola.ai/help-center/ios/phone-calls.md |
| Apple Watch | Aufnahme direkt auf der Uhr, Sync zum iPhone, offline-fähig, hörbarer Gong + Vibration bei Start/Ende (nicht abschaltbar), watchOS 11+ | belegt | alle | https://docs.granola.ai/help-center/ios/apple-watch.md ; https://www.granola.ai/updates (28.07.2026) |
| Screenshots geteilter Bildschirme | Beta: macOS 14+, nur Teams (Chrome, Edge, Desktop-App), nicht Enterprise, benötigt Bedienungshilfen + Bildschirmaufnahme. Bilder liegen als Stapel in der Notiz und fließen in die Notizen ein. | belegt (OCR/Vision-Details nicht dokumentiert) | Basic/Business | https://docs.granola.ai/help-center/taking-notes/capture-shared-screens.md |
| Audio-Datei-Import | **Nicht** unterstützt; Web-App kann nicht transkribieren | belegt | - | https://docs.granola.ai/help-center/taking-notes/transcription.md |
| Audio-/Video-Aufzeichnung, Wiedergabe | **Nicht geplant** | belegt | - | https://docs.granola.ai/help-center/feature-requests.md |

### 2.2 Notizen, Templates, Nachbearbeitung

| Feature | Beschreibung | Technik (belegt/vermutet) | Abo-Stufe | Quelle |
|---|---|---|---|---|
| Eigene Notizen (Editor) | Leeres Blatt, Markdown-Kürzel (#, -, [ ]), Fett/Kursiv/Unterstrichen, Bilder per Drag-and-drop/Einfügen, automatische Korrektur von Tippfehlern und Abkürzungen. Notizen sind optional. | belegt; Dokumente intern im ProseMirror-Format (sekundär) | alle | https://docs.granola.ai/help-center/taking-notes/taking-notes-in-granola.md ; https://github.com/getprobo/reverse-engineering-granola-api |
| "Enhanced Notes" | Nach dem Meeting verschmilzt die KI **(1) Transkript, (2) Roh-Notizen, (3) Kalenderdaten**. **Nutzertext erscheint schwarz, KI-Text grau.** Lupe neben jedem Punkt zeigt die Quelle (Transkriptstelle oder Roh-Notiz). Nutzer-Überschriften geben die Struktur vor, die KI füllt sie. Bearbeitungen wirken nur auf diese eine Notiz, nicht auf künftige. | belegt (Blends aus OpenAI + Anthropic, Prompts laufend nachjustiert) | alle | https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes.md ; https://www.granola.ai/blog/announcement |
| Regenerieren | Mit anderem Template, per Chat-Feedback oder Knopf; bei fehlgeschlagener Erzeugung "Retry" über einen **alternativen Fallback-Weg zu den LLMs** | belegt | alle | https://docs.granola.ai/help-center/troubleshooting/notes-not-generating.md |
| Notizen per Anweisung ändern | "Edit your notes just by asking": Ton, Länge, Namen korrigieren, übersetzen (Chat-Schreibaktionen auf markierte Abschnitte) | belegt | alle | https://www.granola.ai/updates (24.07.2025) ; https://docs.granola.ai/help-center/getting-more-from-your-notes/chatting-with-your-meetings.md |
| Templates | Definieren Struktur und Detailtiefe der Enhance-Stufe. 29 vorgefertigte (Okt. 2024), auswählbar nach Nutzertyp; eigene Templates (Zweck/Kontext, Länge/Stil, Struktur, Iteration); privat oder per Firmen-Domain teilbar. Nur Desktop; iOS-Notizen erhalten Template nach Sync. Keine Auto-Auswahl dokumentiert. | belegt (Template = Prompt-Text) | alle | https://docs.granola.ai/help-center/taking-notes/customise-notes-with-templates.md ; https://www.granola.ai/blog/meeting-recipes-repeatable-formats |
| Recipes | Gespeicherte Chat-Prompts (Slash-Befehl "/", Reiter Discover / My Recipes / Workspace Recipes), Geltungsbereich einzelnes Meeting oder mehrere, optional festes Modell (Business+), teilbar privat/Workspace/Link ("Remix"). **Keine Variablen/Platzhalter.** Beispiele: /How I have changed, /How I like to work, /Prep my day. | belegt | alle; Modellwahl Business+ | https://docs.granola.ai/help-center/getting-more-from-your-notes/recipes.md ; https://www.granola.ai/blog/granola-chat-just-got-smarter |
| Action Items / Checkliste | Chat extrahiert Aktionspunkte, fügt Checkliste in Notiz ein | belegt (Update 2026) | alle | https://www.granola.ai/updates |
| Follow-up-Mail | Automatischer Mail-Entwurf nach externen Meetings aus Transkript, Kalender und bisherigen Gmail-Konversationen; nur Gmail/Google, mind. 1 externer Gast, max. 10 Teilnehmer, Anhänge bis 25 MB, ca. 10 s Rückgängig; Outlook nicht unterstützt; unterdrückt Entwurf, wenn Mail unpassend | belegt | bezahlt/Testphase | https://docs.granola.ai/help-center/taking-notes/follow-up-emails.md |
| Workflows im Chat | Mail-Entwurf, Slack-Nachricht, Google-Kalender-Termin auslösen, **immer mit Bestätigung**; persönliche Connectors (Gmail, Slack, Google Calendar) unter Einstellungen -> Connectors | belegt | laut Doku ohne Stufenangabe | https://docs.granola.ai/help-center/getting-more-from-your-notes/workflows-in-chat.md |
| Sprach-Diktat im Chat | Mikrofon-Symbol für gesprochene Fragen (startet keine Transkription) | belegt | alle | https://docs.granola.ai/help-center/getting-more-from-your-notes/granola-chat-dictation-vs-transcription.md |

### 2.3 Chat, Kontext, Organisation

| Feature | Beschreibung | Technik (belegt/vermutet) | Abo-Stufe | Quelle |
|---|---|---|---|---|
| Chat je Meeting (auch live) | Cmd/Ctrl+J; "Was habe ich verpasst?", "Was soll ich fragen?". Live-Chats sind flüchtig. Seit 16.01.2025. | belegt | alle | https://www.granola.ai/updates ; https://www.granola.ai/blog/chat-with-meetings-search-analyze-ai-2026 |
| Chat über alle Meetings / Ordner / Auswahl / Person / Firma | Vier Kontexte: Startseite (alle), einzelnes Meeting, Ordner, ausgewählte Meetings (Checkboxen); dazu Person/Firma. Dateien anhängbar (nicht iOS). Chat-Verlauf bleibt persönlich, auch bei geteilten Notizen. | belegt | Basic: nur letzte 30 Tage, nur "Auto"-Modell; Business/Enterprise: volle Historie + Modellwahl | https://docs.granola.ai/help-center/getting-more-from-your-notes/chatting-with-your-meetings.md |
| Agentischer Chat (Neubau 21.04.2026) | Suche über Notizen, Transkripte und Team-Space; **Inline-Zitate**, "Coverage Notes" (was wurde durchsucht), Strategie "Search -> Shortlist -> Read -> Repeat" | belegt (Blogpost); Modelle nicht genannt | alle | https://www.granola.ai/blog/how-granola-thinks-about-designing-agents ; https://www.granola.ai/blog/granola-chat-just-got-smarter |
| Modellwahl im Chat | Auto / Standard / "Thinking"-Modelle; ein Thinking-Modell "Fable 5" (laut Doku, Anthropic; Enterprise: Admin muss freischalten, Anthropic speichert Logs 30 Tage) | belegt (Doku) + sekundär | Business/Enterprise | https://docs.granola.ai/help-center/getting-more-from-your-notes/understanding-model-selection-in-granola-chat.md |
| Spaces und Ordner | "My notes" (privat) + Team-Space; Admins legen weitere Spaces an; Ordner mit einer Verschachtelungsebene; Notiz kann in mehreren Ordnern liegen; Auto-Add für wiederkehrende Meetings; Freigabe: eingeladene Personen / Workspace / Link. Seit 25.03.2026. | belegt | alle (Doku ohne Stufenlimit) | https://docs.granola.ai/help-center/sharing/folders/spaces-and-folders.md |
| People und Companies | Automatisch aus Kalendereinträgen abgeleitet (Teilnehmerliste + Anreicherung um Foto, Jobtitel, Firma); Chat je Person/Firma; kein manuelles Anlegen | belegt | Basic: 30-Tage-Fenster; Business: volle Historie | https://docs.granola.ai/help-center/people-and-companies.md |
| Pre-Meeting Briefs | Über Nacht erzeugte 2-3 Stichpunkte je externem Meeting aus früheren Notizen, Team-Notizen, Web (z. B. LinkedIn), Agenda, optional Gmail; mit Zitaten; Daumen-runter-Feedback. Mac, Windows, iPhone. Seit 20.05.2026. | belegt | Business/Enterprise, Google Calendar nötig | https://docs.granola.ai/help-center/taking-notes/pre-meeting-briefs.md |
| Profil/Kontext | Nutzerprofil (Rolle, Nutzertyp) fließt in Notizerzeugung ein | belegt (Doku) | alle | https://docs.granola.ai/help-center/getting-started/granola-101.md |
| Suche | Schnelle Volltext-Suche inkl. Person/Firma, offline verfügbar (Dez. 2024) | belegt; Zweitquellen: serverseitige Vektorsuche (Turbopuffer) | alle | https://www.granola.ai/updates ; https://getroutines.ai/transparency/granola-ai |
| Kalender | Google Calendar und Microsoft 365/Outlook (Exchange Online; keine On-Prem-Server, **kein Apple Calendar**). Erinnerung 1 Minute vor Beginn (nur Termine mit 2+ Teilnehmern); Klick öffnet Call-URL und startet Transkription. Ad-hoc-Erkennung über **Mikrofon-Nutzung** ("Meeting detected", "Huddle detected", "Call detected"); Zuordnung zum Termin, wenn Call innerhalb von 15 Min. zum Termin beginnt. Abgesagte Termine werden ausgeblendet. | belegt | alle | https://docs.granola.ai/help-center/getting-started/syncing-your-calendars.md ; https://docs.granola.ai/help-center/taking-notes/notifications.md |
| Offline | Notizen lokal gecacht, offline les-/bearbeitbar; **Transkription und KI benötigen Internet** | belegt (Cache); Transkription cloudbasiert belegt durch Allowlist | alle | https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md ; https://docs.granola.ai/help-center/troubleshooting/network-troubleshooting.md |
| Papierkorb, @Mentions | Gelöschte Notizen wiederherstellbar; @Erwähnungen von Teammitgliedern (Dez. 2025) | belegt | alle | https://releasebot.io/updates/granola |

### 2.4 Teilen, Export, Integrationen, Schnittstellen

| Feature | Beschreibung | Technik (belegt/vermutet) | Abo-Stufe | Quelle |
|---|---|---|---|---|
| Teilen per Link | "Anyone with the link" / "Only my company" / "Private"; Empfänger sind Betrachter (können Chat nutzen), Bearbeiter nötig für Templates. **Web-Ansicht zeigt nur die Zusammenfassung, kein Transkript.** Standardwert "jeder mit Link" sorgte im April 2026 für Kritik (The Verge). | belegt | alle; Admin-Richtlinien Enterprise | https://docs.granola.ai/help-center/sharing/sharing-notes.md ; https://docs.granola.ai/help-center/consent-security-privacy/sharing-controls.md ; https://aitoolly.com/ai-news/article/2026-04-03-granola-privacy-alert-ai-notes-viewable-via-link-and-used-for-training-by-default |
| Kopieren/Export | "Copy notes" als Markdown (nur Zusammenfassung). Massen-Export **nur als CSV per E-Mail** (Titel, Zusammenfassung, Transkripte), 1 Export/24 h, Link 24 h gültig; nur Notizen mit Zusammenfassung. | belegt | Basic/Business; Enterprise per Admin-Schalter | https://docs.granola.ai/help-center/sharing/exporting-notes.md |
| Slack | Notiz-Links in Kanäle, Auto-Post je Ordner | belegt | Basic: Slack (laut Preisseite "basic Slack"); Details je Quelle unterschiedlich | https://docs.granola.ai/help-center/sharing/integrations/slack.md ; https://www.granola.ai/pricing |
| Notion | Einzelklick-Export in Notion-Datenbank | belegt | Business+ | https://docs.granola.ai/help-center/sharing/notion.md |
| Zapier | Verbindung zu 8.000+ Apps (Juli 2025) | belegt | Business+ | https://docs.granola.ai/help-center/sharing/integrations/zapier.md |
| CRM: HubSpot, Attio, Affinity | Notizen/Aktionspunkte an Kontakt/Firma/Deal; Auto-Sync je Ordner. Salesforce laut Preisseite Enterprise ("+ others"), Doku erwähnt es nicht. | belegt | Business+ (Salesforce: Enterprise) | https://docs.granola.ai/help-center/sharing/integrations/hub-spot.md ; https://www.granola.ai/pricing |
| MCP-Server | `https://mcp.granola.ai/mcp`, Streamable HTTP, OAuth mit Dynamic Client Registration bzw. Enterprise-Managed Authorization; Tools: `query_granola_meetings`, `list_meetings`, `list_meeting_folders`, `get_meetings`, `get_meeting_transcript`, `get_account_info`; Clients: Claude, ChatGPT, Cursor, Claude Code; ca. 100 Anfragen/Min.; **nur Remote, kein lokaler MCP**, keine API-Keys | belegt | Basic: nur 30 Tage, ohne Transkript; Business: alles; Enterprise: Admin steuert | https://docs.granola.ai/help-center/sharing/integrations/mcp.md |
| Öffentliche API | Bearer-Key `grn_...` (Einstellungen -> Connectors), persönliche und Workspace-Keys; `GET /v1/notes`, `/v1/notes/{id}`, `/v1/notes/{id}/transcript`; Limits: Burst 25 pro 5 s, 5/s dauerhaft; Webhooks, Audit-Events, Legal-Holds-API (Enterprise-Themen). Changelog v1.0.0 (02/2026) bis v1.5.0 (09/2026). | belegt | Business/Enterprise | https://docs.granola.ai/introduction.md ; https://docs.granola.ai/api-reference/changelog.md |
| Mobile | iOS (04/2025, iPhone; kein iPad-Chat), Android (01.07.2026), Apple Watch (28.07.2026); Sync mit Desktop; Vorlagen nur am Desktop | belegt | alle | https://www.granola.ai/updates |
| Windows-App | Seit 11.06.2025; Windows 10/11; Installer `.exe` (pro Nutzer), Auto-Update alle ca. 10 Min. | belegt (Datum, Installer); Windows-Version nur sekundär | alle | https://www.granola.ai/updates ; https://docs.granola.ai/help-center/getting-started/managed-installations.md |
| Team/Enterprise | Workspaces, Nutzergruppen, SSO + SCIM, Audit-Events, Transkript-Auto-Löschung erzwingbar, Trainings-Opt-out workspace-weit, HIPAA (mit BAA), Legal Holds | belegt | Enterprise (SSO laut Drittquelle ab 50 Plätzen) | https://docs.granola.ai/help-center/consent-security-privacy/is-granola-hipaa-compliant.md ; https://zackproser.com/blog/granola-enterprise-security-features |
| Transparenz gegenüber Teilnehmern | Automatische Chat-Nachricht beim Start ("Hey, I'm using www.granola.ai to transcribe..."), Wasserzeichen im eigenen Videobild (laut Platformer per **virtueller Kamera**), Labs-Funktion "Heads Up" (Google-Kalender-Add-on, Teilnehmer müssen einen Hinweisschirm bestätigen, 22.12.2025) | belegt; virtuelle Kamera: sekundär | Nutzer einzeln aktivierbar; Admin-Rollout Enterprise | https://docs.granola.ai/help-center/consent-security-privacy/transparency-solutions/introduction.md ; https://www.platformer.news/granola-chris-pedregal-interview/ |
| Aufbewahrung | Audio: nicht gespeichert. Transkripte/Notizen: unbegrenzt, außer Auto-Löschung (Nutzer: 1 Tag bis 1 Jahr, 1 Woche Karenz; Enterprise: erzwungen, sofort wirksam, unwiderruflich; danach keine Neu-Erzeugung der Notizen möglich, Chat nur noch auf Notizbasis) | belegt | Nutzer-Auto-Löschung "Preferences"; Enterprise-Erzwingung | https://docs.granola.ai/help-center/consent-security-privacy/transcript-auto-deletion.md |

## 3. Funktionsweise und Tech-Stack

### 3.1 Client
- **Electron** (belegt): Stellenausschreibung "Product Engineer" (London, Fokus Electron, React, Node.js, TypeScript, Windows- und macOS-Auslieferung) https://simplify.jobs/p/120b247a-ed5b-48cf-a3c0-35e0a2bec065/Product-Engineer ; HN-Einstellungspost "TypeScript, React, Electron, AWS, various LLM tech" https://news.ycombinator.com/item?id=43858740 ; Engineering-Blog "back-button" beschreibt React Router + Electron-Main-Prozess + IPC-Kanal, macOS-Menüleiste, Benachrichtigungen und den "Nub" (Aufnahmeanzeige außerhalb der App) https://www.granola.ai/blog/back-button .
- Lokale Daten (sekundär, Issue im Repo `openclaw/graincrawl`): unter `~/Library/Application Support/Granola/` liegen neuere Stände nur noch **verschlüsselt** (`supabase.json.enc`, `cache-v6.json.enc`, Schlüssel `storage.dek`, Datenbank `granola.db`); Verschlüsselung über **Electron `safeStorage`** + AES-GCM. Frühere Versionen: Klartext `supabase.json` und `cache-v3.json` (Doppel-JSON), ab v6 direktes JSON. https://github.com/openclaw/graincrawl/issues/15 ; https://github.com/theantichris/granola/issues/22
- Pedregal (TechCrunch, 25.03.2026): der lokale Cache sei "nicht für KI-Workflows gedacht" gewesen, daher API. https://techcrunch.com/2026/03/25/granola-raises-125m-hits-1-5b-valuation-as-it-expands-from-meeting-notetaker-to-enterprise-ai-app/
- Sicherheitsbericht zur Desktop-App: App konnte auf externe Seite gelenkt werden, die Zugriff auf die angemeldete Sitzung hatte; Fix = Navigationssperre für nicht vertrauenswürdige URLs. Lehre für einen Nachbau: WebView-Navigation hart einschränken. https://docs.granola.ai/help-center/policies/security-contributions/desktop-app-navigation-vulnerability.md
- Auth über **WorkOS** (Refresh-Token-Rotation, Einmal-Token), Push/Benachrichtigungen über **Knock**, Diagnose-Logs über AWS Cognito + CloudWatch. https://docs.granola.ai/help-center/troubleshooting/network-troubleshooting.md ; https://github.com/getprobo/reverse-engineering-granola-api

### 3.2 Audioerfassung
- **Zwei getrennte Quellen** (belegt): Mikrofon = "Me", Systemaudio = "Them". Es lässt sich **keine einzelne App isolieren**; Hintergrundmusik landet im Transkript ("combined audio stream"). https://docs.granola.ai/help-center/taking-notes/transcription.md
- **macOS** (belegt: Berechtigungen; Vermutung: API): Setup verlangt "Microphone" und "Screen & System Audio Recording" (macOS 14+). Das passt zu **ScreenCaptureKit oder Core Audio Taps**; welche der beiden genutzt wird, ist nicht dokumentiert. Die Aussage "Granola nutzt ScreenCaptureKit oder Core-Audio-Taps, Electron mache das nicht selbst" stammt aus einer Suchzusammenfassung und ist **Vermutung**. https://docs.granola.ai/help-center/getting-started/setting-up-granola-for-the-first-time.md
- **Windows** (belegt: Verhalten; Vermutung: API): keine Systemberechtigung nötig außer Mikrofon. Doku: "On Windows, if you're using a meeting app we haven't added support for yet, sound from other people on the call may not be picked up" - **das deutet auf app-bezogene Erfassung** (Vermutung: WASAPI Process Loopback bzw. Anwendungsliste) statt reinem Geräte-Loopback. Ein Drittanbieter-Review behauptet reinen WASAPI-Loopback des gesamten Ausgabegeräts (https://saas.com.ai/granola-for-windows/); das widerspricht der Doku-Aussage und ist unbelegt. Teams-Desktop unter Citrix/Azure Virtual Desktop liefert kein Fremdaudio; Kaspersky kann Audiozugriff blockieren; Bluetooth-Headsets schalten in Telefonqualität. https://docs.granola.ai/help-center/troubleshooting/transcription-issues.md
- **Echo**: Pedregal (Creator Economy): eigenes **Echo-Cancelling-System, unabhängig von KI**, für Szenarien mit und ohne Kopfhörer; das Team feilt an Randfällen wie "AirPods während eines Zoom-Calls herausnehmen". Laut Zusammenfassung (Sekundärquelle) läuft die Echo-Unterdrückung **on-device**. https://creatoreconomy.so/p/the-hidden-rules-behind-successful-ai-products-chris-pedregal ; https://shotcast.substack.com/p/ux-and-not-llm-chris-pedregal-granolas ; https://michaelgoitein.substack.com/p/granolas-revolutionary-ai-strategy
- Gerätewahl: "Auto" folgt dem Mikrofon der Meeting-App; manuelle Auswahl im Transkriptfenster. https://docs.granola.ai/help-center/troubleshooting/transcription-issues.md
- Mobil: Audio wird während des Meetings **temporär zwischengespeichert**, nach der Transkription gelöscht, Transkript kommt verzögert per Benachrichtigung; iOS kann Zoom/Meet-Audio anderer Apps nicht abgreifen (OS-Beschränkung). https://docs.granola.ai/help-center/ios/transcription.md ; https://docs.granola.ai/help-center/feature-requests.md

### 3.3 Spracherkennung (STT)
- **Anbieter: Deepgram und AssemblyAI** (belegt über Netzwerk-Allowlist: `api.deepgram.com`, `streaming.us|eu.assemblyai.com`; Protokoll "ausgehende sichere WebSockets über TCP 443") und Datenschutzhinweise. https://docs.granola.ai/help-center/troubleshooting/network-troubleshooting.md ; https://www.granola.ai/security
- **Streaming**: Desktop transkribiert live über WebSockets, Fehlerbild "Transkription startet und stoppt", wenn VPN/Proxy die Anbieter blockiert (belegt). Vermutung: Client verbindet sich mit kurzlebigen, vom Granola-Backend ausgestellten Tokens direkt zum Anbieter (Dez. 2024 wurde die iOS-Transkription nachweislich auf Server-Seite verlegt, siehe Post-Mortem).
- **Aufgabenteilung** (Vermutung): Post-Mortem sagt, die Produktions-macOS-App (2024/25) nutze "einen anderen Transkriptionsdienst" als AssemblyAI, also Deepgram; AssemblyAI kam über iOS-Beta und später als regionaler EU/US-Endpunkt, plausibel für Multi-Language und Mobil. https://docs.granola.ai/help-center/policies/security-reports/post-mortem-assembly-ai-api-key-exposure.md
- **Kosten**: Transkription sei der größte Kostentreiber (Zusammenfassung einer FirstMark-Podcast-Folge, Sekundärquelle; Originalfolge https://podcasts.apple.com/us/podcast/how-to-build-a-beloved-ai-product-granola-ceo-chris-pedregal/id1686238724?i=1000722944155 ).
- **Genauigkeit** laut Nutzern/Tests (Konkurrenz-Blog, mit Vorbehalt): ca. 85-90 %; Zahlen und Codes fehlerhaft ("312.000 USD" -> "312,1 Mio."; "NS-840-B17" -> "NS-40-B17"). https://www.happyscribe.com/blog/granola-ai-review

### 3.4 Sprecherzuordnung im Detail
- **Desktop-Standard**: Kanaltrennung, kein Klang-Diarisierung. Ich = Mikrofon, alle anderen = "Them" in einem Strom.
- **Sprecher-Tags**: Auslesen von Anzeigename und aktiver-Sprecher-Markierung der Meeting-App (Accessibility-API; Meet zusätzlich Chrome-Erweiterung). Genau darum der Zwang zu bestimmten Apps und Versionen (Zoom nur Workplace-Desktop; kein Web-Client). Ein Drittbericht nennt eine Lücke für Zoom auf Windows und "Teams ohne Sprecherlabels"; die Herstellerdoku (2026) führt Windows und Teams-Desktop ausdrücklich als unterstützt, die Drittaussage ist daher vermutlich veraltet.
- Stephenson (Sources/Access-Podcast): **bessere Sprecheridentifikation ist "wichtige Priorität"**. https://sources.news/p/whats-next-for-granola-access-podcast
- Öffentliche API v1.3.0 (07/2026): Transkript-Elemente kennzeichnen jetzt "Note-Taker" vs. "Teilnehmer" (belegt). https://docs.granola.ai/api-reference/changelog.md

### 3.5 LLM-Schicht und Prompt-Ansatz für Enhanced Notes
- Modelle: "Mischung aus OpenAI und Anthropic", laufender Wechsel je Anwendungsfall; kein Bring-your-own-Model (wird für Enterprise geprüft). https://docs.granola.ai/help-center/consent-security-privacy/model-training.md ; https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes.md . Historie: GPT-4o zum Start (05/2024, https://www.granola.ai/blog/announcement), GPT-5 in "Ask Granola" (08/2025, https://x.com/meetgranola/status/1953858355900137474). Sekundär: "dynamisches Routing über OpenAI, Anthropic, Google" (https://michaelgoitein.substack.com/p/granolas-revolutionary-ai-strategy). Nach Unterauftragsverarbeiter-Liste zusätzlich xAI und Fireworks.ai (siehe 3.8).
- **Prompt-Philosophie** (belegt, Pedregal): das Modell ist wie ein "Praktikant am ersten Tag: klug, aber ohne Kontext". Statt starrer Anweisungen liefert man **Kontext**: Teilnehmer und Rollen, Firmenhintergründe, was für die Beteiligten zählt, Nutzerprofil. Bsp. VC: "Sie müssen eine Investitionsentscheidung treffen, deshalb sind diese Details wichtig." https://creatoreconomy.so/p/the-hidden-rules-behind-successful-ai-products-chris-pedregal
- **Verschmelzung von Nutzer- und KI-Text**: Roh-Notizen "zeigen der KI, was wichtig ist" (Anker), Transkript liefert Belege; Nutzer-Überschriften strukturieren; Ausgabe wird zeilenweise mit Herkunft (Lupe) verknüpft. Ausführung: Vermutung, dass jede Ausgabezeile Referenzen auf Roh-Notiz-Blöcke oder Transkript-Bereiche trägt (Umsetzung nicht offengelegt). https://www.granola.ai/blog/announcement ; https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes.md
- **Evaluation**: manuelle, menschenzentrierte Evals ("Meeting-Notizen bewerten heißt Informationen nach Wichtigkeit ordnen"), keine Vollautomatisierung. https://creatoreconomy.so/p/the-hidden-rules-behind-successful-ai-products-chris-pedregal
- **Agenten-Design** (Blog 05/2026): Ziel "nützlich + ehrlich über Grenzen"; **Breite vor Tiefe** (Transkripte kosten ca. **10x mehr Token** als strukturierte Notizen; besser 100 Meetings auf Zusammenfassungsniveau als 10 im Volltext); **Suche vor Vollständigkeit** (Search -> Shortlist -> Read -> Repeat); Transparenz durch Coverage Notes. https://www.granola.ai/blog/how-granola-thinks-about-designing-agents
- **Streaming der Antworten**: eigene Endpunkte `stream.api.granola.ai` ("Streamed summaries and chat", Antwort-Pufferung muss ausgeschaltet sein) und `ws.public-api.granola.ai` (WebSocket-Auslieferung von KI-Antworten). https://docs.granola.ai/help-center/troubleshooting/network-troubleshooting.md
- **Latenz**: nicht dokumentiert. Belegt nur: Erzeugung "kann ein paar Minuten dauern, besonders bei langen Meetings"; iOS-Notizen brauchen länger als Desktop. Live-Transkript-Latenz: Vermutung im Bereich der Streaming-Anbieter (typisch unter 1 s), nicht belegt.
- Trend (Platformer, 07/2026): Erforschung, **verbatim-Transkripte nach einer Frist zu löschen** und das Wissen in ein "Arbeitsgedächtnis" zu komprimieren; Prototyp "enhanced transcripts" für Agenten (PII-Entfernung, Projektnamen auflösen, Kontakte identifizieren). https://www.platformer.news/granola-chris-pedregal-interview/

### 3.6 Backend, Datenmodell, Schnittstellen (Reverse Engineering, sekundär)
- Endpunkte des Clients (Stand der Analyse 2025): `POST /v2/get-documents` (100er-Pagination), `/v1/get-document-transcript`, `/v1/get-workspaces`, `/v2/get-document-lists` (Ordner), `/v1/get-documents-batch`. Geteilte Dokumente kommen nur über den Batch-Endpunkt. https://github.com/getprobo/reverse-engineering-granola-api
- Transkriptsegment: `source` (microphone/system), `text`, `start_timestamp`, `end_timestamp`, `confidence`.
- Dokument: ProseMirror-JSON; ein Dokument gehört einem Workspace und mehreren Ordnern.
- Cache-Dateiname `supabase.json` legt nahe, dass früher **Supabase** für Auth/Backend im Spiel war (Vermutung, Dateiname ist der einzige Hinweis); heute WorkOS.

### 3.7 Speicherung, Sicherheit, Datenschutz
- **Audio**: wird nicht gespeichert. Desktop: fließt live vom Gerät zum Transkriptionsanbieter; Mobil: kurz gecacht, nach Transkription "aus allen Granola- und Drittsystemen" gelöscht. https://www.granola.ai/security ; https://docs.granola.ai/help-center/ios/transcription.md
- **Ablage**: AWS (USA), VPC, verschlüsselt im Ruhezustand und bei der Übertragung, tägliche Backups; **keine EU-/UK-/Kanada-/Australien-Residenz**. https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md ; https://www.granola.ai/security
- **Zertifizierung**: SOC 2 Type II (07.07.2025). DSGVO: DPA mit EU- und UK-SCC. HIPAA: laut Preisseite/FAQ nur Enterprise mit BAA (ältere Drittberichte behaupten "kein BAA", vermutlich veraltet). FERPA: nein. https://www.granola.ai/updates ; https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md
- **Training**: Drittanbieter (OpenAI, Anthropic) dürfen nicht trainieren (Enterprise-Verträge). Granola selbst nutzt **anonymisierte Daten zur Modellverbesserung: Standard AN** (Basic/Business), Opt-out je Nutzer; Enterprise standardmäßig AUS, Opt-out nicht rückwirkend garantiert. https://docs.granola.ai/help-center/consent-security-privacy/model-training.md
- **Vorfälle/Berichte** (veröffentlicht): AssemblyAI-API-Key war über den Konfigurations-Endpunkt der iOS-TestFlight-Beta (333 Nutzer, 27.11.2024-11.03.2025) einsehbar; Tenable griff auf 29 Transkripte von 16 Beta-Nutzern zu; Schlüssel in AWS Secrets Manager, CI-Secret-Scanning. Weitere Berichte: Workspace-Auto-Join für Legacy-Google-Konten, Google-Session-Logout, Desktop-Navigation. https://docs.granola.ai/help-center/policies/security-reports/post-mortem-assembly-ai-api-key-exposure.md
- **Rechtliches**: Sammelklage Chamberlain v. Granola (30.07.2026, N.D. California; CIPA-All-Party-Consent, "heimliche Aufnahme", Standard-Training) https://www.computerworld.com/article/4206255/granola-lawsuit-raises-concerns-over-ai-note-taking-app-privacy.html . Presse (The Verge, 04/2026): Standard "Link-Freigabe für jeden mit dem Link" trotz Aussage "standardmäßig privat" (Doku benennt den Ausliefer-Standard bis heute nicht). https://getroutines.ai/transparency/granola-ai
- Kein eigener MFA/2FA (Delegation an Google/Microsoft/SSO), keine Ende-zu-Ende-Verschlüsselung (Drittaudit). https://getroutines.ai/transparency/granola-ai

### 3.8 Unterauftragsverarbeiter / eingesetzte Dienste

Direkt lesbar war die Trust-Center-Seite nicht; die Liste stützt sich auf Auditbericht eines Dritten (16 Einträge, alle US außer einem UK-Support-Anbieter) plus Suchtreffer der Trust-Center-Seite. **Sekundär, vor Verwendung im Vertrag am Original prüfen:** https://trust.granola.ai/subprocessors

| Zweck | Dienst | Beleg |
|---|---|---|
| Transkription | Deepgram, AssemblyAI | belegt (Allowlist + Datenschutz + Security-Seite) |
| LLM/Inferenz | OpenAI, Anthropic, Google Cloud, xAI, Fireworks.ai | belegt: OpenAI/Anthropic (Doku); sekundär: Google, xAI, Fireworks (Audit) |
| Vektorsuche | Turbopuffer | sekundär (Audit) |
| Web-Recherche/Anreicherung (Briefs, People/Companies) | Parallel Web Systems | sekundär (Audit) |
| Hosting | AWS (Cognito, CloudWatch Logs, VPC, Secrets Manager) | belegt |
| Auth/SSO | WorkOS | belegt |
| Benachrichtigungen | Knock | belegt |
| Zahlung | Stripe | belegt (Datenschutzerklärung) |
| Vorgehen bei Änderungen | 10 Tage Vorankündigung, 30 Tage Einspruchsfrist | belegt (DPA) |

Quellen: https://docs.granola.ai/help-center/policies/privacy-policy.md ; https://docs.granola.ai/help-center/policies/data-processing-addendum.md ; https://getroutines.ai/transparency/granola-ai

## 4. UX-Details und Nutzerstimmen

### 4.1 Was das Produkt gut macht (belegt/Nutzerurteile)
1. **Notizblock statt Bot/Recorder**: "The note is the product" (Review), Bedienung wie Apple Notes; Ziel "a notepad, not a recorder". Nutzer: "Kein Bot ... 'ein dritter Teilnehmer mit Roboter-Avatar' im Gründer-Call". https://www.granola.ai/blog/why-granola-doesnt-use-a-bot ; https://www.producthunt.com/products/granola/reviews
2. **Schwarz/Grau-Prinzip + Lupe** (Herkunft jedes Punktes) baut Vertrauen und erlaubt schnelle Prüfung. https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes.md
3. **Nutzernotizen lenken die KI**: wenig tippen, Fokus im Gespräch; Ergebnis "spiegelt meine Prioritäten". https://zackproser.com/blog/granola-ai-review
4. **Chat über alle Meetings** mit Zitaten, Recipes ("Recipes galore, and you can remix them"), Cmd+J überall. https://www.happyscribe.com/blog/granola-ai-review
5. **Design-Disziplin**: Marke, Typografie (Quadrant/Melange, 02/2026), Blog "Don't animate height!", Engineering-Blog zu Navigation. App-Store-Wertung 5,0 (14.000 Bewertungen), G2 ca. 4,7-4,8, Product Hunt 4,8. https://apps.apple.com/us/app/granola-ai-meeting-notes/id6739429409 ; https://www.g2.com/products/granola/reviews
6. **Unaufdringliche Erinnerungen** (Aufblenden 1 Min. vor Termin mit "Take Notes", Ad-hoc-Erkennung), Fenster positioniert sich neben den Call. https://docs.granola.ai/help-center/taking-notes/notifications.md
7. **Stärke des Fokus** ("surprisingly unambitious" - erst eine Sache extrem gut). https://www.cognitiverevolution.ai/calm-ai-for-crazy-days-inside-granola-s-design-philosophy-with-co-founder-sam-stephenson-newsletter/
8. **Freier Chat in der Gratisstufe** wird als Wettbewerbsvorteil genannt (tl;dv-Review), allerdings mit 30-Tage-Grenze. https://tldv.io/blog/granola-review/

### 4.2 Kritik und Schmerzpunkte (Chancen für die Eigenentwicklung)
| # | Kritikpunkt | Belege |
|---|---|---|
| 1 | **Kein Audio, keine Wiedergabe -> keine Verifikation.** Zahlen, Namen, Zusagen lassen sich nicht nachhören; Fehler wie "312.000" -> "312,1 Mio." bleiben unerkannt. Audio-/Videoaufzeichnung ist ausdrücklich "nicht geplant". | https://www.happyscribe.com/blog/granola-ai-review ; https://tldv.io/blog/granola-review/ ; https://docs.granola.ai/help-center/feature-requests.md |
| 2 | **Sprecherzuordnung schwach**: Desktop nur "Me/Them"; Namen nur über App-UI-Auslesen (nicht im Zoom-Web-Client, nur Teams-/Zoom-Desktop; ältere Berichte nennen zusätzlich Windows-Lücken); ab 3+ Personen bricht die Zuordnung ein; Präsenzmeetings mit einem Mikrofon nicht lösbar. Stephenson nennt es selbst als Priorität. | https://anarlog.so/blog/granola-ai-complaints/ ; https://www.happyscribe.com/blog/granola-ai-review ; https://sources.news/p/whats-next-for-granola-access-podcast |
| 3 | **Stille Aufnahmeausfälle**: Sitzung wirkt aktiv, es wird nichts erfasst, kein Alarm; Berichte über nur ca. 60 % erfolgreiche Aufnahmen; mangels Audio kein Nachholen. Zusätzlich manueller Start (Auto-Start nur "in Prüfung"). | https://anarlog.so/blog/granola-ai-complaints/ ; https://www.aitooldiscovery.com/guides/granola-ai-reddit ; https://docs.granola.ai/help-center/feature-requests.md |
| 4 | **Datenschutz/DSGVO**: US-only-Cloud, Trainings-Default an (außer Enterprise), heimliche Aufnahme (Sammelklage), Standard-Linkfreigabe, keine Datenresidenz in der EU; Deutsche Kritik zu § 201 StGB stammt von Konkurrenz (Vorsicht). | https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md ; https://www.computerworld.com/article/4206255/granola-lawsuit-raises-concerns-over-ai-note-taking-app-privacy.html ; https://www.sally.io/blog/granola-alternative |
| 5 | **Sprachen**: Desktop-Zusammenfassungssprache "Englisch oder automatisch" (meetergo-Test), bei Denglisch teils englische Notizen trotz deutschem Gespräch; Dialekt/Schweizerdeutsch/Fachbegriffe schwach; mobil eine Sprache pro Meeting; Vokabelliste nur 30/50 Begriffe. | https://meetergo.com/blog/granola-ai ; https://docs.granola.ai/help-center/customising-granola/multi-language.md |
| 6 | **Preis-/Paywall-Mechanik**: Gratisstufe zeigt nur 30 Tage, ältere Notizen "gespeichert, aber gesperrt"; Business 14 USD nur monatlich (kein Jahresrabatt); Enterprise 35 USD; Rebrand-Kritik 02/2026. | https://www.granola.ai/pricing ; https://docs.granola.ai/help-center/managing-your-account/subscriptions-and-billing.md ; https://www.hedy.ai/post/granola-redesign-alternative-hedy/ |
| 7 | Sonstige Lücken: kein Web-Transkribieren, kein Datei-Import, Export nur als CSV per E-Mail (1 x/24 h), Web-Ansicht ohne Transkript, kein Bring-your-own-Model, Kalender nur Google/Outlook, Anmeldung nur mit Google/Microsoft (Kalender-Zwang), Zoom/Teams/Meet-Abhängigkeit für Namen, Windows-Verhalten bei nicht unterstützten Apps. | https://docs.granola.ai/help-center/sharing/exporting-notes.md ; https://tldv.io/blog/granola-review/ ; https://docs.granola.ai/help-center/getting-started/syncing-your-calendars.md |
| 8 | Halluzinationen/Fehlzuordnungen: Zusagen werden falschen Personen zugeordnet, Konditionalsätze verloren. Granola mindert das, weil es Nutzertext verankert. | https://www.granola.ai/blog/meeting-action-items-ai-extraction ; https://overtheanthill.substack.com/p/granola |

## 5. Preise und Abo-Grenzen

Offizielle Seite (abgerufen 29.09.2026): https://www.granola.ai/pricing

| | Basic | Business | Enterprise |
|---|---|---|---|
| Preis | 0 USD | 14 USD/Nutzer/Monat (nur monatlich) | ab 35 USD/Nutzer/Monat (monatlich oder jährlich) |
| Meeting-Historie im Client | 30 Tage (älteres bleibt gespeichert, ist gesperrt) | unbegrenzt | unbegrenzt |
| KI-Notizen | ja | ja | ja |
| Chat über mehrere Meetings | nur "Auto"-Modell, 30 Tage | Auto + Modellwahl (Standard/Thinking) | wie Business (Fable 5 nur per Admin) |
| Integrationen | nur Basis-Slack | Notion, Zapier, HubSpot, Affinity (Attio laut Blog/Doku), Slack | + Salesforce u. a. |
| API | nein | ja (persönliche + Workspace-Keys) | ja + Admin-/Enterprise-API, Webhooks |
| MCP | 30 Tage, kein Transkript-Tool | volle Historie inkl. Transkript | Admin-Steuerung |
| Briefs, Follow-up-Mails | nein | ja | ja |
| SSO/SCIM, Audit, Auto-Löschung, HIPAA/BAA, Trainings-Opt-out workspace-weit | nein | nein | ja |
| Trainings-Opt-out | pro Nutzer | pro Nutzer | workspace-weit, Standard AUS |
| Enthalten in allen | Mobile, Sprecher-Tags, geteilte Ordner, SOC 2 | | |

- Testphase für Business/Enterprise (Dauer nicht genannt); Start-ups/Studierende: 12 Monate Business gratis auf Antrag. https://docs.granola.ai/help-center/managing-your-account/subscriptions-and-billing.md
- **Widersprüche**: Ältere Quellen nennen eine "Individual"-Stufe für 18 USD und ein Limit von "25 Notizen gesamt" in der Gratisstufe; nach der Neustrukturierung (02/2026) gibt es nur drei Stufen und das 30-Tage-Fenster. Das Blog-Dokument "free vs paid" führt bei Basic "Integrationen: keine", Doku/Preisseite "Slack". https://www.granola.ai/blog/granola-free-vs-paid-features-each-plan ; https://www.hedy.ai/post/granola-redesign-alternative-hedy/

## 6. Neuerungen 2025-2026 (Auswahl, chronologisch)

Quelle für alle Zeilen: https://www.granola.ai/updates und https://releasebot.io/updates/granola

| Datum | Neuerung |
|---|---|
| 16.01.2025 | "Ask Granola" während eines Meetings |
| 12.02.2025 | Export nach Notion |
| 11.03.2025 | Chat über Person/Firma |
| 30.04.2025 | Granola für iOS |
| 08.05.2025 | Chat über alle Meetings |
| 14.05.2025 | **Granola 2.0** ("Second brain for your team"), Series B 43 Mio. USD bei 250 Mio. USD Bewertung (https://techcrunch.com/2025/05/14/ai-note-taking-app-granola-raises-43m-at-250m-valuation-launches-collaborative-features/) |
| 22.05.2025 | 10 Sprachen, gemischte Sprachen im Meeting |
| 05.06.2025 | Datei-Upload |
| 11.06.2025 | **Granola für Windows** |
| 07.07.2025 | SOC 2 Type 2 |
| 14.07.2025 | Team-Ordner |
| 24.07.2025 | Notizen per Anweisung bearbeiten |
| 28.07.2025 | Zapier |
| 08.09.2025 | People und Companies, Attio, Zapier |
| 15.09.2025 | Telefonate |
| 17.09.2025 | "Shared with me" |
| 30.09.2025 | **Recipes**, neuer Chat |
| 22.12.2025 | Labs: "Heads Up" (Einwilligungshinweis) |
| 15.01.2026 | Anmeldung mit Microsoft, Outlook-Kalender, Teams-Join |
| 28.01.2026 | Transkriptabschnitte löschen |
| 02.02.2026 | Neues Branding |
| 04.02.2026 | **MCP-Server** |
| 25.03.2026 | Series C 125 Mio. USD (1,5 Mrd. USD), Spaces, persönliche und Enterprise-API |
| 21.04.2026 | Agentischer Chat, Inline-Zitate, neue Recipes |
| 20.05.2026 | **Briefs** (Mac, Windows, iPhone) |
| 01.07.2026 | Android-App |
| 28.07.2026 | Apple-Watch-App |
| 02.09.2026 | 32 Sprachen |
| 09/2026 | API v1.5.0 (private Notizen); CRM-Sync-Meldungen in Drittquellen |

## 7. Widersprüche und Wissenslücken (ehrlich benannt)

- **Windows-Systemaudio**: offizielle Doku spricht von "Apps, die wir noch nicht unterstützen" (app-bezogen); Drittquelle behauptet Vollgerät-Loopback. Ungeklärt. Für den Nachbau irrelevant, aber wichtig für die eigene Wahl (Prozess-Loopback vs. Geräte-Loopback).
- **Mac-API** (ScreenCaptureKit vs. Core Audio Tap): nicht dokumentiert.
- **Welcher STT-Anbieter für welche Sprache/Plattform**: nicht dokumentiert (nur Anbieter und Endpunkte).
- **Echo-Cancelling-Verfahren** (Algorithmus, on-device): nur Aussage des Gründers, kein Detail.
- **LLM je Feature**: keine verbindliche Zuordnung; "Fable 5" ist laut Doku ein Thinking-Modell, sein Anbieter (Anthropic) folgt aus der Log-Aufbewahrung.
- **Latenzen**: nirgends veröffentlicht.
- **Nutzerzahlen/Umsatz**: nicht offengelegt; Zahlen wie "10 % Wachstum pro Woche" (05/2025), "80-100 Tsd. wöchentliche Nutzer" sind Schätzungen aus Sekundärquellen.
- **Sprachanzahl**: 32 (Update 02.09.2026) gegenüber älteren Angaben 10 (Desktop) bzw. "31 Desktop / 17 Mobil" (Review). Ich folge dem Update.
- **Export-Umfang**: Doku 2026 sagt "inkl. Transkripte", Blog von anarlog sagt "ohne Transkripte". Ich folge der Doku.
- **Zeilenweise Herkunft (Lupe)**: Funktionsweise ist sichtbar, aber die interne Umsetzung nicht beschrieben.

## 8. Ableitungen für den Nachbau (Empfehlungen, keine Granola-Fakten)

Nachfolgendes sind Vorschläge des Recherche-Workers zur Nutzung der Befunde, ausdrücklich **Vermutung/Empfehlung**.

1. **Audio behalten (lokal, optional, verschlüsselt)**: größter Kritikpunkt Nr. 1 und Nr. 3. Zeitstempel je Segment + Audio-Wiedergabe der Zitatstelle ("Lupe" führt direkt zum Ton) schlägt Granola. Aufbewahrungsfrist einstellbar, Standard kurz.
2. **Zwei Kanäle wie Granola, aber mit echter Diarisierung**: Mikrofon = Ich (sicher), Systemkanal zusätzlich akustisch diarisieren (lokal, z. B. ONNX-Modelle); Namen aus Meeting-UI (Windows UI Automation) nur als Zusatzquelle, nicht als einzige.
3. **Windows zuerst**: WASAPI Loopback (Prozess-Loopback ab Windows 10 2004 ermöglicht Erfassung einzelner Apps und vermeidet Musik/Benachrichtigungen im Transkript) plus AEC vor der STT.
4. **Ausfall-Erkennung**: Pegel-/Stille-Wächter, sichtbarer Alarm, automatische Wiederaufnahme bei Geräte-/Headset-Wechsel (Bluetooth-Profilwechsel!).
5. **Lokal-first als Alleinstellung**: STT lokal (auch Deutsch, eigenes Vokabular ohne 30er-Grenze), LLM lokal oder frei wählbar (BYO-Modell, auf Wunsch EU-Anbieter). Damit entfällt die Datenschutz-Kritik (US-Cloud, Training, Einwilligung) weitgehend.
6. **Enhanced-Notes-Prinzipien übernehmen**: Nutzerstichpunkte als Anker, Schwarz/Grau-Kennzeichnung, Quellverweis je Zeile, Templates als Prompt-Text, Kalender-/Teilnehmerkontext, "Ich"-Profil. Kontext statt starrer Anweisungen (Pedregal).
7. **Chat-Agent mit Breite vor Tiefe**: Notizen als Index (Transkripte kosten ca. 10x Token), Suche -> Shortlist -> Lesen, Zitate, "Coverage Notes". Lokal umsetzbar mit Embeddings in SQLite.
8. **Deutsch zuerst prüfen**: Ausgabesprache getrennt von Gesprächssprache, Code-Switching (Denglisch), Zahlen- und Fachbegriffe, Vokabelliste unbegrenzt.
9. **Offene Schnittstellen früh**: lokaler MCP-Server (Granola kann MCP nur remote) und Markdown/JSON-Export pro Notiz (Granola nur CSV per E-Mail).
10. **Einwilligung**: Hinweis-Funktionen (Chat-Nachricht, Wasserzeichen) ab Version 1; wichtig wegen Sammelklage und deutscher Rechtslage.

## 9. Quellenverzeichnis (Auswahl nach Abschnitt)

**Hersteller (Primär)**
- Startseite, Preise, Security: https://www.granola.ai/ ; https://www.granola.ai/pricing ; https://www.granola.ai/security
- Updates/Blog: https://www.granola.ai/updates ; https://releasebot.io/updates/granola ; https://www.granola.ai/blog ; https://www.granola.ai/blog/announcement ; https://www.granola.ai/blog/why-granola-doesnt-use-a-bot ; https://www.granola.ai/blog/how-granola-thinks-about-designing-agents ; https://www.granola.ai/blog/granola-chat-just-got-smarter ; https://www.granola.ai/blog/granola-mcp ; https://www.granola.ai/blog/back-button ; https://www.granola.ai/blog/meeting-recipes-repeatable-formats ; https://www.granola.ai/blog/series-c ; https://www.granola.ai/blog/granola-free-vs-paid-features-each-plan
- Doku-Index: https://docs.granola.ai/llms.txt ; Transkription https://docs.granola.ai/help-center/taking-notes/transcription.md ; Sprecher https://docs.granola.ai/help-center/taking-notes/speaker-attribution.md ; Enhanced Notes https://docs.granola.ai/help-center/taking-notes/ai-enhanced-notes.md ; Netzwerk https://docs.granola.ai/help-center/troubleshooting/network-troubleshooting.md ; FAQ https://docs.granola.ai/help-center/consent-security-privacy/security-privacy-data-faqs.md ; Modelle/Training https://docs.granola.ai/help-center/consent-security-privacy/model-training.md ; Datenschutz https://docs.granola.ai/help-center/policies/privacy-policy.md ; DPA https://docs.granola.ai/help-center/policies/data-processing-addendum.md ; API https://docs.granola.ai/introduction.md ; API-Changelog https://docs.granola.ai/api-reference/changelog.md ; MCP https://docs.granola.ai/help-center/sharing/integrations/mcp.md ; Post-Mortem https://docs.granola.ai/help-center/policies/security-reports/post-mortem-assembly-ai-api-key-exposure.md ; Feature-Requests https://docs.granola.ai/help-center/feature-requests.md

**Interviews/Presse**
- Platformer (Pedregal, 07/2026): https://www.platformer.news/granola-chris-pedregal-interview/
- Creator Economy: https://creatoreconomy.so/p/the-hidden-rules-behind-successful-ai-products-chris-pedregal
- Shotcast: https://shotcast.substack.com/p/ux-and-not-llm-chris-pedregal-granolas
- Sources/Access (Stephenson): https://sources.news/p/whats-next-for-granola-access-podcast
- Cognitive Revolution (Stephenson): https://www.cognitiverevolution.ai/calm-ai-for-crazy-days-inside-granola-s-design-philosophy-with-co-founder-sam-stephenson-newsletter/
- TechCrunch Series C: https://techcrunch.com/2026/03/25/granola-raises-125m-hits-1-5b-valuation-as-it-expands-from-meeting-notetaker-to-enterprise-ai-app/
- Computerworld (Klage): https://www.computerworld.com/article/4206255/granola-lawsuit-raises-concerns-over-ai-note-taking-app-privacy.html
- MAD-Podcast/FirstMark: https://podcasts.apple.com/us/podcast/how-to-build-a-beloved-ai-product-granola-ceo-chris-pedregal/id1686238724?i=1000722944155

**Reverse Engineering / Community**
- https://github.com/getprobo/reverse-engineering-granola-api ; https://github.com/openclaw/graincrawl/issues/15 ; https://github.com/theantichris/granola/issues/22 ; https://news.ycombinator.com/item?id=43858740 ; https://news.ycombinator.com/item?id=44725306 ; https://www.recall.ai/blog/granola-ai-alternatives ; https://github.com/fastrepl/anarlog

**Reviews/Audits (Drittquellen, teilweise Konkurrenz)**
- https://www.happyscribe.com/blog/granola-ai-review ; https://zackproser.com/blog/granola-ai-review ; https://tldv.io/blog/granola-review/ ; https://anarlog.so/blog/granola-ai-complaints/ ; https://www.aitooldiscovery.com/guides/granola-ai-reddit ; https://www.producthunt.com/products/granola/reviews ; https://www.g2.com/products/granola/reviews ; https://getroutines.ai/transparency/granola-ai ; https://drel.ai/blog/granola-ai-security-review ; https://zackproser.com/blog/granola-enterprise-security-features ; https://www.sally.io/blog/granola-alternative ; https://meetergo.com/blog/granola-ai ; https://www.hedy.ai/post/granola-redesign-alternative-hedy/ ; https://saas.com.ai/granola-for-windows/ ; https://simplify.jobs/p/120b247a-ed5b-48cf-a3c0-35e0a2bec065/Product-Engineer
