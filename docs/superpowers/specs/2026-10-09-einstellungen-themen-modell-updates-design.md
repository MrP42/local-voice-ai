# Einstellungen nach Thema, Sprung aus der Fußleiste, Modell-Updates (Entwurf 09.10.2026)

Auftrag Patrick: Der Reiter „KI-Textverbesserung" enthält Modellwahl, Verbindungen und
Verbrauch und heißt falsch. Die Einstellungen sollen nicht nach Funktionsmodulen sortiert
sein. Aus der Fußleiste soll man schnell zu den Anbieter-Einstellungen kommen. Neue Modelle
der Anbieter sollen erkannt und auf Wunsch automatisch übernommen werden.

Aufbau: drei Teile, je ein PR, aufgesetzt auf die Kette #76–#81 (Version 0.21.10).
Kein Tag vor der Abnahme.

## Teil 1 — Gliederung nach Thema

Reiter: **Eingabe · Ausgabe · KI-Modelle & Anbieter · App · Über** (+ Debug nur bei Debug-Modus).

| Reiter (id) | Inhalt |
|---|---|
| Eingabe (`input`) | DictationTab-Inhalt (Kürzel, Erkennung, Einfügen, Besprechungs-Link, Diktat-Test), Mikrofon (aus SoundTab), Kürzel „mit Textverbesserung diktieren", Textverbesserungs-Prompts |
| Ausgabe (`output`) | ReadAloudTab, Töne und Signale (aus SoundTab) |
| KI-Modelle & Anbieter (`models`) | ComplianceSettings, aktives Modell, Standard-Effort, LlmConnectionsSettings, UsageOverview, Schalter „Neue Modelle" (Teil 3) |
| App (`app`) | AppTab unverändert |
| Über (`about`) | unverändert |

- Reine Verschiebung von Komponenten; SoundTab und `PostProcessingSettings` werden aufgeteilt.
- Gespeicherte Reiter-ID (`settings.tab`) mit alter ID → Rückfall auf `input` (bestehender `isTabId`-Schutz).
- Übersetzungen: neue Schlüssel `settings.app.tabs.{input,output,models}` in de/en; übrige Sprachen fallen auf en zurück.
- `apps/local-voice/AGENTS.md` (Tabelle „Where a new setting goes") und der Kommentar in
  `AppSettings.tsx` werden angepasst.

## Teil 2 — Sprung aus der Fußleiste

- `footer/LlmSelector.tsx`: letzter Eintrag der Liste „Anbieter-Einstellungen …" und derselbe
  Eintrag im Rechtsklick-Menü des Modellfelds.
- Aktion: `settings.tab` auf `models` setzen, Einstellungen öffnen, zu „Verbindungen" scrollen.
- Beide Wege nutzen eine gemeinsame Funktion (`openProviderSettings`).

## Teil 3 — Neue Modelle erkennen

**Zeitpunkt:** Start der App (hinter dem Update-Check, im Hintergrund), danach einmal täglich
solange die App läuft. Netzfehler und fehlende CLI bleiben still (Log-Zeile).

**Quellen je Verbindung:**
- Codex-Abo: Katalog `~/.codex/models_cache.json` (vorhanden, `cli.rs`).
- API-Anbieter / Ollama: `llm_list_remote_models` (vorhanden).
- Claude-Abo (kein Katalog): Kandidaten ausprobieren. Aus bekannten vollen Namen Nachfolger
  ableiten (`claude-haiku-5-5` → `5-6`, `opus` analog, Muster `claude-<familie>-<major>-<minor>`),
  je Kandidat ein `claude -p --model … --effort low`-Aufruf mit Minimalprompt. Antwortet die CLI,
  ist das Modell verfügbar. Ergebnis je Kandidat gespeichert (nicht erneut prüfen; Fehlschläge
  frühestens nach 7 Tagen wieder). Die feste Liste `CLAUDE_MODELS` bleibt Grundstock.

**„Neu":** angeboten von der Verbindung, aber nicht in `llm_models`.

**Ersetzen:** Gleiche Familie (Haiku/Sonnet/Opus/Fable, bei GPT der Namensstamm ohne Version
etwa `-sol`/`-luna`) und höhere Version → ersetzt das ältere Modell: älteres entfällt aus der
Liste; war es aktiv, wird das neue aktiv; Effort und Fast-Wahl werden übernommen. Ohne
Familienpartner: Modell wird freigegeben und als „neu" gekennzeichnet, ohne Ersatz.

**Einstellung:** `llm_auto_update_models` ∈ `ask` (Voreinstellung) | `on` | `off`.
- `ask`: beim ersten Fund Dialog „X ist neu und ersetzt Y. Künftig automatisch übernehmen?" mit
  **Ja, immer** (→ `on`) · **Nur dieses Mal** (bleibt `ask`) · **Nein, ich wähle selbst** (→ `off`).
- `on`: ohne Rückfrage ersetzen, Hinweis-Toast.
- `off`: keine Ersetzung, neue Modelle erscheinen nur in den Verbindungen mit Kennzeichnung „neu".
- Dreifachauswahl im Reiter „KI-Modelle & Anbieter".

**Regelwerk:** Ein vom Compliance-Regelwerk gesperrtes neues Modell wird nie aktiv
(gleiche Prüfung wie `llm_set_active_model`).

**Bausteine:** Rust `managers/llm/updates.rs` (Erkennung, Familien-/Versionsvergleich,
Ersetzen, reine Funktionen testbar), Befehle `llm_check_new_models`, `llm_apply_model_update`,
Einstellungs-Befehl; Frontend: Start-Hook, Dialog, Schalter. Claude-Aufruf hinter einem
Trait/Funktionszeiger für Tests ersetzbar.

## Tests

- Rust: Familien-/Versionserkennung, Kandidatenableitung, Ersetzen (aktiv/inaktiv, Effort übernommen,
  gesperrt), Wiederholsperre. Claude-Aufruf per Attrappe.
- Playwright: neue Reiter und Inhalte, alter gespeicherter Reiter, Sprung aus Fußleiste (Liste und
  Rechtsklick), Dialog mit drei Antworten, Schalter.
- Build nur auf `D:\lv-build`, nur betroffene Tests + tsc; Installer in den App-Update-Ordner.
- Nicht angefasst: vorbestehendes Prettier-/Clippy-Rot.

## Nicht Teil dieses Vorhabens

Neue Sidebar-Einträge, Änderungen an Integrationen, Modell-Preise/-Limits aus dem Netz,
Kostenfunktionen.

## Festlegungen aus dem Codex-Review (09.10.2026, gpt-6-astra)

Teil 1/2:
- Deep-Link `TtsSettings.tsx` („Stimmen verwalten") schreibt `readaloud` → wird `output`. Alte IDs werden einmalig
  abgebildet: `dictation`→`input`, `readaloud`/`sound`→`output`, `postprocessing`→`models`.
- Der Sprung aus der Fußleiste ist ein Ereignis (`lv-open-settings-tab`, Detail `{tab, anchor}`), das die
  Seite wechselt UND den Reiter reaktiv setzt, danach zum Anker scrollt; `usePersistentState` allein reicht
  bei schon geöffneten Einstellungen nicht.
- Mitzuziehen: Playwright-Selektoren (`llm-connections.spec.ts`, `llm-local.spec.ts` u. a.) und
  `src/content/help/einstellungen.*.md`.

Teil 3:
- **Probing nur für aktive, nicht gesperrte Verbindungen** (`enabled` und Compliance ≠ Blocked), auch bei
  Codex/API/Ollama-Abfragen.
- **Claude-Probe isoliert** wie der bestehende CLI-Aufruf (kein Nutzerkontext/keine Hooks), Kandidaten
  höchstens 3 je Start, Einzel-Timeout 30 s, Gesamtbudget 60 s, Abbruch beim ersten Limit-/Auth-Fehler,
  Start-Prüfung verzögert (nach Fensteranzeige, Hintergrund). Kandidatenmuster
  `claude-<familie>-<major>-<minor>[-<datum>]`: Nachfolger = Minor+1 und Major+1 mit Minor 0/5; Datumssuffix
  wird bei Nachfolgern weggelassen.
- **Ersetzen migriert ALLE Verweise** atomar in einem `write_settings`: `llm_active_model_id`,
  `post_process_models`-Spiegel (`sync_legacy_from_llm`), `tts_tag_model`, weitere Felder, die `llm_models`-IDs
  halten (vor Umsetzung per Suche nach `connection_id:remote_id`-Verwendungen vollständig auflisten).
- **Preise:** Das neue Modell erbt keine Altpreise. Fehlen Preise, ist es als „Preis unbekannt" markiert und
  zählt für Budgets mit dem Preis des ersetzten Modells als Obergrenze (nie 0).
- **Laufende Aufrufe:** Verbrauchsbuchung nutzt den beim Aufrufstart bestimmten Modell-Snapshot; Ersetzen
  wartet nicht, entfernt aber die alte Konfiguration erst, wenn keine Buchung mehr auf sie zeigt (Alias-Eintrag
  `replaced_by` bleibt in `llm_model_history`).
- **Erkennungsverlauf:** `llm_model_history` (persistiert): je `connection_id:remote_id` Status
  `seen`/`replaced`/`dismissed`. Nur nie gesehene Modelle sind „neu"; ersetzte oder bewusst entfernte tauchen
  nicht erneut auf.
