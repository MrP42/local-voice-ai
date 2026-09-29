# M5/M6-Entwurf: Kalender, Meeting-Erkennung, Personen, Brief, Follow-up, Export, lokaler MCP (F15–F21, AK9/AK10)

Stand 29.09.2026, Paket P5 (lv-architect). Pfade: RS = `apps/local-voice/src-tauri/src`, FE = `apps/local-voice/src`, FX = `apps/local-voice/src-tauri/tests/fixtures`. Spike-Code: `C:\Users\wolff\lva-spikes\m5\` (`mic.ps1`, `mic_latency.py`, `ics/`, `guistdio/`, `proto.html`).
Leitlinie: Nichts verlässt den Rechner außer dem ICS-/Graph-Abruf. Brief und Follow-up laufen über den M4-Chat (Recipes), der MCP-Server liest nur. Einstellungen nur in der Gruppe „Besprechungen“ (DictationTab.tsx:63), kein neuer Menüpunkt, kein neuer Reiter.

## 1. Verifikation und Messungen (diese Maschine, Windows 11 26200, ohne Admin)

| # | Frage | Ergebnis | Beleg |
|---|---|---|---|
| V1 | ConsentStore `HKCU\…\CapabilityAccessManager\ConsentStore\microphone` ohne Admin lesbar? | ja, 91 Einträge (`NonPackaged\<Pfad mit #>` + Paketfamilien), Werte `LastUsedTimeStart/Stop` (FILETIME), aktiv ⇔ `Stop == 0` | `mic.ps1` |
| V2 | Latenz Mikrofon öffnen → Registry `Stop=0` | 57–58 ms (Läufe 2/3, Polling 50 ms); Lauf 1 817 ms inkl. PortAudio-Init. Schließen → `Stop≠0`: ≤ 2 ms | `mic_latency.py` |
| V3 | `RegNotifyChangeKeyValue` (Teilbaum) statt Polling | feuert, aber **mehrfach je Übergang** (Start und Stop werden getrennt geschrieben) → nach jedem Ereignis ohnehin Vollscan nötig | `mic_latency.py` |
| V4 | Kosten Vollscan (alle Schlüssel) | Median 1,6 ms, max 3,9 ms (Python/winreg, 91 Schlüssel); bei 2-s-Intervall < 0,1 % eines Kerns (Rust eher weniger) | `mic_latency.py` |
| V5 | Wie erscheinen Meeting-Apps? | Teams neu = Paket `MSTeams_8wekyb3d8bbwe` (Sitzung 21.09. 08:00–08:42 sichtbar); Webex = `…#CiscoSparkLauncher#CiscoCollabHost.exe`; Chrome = `…#chrome.exe` (nur Browser, kein Tab); WhatsApp = Paket `5319275A.WhatsAppDesktop_…`; `msedgewebview2.exe` mit Versionspfad (nicht zuordenbar). **Zoom nicht installiert → ungeprüft [?]**. Eigene App (`…#Local Voice AI#local-voice-ai.exe`) steht dauerhaft aktiv → Eigenfilter Pflicht | `mic.ps1` |
| V6 | calcard 0.3.14 (Stalwart, Apache-2.0/MIT): Outlook-Serie mit `TZID=W. Europe Standard Time` + VTIMEZONE | Windows-Zonenname aufgelöst, Sommerzeitwechsel 25.10. korrekt (10:00 CEST → 10:00 CET), EXDATE entfernt, RECURRENCE-ID verschiebt korrekt | `ics/fixture.ics` |
| V7 | calcard: Override erbt nichts | verschobene Instanz ohne ATTENDEE/Teams-URL; `STATUS:CANCELLED`-Override wird **mitgeliefert** → Erben vom Master und Filtern macht unser Code | `ics/` |
| V8 | calcard-Fehler: Serie mit UTC-DTSTART (`…Z` + RRULE) | **um Ortsversatz verschoben** (08:00Z → 06:00Z); Einzeltermin mit Z korrekt. Umgehung `DTSTART;TZID=UTC:` vor dem Parsen → korrekt, EXDATE in Z greift | `ics/utc.ics`, `utc2.ics` |
| V9 | calcard `expand_dates(tz, limit)`: Limit ist **global** | tägliche Serie seit 2000 verbraucht das Limit (20 000), spätere Wochenserie liefert 0 Instanzen → je Serie (UID-Gruppe) einzeln expandieren | `ics/big.ics` |
| V10 | ICS-Leistung | 2,4 MB / 3 006 Komponenten: Parsen 10,7 ms, Expandieren 6,5 ms (Release); Spike-EXE 2,05 MB; neue Abh. calcard → chrono-tz, ahash, mail-builder, mail-parser (alle MIT/Apache) | `ics/` |
| V11 | Alternativen ICS | `icalendar` 0.17.13 (MIT/Apache, keine Expansion) + `rrule` 0.14 (MIT/Apache, kennt keine Windows-Zonen, kein VTIMEZONE) = mehr Eigencode; `ical` 0.11 Lizenz „unknown“ → raus | `cargo info` |
| V12 | HTML→PDF mit Chromium (headless Edge als Stellvertreter für WebView2 `PrintToPdf`) | 1 Seite, 37,9 KB; Umlaute, „“, €, ☐ korrekt und als Text extrahierbar; < 1 s | `proto.html` |
| V13 | WebView2 `PrintToPdf` erreichbar? | `webview2-com(-sys) 0.38.2` ist über wry schon in Cargo.lock, `ICoreWebView2_7::PrintToPdf` vorhanden, nutzt `windows 0.61` wie wir. **Verstecktes Fenster nicht gemessen** → erster Schritt P6b | Cargo.lock, Quellen |
| V14 | Formatierte Zwischenablage | `tauri-plugin-clipboard-manager 2.3.2` hat `write_html(html, alt_text)` (arboard 3.6.1 → CF_HTML); keine neue Abh., keine Capability (Aufruf aus Rust). MinutesView.tsx:87 nutzt schon `ClipboardItem` | Plugin-Quelle `desktop.rs:72` |
| V15 | EXE im Windows-GUI-Subsystem als stdio-Server | funktioniert mit Pipes (Python `Popen`, wie Node-`spawn` bei Claude/Codex): Antwort 6,2 ms, Exit 0 | `guistdio/` |
| V16 | Crates | rmcp 3.5.0 (Apache-2.0, offiziell, 3 Hauptversionen in ~1 Jahr), keyring 4.2.0 (nicht im Lock), printpdf 0.12.8 (MIT, eigenes Layout + Fonteinbettung), genpdf 0.2.0 (2021, veraltet), oauth2 5.0 (unnötig). Vorhanden: winreg 0.55, sysinfo 0.39, reqwest 0.12, sha2, base64, rand, zip, zeroize, pdf-extract, `windows 0.61`. rustc 1.97.1 | `cargo info`, Cargo.toml |
| V17 | Graph | nicht gemessen (keine Registrierung, Auftrag). Aussagen aus lokaler-stack.md §7 (Q49/Q50) und MS-Doku | – |

## 2. Architektur

```
Kalenderquellen (ICS-URL | Graph)  ──CalendarService (tokio, 15 min/5 min, ETag)──► meetings.db: calendar_events (+attendees)
MicUsageWatcher (Thread, 2 s, Registry)──Detector (rein)──┐           │ Reminder-Tick 15 s (rein: due_reminders)
                                                          ▼           ▼
                                      MeetingPrompt-Fenster „meeting_prompt“ (always-on-top, ohne Fokusraub)
                                      [Aufnahme starten] → Einwilligungsschritt → meetings_start_from_event
                                      [Beitreten] opener(join_url) · [Vorbereiten] → M4-ChatPanel mit Recipe
Personen: humans (+Aliasse) ◄── Teilnehmende (Kalender) + speakers.display_name (M3) ──► meeting_participants
Export: ExportBundle ─► md/txt/docx (vorhanden) · html (neu) · pdf (WebView2) · srt/vtt · json · Zwischenablage HTML+Text
Follow-up: Recipe builtin (M4) → MailDraft → Kopieren | mailto | .eml   ·   MCP: local-voice-ai.exe --mcp (stdio, nur lesend)
```
Module: neu `RS/managers/calendar/{mod,model,ics,fetch,secret,service,reminder,graph}.rs`, `RS/managers/meeting_detect/{mod,source,catalog,detector}.rs`, `RS/managers/people/{mod,normalize}.rs`, `RS/managers/meetings/{mail,pdf}.rs`, `RS/mcp/{mod,protocol,tools}.rs`, `RS/meeting_prompt.rs` (Fenster, Muster `overlay.rs:386`), FE-Einstieg `FE/meeting-prompt/` (Muster `FE/overlay/`, vite-Eintrag).

## 3. Entwurf je Funktion

**F15 Kalender.** Quelle 1 ICS-URL (Outlook/M365 „Kalender veröffentlichen“, Google „Privatadresse im iCal-Format“, Nextcloud, iCloud; `webcal://` → `https://`). Abruf reqwest, 15 min + „Jetzt aktualisieren“, `If-None-Match`/`If-Modified-Since`, Timeout 30 s, max. 20 MB, ≤ 5 Umleitungen, Antwort muss mit `BEGIN:VCALENDAR` beginnen (Google liefert bei falscher URL HTML). Verarbeitung: `normalize_utc_rrule_starts` (V8) → Parser → Gruppen je UID (Master + Overrides + VTIMEZONEs) → `expand_dates(default_tz, 20 000)` je Gruppe (V9) → Fenster [jetzt−30 T, jetzt+30 T] → Overrides erben ATTENDEE/ORGANIZER/Join-URL/LOCATION vom Master, wenn sie fehlen (V7) → `STATUS:CANCELLED` bleibt im Cache mit `cancelled=1`, wird nie angezeigt/erinnert. Standardzone = Windows-Zonenname aus `HKLM\SYSTEM\CurrentControlSet\Control\TimeZoneInformation\TimeZoneKeyName` (calcard löst Windows-Namen), Rückfall UTC + Warnung. Ganztägig (`VALUE=DATE`) → `all_day`, keine Erinnerung. Join-URL: `X-MICROSOFT-SKYPETEAMSMEETINGURL`, sonst Regex über LOCATION/DESCRIPTION (`teams.microsoft.com/l/meetup-join`, `meet.google.com/`, `zoom.us/j/`, `*.webex.com/`). Ersetzen je Quelle in EINER Transaktion; `reminded_at`/`dismissed_at` bleiben per Schlüssel `source:uid:start_ms` erhalten.
Quelle 2 Graph (R3, P5f): Auth-Code-Flow mit PKCE, Systembrowser über opener, Loopback `http://127.0.0.1:<freier Port>` (Entra ignoriert den Port bei localhost-Umleitungen laut Doku), `state` + 5-min-Timeout, Mandant `common`, Scopes `Calendars.Read offline_access User.Read`. Geräte-Code-Flow nur Rückfall: viele Mandanten sperren ihn per bedingtem Zugriff; er braucht keinen Listener, zeigt aber einen Code zum Abtippen. Abruf `GET /me/calendarView?startDateTime&endDateTime&$select=subject,start,end,isAllDay,isCancelled,attendees,organizer,onlineMeeting,location,bodyPreview&$top=100`, `Prefer: outlook.timezone="UTC"`, Paging über `@odata.nextLink`, 429 → `Retry-After`. Eigenes /me → `is_self`. Handgeschrieben mit reqwest/sha2/base64/rand (kein oauth2-Crate). Google: nur über ICS.
**Geheimnisse** (ICS-URL = Lesezugriff auf den Kalender, Refresh-Token): `calendar/secret.rs` mit DPAPI (`CryptProtectData`, Benutzerbereich) in `<appdata>/secrets/<id>.bin`; nicht in `settings_store.json`, nicht in meetings.db, nicht im Geräte-Sync. Grund gegen Credential Manager/keyring: Blob-Grenze 2 560 Byte (Refresh-Token können größer sein). Neues `windows`-Feature `Win32_Security_Cryptography`. UI zeigt URL maskiert (`…/calendar.ics` + Host).
**Erinnerung 1 min vorher** (`reminder.rs`, rein): fällig, wenn `start−lead ≤ jetzt < start+2 min`, nicht ganztägig, nicht abgesagt, nicht erinnert/verworfen, keine Aufnahme läuft, und (≥ 2 verschiedene Teilnehmende inkl. Organisator ODER Join-URL ODER Quelle ohne Teilnehmerdaten ODER Einstellung „alle Termine“). Nach Ruhezustand: Termine, deren Start > 2 min vorbei ist, werden nicht nachgeholt. Tick 15 s.
**Start aus Termin**: `meetings_start_from_event` nimmt Titel = Termintitel (nur wenn der Nutzer keinen eingegeben hat), Vorlage = `template_id` der letzten Besprechung derselben Serie (UID) sonst Standard, legt nach `recorder.start` die Verknüpfung + Teilnehmende an. `meetings_start` bleibt unverändert (Konflikt P1c/P1f vermieden). Manuell gestartete Aufnahme: läuft genau ein Termin im Fenster ±15 min → automatisch verknüpfen (`linked_by='auto'`), sonst Auswahl im Detail.

**F16 Ad-hoc-Erkennung.** `MicUsageSource`-Trait (Registry echt, Fake im Test) → `Detector::step` (rein) alle 2 s. Regeln: Eigenfilter (`current_exe` in #-Form + alle Pfade mit `\local-voice-ai.exe`/`\sprechstift.exe`); Katalog (Teams neu/klassisch, Zoom `Zoom.exe`, Webex `CiscoCollabHost.exe`/`atmgr.exe`, Slack, Discord, WhatsApp, Skype, Signal; Browser chrome/msedge/firefox/brave/opera → „Browser-Call (z. B. Google Meet)“); `msedgewebview2.exe` ignoriert (V5); Modus `Aus | Meeting-Apps (Standard) | Alle Apps`; aktiv ≥ 5 s (Teams-Gerätetest, kurze Mikrofonproben); Sitzung = (app_key, LastUsedTimeStart) → pro Sitzung höchstens ein Hinweis; `Stop=0` ohne laufenden Prozess (sysinfo, nur NonPackaged) = Leiche nach Absturz → ignoriert; Aufnahme läuft → kein Hinweis; Mikrofon wieder frei → offener Hinweis schließt sich. Termin läuft ±15 min → Hinweis nennt Termin und startet mit dessen Titel/Teilnehmenden (Granola-Regel). Kosten: ~2 ms je Scan (V4), keine COM-Objekte, kein Mikrofonzugriff. Latenz: Poll 2 s + 5 s Entprellung. `IAudioSessionManager2` (Sitzungen je Aufnahmegerät mit PID) ist für v1 nicht nötig: COM-MTA-Thread, alle Endpunkte inkl. virtueller Geräte iterieren, mehr Code; Nutzen erst für Browser-Tab-Zuordnung (PID → Fenstertitel) → spätere Option. macOS: `Unsupported` (Einstellung ausgeblendet), später CoreAudio `kAudioDevicePropertyDeviceIsRunningSomewhere`.

**Hinweisfenster** `meeting_prompt` (neues Label, eigener vite-Eintrag, `always_on_top`, `skip_taskbar`, `focused(false)`, unten rechts, 360×150): Zustand 1 „Jour fixe Vertrieb beginnt in 1 min · 3 Teilnehmende“ bzw. „Besprechung erkannt: Microsoft Teams“ mit [Aufnahme starten] [Beitreten] (nur mit Join-URL) [Vorbereiten] (nur Kalender) [Später ▾ (5 min / Nicht für diese App / Nicht für diesen Termin)]. Zustand 2 (Einwilligung, § 201 StGB): vorhandener Text aus dem Einwilligungsdialog + F26-Hinweis zum Kopieren, Häkchen „Alle Teilnehmenden sind einverstanden“ + Systemton-Häkchen (Default aus `meeting_capture_system`) → [Starten]. Toasts (tauri-plugin-notification) scheiden aus: keine Knopf-Rückrufe auf dem Desktop, fehlende AUMID im Dev-Lauf. Schließt sich nach 3 min ohne Aktion.

**F17 Personen.** `humans` (M8, ungenutzt) wird die Personentabelle; Quellen: Teilnehmende (E-Mail normalisiert: klein, getrimmt), Sprechernamen (`speakers.display_name`, M3), manuell. Abgleich: E-Mail exakt → sonst Alias-Name exakt (normalisiert: „Berg, Anna“ → „anna berg“, Diakritika, Mehrfach-Leerzeichen) → sonst neu. Keine unscharfe Automatik (Fehlzusammenführung wäre still); „Zusammenführen“ manuell. Firma = Domain ohne Freemail-Liste (gmail, gmx, web.de, outlook, hotmail, t-online, icloud, yahoo …). `meeting_participants` hält die Zuordnung Besprechung ↔ Person dauerhaft (auch wenn der Termin aus dem Cache fällt). M4: `resolve_scope` ergänzt `person` um `meeting_participants` (Schnittstelle unverändert, M4 §6 Schritt 1).
**F18 Brief.** Kein eigener Prompt: Recipe `builtin:` „Vorbereitung auf das Gespräch mit {{person}}“ (M4 §7) über `chat::ask`, Scope `Global{ meeting_ids = frühere Besprechungen mit ≥ 1 gemeinsamen Teilnehmenden, max. 20 jüngste }`, `{{person}}` = Namen der Teilnehmenden ohne mich. Auslöser nur per Knopf „Vorbereiten“ (Hinweisfenster, „Nächste Termine“), Antwort als Thread (M4-Speicher, `scope_json` mit `event_uid`) → zweiter Klick öffnet den gespeicherten Brief. Keine gemeinsamen Besprechungen → Knopf deaktiviert mit Tooltip.
**F19 Follow-up-Mail.** Recipe „Follow-up-E-Mail an {{empfaenger}}“ (M4) im Scope der Besprechung; Prompt-Ergänzung (in P4c-Builtin): erste Zeile `Betreff: …`, danach Mailtext ohne Überschriften. `mail::draft_from_answer` entfernt Zitatmarken `[n]`, trennt Betreff, baut Text + HTML. Empfänger = Teilnehmende ohne `is_self` (Einstellung „Meine E-Mail-Adressen“ + Graph /me). Dialog: An/Betreff/Text editierbar, [Kopieren] (HTML+Text), [Im Mailprogramm öffnen] (`mailto:`; kodiert > 1 800 Zeichen → nur An+Betreff, Text in die Zwischenablage + Hinweis „einfügen“), [Als .eml speichern] (MIME multipart/alternative, UTF-8, `X-Unsent: 1` → Outlook klassisch öffnet als Entwurf; neues Outlook/Thunderbird öffnen .eml ggf. nur lesend [?]). EML über `mail-builder` (kommt mit calcard, Apache/MIT). Externer LLM-Anbieter: gelbe Leiste wie M4.
**F20 Export.** `ExportBundle { meeting, participants, notes, enhanced_md (P1b `enhanced_to_markdown`), minutes_md, segments, action_items }`, Teile wählbar. Neu: `markdown_to_html` auf Basis des vorhandenen Block-/Span-Parsers (`export.rs:68,111`, eine Quelle für docx/HTML/PDF), `render_meeting_html` (eigenes CSS, KI-Text grau wie Granola), `segments_to_srt/vtt` (Sprecher als Präfix „Ich:“/Name), JSON `lva-meeting-export@1` (alles inkl. Segment-IDs, Quellen, Teilnehmende; Audio nie). PDF: verstecktes WebView2-Fenster, `NavigateToString(html)` → `PrintToPdf(path, A4, Ränder 15 mm, ohne Kopf/Fuß)`, Timeout 20 s; Fehler → Meldung + „Drucken…“ (`window.print()`) als Rückfall. Zwischenablage: `meetings_copy_formatted` → `write_html(html, text)`. Endung bestimmt Format (Muster `ExportFormat::from_path`).
**F21 MCP.** `local-voice-ai.exe --mcp`: Zweig in `main.rs` VOR `run()` → keine Tauri-Initialisierung, kein Webview, kein Single-Instance-Plugin, kein Log auf stdout. Eigenes JSON-RPC 2.0 über Zeilen (stdio), Protokollversionen `2025-06-18` und `2025-11-25` (Antwort = angefragte, falls bekannt, sonst neueste), Methoden `initialize`, `notifications/initialized`, `ping`, `tools/list`, `tools/call`; sonst `-32601`. Gegen rmcp: 4 lesende Tools, ~400 Zeilen, keine neue Abhängigkeit, Protokolltests mit festen Transkripten; rmcp-API wechselte dreimal in einem Jahr. DB `SQLITE_OPEN_READ_ONLY`, `busy_timeout 2000`, keine Migration (Schema-Version > bekannt → Fehler „App aktualisiert, Server neu starten“), nur `deleted_at IS NULL`. Freigabe: Einstellung `meeting_mcp_enabled` (Default aus) aus `settings_store.json` gelesen; aus → jede Tool-Antwort `isError` mit Hinweis. Tools: `list_meetings(from?, to?, person?, folder?, limit≤50)`, `search_meetings(query, limit≤20)` (M4-FTS, sonst LIKE), `get_meeting(id, parts=[ai_notes,notes,minutes,participants,action_items])`, `get_transcript(id, cursor?)` (≤ 40 000 Zeichen je Seite, abschaltbar per `meeting_mcp_include_transcript`), `list_people(query?)` (ab P5d). Kein „Chat“-Tool: der Client ist selbst das LLM. Einstellungszeile zeigt Kopier-Schnipsel für Claude Code (`claude mcp add local-voice -- "<exe>" --mcp`), Claude Desktop (JSON) und Codex (`config.toml`). Update: der NSIS-Installer beendet laufende `local-voice-ai.exe` (auch `--mcp`); der Client verbindet neu.

## 4. Datenmodell – Migration nach M4 (Index 4; falls M3 vorher mergt: 5)

```sql
CREATE TABLE calendar_sources (id TEXT PRIMARY KEY, kind TEXT NOT NULL /*ics|graph*/, label TEXT NOT NULL,
  account_hint TEXT /*Host bzw. UPN, nie die URL*/, enabled INTEGER NOT NULL DEFAULT 1, has_attendee_data INTEGER NOT NULL DEFAULT 0,
  etag TEXT, last_modified TEXT, last_sync_at INTEGER, last_ok_at INTEGER, last_error TEXT,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
CREATE TABLE calendar_events (key TEXT PRIMARY KEY /*source:uid:start_ms*/, source_id TEXT NOT NULL, uid TEXT NOT NULL,
  title TEXT NOT NULL, starts_at INTEGER NOT NULL /*ms UTC*/, ends_at INTEGER NOT NULL, all_day INTEGER NOT NULL DEFAULT 0,
  cancelled INTEGER NOT NULL DEFAULT 0, location TEXT, join_url TEXT, description TEXT /*≤ 4000*/,
  reminded_at INTEGER, dismissed_at INTEGER, fetched_at INTEGER NOT NULL);
CREATE INDEX idx_cal_events_start ON calendar_events(starts_at);
CREATE TABLE calendar_event_attendees (event_key TEXT NOT NULL, email TEXT /*normalisiert*/, name TEXT,
  organizer INTEGER NOT NULL DEFAULT 0, partstat TEXT, PRIMARY KEY (event_key, email, name));
CREATE TABLE meeting_calendar_links (meeting_id TEXT PRIMARY KEY, event_key TEXT, source_id TEXT, uid TEXT NOT NULL,
  event_start INTEGER NOT NULL, event_title TEXT NOT NULL, linked_by TEXT NOT NULL /*prompt|auto|manual*/, created_at INTEGER NOT NULL);
CREATE INDEX idx_links_uid ON meeting_calendar_links(uid);
ALTER TABLE humans ADD COLUMN email_norm TEXT;
ALTER TABLE humans ADD COLUMN company TEXT;
ALTER TABLE humans ADD COLUMN is_self INTEGER NOT NULL DEFAULT 0;
ALTER TABLE humans ADD COLUMN merged_into TEXT;
CREATE UNIQUE INDEX idx_humans_email ON humans(email_norm) WHERE email_norm IS NOT NULL AND deleted_at IS NULL;
CREATE TABLE human_aliases (kind TEXT NOT NULL /*email|name*/, value_norm TEXT NOT NULL, human_id TEXT NOT NULL,
  PRIMARY KEY (kind, value_norm));
CREATE TABLE meeting_participants (meeting_id TEXT NOT NULL, human_id TEXT NOT NULL, role TEXT NOT NULL /*organizer|attendee|speaker*/,
  source TEXT NOT NULL /*calendar|speaker|manual*/, created_at INTEGER NOT NULL, PRIMARY KEY (meeting_id, human_id));
CREATE INDEX idx_participants_human ON meeting_participants(human_id);
```
Nur `CREATE`/`ADD COLUMN`, Altzeilen bytegleich. `soft_delete_meeting` löscht in derselben Transaktion `meeting_calendar_links` und `meeting_participants` der Besprechung (Personen bleiben). Quelle entfernen: Cache-Zeilen + Geheimnis hart löschen, Links behalten ihren Schnappschuss. Cache-Pflege je Sync: Zeilen außerhalb des Fensters löschen. Zusammenführen: `merged_into` setzen, Aliasse und Teilnahmen umhängen (eine Transaktion).

## 5. Schnittstellen

```rust
// calendar/model.rs (Serialize, Deserialize, specta::Type, Clone, Debug)
pub enum CalendarKind { Ics, Graph }
pub struct CalendarSource { pub id: String, pub kind: CalendarKind, pub label: String, pub account_hint: Option<String>, pub enabled: bool,
    pub has_attendee_data: bool, pub last_sync_at: Option<i64>, pub last_ok_at: Option<i64>, pub last_error: Option<String>, pub event_count: u32 }
pub struct Attendee { pub email: Option<String>, pub name: Option<String>, pub organizer: bool, pub is_self: bool, pub partstat: Option<String> }
pub struct CalEvent { pub key: String, pub source_id: String, pub uid: String, pub title: String, pub starts_at: i64, pub ends_at: i64,
    pub all_day: bool, pub cancelled: bool, pub location: Option<String>, pub join_url: Option<String>, pub description: Option<String>, pub attendees: Vec<Attendee> }
// calendar/ics.rs (rein)
pub fn normalize_utc_rrule_starts(raw: &str) -> std::borrow::Cow<'_, str>;
pub fn parse_and_expand(raw: &str, source_id: &str, from_ms: i64, to_ms: i64, default_tz: &str) -> IcsResult; // {events, warnings, has_attendee_data}
pub fn extract_join_url(fields: &[&str]) -> Option<String>;
// calendar/fetch.rs · secret.rs · service.rs
pub enum FetchOutcome { NotModified, Body { text: String, etag: Option<String>, last_modified: Option<String> } }
pub async fn fetch_ics(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<FetchOutcome, CalendarError>;
pub enum CalendarError { Http(u16), NotCalendar, TooLarge, Timeout, Network(String), Auth, Parse(String) }
pub fn secret_put(name: &str, data: &[u8]) -> Result<(), String>; pub fn secret_get(name: &str) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, String>; pub fn secret_delete(name: &str);
pub struct CalendarService; impl CalendarService { pub fn spawn(app: AppHandle, store: Arc<MeetingStore>) -> Arc<Self>; pub fn sync_now(&self, source_id: Option<String>); }
// calendar/reminder.rs (rein)
pub struct ReminderCtx<'a> { pub now_ms: i64, pub lead_ms: i64, pub recording: bool, pub all_events: bool, pub attendee_data: &'a dyn Fn(&str) -> bool }
pub fn due_reminders<'e>(events: &'e [CalEvent], ctx: &ReminderCtx) -> Vec<&'e CalEvent>;
pub fn event_for_start<'e>(events: &'e [CalEvent], now_ms: i64, window_ms: i64 /*15 min*/) -> Option<&'e CalEvent>; // genau einer, sonst None
// meeting_detect
pub struct MicUsage { pub app_key: String /*"np:<pfad#>" | "pkg:<familie>"*/, pub exe_path: Option<std::path::PathBuf>, pub last_start: u64, pub active: bool }
pub trait MicUsageSource: Send { fn snapshot(&mut self) -> Result<Vec<MicUsage>, String>; }
pub enum AppClass { MeetingApp(&'static str), Browser(&'static str), Ignored, Other }
pub fn classify(app_key: &str) -> AppClass;
pub enum DetectMode { Off, MeetingApps, AllApps }
pub struct DetectCtx<'a> { pub mode: DetectMode, pub recording: bool, pub ignored: &'a [String], pub self_keys: &'a [String], pub process_alive: &'a dyn Fn(&std::path::Path) -> bool }
pub enum DetectEvent { Started { app_key: String, label: String }, Ended { app_key: String } }
pub struct Detector; impl Detector { pub fn step(&mut self, now_ms: u64, snap: &[MicUsage], ctx: &DetectCtx) -> Vec<DetectEvent>; }
// people · meetings/mail.rs · export
pub fn normalize_email(s: &str) -> Option<String>; pub fn normalize_name(s: &str) -> String; pub fn company_from_email(e: &str) -> Option<String>;
impl MeetingStore { pub fn upsert_person(&self, email: Option<&str>, name: Option<&str>, source: &str) -> Result<String>;
    pub fn set_participants(&self, meeting_id: &str, rows: &[(String, &str /*role*/, &str /*source*/)]) -> Result<()>;
    pub fn meetings_with_people(&self, human_ids: &[String], limit: u32) -> Result<Vec<String>>; pub fn merge_people(&self, keep: &str, gone: &str) -> Result<()>; }
pub struct MailDraft { pub to: Vec<String>, pub subject: String, pub body_text: String, pub body_html: String }
pub fn draft_from_answer(answer: &str, meeting_title: &str, to: Vec<String>) -> MailDraft;
pub fn mailto_url(d: &MailDraft, max_len: usize) -> (String, bool /*Text in Zwischenablage*/);
pub fn write_eml(d: &MailDraft, date: chrono::DateTime<chrono::Utc>, boundary_seed: u64) -> Vec<u8>; // deterministisch für Golden-Tests
pub struct ExportParts { pub ai_notes: bool, pub notes: bool, pub minutes: bool, pub transcript: bool, pub participants: bool }
pub fn markdown_to_html(md: &str) -> String; pub fn render_meeting_html(b: &ExportBundle, p: &ExportParts) -> String;
pub fn segments_to_srt(segs: &[StoredSegment], label: &dyn Fn(&StoredSegment) -> String) -> String; pub fn export_json(b: &ExportBundle) -> serde_json::Value;
// mcp/protocol.rs
pub fn serve<R: std::io::BufRead, W: std::io::Write>(input: R, output: W, tools: &dyn ToolHost) -> std::io::Result<()>;
pub trait ToolHost { fn list(&self) -> Vec<ToolSpec>; fn call(&self, name: &str, args: &serde_json::Value) -> ToolResult; }
```
Commands (neu `RS/commands/calendar.rs`, `people.rs`; Export/Mail in `commands/meetings.rs` am Dateiende): `calendar_sources_list`, `calendar_source_add_ics(label, url) -> CalendarSource` (Probeabruf vor dem Speichern), `calendar_source_remove(id)`, `calendar_sync_now(id?)`, `calendar_upcoming(hours) -> CalEvent[]`, `calendar_graph_sign_in() -> CalendarSource` (P5f), `meetings_start_from_event(event_key?, app_key?, consent_confirmed, capture_system) -> Meeting`, `meeting_prompt_dismiss(prompt_id, action)`, `meeting_link_event(meeting_id, event_key?)`, `meeting_participants(meeting_id)`, `people_list(query?)`, `people_get(id) -> PersonDetail`, `people_merge(keep, gone)`, `people_update(id, name, email?)`, `meetings_export(meeting_id, path, parts)`, `meetings_copy_formatted(meeting_id, parts)`, `meeting_followup_draft(meeting_id) -> MailDraft`, `meeting_followup_open(draft, mode: copy|mailto|eml, path?)`. Events: `MeetingPromptEvent {show|close}` an Fenster `meeting_prompt`, `CalendarSyncEvent {source_id, ok, count}`. TS-Typen von Hand in `bindings.ts`.
Einstellungen (`settings.rs`, serde-Default, keine Geheimnisse): `meeting_reminder_lead_s: u32 = 60` (0 = aus), `meeting_reminder_all_events: bool = false`, `meeting_detect_mode = MeetingApps`, `meeting_detect_ignored_apps: Vec<String> = []`, `meeting_self_emails: Vec<String> = []`, `meeting_mcp_enabled = false`, `meeting_mcp_include_transcript = true`, `calendar_graph_client_id: Option<String> = None` (None = eingebaute ID).
Headless (cli.rs, nur anhängen): `--calendar-dump <datei|url> --from --to --json`, `--detect-mic --seconds N [--all-apps] --json`, `--export-meeting <id> --format md|txt|docx|html|pdf|srt|vtt|json --out <pfad>`, `--mcp`.

## 6. UI-Skizze (kein neuer Menüpunkt/Reiter)

**Einstellungen → Besprechungen**: Zeile „Kalender“ mit Quellenliste („Outlook · outlook.office365.com · 23 Termine · 10:42 ✓“ / rot mit Fehler) und [Kalender verbinden] → Dialog mit zwei Wegen: „ICS-Adresse einfügen“ (Anleitung Outlook/Google als Aufklapptext) und „Mit Microsoft anmelden“ (ab P5f). Zeilen „Erinnerung vor Terminen“ (Aus/1/2/5 min, Häkchen „auch ohne Teilnehmende“), „Besprechungen erkennen“ (Aus/Meeting-Apps/Alle Apps + „Ignorierte Apps“), „Meine E-Mail-Adressen“, „Lokaler MCP-Server (nur lesend)“ mit Warntext „Dein KI-Client (Claude, Codex) sendet gelesene Besprechungsinhalte an seinen Anbieter“ + Schnipsel.
**Aufnahmen**: RecorderCard – Titelfeld mit Kalender-Vorschlag (laufender/nächster Termin ±15 min vorbelegt, Chip „aus Kalender · 3 Teilnehmende“, ✕ löst). Darunter einklappbare Karte „Nächste Termine (heute)“ mit [Vorbereiten] und [Aufnahme starten] je Termin. MeetingList: Personen-Chip im Filter (P4d-Chips) „Person: Anna Berg“.
**MeetingDetail**: Kopfzeile mit Termin-Chip und Teilnehmenden-Chips (Klick → Popover: E-Mail, Firma, „Besprechungen mit Anna (7)“ → Liste gefiltert, „Fragen“ → ChatPanel Scope Person, „Personen verwalten…“ → Dialog Umbenennen/Zusammenführen). Export-Menü: Word/TXT/MD (wie bisher) + PDF, SRT, VTT, JSON, „Formatiert kopieren“; Teile-Häkchen (KI-Notizen, Meine Notizen, Protokoll, Transkript, Teilnehmende). Knopf „Follow-up-Mail“ → Dialog (§3 F19).
**Hinweisfenster**: §3.

## 7. Fehlerfälle

| Fall | Verhalten |
|---|---|
| ICS 401/403/404/410 (URL widerrufen, Veröffentlichen vom Admin gesperrt) | Quelle rot mit Klartext, Cache bleibt, nächster Versuch im Intervall; kein Fenster |
| Antwort ist HTML/Login-Seite, > 20 MB, Timeout, kein Netz | `NotCalendar`/`TooLarge`/`Timeout`/`Network`; Cache bleibt, „zuletzt aktuell vor 3 h“ |
| Einzelne Komponente kaputt, unbekannte TZID | Komponente übersprungen, Zähler `warnings`; unbekannte TZID → Standardzone + Warnung |
| UTC-Serie (V8), globales Limit (V9), Override ohne Teilnehmende (V7) | Normalisierung, Expansion je UID, Erben vom Master – je ein Regressionstest |
| Sehr alte Tagesserie (> 20 000 Instanzen bis heute) | Warnung „Serie gekürzt“, Serie ohne Instanzen im Fenster statt falscher |
| Gleicher Termin aus ICS und Graph | Dublette per (uid, start) über Quellen, Graph gewinnt |
| Quelle ohne Teilnehmerdaten (z. B. veröffentlichter M365-Kalender [?]) | `has_attendee_data=0` → Erinnerung nicht an ≥ 2 Teilnehmende gebunden; Personen/Brief leer mit Hinweis |
| Ruhezustand/Uhrzeitwechsel | Tick-basiert; verpasste Erinnerungen > 2 min nicht nachholen |
| Registry fehlt/gesperrt (Gruppenrichtlinie) | Detector `Unsupported`, Einstellung zeigt „auf diesem System nicht verfügbar“ |
| `Stop=0`-Leiche nach Absturz | Prozessprüfung (sysinfo), NonPackaged ohne Prozess ignoriert |
| Mehrere Apps gleichzeitig aktiv | ein Hinweis, Vorrang: mit Termin > Meeting-App > Browser |
| Hinweis während Aufnahme/Diktat | unterdrückt; eigene EXE nie gemeldet |
| Graph: Einwilligung verweigert, Mandant sperrt App/Geräte-Code, Token abgelaufen, 429 | Klartext, Neu-Anmelden-Knopf; `Retry-After` beachten; Refresh schlägt fehl → Quelle „Anmeldung nötig“ |
| DPAPI-Datei fehlt/fremder Benutzer | Quelle „Adresse neu eingeben“ |
| PDF: WebView2 alt/Fehler/Timeout 20 s | Meldung + „Drucken…“-Rückfall; verstecktes Fenster wird immer geschlossen |
| mailto zu lang/kein Mailprogramm | Text in Zwischenablage + Hinweis; opener-Fehler → Kopier-Knopf hervorheben |
| MCP: DB fehlt, Schema neuer, gesperrt, Einstellung aus | leere Liste mit Hinweis / Fehler „neu starten“ / `busy_timeout` 2 s / `isError` mit Aktivierungshinweis; nie Schreibzugriff |
| MCP: Ausgabe auf stdout außer JSON-RPC | verboten; Logs nur stderr; Test prüft jede stdout-Zeile als JSON |

## 8. Testplan

- **ICS-Fixtures** `FX/calendar/`: `outlook_series.ics` (Windows-TZID, EXDATE, verschobener + abgesagter Override, Teams-URL, Sommerzeitwechsel), `google_secret.ics` (TZID Europe/Berlin, UTC-Serie, CN = E-Mail, Meet-Link, abgesagter Einzeltermin), `allday_floating.ics`, `broken.ics` (ungültige Zeilen, offene Komponente, unbekannte TZID); `big.ics` wird im Test erzeugt (3 000 Termine + Tagesserie seit 2000 + Wochenserie danach). Erwartete Instanzen als Tabelle (UTC-Zeiten) im Test.
- **Reminder/Detector** rein, mit festen Uhrzeiten und Fake-Snapshots (Zustandsfolgen als Tabelle).
- **Export-Golden-Files** `FX/export/nordlicht.{md,html,srt,vtt,json,eml}`; Aktualisieren nur mit `LVA_UPDATE_GOLDEN=1`; PDF: `%PDF-` + `pdf-extract` findet Titel und „Größe“.
- **MCP-Protokolltest** in-process (`serve` mit `Cursor`), plus Smoke `scripts/mcp_smoke.py` gegen das Release-Binary mit Sandbox-DB (`LVA_MEETINGS_DIR`).
- **Playwright** mit Attrappe (`tests/workspace.spec.ts`-Muster): `meeting-calendar.spec.ts`, `meeting-prompt.spec.ts`, `meeting-export.spec.ts`, `meeting-followup.spec.ts`.
- Bestehende Suiten: `cargo test … --lib meetings::` bleibt grün (Migration über `MIGRATIONS.len()`, nicht feste Zahl).

## 9. Paketschnitt

Aufwand je Coder-Paket nach Ist-Werten 250–310 kTok → 11 Pakete ≈ 3,0–3,4 MTok. Wellen: **W1** P5a ∥ P5c ∥ P6a ∥ P6e → **W2** P5b ∥ P5d ∥ P6b ∥ P6c → **W3** P5e ∥ P6d ∥ P5f.

**P5a – Kalender-Fundament** · lv-coder-xhigh (Migration, Zeitzonen) · L · Abh.: P4a gemergt (Index 3)
Scope: `store.rs` (Migration §4, `soft_delete_meeting`), neu `RS/managers/calendar/{mod,model,ics,fetch,secret}.rs`, Store-Funktionen Cache/Links, Cargo.toml (`calcard = { version = "=0.3.14", default-features = false }`, windows-Feature `Win32_Security_Cryptography`), `cli.rs`+`lib.rs` (`--calendar-dump`, eigener `// M5-P5a`-Block), `FX/calendar/*`.
Akzeptanz: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib calendar::` → ≥ 22 Tests grün, u. a. `ics::tests::utc_dtstart_series_not_shifted`, `…::exdate_removes_instance`, `…::override_moves_and_inherits_attendees`, `…::cancelled_override_flagged`, `…::windows_tzid_dst_switch`, `…::daily_series_does_not_starve_weekly`, `…::all_day_flagged`, `…::html_response_is_not_calendar`, `secret::tests::dpapi_roundtrip`; `--lib meetings::store` grün inkl. `migration_keeps_m4_rows`, `resync_keeps_reminded_at`; `local-voice-ai.exe --calendar-dump apps/local-voice/src-tauri/tests/fixtures/calendar/outlook_series.ics --from 2026-09-28 --to 2026-11-08 --json` → Exit 0, Titel/Teilnehmende/Startzeiten exakt wie Fixture-Tabelle (AK9 Teil 1).

**P5b – Sync-Dienst, Erinnerung, Hinweisfenster, Start aus Termin** · lv-coder · L · Abh.: P5a; P1f (settings/RecorderCard) gemergt
Scope: neu `calendar/{service,reminder}.rs`, `RS/meeting_prompt.rs`, `FE/meeting-prompt/*` + vite-Eintrag, `capabilities/default.json` (Label), neu `RS/commands/calendar.rs`, `meetings_start_from_event` in `commands/meetings.rs`, `settings.rs` (Erinnerungs-Felder), `DictationTab.tsx` (Zeilen Kalender/Erinnerung + Dialog), `RecorderCard.tsx` (Kalender-Vorschlag, Karte „Nächste Termine“), i18n `meetings.calendar.*`, `meetings.prompt.*`.
Akzeptanz: `cargo test … --lib calendar::reminder` → ≥ 12 Tests (1 min vorher, ≥ 2 Teilnehmende, ohne Teilnehmerdaten, ganztägig, abgesagt, schon erinnert, Aufnahme läuft, Ruhezustand +10 min, `event_for_start` eindeutig/mehrdeutig); `pnpm test:playwright -- tests/meeting-calendar.spec.ts tests/meeting-prompt.spec.ts` → grün (ICS-Dialog ruft `calendar_source_add_ics`; Hinweis → Einwilligung → `meetings_start_from_event` mit `consent_confirmed=true`, ohne Häkchen kein Aufruf; Titelvorschlag aus Termin) (AK9 Teil 2).

**P5c – Ad-hoc-Erkennung** · lv-coder · M · Abh.: keine für den Kern; Anbindung ans Hinweisfenster nach P5b (zweiter Commit)
Scope: neu `RS/managers/meeting_detect/{mod,source,catalog,detector}.rs`, Watcher-Thread + Start in `lib.rs` (`// M5-P5c`), `settings.rs` (Modus, Ignorierliste), `cli.rs` (`--detect-mic`), DictationTab-Zeile.
Akzeptanz: `cargo test … --lib meeting_detect` → ≥ 14 Tests (Eigenfilter, 5-s-Entprellung, neue Sitzung = neuer Hinweis, Ignorierliste, Browser-Etikett, webview2 ignoriert, Leiche ohne Prozess, Ende schließt, Aufnahme unterdrückt, Termin ±15 min verknüpft, Vorrang mehrerer Apps); real: Python öffnet Mikrofon (`C:\Users\wolff\lva-spikes\m5\mic_latency.py`-Muster) und parallel `local-voice-ai.exe --detect-mic --seconds 15 --all-apps --json` → enthält `python.exe` als `Started` ≤ 8 s nach Öffnen, eigene EXE nicht (AK9 Teil 3).

**P5d – Personen** · lv-coder · M · Abh.: P5a, P4a (`resolve_scope`), P4d (Filter-Chips), P1d vor MeetingDetail-Änderung
Scope: neu `RS/managers/people/{mod,normalize}.rs`, Store-Funktionen, `resolve_scope`-Erweiterung, Befüllung aus Link/Start (P5b) und `speakers` (M3, falls da), neu `RS/commands/people.rs`, `MeetingDetail.tsx` (Chips/Popover), neu `…/meetings/people/{PersonPopover,PeopleDialog}.tsx`, `MeetingList.tsx` (Personen-Chip), i18n `meetings.people.*`.
Akzeptanz: `cargo test … --lib people` → ≥ 12 Tests („Berg, Anna“ = „Anna Berg“, E-Mail vor Name, Freemail ohne Firma, Zusammenführen hängt Teilnahmen um, gelöschte Besprechung verschwindet); `pnpm test:playwright -- tests/meeting-people.spec.ts` grün.

**P5e – Pre-Meeting-Brief** · lv-coder · S · Abh.: P4c, P4e, P5d
Scope: `people::brief_scope(event) -> ScopeFilter`, Knopf „Vorbereiten“ (Hinweisfenster, Terminkarte) → ChatPanel mit Recipe + Scope, Thread-Wiederverwendung je `event_uid`.
Akzeptanz: `cargo test … --lib people::brief` ≥ 5 Tests (nur gemeinsame Teilnehmende, ich ausgeschlossen, max. 20 jüngste, leer → deaktiviert); Playwright `-g "Vorbereiten"` grün (Aufruf `meeting_chat_ask` mit Recipe-ID und `meeting_ids`).

**P6a – Export-Renderer** · lv-coder · M · Abh.: P1b (`enhanced_to_markdown`; bis dahin Teil `ai_notes` leer)
Scope: `export.rs` (`markdown_to_html`, `render_meeting_html`, `ExportBundle`, JSON), `subtitle.rs` (`segments_to_srt/vtt`), `meetings_export`/`meetings_copy_formatted` in `commands/meetings.rs`, `cli.rs` (`--export-meeting`), `FX/export/*`.
Akzeptanz: `cargo test … --lib meetings::export meetings::subtitle` → grün, ≥ 12 neue Tests inkl. Golden `nordlicht.{md,html,srt,vtt,json}` und `srt_roundtrip_via_parse_subtitles`; `local-voice-ai.exe --export-meeting <id> --format json --out %TEMP%\m.json` (Sandbox aus P1e-Fixture) → Exit 0, `format == "lva-meeting-export@1"` (AK10 Teil 1).

**P6b – PDF über WebView2** · lv-coder-xhigh (COM-Rückrufe, Hauptthread) · M · Abh.: P6a
Scope: neu `RS/managers/meetings/pdf.rs` (verstecktes Fenster, `NavigateToString`, `PrintToPdf`, Timeout, Aufräumen), Cargo.toml (`webview2-com = "0.38"` nur `cfg(windows)`, gleiche Version wie Lock), Anbindung `--export-meeting --format pdf`. Erster Schritt: Nachweis, dass `PrintToPdf` im unsichtbaren Fenster liefert; sonst Stopp + Bericht (Rückfall Druckdialog).
Akzeptanz: `local-voice-ai.exe --export-meeting <id> --format pdf --out %TEMP%\m.pdf` → Exit 0, Datei beginnt mit `%PDF-`; `cargo test … --lib meetings::pdf -- --ignored` (braucht WebView2) → `pdf-extract` findet Titel und „Größe“; zweimal hintereinander ohne verwaistes Fenster (AK10 Teil 2).

**P6c – Follow-up-Mail** · lv-coder · M · Abh.: P4c (Recipe-Prompt „Betreff:“), P6a; Empfänger aus P5d optional
Scope: neu `RS/managers/meetings/mail.rs` (mail-builder), Commands `meeting_followup_draft/open`, `settings.rs` (`meeting_self_emails`), neu `…/meetings/FollowupDialog.tsx`, i18n `meetings.followup.*`.
Akzeptanz: `cargo test … --lib meetings::mail` → ≥ 10 Tests (Zitatmarken entfernt, Betreff-Zeile, ohne Betreff → „Nachbereitung: <Titel>“, mailto-Kodierung Umlaute/`&`, Kürzung > 1 800 → Zwischenablage, EML-Golden mit `X-Unsent: 1`, RFC-2047-Betreff); `pnpm test:playwright -- tests/meeting-followup.spec.ts` grün (AK10 Teil 3).

**P6d – Export-Oberfläche** · lv-coder · S · Abh.: P6a, P6b, P1d
Scope: `MeetingDetail.tsx`/`MinutesView.tsx` Export-Menü + Teile-Häkchen, Zwischenablage über `meetings_copy_formatted`, i18n.
Akzeptanz: `pnpm test:playwright -- tests/meeting-export.spec.ts` grün (Endungen → `meetings_export` mit Pfad, Teile-Häkchen, „Formatiert kopieren“ ruft den Command) (AK10 Teil 4).

**P6e – Lokaler MCP-Server** · lv-coder-xhigh (Datenschutz-Gate, Protokoll) · M · Abh.: P4a (FTS; sonst LIKE), P5d optional für `list_people`
Scope: neu `RS/mcp/{mod,protocol,tools}.rs`, `main.rs` (früher Zweig `--mcp`), `cli.rs`, Store: `open_read_only(path)` ohne Migration/Seed, `settings.rs` (2 Felder), DictationTab-Zeile mit Schnipseln, neu `scripts/mcp_smoke.py`.
Akzeptanz: `cargo test … --lib mcp::` → ≥ 16 Tests (initialize-Versionen, tools/list-Schemas, -32601/-32700, Notification ohne Antwort, Einstellung aus → isError, Schreibversuch scheitert an READ_ONLY, gelöschte unsichtbar, Seitenbildung Transkript, jede stdout-Zeile gültiges JSON); `python scripts/mcp_smoke.py apps/local-voice/src-tauri/target/release/local-voice-ai.exe` mit Sandbox → Exit 0 (initialize, tools/list, search, get_meeting); danach Patrick: `claude mcp add` + eine Frage.

**P5f – Microsoft Graph** · lv-coder-xhigh (Auth, Geheimnisse) · M · Abh.: P5a, P5b, Client-ID von Patrick (E14)
Scope: neu `calendar/graph.rs` (PKCE, Loopback-Listener 127.0.0.1, Tokenaustausch/-erneuerung, calendarView-Paging, /me), Dialogweg „Mit Microsoft anmelden“, `calendar_graph_client_id`.
Akzeptanz: `cargo test … --lib calendar::graph` → ≥ 10 Tests gegen lokalen Test-HTTP-Server (PKCE-Challenge S256, `state`-Prüfung, Code-Austausch, Refresh, 401 → Anmeldung nötig, 429 Retry-After, nextLink-Paging, UTC-Zeiten, isCancelled); manuell Patrick: Anmelden → Termine der nächsten 7 Tage erscheinen.

## 10. Konfliktstellen mit laufenden Paketen

| Datei | Andere Pakete | Regel |
|---|---|---|
| `store.rs` MIGRATIONS, `soft_delete_meeting` | P4a (Index 3, läuft), M3 (evtl. Migration) | P5a erst nach P4a-Merge; nimmt nächsten freien Index; Tests über `MIGRATIONS.len()` |
| `lib.rs` (Commands, Start, headless, Wächter) | P1b–P1f, P2a–P2e, P4a–P4f | je Paket eigener `// M5-P5x`/`// M6-P6x`-Block, seriell mergen |
| `main.rs` | – | nur P6e (früher `--mcp`-Zweig) |
| `cli.rs` | P1e, P2b2, P4a, P4b, P4f | Flags nur anhängen |
| `settings.rs`, `DictationTab.tsx` | P1f, P4b, P4e | nach P1f; Felder additiv mit serde-Default; Zeilen ans Gruppenende |
| `RecorderCard.tsx` | P1c (fertig), P1f | P5b nach P1f |
| `MeetingDetail.tsx`, `MinutesView.tsx` | P1d, P4e | P5d/P6d nach P1d und P4e |
| `MeetingList.tsx` | P4d, P4e | P5d nach P4e |
| `commands/meetings.rs` | P4b (Indexer-Zeilen) | neue Commands am Dateiende |
| `export.rs`, `subtitle.rs` | – | nur P6a |
| `chat/recipes.rs` (Builtin Follow-up) | P4c | Betreff-Vorgabe im P4c-Briefing ergänzen; sonst P6c ändert nur den Builtin-Text |
| `Cargo.toml` | P2c1 (sonora), P2a, P2f (Vulkan) | P5a/P6b je eine Zeile, `windows`-Feature ans Listenende |
| `capabilities/default.json`, `vite.config.ts` | – | nur P5b |
| `bindings.ts`, `commands/mod.rs`, i18n de/en | alle UI-Pakete | von Hand, nur eigene Typen/Schlüssel (`meetings.calendar.*`, `.prompt.*`, `.people.*`, `.export.*`, `.followup.*`, `.mcp.*`) |

## 11. Owner-Entscheidungen (Vorschlag) und Risiken

- **E10 Erkennung** Standard „Meeting-Apps“ (inkl. Browser), nur Hinweis, nie Auto-Start. Vorschlag: ja.
- **E11 Erinnerung** 1 min vorher nur bei ≥ 2 Teilnehmenden oder Join-URL (Granola-Regel), abschaltbar. Vorschlag: ja.
- **E12 Geheimnisse** (ICS-URL, Graph-Token) DPAPI-verschlüsselt in App-Daten, nie im Einstellungs-Sync. Vorschlag: ja.
- **E13 MCP** standardmäßig aus; beim Einschalten Warnung (Inhalte gehen an den Anbieter des KI-Clients; Aussagen Dritter, § 201 StGB/DSGVO); Transkript-Tool abschaltbar. Vorschlag: ja.
- **E14 Graph-Client-ID**: Patrick registriert in Entra „Local Voice AI“ (Mobil/Desktop, Umleitung `http://localhost`, öffentliche Client-Flows = Ja, `Calendars.Read`, `offline_access`, `User.Read`, Konten: beliebige Organisation + privat). Ohne Herausgeber-Verifizierung verlangen fremde Mandanten ggf. Admin-Zustimmung; Feld für eigene Client-ID unter „erweitert“. Vorschlag: ja, P5f erst nach AK9.
- **E15 PDF** über WebView2 (Windows); macOS vorerst Druckdialog. Vorschlag: ja.
- **E16 Briefs** nur per Knopf, kein nächtlicher LLM-Lauf (Ressourcen, AK5). Vorschlag: ja.
- **E17 Massenexport** (Auswahl → ZIP aus MD+JSON) nicht in M6; Nachfolgepaket bei Bedarf. Vorschlag: ja.
- Risiko calcard jung (0.3.x, Fehler V8/V9 gefunden): Version festnageln, Regressionstests je Befund, Rückweg `icalendar`+`rrule` bleibt hinter `parse_and_expand` gekapselt.
- Risiko veröffentlichte M365-Kalender ohne Teilnehmende [?]: mit Patricks echter ICS-URL prüfen (P5a-Bericht); dann trägt erst Graph (P5f) F17/F18 für Outlook. Aktualität veröffentlichter Kalender laut Recherche 3–24 h [?].
- Risiko Firmen-Proxy: reqwest nutzt nur Proxy-Umgebungsvariablen, nicht den Windows-Systemproxy → Fehlermeldung nennt es.
- Risiko Hinweisfenster stört (Fokus, Vollbild-Präsentation): `focused(false)`, 3-min-Auto-Schließen, „Nicht für diese App“.
- Offen: Zoom-/Webex-Klassik-Pfade und Slack-Paketname nicht auf dieser Maschine geprüft → Katalog im P5c-Bericht mit Quelle je Eintrag.
