# Handoff — Local Voice AI: Vorlesen als eine Seite, Kontexthilfe, Multi-Device-Sync (E5–E7)

Datum: 2026-09-14 (fortgeschrieben bis abends, Stand 0.17.15) · Session: Claude Code (VS Code,
Auto-Mode) · Projekt: `C:\Users\wolff\local-voice-project`

## Stand beim Handoff (Kurzfassung für den Einstieg)

- Zweig `feat/vorlesen-eine-seite` (Basis: PR-24-Zweig), alles gepusht, PR #25 mit
  Nachträgen je Version als Kommentare. Letzter Commit: „fix(tts): Stimme unter der
  Transportzeile statt darin". Installer **0.17.15 war beim Handoff im Bau**
  (`apps/local-voice/scripts/dev.ps1 bundle`); 0.17.14 liegt fertig. Signatur-Fehler am
  Build-Ende ist normal (kein privater Updater-Schlüssel lokal).
- Versionskette heute: 0.17.0 (PR 24) → 0.17.1 … 0.17.15, je Abnahmestand ein Installer unter
  `apps/local-voice/src-tauri/target/release/bundle/nsis/`. Details je Version weiter unten.
- Patricks letzte Richtung: Vorlesen-UI feinschleifen (zweispaltig, Bedienspalte rechts,
  Tempo als Wert in der Transportzeile). Screenshot-Prüfung über den Playwright-Test „all five
  content pages share one head" → `apps/local-voice/test-results/page-tts.png` und
  `page-tts-narrow.png`; nach jeder UI-Änderung anschauen (Read-Tool), das hat zwei
  Layoutfehler sofort gezeigt.
- Arbeitsregel, die sich bewährt hat: Patch-Skripte mit Nicht-ASCII als Datei in den
  Scratchpad schreiben und per `python <datei>` ausführen (Bash-Heredoc bricht sonst);
  während `tauri build` keine App-Quellen anfassen (Vite bündelt am Anfang, Rust danach).

## Laufender Faden

Patricks Brief vom 14.09. („Vollgas, nicht anhalten"): Reife der Kernfunktionen, einheitliche
Inhaltsseiten, Vorlesen als Vollbild-Arbeitsfläche ohne Klappen, Stimmwechsler raus, Verlauf
links und Dateien rechts mit Funktion, Kontexthilfe, Multi-Device mit Login. Zerlegung und
Entscheidungen: `docs/ROADMAP.md` (Nachtrag 14.09. + „Stand 14.09. abends"). Alles umgesetzt
bis auf die Punkte unter „Bei Patrick".

**Zweige und PRs (nichts gemergt, alles gepusht):**
- PR #24 `feat/sprachmodelle-l1-anbieter` → main: Sprachmodelle L1–L4 + Ledger, Version 0.17.0,
  enthält PR #23 und #21 (beide geschlossen). Merge vom Klassifikator blockiert → Patrick.
  Danach `git tag app-v0.17.0` auf main.
- PR #25 `feat/vorlesen-eine-seite` → Basis = Zweig von #24 (nach dessen Merge auf main
  umstellen): E5 (7 Commits), E7, E6-Client, Versionen 0.17.1 (E5/E7) und 0.17.2 (Sync).
- wai-portal Zweig `feat/voice-sync` (`C:\Users\wolff\OneDrive - Ingenieurbüro Wolff\
  Selbständigkeit\wai-portal`): **UNCOMMITTED** (Commit vom Klassifikator blockiert), 6 Dateien:
  `app/Controllers/Api/VoiceSyncController.php`, `app/Services/VoiceSyncService.php`,
  `app/Services/ApiTokenService.php` (Scope `voice.sync`), `config/routes.php`,
  `database/migrations/0024_voice_sync.php`, `tests/cases/55_voice_sync_test.php`.
  Suite 697/697. Achtung: `ApiTokenService.php` trägt zusätzlich fremde uncommitted
  Training-Studio-Scopes — beim Commit nur die eigenen Hunks nehmen (`git add -p`).

**Installer (Abnahmestände):** `apps/local-voice/src-tauri/target/release/bundle/nsis/`
`Local Voice AI_0.17.1_x64-setup.exe` (E5/E7), `…0.17.2…` (mit Sync) und `…0.17.3…`
(dazu Qwen 3.5 4B/9B und Gemma 4 E4B/12B im Sprachmodell-Katalog, Speicherprognose kennt
`qwen35`/`gemma4`). Updater-Signatur fehlt lokal wie immer (kein privater Schlüssel) — normal.
Warum die beiden vorher fehlten: der Katalog war am 12.09. von Hand mit vier Einträgen
befüllt; GGUFs (unsloth) und llama.cpp b10938 (13.09.) tragen beide Familien längst.
`…0.17.4…`: Piper-Stimmen direkt im Stimmen-Dropdown der Vorlesen-Seite („Name · Piper",
Wert `piper:<id>`, schaltet `tts_engine`/`tts_piper_voice`; Fish-Stimme schaltet zurück).
`…0.17.5…`: Piper spricht keine `<Marker>` und keine `[Tags]` mehr — `utterances()` streicht
alle Marker, wenn `caps.voice_switching` fehlt; `fetch_wav` streicht alle Tag-Spans, wenn
`caps.style_tags` fehlt (`protocol::strip_speaker_markers` / `strip_tag_spans`). Offen
bleibt: mit Fish werden UNBEKANNTE `<Name>`-Marker weiterhin vorgelesen (nur bekannte
Stimmen schalten) — Entscheidung, ob unbekannte Marker auch dort verschwinden sollen.
`…0.17.6…`: Piper wählt die Stimme nach der Sprache des Satzes (`managers/tts/lang.rs`,
Stoppwörter de/en/fr/es/it; `PiperEngine::resolve_auto` + `paths_for`; Setting
`tts_piper_auto_language`, Standard an, Schalter im Reiter Vorlesen bei Engine Piper).
Grenze: nur Sprachen mit geladener Piper-Stimme; unklare Sätze (Eigennamen, Zahlen) nimmt
die gewählte Stimme.
`…0.17.7…`: fünf englische Piper-Stimmen im Katalog (Lessac HQ, Ryan HQ, Amy MQ, Alan GB,
Alba GB; Katalog-Test zählt jetzt 10). Dropdown-Label „Name · Sprache · HQ/MQ/LQ · Piper"
(`piperVoiceLabel` in TtsSettings, Sprache über `Intl.DisplayNames`). Unbekannte
`<Marker>` werden bei JEDER Engine gestrichen (`process_speaker_chunk`) — die Frage von
0.17.5 ist damit entschieden.
`…0.17.8…`: Stimme je Reiter (Original/Übersetzung/Zusammenfassung) im Seitenstand
(`state.json.voices`, Werte wie im Dropdown); `applyVoiceValue`/`voiceValue` in TtsSettings,
Effekt beim Reiterwechsel. 0.17.7 wurde gebaut, während dieser Commit entstand — deshalb 0.17.8.
`…0.17.9…`: Stimmenverwaltung (`VoiceLibrary`) jetzt unter **Einstellungen → Vorlesen**
statt Modelle-Seite (Entscheidung Patrick 14.09. abends, revidiert gegenüber Mittag).
Dropdown endet mit „Stimmen verwalten …" → `lv-navigate`-Ereignis (App.tsx) + localStorage
`lva.ui.settings.tab = readaloud`. Engine- und Piper-Stimmen-Auswahl im Reiter entfernt
(Dropdown übernimmt), Schalter „Sprache automatisch erkennen" bleibt, jetzt immer sichtbar.
Kontexthilfe entsprechend angepasst.

## Was gebaut wurde (Kurz, Details in den Commits)

- **E5:** `VoiceChangerCard`/`ReadingCard` weg (Rust-Pfade bleiben), Seed+Stimmen als
  `settings/tts/voices/VoiceLibrary.tsx` am Ende der Modelle-Seite, `PageShell` als Rahmen
  aller fünf Seiten, Seitenliste mit `modified_ms`/`preview` aus `pages_list`
  (`commands/pages.rs`, bindings von Hand), Editor `rows=14`. Screenshot-Test schreibt
  `test-results/page-*.png`.
- **E7:** `components/help/HelpPanel.tsx` + `src/content/help/vorlesen.{de,en}.md`,
  Reiter „Hilfe" in `FilesSidebar`, Knopf im Seitenkopf.
- **E6:** Spec `docs/superpowers/specs/2026-09-14-multi-device-sync-design.md` (Abweichung
  zur Spec: AAD bindet Benutzer/Sammlung/Objekt, NICHT die Revision — einfacher, im Code
  dokumentiert). Rust `src-tauri/src/sync/{crypto,account,ledger,collect,client,engine,mod}.rs`,
  Commands `sync_login/logout/status/now/touch/default_device_name/hub_status`, State
  `SyncEngine`, Schleife 60 s + Anstoß. UI `settings/SyncAccountCard.tsx` im Reiter Allgemein.
  Konto-Datei `<appdata>/sync.json`, Ledger `sync_ledger.json`, Hub-Default
  `https://portal.wolffappliedai.de`.

## Bei Patrick (in dieser Reihenfolge)

1. **Portal deployen + Migration** (Klassifikator blockiert Deploy-Skripte). Im wai-portal-Ordner,
   PowerShell 7, `WAI_SFTP_PASS` aus `$HOME\.wai-deploy\secrets.env`:
   ```
   .\deploy-portal.ps1 -Files app/Controllers/Api/VoiceSyncController.php,app/Services/VoiceSyncService.php,app/Services/ApiTokenService.php,config/routes.php,database/migrations/0024_voice_sync.php
   # SSH (Posh-SSH wie in tools\deploy-bewerbungen-hub.ps1): cd ~/portal && /usr/local/bin/php8.3 bin/console.php migrate
   # Smoke: POST https://portal.wolffappliedai.de/api/v1/voice/auth/login mit falschem Passwort -> 401 invalid_credentials
   ```
   Vorher prüfen, ob `ApiTokenService.php` auf dem Server die Training-Scopes schon hat (sonst
   nimmt der Upload die fremden Zeilen mit — additiv, aber nicht meins).
2. **Portal-Commit** (nur eigene Hunks) auf `feat/voice-sync`.
3. **Erster echter Login** in der App (0.17.2): Einstellungen → Allgemein → Konto & Geräte,
   Portal-Konto. Dann Seite anlegen, auf dem Mac anmelden, Seite erscheint → Abnahme-Artefakt
   (Screenshots + `voice/sync/status`).
4. PR #24 mergen, Tag `app-v0.17.0`; dann PR #25 auf main umstellen, mergen, Tag `app-v0.18.0`
   (Releases bündeln).

`…0.17.10…` (abends): **Codex-Review der Sync-Logik eingearbeitet** (12 Befunde, 7 hoch;
Ausgabe: Scratchpad `codex_review_sync.out` der Session, Kern im Commit `fix(sync): Haertung`):
Konfliktkopie am frischen Stand unter `pages::lock()`, `write_atomic` für Index/Seitenstand/
Ledger, `DeadEntry{reason,hash}`, Index-Lesefehler ≠ leer, `sync.json` 0600/icacls,
`check_hub_url`, `account_guard` bei Login/Logout, Push-Batches ≤ 400 KiB, Pull mit
Fortschrittszwang. NICHT umgesetzt (Stufe 2): Umschlüsselung bei Passwortwechsel (Befund 8),
`superseded` ohne `current` bleibt „Basis 0, neu senden" (Befund 11). Internes Review der
Vorlesen-/Piper-Änderungen: keine Befunde. Kontexthilfe jetzt auf allen fünf Seiten
(`PageShell help=…`, Texte `src/content/help/{verlauf,aufnahmen,modelle,einstellungen}`).

`…0.17.11…`: Knopf „Text aufbereiten" (Sparkles, Original-Reiter neben dem Mikrofon) →
`tts_tidy_text` → `summarizer::tidy` (blockweise, kein Reduce, < 60 % Restlänge = Fehler),
Rückgängig im Toast. Zusammenfassung: `language_clause` nennt die erkannte Sprache im
Prompt („write in German") — Ursache für „er fasst die Übersetzung zusammen" war ein
kleines Modell, das auf den englischen Prompt englisch antwortete; die Quelle war immer
das Original (`summarize` nutzt `text`). Tooltip sagt das jetzt.

`…0.17.12…`: **Vorlesen zweispaltig** (Entscheidung Patrick 14.09. abends): `PageShell fill`,
`.workspace-main--fill` (kein Gesamtscroll), Editor-Spalte `.tts-editor` mit
`.tts-editor__fill` (textarea 100 %), Bedienspalte `.tts-controls` (w-72, scrollt selbst) mit
Transport, Tempo, Stimme, Speichern, aktueller Satz, Ausdruck & Sprechstil, Schreibregeln.
Unter 1100 px (`@media` in App.css): Spalten übereinander, Seite scrollt als Ganzes, textarea
`height:auto`. Screenshots: `test-results/page-tts.png`, `page-tts-narrow.png`.

`…0.17.13…`: „Ausdruck & Sprechstil" (TagPalette + AutoTagBar) unter dem Textfenster in der
Editor-Spalte; Aktionen (+ Hinzufügen, Diktieren, Text aufbereiten / Übersetzen / Zusammenfassen
mit Optionen) und „Audio speichern" als beschriftete, volle Knöpfe in `.tts-controls`.
Neue Kurz-Labels `tts.add.short`, `tts.translateShort`.
`…0.17.14…`: AutoTagBar (Knopf, Anbieter, Gerät) in `.tts-controls__autotag` unter „Text
aufbereiten", per CSS gestapelt; die Klappe unter dem Text trägt nur noch die Palette.
`…0.17.15…`: Tempo als `.mbtn--text`-Chip hinter dem letzten Trenner der Transportzeile
(`speedOpen`, Listbox mit `SPEEDS`); Stimme/Optionsfelder `w-full`; Zusammenfassen-Knopf
nach den Optionen; `AutoTagBar showSettings={false}` in der Spalte, Anbieter/Gerät als
`SettingContainer` im Reiter Vorlesen (`DEFAULT_TAG_PROVIDER_UI_VALUE` exportiert).

## Release 0.18.0 — Stand 14.09. abends (bei Patrick)

Patrick hat entschieden: der aktuelle Stand geht als Release raus. Version ist auf 0.18.0
gesetzt (`43e6af2` auf `feat/vorlesen-eine-seite`, gepusht). Ein Tag `app-v0.17.0` entfällt,
alles wird in `app-v0.18.0` gebündelt. Der Klassifikator blockiert `gh pr merge` UND
`gh pr edit --base` — deshalb bleiben diese Schritte bei Patrick. Stand HEAD ist gegen
`origin/main` konfliktfrei (`git merge-tree` geprüft), `main` hat keinen Branch-Schutz.

Release-Notes: `.claude/handoffs/2026-09-14-release-notes-0.18.0.md`.

Ablauf (in dieser Reihenfolge, aus dem Repo-Wurzelverzeichnis):

```
gh pr edit 25 --base main
gh pr merge 25 --merge
git checkout main && git pull --ff-only
git tag app-v0.18.0 && git push origin app-v0.18.0
```

PR #24 und #23 gelten nach dem Merge von #25 als enthalten (GitHub markiert sie als
gemergt oder sie lassen sich schließen). PR #21 (Strg-Fix) ist NICHT enthalten und
bleibt offen.

Wenn der Workflow `release-windows.yml` am Release-Erstellen scheitert (Actions-Token darf
keine Releases anlegen, siehe Memory):

```
gh release create app-v0.18.0 --title "Local Voice AI 0.18.0" --notes-file .claude/handoffs/2026-09-14-release-notes-0.18.0.md
gh run list --workflow release-windows.yml -L 1
gh run rerun <run-id>
```

Danach wie gehabt: Portal `feat/voice-sync` committen, deployen, Migration 0024, erster
Login PC↔Mac.

## Diktat-Startlatenz — PR #27 (15.09., Zweig `fix/diktat-startlatenz`, Version 0.18.1)

Patricks Auftrag (Video Everlast AI: 600 ms durch PowerShell-Berechtigungsprüfung): hier gibt
es keine PowerShell-Prüfung. Belegte Ursache: On-Demand + `lazy_stream_close=false` → jedes
Diktat Kaltstart; WASAPI/USB-Mikro liefert erst 190–630 ms nach play() das erste Sample;
Overlay/Ton kamen vorher. Fix: lazy_stream_close Standard an (Schema 3, einmalig), Idle 5 min,
Bereit-Signal (`with_capture_ready_callback`) → Overlay/Ton erst bei Audio, Log
`capture ready N after request`. Messung (Hardware-Test `capture_latency_cold_vs_warm`,
`cargo test capture_latency -- --ignored --nocapture`): kalt 428–580 ms → warm 9–10 ms.
Bericht: `docs/latenz-diktatstart-2026-09-15.md` (inkl. Codex-Zweitanalyse, gleicher Befund).
Nebenbei: Piper-Katalogtest 5 → 10 Stimmen repariert (`a48eba4`). PR #27 offen, Merge bei
Patrick (Freigabe für `gh pr merge` liegt in settings.local.json — nur auf Zuruf nutzen).
Installer 0.18.1 lokal unter `apps/local-voice/src-tauri/target/release/bundle/nsis/`.

## Endtext verschluckt — PR #28 (15.09., Zweig `fix/diktat-endtext` auf #27, Version 0.18.2)

Patricks Frage: „Warum wird der Text manchmal am Ende abgeschnitten?" Belegt im Log: der
Abschluss-Textrest wird 80–130 ms nach dem Stopp-Druck getippt, das Release von Strg+Win
kommt erst danach; Chromium-Ziele (VS Code) verwerfen Zeichen mit gehaltenem Strg.
Fix: `input::wait_for_modifiers_released` (GetAsyncKeyState, max 1,5 s) vor jedem Fragment
(`refinement/injection.rs::paste_fragment`) und vor `clipboard::paste`. Zweiter Befund:
`start_stream` setzte den Einfügezeiger vor der Worker-Prüfung zurück → Schnell-Neustart
während der Finalisierung hätte das vorige Diktat doppelt getippt; Reihenfolge getauscht.
Hypothese-Status: Timing belegt, das Verschlucken selbst nicht am Zielprogramm reproduziert —
Patrick prüft mit 0.18.2 (Log-Zeile `waited … for modifier keys`).

Offen (Design vorgeschlagen, Entscheidung Patrick): **Fortsetzungsfenster** — Stopp finalisiert
den Stream nicht sofort, sondern hält ihn ~8 s offen; ein Neustart im Fenster füttert denselben
Stream weiter, das Modell setzt den Satz nahtlos fort (keine Großschreibung/Punkt-Bruch).
Berührt Stopp-Pfad (`actions.rs::stop`), History je Lauf, Overlay-Zustand „pausiert".
Satzbruch bei Denkpause INNERHALB eines Diktats ist Modellverhalten (Nemotron setzt
Interpunktion nach Pause); Hebel wäre die Satz-Verfeinerung (`refine_enabled`, aktuell aus).

## Nachmittag 15.09.: Diktat-Audio (PR #30), lokale Updates (PR #31), Sync-Konzept

PR-Kette (in dieser Reihenfolge mergen): #27 Startlatenz → #28 Endtext → #30 Diktat-Audio →
#31 lokale Updates. Jeder Stand hat einen Installer: 0.18.1 … 0.18.4 unter
`apps/local-voice/src-tauri/target/release/bundle/nsis/`.

- **Diktat-Audio (0.18.3):** Setting `dictation_audio` off|mute|duck|pause, Standard duck 10 %
  (`dictation_audio_duck_percent`), Schema 4 migriert `mute_while_recording`. Windows: Endpunkt-
  lautstärke + WinRT Media.Control (Pause/Fortsetzen aller spielenden Sitzungen); macOS/Linux
  Lautstärke per osascript/wpctl/pactl, Pause → Mute. Reiter Mikrofon & Töne, Komponente
  `DictationAudio.tsx`. Cargo-Features Media_Control/Foundation/Foundation_Collections.
- **Lokale Updates (0.18.4):** `local_update.rs` + Setting `local_update_dir`; Fußleiste zeigt
  „Update X.Y.Z lokal verfügbar", Klick startet Installer `/P` und beendet die App. Hintergrund:
  GitHub-Updater ohne Signaturschlüssel liefert nie etwas. Patrick: Ordner in Einstellungen →
  Allgemein → Updates auf `…\target\release\bundle\nsis` setzen.
- **Sync ohne Portal:** Konzept `docs/superpowers/specs/2026-09-15-sync-ohne-portal-konzept.md`
  (Speicher-Trait; Ordner-Backend zuerst, dann GitHub-Repo per Device Flow, Google Drive später;
  Sync-Passphrase statt Portal-Passwort). Entscheidung bei Patrick, nichts gebaut.
- **Offen, Entscheidung Patrick:** Fortsetzungsfenster (Stream ~8 s offen halten, Neustart setzt
  Satz nahtlos fort), siehe Abschnitt Endtext.

## Offen / nächste Schritte (Code)

- Fußleisten-Symbol für den Sync-Zustand (Spec Abschnitt 7) — noch nicht gebaut.
- Sync Stufe 2: Umschlüsselung bei Passwortwechsel, Auth-Geheimnis statt Klartext-Passwort.
- `TtsSettings.tsx` weiter zerlegen (1700 Zeilen; Editor/Player/Server-Dialoge).
- Stufe 2 Sync: Sprecher-Registry + Klonstimmen, Umschlüsselung bei Passwortwechsel,
  Auth-Geheimnis statt Klartext-Passwort, 2FA, Registrierung.
- Vorbestehend rot, nicht angefasst: prettier, clippy `approx_constant`, `cargo deny`
  (LGPL `mp3lame-*`), `check:translations` (tsx fehlt, ~306 tts.*-Keys in 22 Sprachen).

## Gotchas dieser Session

- **Bash-Tool: Heredocs mit Nicht-ASCII (—, …, ü) brechen mit „unexpected EOF"** → Skripte
  per Write-Tool in den Scratchpad schreiben und `python datei.py` aufrufen.
- Auto-Mode-Klassifikator blockierte: `gh pr merge`, Commit und Datei-Write im wai-portal
  (teils), Deploy-Skript-Write. Nicht umgehen — an Patrick geben.
- Playwright-Mocks: jeder neue Tauri-Command, den eine Seite beim Rendern ruft, muss im Mock
  etwas Sinnvolles liefern (ModelsSettings stürzte an `null.length`; `sync_status` → null
  wird jetzt in der Karte abgefangen).
- `pages_list` erzeugt bei leerem Index eine erste Seite — Tests, die den Index mocken,
  liefern `[]`, und das ist ok.
- Build-Regel eingehalten: während `tauri build` keine App-Quellen anfassen (Docs/Portal ok).

## Empfohlene Skills für die nächste Session

- `superpowers:systematic-debugging`, falls der erste Login/Abgleich hakt (Log:
  `sync::` Meldungen, `sync.json`, `sync_ledger.json`).
- `superpowers:requesting-code-review` (Codex) für `sync/crypto.rs`, `engine.rs`,
  `VoiceSyncService.php` — Gate-/Sicherheitslogik, Review lohnt.
- `superpowers:finishing-a-development-branch` für PR #24/#25.
