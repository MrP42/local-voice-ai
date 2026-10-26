# TTS-Anbieter: Kommandozeile und Automatisierung

Stand 26.10.2026 · Goal `tts-provider` (Paket D2). Quelle der Wahrheit ist der Code; Fundstellen `Datei:Zeile` beziehen sich
auf diesen Stand. **A/** = `apps/local-voice/src-tauri/src/`. Einbindung der Schicht: [adapter-contract.md](adapter-contract.md), §11.

**Grundsatz:** Kommandozeile und Agentenbrücke können Anbieter **lesen, prüfen, messen und wählen**, aber **nie freigeben**.
Opt-in, Lizenz-Zustimmung und Fernzugriff setzt ausschließlich der Nutzer in der App (Einstellungen > Ausgabe >
Sprachausgabe-Anbieter).

## 1. Kommandozeile

Alle Betriebsarten laufen headless (kein Fenster) über das App-Programm (`local-voice-ai.exe`); der Einstieg ist
`run_provider_modes` (A/tts_cli.rs:590, aufgerufen aus A/lib.rs:2683). Es darf **genau eine** Betriebsart gewählt werden;
`--tts-providers --tts-bench` o. ä. endet mit Exit 2 (A/tts_cli.rs:593-599). Die Schalter sind in A/cli.rs:548-628 definiert.

| Schalter | Wirkung |
|---|---|
| `--tts-providers` | alle Anbieter auflisten: Stufe, Fähigkeiten, Lizenz, Gesundheit (4 s Frist je Anbieter), Freigabestand aus den App-Einstellungen (`enabled`, Lizenz bestätigt, Fernzugriff, gesperrt und warum). Nur lesend. |
| `--tts-voices` | Stimmen von `--tts-provider` (optional gefiltert mit `--tts-lang`); gilt als Nutzung, ein nicht freigegebener Anbieter ergibt Exit 3. Frist 10 s. |
| `--tts-provider <id>` | Anbieter für `--tts-voices`, `--tts-test`, `--tts-bench`: `fish`, `piper`, `phonon`, `openai-speech`, `ollama-speech`. |
| `--tts-test` | Selbsttest. **Ohne** `--tts-provider` unverändert der bisherige Fish-Selbsttest; **mit** `--tts-provider` läuft er durch diesen Anbieter (A/tts_cli.rs:738). Dazu `--tts-text`, `--tts-voice`, `--tts-lang`, `--tts-out-wav <datei>`. Frist 120 s. |
| `--tts-bench` | Benchmark eines Anbieters (Schnittstelle zu `scripts/bench/tts`, [README](../../scripts/bench/tts/README.md)). Braucht `--tts-provider`, `--bench-corpus <jsonl>`, `--bench-out <ordner>`. |
| `--tts-lang <code>` | Filter bei `--tts-voices`; Sprache bei `--tts-test` (Vorgabe: aus dem Text erkannt) und `--tts-bench` (Vorgabe `de`). |
| `--tts-voice <id>` | Stimme dieses Anbieters für den Lauf (Piper-Stimmenkennung bzw. Stimmenname von phonon, openai-speech, ollama-speech; bei Fish die Referenzstimme). |
| `--tts-exe <pfad>` | Programm für `phonon` (nur für diesen Lauf, nie gespeichert). |
| `--tts-model <name>` | Modell für `phonon`, `openai-speech`, `ollama-speech`. |
| `--tts-base-url <url>` | Serveradresse für `openai-speech`, `ollama-speech`. |
| `--bench-subset smoke\|full` | Teilmenge des Korpus (Vorgabe `full`; `smoke` = als Smoke markierte Sätze). |
| `--bench-warm <n>` / `--bench-cold <n>` | warme (Vorgabe 3) und kalte (Vorgabe 1) Synthesen je Satz. |
| `--bench-out <ordner>` | Ausgabe: `rows.jsonl`, `env.json`, `wav/<utterance_id>.wav`. |
| `--json`, `--out <datei>` | bestehende Schalter: `--json` schreibt die Nutzdaten als JSON auf stdout (Liste als Array, eine Zeile je Anbieter), `--out` zusätzlich/allein in eine Datei. |

Ein Schalter, den ein Anbieter nicht kennt, ist Exit 2 mit Klartext (`--tts-base-url` für phonon, `--tts-exe` für die Server,
alle drei für `fish`/`piper`; A/tts_cli.rs:209-257).

### Konfigurationsvorrang

Argument > Umgebungsvariable `LVA_TTS_<ANBIETER>_<FELD>` > Einstellungen > Vorgabe (A/tts_cli.rs:296, Regeln
P/policy.rs:9-29, P = `managers/tts/providers/`). Überschreibbar sind nur Programmpfad, Modell, Adresse und Stimme:
`LVA_TTS_PHONON_EXE_PATH`, `LVA_TTS_PHONON_MODEL`, `LVA_TTS_PHONON_VOICE`, `LVA_TTS_OPENAI_SPEECH_BASE_URL`,
`LVA_TTS_OPENAI_SPEECH_MODEL`, `LVA_TTS_OPENAI_SPEECH_VOICE`, `LVA_TTS_OLLAMA_SPEECH_BASE_URL`,
`LVA_TTS_OLLAMA_SPEECH_MODEL`, `LVA_TTS_OLLAMA_SPEECH_VOICE`. Die Einstellungen werden **nie geschrieben** (Test
`der_plan_aendert_die_einstellungen_nie`, A/tts_cli.rs:1059). Eine überschriebene Adresse außerhalb dieses Rechners bleibt
gesperrt, solange die App den Fernzugriff nicht erlaubt hat (`check_use` sieht die wirksame Adresse; Test :1011).

### Exit-Codes (A/tts_cli.rs:20-27, 68-73)

| Code | Bedeutung |
|---|---|
| 0 | fertig (Benchmark: Zeilen geschrieben, auch bei einzelnen Fehlschlägen) |
| 1 | Fehler: Anbieter nicht bereit, Datei-/Ordnerfehler, Synthese gescheitert, Benchmark ohne eine geschriebene Zeile |
| 2 | falsche Eingabe: unbekannter Anbieter (die Meldung nennt die bekannten), fehlendes oder unpassendes Argument, Korpusfehler, mehrere Betriebsarten, `bad_request`/`invalid_config` |
| 3 | gesperrt: Opt-in, Lizenz oder Fernzugriff fehlt in der App (`license_blocked`, `privacy_blocked`) |
| 4 | Benchmark abgebrochen nach fünf Fehlschlägen in Folge (die Zeilen bleiben) |
| 130 | Benchmark mit Strg+C beendet (Zeilen bleiben, ein selbst gestarteter Fish-Server wird beendet; Aufräumen höchstens 15 s, Windows) |

Die Abbildung der Fehlerklassen steht in `exit_code_for` (A/tts_cli.rs:124): `LicenseBlocked`/`PrivacyBlocked` → 3,
`BadRequest`/`InvalidConfig` → 2, alles andere → 1. Die Meldung bei Exit 3 nennt den Grund und sagt, dass die Freigabe nur
die App erteilt (`blocked_message`, A/tts_cli.rs:133).

### Beispiele

```
# Übersicht, maschinenlesbar
local-voice-ai.exe --tts-providers --json --out anbieter.json

# Stimmen eines Anbieters, nur Deutsch
local-voice-ai.exe --tts-voices --tts-provider piper --tts-lang de

# Selbsttest durch einen Anbieter, WAV als Beleg
local-voice-ai.exe --tts-test --tts-provider piper --tts-text "Guten Tag." --tts-out-wav probe.wav

# Phonon mit anderem Programm nur für diesen Lauf
local-voice-ai.exe --tts-test --tts-provider phonon --tts-exe D:\tools\ptts.exe --tts-voice Zoey

# Generischer Server per Umgebung (PowerShell)
$env:LVA_TTS_OPENAI_SPEECH_BASE_URL = "http://127.0.0.1:8880/v1"
local-voice-ai.exe --tts-voices --tts-provider openai-speech

# Benchmark: Smoke-Teilmenge, 3 warme + 1 kalter Lauf je Satz
local-voice-ai.exe --tts-bench --tts-provider piper --bench-corpus scripts\bench\tts\corpus\de-v1.jsonl `
    --bench-subset smoke --bench-out D:\lv-build\bench\piper-smoke

# Exit-Code in einem Skript auswerten (3 = in der App freigeben)
local-voice-ai.exe --tts-voices --tts-provider phonon; if ($LASTEXITCODE -eq 3) { "In der App freigeben" }
```

Den Benchmark ruft normalerweise der Python-Harness auf (`tts_bench.py run --exe <pfad>`); er bringt Messordner,
Ressourcenmessung, ASR-Rücklesung und Berichte mit. Das Zeilenformat steht in `scripts/bench/tts/schema/README.md`.

### Benchmark-Verhalten (A/managers/tts/providers/bench.rs:1-60)

Jede Synthese läuft über `stream_synthesize` der Registry, **ohne Wiederholung** (ein Fehlschlag wird als Zeile
gemeldet), strikt nacheinander. `cold`: je Satz zuerst Zurücksetzen (Fish: ein **selbst gestarteter** Server wird neu
gestartet, ein fremder bleibt unberührt und wird in der Zeile vermerkt), dann frisch gebaute Registry; `warm`: eine
Aufwärm-Synthese ohne Zeile, dann alle Läufe. Bei Piper, Phonon und den Servern ist `process_start_ms` nicht messbar
(Hinweis in `env.json`, A/tts_cli.rs:396-410). Je Synthese höchstens `MAX_AUDIO_BYTES`; `rows.jsonl` wird zeilenweise
geschrieben, WAV und `env.json` über `.part` und Umbenennen.

## 2. Agentenbrücke

Die Brücke (`local-voice-ai.exe --mcp` bzw. `ctl`, Beschreibung in [docs/AGENTEN-ANBINDEN.md](../AGENTEN-ANBINDEN.md), Zeilen 78-85)
führt acht Sprachausgabe-Werkzeuge. Rechte, Freigaben und Audit erledigt die Brücke **vor** dem Aufruf; Katalog:
A/agent_bridge/catalog.rs:76-122, Handler A/agent_bridge/tools/tts_providers.rs.

| Werkzeug | Recht | Wirkung |
|---|---|---|
| `tts_providers_list` | `tts.read` | alle Anbieter mit Stufe, Fähigkeiten, Lizenz, Gesundheit (4 s Frist je Anbieter), Freigabestand und Sperrgrund |
| `tts_provider_capabilities` | `tts.read` | Sprachen, Streaming, Geräte, Plattformen, Lizenz und Freigabestand EINES Anbieters (`id`) |
| `tts_provider_voices` | `tts.read` | Stimmen (`id`, optional `language`); gilt als Nutzung: gesperrt = Sperrgrund; höchstens 200 Stimmen, Frist 10 s |
| `tts_provider_health` | `tts.read` | Gesundheit mit Frist; bei „nicht bereit“ mit Grund und Hinweis |
| `tts_benchmark_results` | `tts.read` | ohne Argument: Liste der Zusammenfassungen (höchstens 100 Läufe); mit `run_dir` (Kennung, kein Pfad; optional `source` `docs` oder `bench`): deren `results.json`, höchstens 512 KiB |
| `tts_provider_select` | `tts.render` | wählt Anbieter (`id`) und Stimme (`voice`) fürs Vorlesen, **nur wenn `check_use` es erlaubt**; die Stimme muss in `tts_provider_voices` stehen, ohne `voice` bleibt die bisherige |
| `tts_page_create` | `tts.render` | Seite der Vorlesen-Bibliothek anlegen (bestehend) |
| `tts_render_audio` | `tts.render` | Audiodatei aus einer Seite erzeugen (bestehend); bei `tts_engine = provider:<id>` läuft jeder Satz durch `ProviderEngine::guard` |

* **Rechte:** `tts.read` ist neu (`Capability::TtsRead`, A/managers/integrations/model.rs) und rein lesend; `tts.render`
  gab es schon. Die Namen stehen in der Rechteliste der Brücke.
* **Lesbare Orte** von `tts_benchmark_results`: `%LOCALAPPDATA%\lva-bench\tts\<Lauf>\report\results.json` (Umgebung
  `LVA_BENCH_DIR` ersetzt `%LOCALAPPDATA%\lva-bench`) und `docs/tts/benchmarks/<Datum>/results.json` (`LVA_TTS_BENCH_DOCS`
  ersetzt den Ort); Kennung ohne Pfadanteile, der aufgelöste Pfad muss im Ordner liegen, auch bei Verknüpfungen
  (A/agent_bridge/tools/tts_providers.rs, `bench_roots_from`).
* **Absicherung** (Fehlerfälle-Tabelle in der Moduldoku, Tests in `tools/tts_providers/tests.rs`): gleichzeitige Wahl unter
  derselben Sperre wie die App; `tts_provider_select` prüft die Guardrails zweimal (vor der Stimmenliste und beim
  Schreiben); Antworten und Texte sind begrenzt und geschwärzt; ohne Anbieterschicht (headless Sandbox) antworten die
  Werkzeuge mit Klartext statt Panik.

## 3. Was bewusst fehlt

* **Keine Freigabe per Kommandozeile oder Brücke.** Es gibt keinen Schalter und kein Werkzeug für Opt-in,
  Lizenz-Zustimmung oder Fernzugriff; die Befehle `tts_provider_set_enabled`, `tts_provider_acknowledge_license`,
  `tts_provider_set_allow_remote` sind nur der Oberfläche vorbehalten, sonst könnte ein Agent die Lizenz- und
  Datenschutzsperre selbst aufheben (P/policy.rs:39-44; A/commands/tts_providers.rs:9-19; A/tts_cli.rs:7-18). Auch
  Umgebungsvariablen und `tts_provider_set_config` setzen keine Freigabe. Gesichert durch die Tests
  `die_cli_hat_keinen_schalter_fuer_freigaben` (A/tts_cli.rs:1076) und `no_tool_can_change_enabled_acks_or_remote`
  (A/agent_bridge/tools/tts_providers/tests.rs:640).
* **Kein `tts_benchmark_smoke` (kein Werkzeug, das Messläufe startet).** Im Katalog, in den Handlern und im Commit
  `1ad486f5` gibt es kein solches Werkzeug, und der Code begründet das Fehlen nicht ausdrücklich. Belegbar ist: die
  Brückenwerkzeuge **laden kein Modell** und starten keine Last (Moduldoku A/agent_bridge/tools/tts_providers.rs:48-49);
  ein Benchmark dagegen startet Anbieterprozesse, läuft Minuten bis Stunden strikt nacheinander und verlangt laut Plan
  exklusive GPU (implementation-plan.md §7, bench.rs:1-60). Ein Agent liest deshalb nur **fertige** Ergebnisse
  (`tts_benchmark_results`); gemessen wird per `--tts-bench` bzw. Harness unter Aufsicht des Nutzers.
* **Kein Konfigurations-Setter** über Kommandozeile oder Brücke: Modell, Programm und Adresse lassen sich nur je Lauf
  überschreiben (CLI) oder in der App speichern.
* **Keine Probestimme, kein Abbruch** über die Brücke: `tts_provider_preview`/`tts_provider_cancel` sind Befehle der
  Oberfläche.
