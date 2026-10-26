# TTS-Anbieter: Änderungsprotokoll

Stand 26.10.2026 · Zweig des Goals `tts-provider` (Diff gegen `origin/main`: 86 Dateien, +30 569/−57 Zeilen,
`git diff --shortstat origin/main..HEAD`). Gegenstand ist die Einführung austauschbarer Sprachausgabe-Anbieter;
Einstieg: [README.md](README.md).

## Änderungen (aus `git log origin/main..HEAD`)

| Commit | Inhalt |
|---|---|
| `d7e806de` | Recherche, Umsetzungsplan und Goal für modulare TTS-Anbieter (Paket P0) |
| `39efdb9c` | reproduzierbarer deutscher TTS-Benchmark: Korpus `de-v1`, Harness, Hörtest-Paket (P5) |
| `3a035ed4` | Benchmark: ASR-Engine `app-cli` (Whisper large-v3-turbo Q8 ohne Download) für WER/CER |
| `3c2fefa9` | dyn-fähiger Anbietervertrag mit Registry, Abbruch, Retry, Mock und Vertragstest-Satz; Hüllen für Fish und Piper (P1) |
| `732cb1bd` | Adapter `phonon` (ptts-CLI, offline), OpenAI-kompatible Sprachausgabe (PCM-Chunk-Streaming, nur Loopback), `ollama-speech` mit Versions-/Plattformprüfung (P2) |
| `178d18e0` | Doku: Adaptervertrag, Installation/Fehlersuche, Lizenzübersicht (D1) |
| `630c3793` | Anbieter in der App: Einstellungen mit Vorrang, Lizenz-/Privacy-Sperren, Engine-Variante `provider:<id>`, Tauri-Befehle mit Stream-Probestimme (P3a) |
| `2b527389` | UI: Gruppe „Sprachausgabe-Anbieter“ im Reiter Ausgabe (fähigkeitsgesteuert, Freigabe-Ablauf, Probestimme mit Abbruch) (P4) |
| `1ad486f5` | CLI (`--tts-providers`, `--tts-voices`, `--tts-provider`, `--tts-bench`) und Agentenbrücken-Werkzeuge (P3b) |
| (ungesichert) | diese Doku-Runde (D2): Abschnitt 11 des Vertrags, `cli-and-automation.md`, `README.md`, dieses Protokoll, Verweise in beiden READMEs |

## Dateigruppen

| Gruppe | Pfade |
|---|---|
| Anbieterschicht (Rust) | `apps/local-voice/src-tauri/src/managers/tts/providers/` (`contract`, `registry`, `builtin`, `phonon`, `openai_speech`, `ollama_speech`, `policy`, `setup`, `host`, `pipeline`, `preview`, `bench`, Testhilfen) |
| Kern und Einstellungen | `managers/tts/engine.rs`, `managers/tts/mod.rs`, `managers/tts/player.rs`, `settings.rs` (Feld `tts_providers`), `lib.rs`, `shortcut/mod.rs` |
| Tauri-Befehle | `commands/tts_providers.rs`, `commands/mod.rs`, `src/bindings.ts` |
| CLI | `cli.rs`, `tts_cli.rs` |
| Agentenbrücke | `agent_bridge/catalog.rs`, `agent_bridge/tools/tts_providers.rs` (+ `tests.rs`), `agent_bridge/tools/app_host.rs`, `managers/integrations/model.rs` (Recht `tts.read`) |
| Oberfläche | `src/components/settings/tts/providers/*`, `TtsSettings.tsx`, `ReadAloudTab.tsx`, `stores/settingsStore.ts`, `i18n/locales/{de,en}/translation.json` |
| Benchmark | `scripts/bench/tts/` (Harness, Korpus, Schema, Konfiguration, pytest) |
| Doku und Koordination | `docs/tts/`, `docs/AGENTEN-ANBINDEN.md`, `koordination/tts-provider/` |

## Tests

Die Commit-Nachrichten nennen keine Testzahlen; **Zahlen siehe PR**. Belegbar im Baum:

* Vertragstest-Satz `contract_suite` (`providers/contract_tests.rs`), je Adapter aufgerufen; Fake-`ptts`
  (`phonon_fake.rs`) und Fake-HTTP-Server (`test_http.rs`).
* Unit-Tests in `policy.rs`, `pipeline.rs`, `preview.rs`, `bench.rs`, `tts_cli.rs`, `commands/tts_providers.rs`,
  `settings.rs` (alte Einstellungsdateien) und `agent_bridge/tools/tts_providers/tests.rs`.
* Playwright: `apps/local-voice/tests/tts-providers.spec.ts`, angepasst `voice-availability.spec.ts`.
* Benchmark-Harness: `scripts/bench/tts/test_tts_bench.py` (pytest, ohne App, Netz und GPU).
* Live-Läufe echter Anbieter: Messlauf P6 vom 26.10.2026 (Piper und Fish), siehe „Benchmark-Ergebnisse“; Phonon nicht gemessen.

## Benchmark-Ergebnisse

Messlauf 26.10.2026 (Piper CPU, Fish CUDA, deutscher Korpus `de-v1`, Debug-Build 0.21.18, GPU nicht exklusiv), je 0 Fehler:

* Piper `de_DE-thorsten-high`: TTFA = Gesamtdauer, Median 1069 ms (P95 2155 ms), RTF 0,255; WER 6,64 %, CER 3,10 %.
* Fish `s2-pro`: TTFA Median 3733 ms (P95 13616 ms), RTF 0,820; Serverstart kalt Median 68 s; WER 4,85 %, CER 2,33 %; VRAM-Delta 20,3 GB.
* Keine der Zielwerte TTFA < 200 ms und RTF < 0,2 erreicht; WER < 5 % nur von Fish. Phonon, Ollama-Speech, Kokoro, XTTS-v2, F5-TTS nicht gemessen (Gründe im Bericht).
* Methode, Grenzen, Befehle: [benchmark.md](benchmark.md); Rohbericht: [benchmarks/2026-10-26/report.md](benchmarks/2026-10-26/report.md). Hörtest-Paket erzeugt, noch nicht bewertet.

## Bekannte Grenzen

* **B2 Piper-Abbruch**: ein Abbruch beendet das Warten sofort, den Piper-Prozess aber erst nach seiner eigenen Frist
  (Auftragsfrist + 2 s); ein echtes Kill bräuchte eine Änderung an `piper.rs`. Siehe
  [installation-troubleshooting.md](installation-troubleshooting.md), §7.
* **R1 Phonon-CLI nur satzweise**: `ptts.exe` schreibt nur Dateien, daher `StreamingMode::Sentence` und ein Prozess je Satz;
  echtes Streaming gäbe es nur über die Phonon-Server oder Ollama.
* **Ollama-Speech nur macOS laut Upstream**: unter Windows meldet der Anbieter „nicht verfügbar“ mit Grund; `probe_speech`
  prüft es.
* **Fish-Pfad im Benchmark ungetestet** bis zum Messlauf: die Fish-Hülle ist nur mit einer Fake-`SynthFn` geprüft
  (`host.rs` hat bewusst keine Tests), der echte Server-Neustart bei `cold` ist nicht gemessen.
* **Lizenz-Zustimmung ohne Widerruf-Befehl**: ein Opt-out nimmt die Freischaltung zurück, die gespeicherte Zustimmung bleibt
  (gilt nur bei unverändertem Fingerabdruck). Siehe [licenses.md](licenses.md), §3.
* Latenz, RTF und VRAM der Experimental-Anbieter sind bis zum Messlauf nicht verifiziert.

## Offene Entscheidungen (Owner)

* **O1 Phonon `40m-de`**: einzige deutsche Phonon-Gewichte, gated und CC BY-NC 4.0; Bedingungen auf Hugging Face annehmen
  und `HF_TOKEN` setzen. Vorschlag laut Goal: nur interne, nichtkommerzielle Evaluation, kein Ausliefern.
* **P2L Ausführung `ptts.exe`**: der Auto-Mode-Klassifikator sperrt das Ausführen der heruntergeladenen `ptts.exe`
  (Befund B1); der Live-Lauf von Phonon (WAV-Beleg, Benchmark-Zeile) wartet auf die Erlaubnis.
* **B3 thorsten-Lizenzwiderspruch**: Datensatz CC0 laut Model Card, im App-Katalog zugleich `non_commercial: true`
  (Ableitung von der Lessac-Stimme); die Auslieferbarkeit ist nicht geklärt. Siehe [licenses.md](licenses.md), §2.
