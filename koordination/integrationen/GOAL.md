---
thema: integrationen
titel: Integrationen: Register fuer Kalender, Mail, Speicher, Wissen und Agenten (MCP/CLI lesend+schreibend)
state: EXECUTING
vorzustand: -
pausengrund: -
issue: 66
repo: MrP42/local-voice-ai
branch: feat/integrationen
iteration: 1
erstellt: 2026-09-30
aktualisiert: 2026-09-30T23:07
---

# Goal: Integrationen: Register fuer Kalender, Mail, Speicher, Wissen und Agenten (MCP/CLI lesend+schreibend)

## Zielzustand
Ein YouTube-Link lässt sich in „Aufnahmen“ einfügen und wird zur Quelle: Video in der App ansehen, Untertitel (falls
verfügbar) und eigene Transkription nebeneinander vergleichen, eine Fassung wählen oder per KI zusammenführen und
zusammenfassen — und zu jedem erzeugten Inhalt zeigt ein Rechtsklick „Herkunft“ Modell, Token, Dauer, Zeitpunkt,
Quellen, Konfidenz und Auslöser. Danach verwaltet eine neue Seite „Integrationen“ zwischen „Modelle“ und
„Einstellungen“ beliebig viele Verbindungen (YouTube, Kalender, Postfächer, Ordner/OneDrive, Obsidian-Vault,
WAI-Wissensbasis, Agentenzugänge) mit Richtung und einem Recht je Fähigkeit (aus / fragen / erlaubt), und externe
Agenten steuern die App über MCP und `local-voice-ai.exe ctl` nur im Rahmen dieser Rechte, jede Aktion im Audit-Log.

## Scope
**Bündel 1 — zuerst lieferbar (A1–A3):**
- Fundament: Register-Kern (Datenmodell, Übernahme der `calendar_sources`, Rechte, Audit) und **Provenienz**
  (`provenance`-Tabelle, Verweis auf `usage_event`, Rückfall auf `generation_metadata_json`).
- YouTube-Quelle: Link (Video) in Aufnahmen einfügen → Besprechung mit Quelle `youtube`, Metadaten (Titel, Kanal,
  Vorschaubild, Dauer), Ansehen in der App im eingebetteten YouTube-Player; Audio-/Dateiweg je Owner-Entscheidung E1.
- Untertitel (manuell/automatisch, Sprachwahl) laden, sofern der entschiedene Weg das erlaubt; unabhängig davon
  eigene Transkription; **Vergleichsansicht** (Wort-Diff), Fassung wählen oder **KI-Zusammenführung** (Schema-gebunden,
  mit Provenienz); Zusammenfassung über den vorhandenen KI-Notizen-/Protokollpfad.
- Kontextmenü „Herkunft“ an Transkript, KI-Notizen, Protokoll, Zusammenfassung (Dialog).

**Bündel 2 (A4–A8):**
- Seite „Integrationen“: Liste, Katalog mit Assistent je Art, Detail (Richtung, Fähigkeiten-Matrix, Test, Protokoll),
  Freigabedialog; Umzug von Kalender-UI und MCP-Schalter.
- Microsoft-365-Konto (Kalender lesen/schreiben, Mail senden, OneDrive), SMTP-Postfach (App-Passwort), Ordner
  (lokal/OneDrive-Sync), Obsidian-Vault (AI-OS-Frontmatter), WAI-Wissensbasis (MCP-Client, suchen/lesen).
- Agentensteuerung: Named Pipe zur laufenden App, Client-Token, Rechte je Werkzeug; MCP schreibend (Aufnahme
  starten/stoppen, Datei transkribieren, Session/Besprechung anlegen, Vorlesen-Seite + Audio, YouTube-Link als Quelle
  anlegen) und `ctl`-CLI.

## Non-Scope
- **Playlist** (Folgeschritt, eigenes Paket A9 nach Bündel 1, nicht im Budget).
- Werbung im eingebetteten Player blockieren oder verändern (Developer Policies, BGH I ZR 131/23).
- Bündeln von yt-dlp/Deno im Installer (siehe E1).
- Workflow-Engine (Goal B), lokaler LLM-Agent (Goal C), Google-OAuth, IMAP-Lesen, `wissen:write` im AI-OS-Repo,
  Webhook- und „eigener MCP-Server“-Integration (nach B verschoben), macOS-Kanal, DLP-Klassen.

## Akzeptanzkriterien
- [x] AK1 — Fundament: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib integrations:: provenance::` → ≥ 30 Tests grün, u. a. Migrationstest (Fixture mit 2 `calendar_sources` → 2 Integrationen gleicher ID, zweiter Start ohne Dubletten) und Provenienz: Protokoll-Erzeugung legt einen Eintrag mit Modell, Token, Dauer, `usage_event_id` an; alte Dokumente liefern Herkunft aus `generation_metadata_json`.
- [x] AK2 — YouTube-Quelle: Playwright `youtube-source.spec.ts` → Link `https://www.youtube.com/watch?v=…` (auch `youtu.be/…`, `shorts/…`) in Aufnahmen einfügen erzeugt eine Besprechung mit Quelle „YouTube“, Titel und Kanal; ungültige/Playlist-Links → verständliche Meldung; Rust-Test für Link-Normalisierung (≥ 10 Fälle).
- [ ] AK3 — Ansehen: Installer, echtes Video → Player in der Inhaltsspalte spielt ab, Position springt beim Klick auf ein Transkript-Segment; kein verschachteltes iframe, keine Veränderung der Werbung (Code-Review-Punkt).
- [x] AK4 — Untertitel + eigene Transkription: für ein Video mit Untertiteln liegen beide Fassungen an derselben Besprechung (Quelle je Fassung sichtbar); ohne Untertitel → nur eigene Fassung mit Hinweis; Sprachauswahl bei mehreren Spuren (Rust-Tests gegen Fixture-VTT, manuell 1 Video).
- [x] AK5 — Vergleich/Zusammenführen: Playwright → Diff-Ansicht markiert Einfügungen/Löschungen wortweise; „Fassung wählen“ setzt das aktive Transkript; „Zusammenführen“ erzeugt eine dritte Fassung mit Provenienz (Quellen = beide Fassungen, Modell, Token); Rust-Test: Zusammenführung verwirft Ausgaben, die das Schema verletzen oder > 20 % Text erfinden (Längen-/Überdeckungsprüfung).
- [x] AK6 — Zusammenfassung + Herkunft: Zusammenfassung eines YouTube-Videos wird erzeugt; Rechtsklick „Herkunft“ auf Transkript, Zusammenfassung und Protokoll öffnet Dialog mit Modell, Token, Dauer, Zeitpunkt, Quellen, Konfidenz (falls vorhanden) und Auslöser (Playwright).
- [ ] AK7 — Seite Integrationen: Playwright `integrations.spec.ts` → Eintrag zwischen „Modelle“ und „Einstellungen“; Katalog ≥ 7 Arten; Ordner-Integration anlegen, Richtung/Fähigkeitsmodus ändern übersteht Neuladen; Kalenderquellen als Karten; MCP-Schalter unter Einstellungen > Besprechungen durch Verweis ersetzt, `meeting_mcp_enabled` wirkt unverändert.
- [ ] AK8 — Konten und Ziele: `--lib integrations::m365 integrations::smtp integrations::folder integrations::obsidian integrations::wissen` → ≥ 30 Tests gegen Test-Server/Sandbox (Scopes nur für eingeschaltete Fähigkeiten, `sendMail`, OneDrive-Upload klein und per Upload-Session, 401→Refresh; SMTP an Test-Server; Pfad-Sandbox gegen `..`/Junction; Vault-Notiz mit Frontmatter-Golden; `wissen_suchen` mit Scope-Fehler-Meldung); manuell Patrick: Testmail, Datei in OneDrive, Suche in der Wissensbasis.
- [ ] AK9 — Agentenbrücke: `--lib agent_bridge::` → ≥ 15 Tests (Pipe nur aktueller Benutzer, Remote abgewiesen, Token ungültig/zurückgezogen → abgelehnt + Audit, „aus“ → Werkzeug fehlt in `tools/list`, „fragen“ → Freigabe, 30 s ohne Antwort → `pending` + ID).
- [ ] AK10 — MCP/CLI schreibend: `python apps/local-voice/scripts/mcp_smoke.py --write` gegen Release-Binary + laufende Sandbox-App → Exit 0 (`transcribe_file` → `meeting_id`; `tts_page_create` + `tts_render_audio` → WAV; `add_youtube_source` → Besprechung; `start_recording` ohne Einwilligung in der App startet nie eine Aufnahme); `ctl status --json` Exit 0, ohne App Exit 2, Werkzeug „aus“ Exit 3.
- [ ] AK11 — Audit: alle Aktionen aus AK8–AK10 in `--audit-dump --json` und in der UI; Aufbewahrung gedeckelt (Test).
- [ ] AK12 — Anfassbar: nach Bündel 1 Installer (Patch +1) mit YouTube-Ablauf und Screenshots (Player, Diff, Herkunft) — **erste Abnahme durch Patrick**; nach Bündel 2 Installer + Screenshots (Liste, Katalog, Rechte-Matrix, Freigabe, Audit).

## Quality Gates
- [ ] QG1 — `cargo test --lib` gesamt grün (Sandbox, `CARGO_BUILD_JOBS=8`); neue Dateien ohne neue Clippy-Warnungen.
- [ ] QG2 — `npx tsc --noEmit` Exit 0; Playwright-Suite grün; eslint/prettier nur berührte Dateien.
- [ ] QG3 — i18n de + en, echte Umlaute.
- [ ] QG4 — Externes Sicherheitsreview für Rechte, Pipe, Token, Freigabe (A1, A7, A8) und für den YouTube-Dateiweg (Prozessaufruf, Pfade).
- [ ] QG5 — Systemschutz: jeder Kindprozess (z. B. externes yt-dlp, falls E1 so entschieden) über `process_guard`; Modellstarts über das RAM-Gate; Aufruf-Obergrenzen je Agent-Client.
- [ ] QG6 — Doku: `docs/INTEGRATIONEN.md` (YouTube inkl. Rechtshinweis, Rechte, Agentenzugang mit `claude mcp add`-Beispiel), Hilfe-Texte, Handoff.
- [ ] QG7 — Budget 2,7 MTok (Spanne 2,3–3,5); Meldung bei 50 % und 80 %, harter Stopp bei 150 %.

## Constraints
- Start erst nach Granola-Goal (#59) Runde 3 und Aufnahmen-Oberfläche (#64) bzw. deren Meilensteinen laut Abhängigkeiten.
- Lokal, ohne Abo; Systemschutz (process_guard, RAM-Gate); Einwilligungsdialog vor jeder Aufnahme (§ 201 StGB).
- Branch + PR, keine Formatierläufe über fremde Dateien (AGENTS.md).

## Architekturprinzipien
Siehe `vorschlag.md` → Architektur-Skizze (Module, Datenmodell, Rechte).

## Dependencies
- **aufnahmen-ui** (läuft): A2 braucht das Spaltengerüst (M2), A8 die Sessions (M3). A1 ist reines Backend und kann
  sofort parallel starten.
- **Goal B** braucht A1 (Grants, Audit, Approvals, Provenienz), A3 (YouTube-Transkript/Zusammenfassung als Aktionen),
  A5/A6 (Mail, Ordner, Vault, Wissen), A7 (Agenten).
- **Goal C** braucht A1 (Aufrufer `agent_local`, Provenienz mit Konfidenz) und A6 (Vault/Wissen).
- Basis: `feat/granola-besprechungen` (P5 Kalender, P6a/b Export, P6c Mail, P6e MCP, M7 KI-Notizen).

## Risiken / Owner-Entscheidungen
- R1 **Rechtsrisiko YouTube-Download** (OLG Hamburg 5 U 54/23: Rolling Cipher = wirksame Schutzmaßnahme; ToS verbietet
  Download) → E1; Standard ist der ToS-konforme Player; kein Bündeln von yt-dlp; vor Verteilung an Dritte anwaltlich
  prüfen.
- R2 **Brüchigkeit yt-dlp** (PO-Token, SABR, Deno-Pflicht) → nur als extern installiertes Werkzeug mit Versionsanzeige,
  Fehler klar melden („Werkzeug veraltet – bitte aktualisieren“), nie still scheitern.
- R3 Loopback-Mitschnitt bei Weg A enthält Werbung und läuft in Echtzeit → Werbeabschnitte bleiben im Rohaudio;
  Hinweis in der UI; Untertitel dann nicht verfügbar (API nur für eigene Videos).
- R4 KI-Zusammenführung erfindet Text → Schema, Überdeckungsprüfung gegen beide Fassungen, Provenienz, Diff zur Kontrolle (AK5).
- R5 Konflikt mit aufnahmen-ui (Inhaltsspalte, Sessions, Sidebar) → A2 nach aufnahmen-ui M2, A8 nach M3; Player als
  eigenständige Komponente in der Inhaltsspalte.
- R6 Migration Kalenderquellen → 1:1-ID, `calendar_sources` bleibt, Backup, Migrationstest.
- R7 Graph-Scopes fehlen in der Entra-Registrierung → E5 vor A5.
- R8 Prompt-Injection über Video-/Besprechungsinhalte an Agenten → Schreibendes „fragen“, Freigabe in der App, keine
  freien Empfänger.
- R9 MCP-Spec 2026-07-28 (`_meta` je Anfrage) → Versionsverhandlung mit Claude Code und Codex in A8 testen.

**Owner-Entscheidungen (Patrick, offen):**
- **E1 YouTube-Weg** — Optionen: (A) nur eingebetteter Player, ToS-konform, mit Werbung, Transkript per Loopback-Mitschnitt
  in Echtzeit; (B′) zusätzlich ein **von dir selbst installiertes** yt-dlp (+ Deno) als externes Werkzeug, Pfad in der
  YouTube-Integration, Schalter „privat/experimentell“, Standard aus, nicht im Installer gebündelt → werbefrei lokal
  ansehen, schnell transkribieren, Untertitel; (C) yt-dlp bündeln. **Empfehlung: A als Standard, B′ nur für deine
  eigene Nutzung hinter dem Schalter; C nein.** Vor Weitergabe der App an Dritte: Rechtsprüfung.
- E2 Register und Provenienz in `meetings.db` (gleiche Migrationskette, MCP liest mit) — **Empfehlung: ja**.
- E3 Standardrechte: schreibend „fragen“, externe Agenten „aus“, Aufnahme nie ohne Einwilligungsdialog — **Empfehlung: so**.
- E4 Wissen in den RAG über den Vault statt neuem `wissen:write`-Endpunkt — **Empfehlung: Vault jetzt**.
- E5 Entra-App um `Calendars.ReadWrite`, `Mail.Send`, `Files.ReadWrite` erweitern — **Empfehlung: vor A5**.
- E6 Kalender- und MCP-Einstellungen ziehen auf die neue Seite (alter Ort nur Verweis) — **Empfehlung: ja**.
- E7 Datenklasse für Vault-Notizen aus Besprechungen/Videos: `confidential` (Besprechungen), `internal` (öffentliche
  Videos) — **Empfehlung: so**.

## Meilensteine
Pakete und Bündel: `vorschlag.md` → Paketschnitt; Budget: 8 Pakete × ~275 kTok = 2,2 MTok + Reviews/Nacharbeit ~20 % → **2,7 MTok** (Spanne 2,3–3,5), unverändert gegenüber dem ersten Entwurf; dafür wurden Webhook und „eigener MCP-Server“ nach Goal B verschoben und SMTP/Ordner/Obsidian/Wissen in ein Paket (A6) gelegt. Bündel 1 allein: ≈ 1,0 MTok. A9 Playlist: +0,25 MTok, falls gewünscht.

## Evidence
- 2026-09-30T20:15 AK1 erfüllt — A1 20f0e763: cargo test --lib -- integrations:: provenance:: 153 passed; Migrationstest idempotent; Protokoll-Provenienz mit usage_event_id
- 2026-09-30T22:10 AK2 erfüllt — A2: youtube-source 20 passed, Link-Normalisierung >40 Faelle, echter oEmbed-Abruf
- 2026-09-30T23:07 AK4 erfüllt — A3 2f0c7642: VTT-Fixture-Tests, echter yt-dlp-Lauf 4 Spuren
- 2026-09-30T23:07 AK5 erfüllt — A3 2f0c7642: youtube-versions Diff/Fassung waehlen/Zusammenfuehren, Merge-Schutz-Tests
- 2026-09-30T23:07 AK6 erfüllt — A3 2f0c7642: Herkunft-Dialog an Transkript/KI-Notizen/Protokoll, Protokoll auf YouTube-Besprechung

## Blocker
-

## Entscheidungen
- 2026-09-30 Patrick: gemeinsamer Rahmen Aufnahmen-UI + YouTube-Bündel auf 4,3 MTok angehoben (80-%-Meldung bei ~2,75).
- 2026-09-30 Patrick: alle Owner-Entscheidungen wie in vorschlag.md empfohlen: E1 YouTube = offizieller eingebetteter Player als Standard, yt-dlp nur als selbst installiertes Werkzeug hinter Schalter „privat“ (Standard aus, nicht im Installer); E2 Register + Provenienz in meetings.db; E3 schreibende Fähigkeiten „fragen“, externe Agenten „aus“, Aufnahme nie ohne Einwilligungsdialog. Jetzt umsetzen: Bündel 1 (A1–A3, ~1,0 MTok) im gemeinsamen Rahmen 3,5 MTok mit #64; A1 parallel zu #64-M1, A2/A3 nach #64-M2.
- 2026-09-30 Patrick: neue Seite „Integrationen“ zwischen Modelle und Einstellungen; Register ohne Mengengrenze, Richtung/Rechte je Verbindung, RAG + Obsidian als Schwerpunkt, MCP/CLI auch steuernd. Erste Integration: YouTube (#65) inkl. Werbefrei-Wiedergabe, Untertitel vs. Transkript, Zusammenführen, Zusammenfassung; Herkunft/Audit (Modell, Tokens, Quellen, Konfidenz) per Rechtsklick.

## Nächste empfohlene Aktion
Owner-Entscheidungen von Patrick einholen, dann `goal.py set --state READY`.

## Verlauf
- 2026-09-30T17:01 DISCOVERY — Goal State angelegt
- 2026-09-30T17:25 DISCOVERY (Runde 0) — Metadaten: issue=66
- 2026-09-30T19:27 READY (Runde 0) — Owner-Entscheidungen 30.09. eingetragen, Rahmen 3,5 MTok gemeinsam
- 2026-09-30T19:28 PLANNING (Runde 1) — Pakete geplant
- 2026-09-30T19:28 EXECUTING (Runde 1) — U1 (wt-aui) und A1 (wt-int) laufen parallel; Rahmen 3,5 MTok gemeinsam

