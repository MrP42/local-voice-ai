# Handoff — Local Voice AI: Vorlesen als eine Seite, Kontexthilfe, Multi-Device-Sync (E5–E7)

Datum: 2026-09-14 · Session: Claude Code (VS Code, Auto-Mode) · Projekt: `C:\Users\wolff\local-voice-project`

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

## Offen / nächste Schritte (Code)

- Fußleisten-Symbol für den Sync-Zustand (Spec Abschnitt 7) — noch nicht gebaut.
- Codex-Review der Schlüssel-, Konto- und Konfliktlogik (Spec Abschnitt 10, Schritt 5).
- Hilfetexte für Diktat/Verlauf/Aufnahmen/Modelle/Einstellungen (Rahmen steht, nur Vorlesen
  hat Inhalt).
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
