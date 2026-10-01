# P7b – QG4 Systemschutz: knapper RAM → sauberer Abbruch

**Gate (GOAL.md):** alle neuen Modelle/Prozesse hinter RAM-Start-Gate/Deckel; Test mit knappem RAM → sauberer Abbruch statt Einfrieren.
**Ergebnis: erfüllt.** Alle fünf Läufe enden ohne Zeitüberschreitung (0,04 s bis 18 s). Kein eigener Prozess bleibt übrig. Die Besprechung steht danach auf `ready`, das Live-Transkript bleibt erhalten. Jeder Schritt meldet seinen Fehler- oder Rückfall-Code.
Stand 30.09.2026, Release-Build `feat/granola-p7b` mit `--features gpu-vulkan` (Kopie unter `%LOCALAPPDATA%\lva-bench\p7b-gpu`).

## Weg: knapper RAM ohne echten Speicherdruck

Der Rechner wird **nicht** gefüllt. Neuer Testschalter `LVA_TEST_FREE_RAM_MB=<n>` in `process_guard::available_ram_mb()` (`src-tauri/src/process_guard.rs:36-58`). Er deckelt den gemessenen freien RAM auf n MB. Er ist das einzige Messglied für alle Gates (per `grep available_memory()` geprüft: keine zweite Messstelle). Deshalb sieht jedes Gate im Besprechungspfad „knapper RAM“ genau so, wie es auf einem vollen Rechner käme.
Der Schalter kann Gates nur **schließen**: Er bildet das Minimum mit dem Messwert, ein Deckel über dem Messwert bewirkt nichts. Ein Deckel von 0 wird zu 1 und öffnet deshalb nicht den Fall „nicht messbar“. Abgesichert durch den Test `process_guard::tests::test_cap_only_ever_lowers_the_free_ram`.

## Befehl → Ergebnis

`pwsh -File koordination/granola-besprechungen/tools/p7b-qg4.ps1 -AppExe <exe> -Out abnahme/p7b-qg4.json`.
Jeder Lauf: Job-Objekt mit 16 GB Deckel, BelowNormal, Kill-on-close; RAM-Start-Gate ≥ 10 GB frei; Sandbox `LVA_MEETINGS_DIR`/`HF_HOME`. Audio: `scene1_status`, 4,3 min, Mikrofon + Systemton.

| Lauf | Deckel | Befehl (gekürzt) | Exit | Dauer | Ergebnis |
|---|---|---|---|---|---|
| A0 Kontrolle | – | `--simulate-meeting … --final-model Qwen3-ASR-1.7B --notes` | 0 | 48,1 s | Enddurchlauf gelaufen (Vulkan), Sprecher `done`, KI-Notizen ok (21 s, 23 Einträge), `ready`; Spitze 7,8 GB (Job) |
| A | 5000 MB | wie A0 | 0 | 18,4 s | Plan `Keep(LowRam)` → `final_pass_skipped`, Live-Transkript bleibt; Sprecher `skipped` (`low_memory`, „4.9 GB frei, gebraucht etwa 1.0 GB plus 6 GB Reserve“); KI-Notizen `memory_low` nach 0,4 s, **kein llama-server gestartet**; Status `ready`; Spitze 1,5 GB |
| B | 5000 MB | `--reindex-meetings` (gleiche Sandbox) | 3 | 0,3 s | Volltext-Index gebaut (6 Chunks), Vektorstufe `memory_low` („Vektorstufe pausiert 10 min“), kein Embedding-Server |
| C | 2000 MB | `--export-meeting <id> --format pdf` | 1 | 0,04 s | `pdf_low_memory: Zu wenig freier Arbeitsspeicher für den PDF-Export (2.0 GB frei)`, kein WebView2 gestartet, keine Datei |
| D | 1500 MB | wie A (unter der Notgrenze des Wächters, 2 GB) | 0 | 18,0 s | wie A (alle Gates zu), `ready` |

Rohdaten: `abnahme/p7b-qg4.json` (je Lauf Exit, Dauer, Spitze, Reste, Logzeilen); Logs in `%LOCALAPPDATA%\lva-bench\results\p7b\p7b-qg4-*.log`.
„Die App reagiert“ heißt hier: Der Prozess beendet sich selbst mit Code und vollständigem JSON, nicht durch Zeitlimit. Die Oberfläche (GUI) wurde nicht unter knappem RAM getestet.

## Neue Kindprozesse und Modellstarts des Goals – je Gate/Deckel mit Fundstelle

| Start | Prozess | Gate | Deckel / Begrenzung | Fundstelle |
|---|---|---|---|---|
| llama-server für KI-Notizen, Chat, Brief | Kindprozess | `check_ram_for_start(Modellgröße)` | Job-Objekt: RAM = frei − 6 GB, 75 % CPU, BelowNormal, Kill-on-close; Wächter stoppt ihn | `managers/llm/server.rs:633`, `:641`; Wächter `lib.rs:244` |
| zweiter llama-server BGE-M3 (Embeddings, M4-P4b) | Kindprozess | eigenes Gate `EMBED_RAM_NEED_MB` = 2048, danach dasselbe Start-Gate | derselbe Job-Deckel (gleiche `start`-Funktion); Wächter ruft `stop_embedding()` | `managers/llm/mod.rs:299`, `server.rs:633/641`, `lib.rs:248` |
| Sortformer-Diarisierung (M3) | im App-Prozess (FFI) | `check_ram_for_start(1024 + PCM)` vor dem Laden; beim Enddurchlauf `AppDiarizer::check_ram` | höchstens einer (`DIARIZER_SLOT`), wird vor dem End-STT-Modell freigegeben | `diarize/engine.rs:205-207`, `speakers.rs:674`, `final_pass.rs:1297` |
| Enddurchlauf Qwen3-ASR 1.7B / Whisper large-v3 (M2-P2d) | im App-Prozess (FFI) | Plan: `ram_need_mb` (Datei × 1,5 + 512) + 6 GB Reserve → `Keep(LowRam)`; vor dem Laden noch einmal `check_ram_for_start`; VRAM ≥ 4 GB | ein Modell zur Zeit (`try_start_loading`, altes Modell wird vorher freigegeben) | `final_pass.rs:192`, `:248`, `:1393` |
| WebView2 für PDF (M6-P6b) | Kindprozesse (msedgewebview2) | `ram_gate`: 700 MB + 2 GB Notgrenze | 20 s Zeitlimit, verwaiste Läufe werden weggeräumt (`sweep_stale`) | `managers/meetings/pdf.rs:166`, `:238` |
| Kalender-Sync (M5) | Task im App-Prozess | `check_ram_for_start(400)` | – | `managers/calendar/service.rs:449`, `:499` |
| Parakeet v3 GGUF als Live-Modell (P2g) | im App-Prozess | **kein RAM-Gate** (siehe Befund 2) | – | Laden über `transcription.rs:575` ohne `process_guard` |

Speicherbedarf gemessen (Job-Spitze, App + llama-server): 60-min-Besprechung mit Enddurchlauf, Sprechern und KI-Notizen 8,3 GB (Qwen3-ASR) bzw. 7,3 GB (Whisper large-v3), siehe `p7b-qg3-gpu-*.json`.

## Befunde

1. **Klein – der Wächter lässt sich headless nicht beobachten.** Er prüft alle 5 s (`WATCH_INTERVAL`). In Lauf D schließen die Gates schon vorher, und der Prozess endet nach 18 s, bevor der Wächter die 1,5 GB sieht. Seine Auslösung ist in P7b nicht belegt. Sie ist auch nicht als Test abgedeckt: `spawn_memory_watchdog` hat keinen Einheitstest.
2. **Klein – das Live-Modell für Besprechungen lädt ohne RAM-Gate.** In A und D lud Parakeet trotz 1,5 GB „frei“ (Spitze 1,5 GB Job). Die Datei ist klein (0,7 GB GGUF bzw. 0,6 GB ONNX), das Risiko also gering. Es ist aber der einzige Modellstart im Besprechungspfad ohne Gate. Ein Fix wäre `check_ram_for_start` vor `load_model_with_device` beim Aufnahmestart, samt Nutzer-Hinweis. Das ist ein Produktentscheid, deshalb nicht in P7b umgesetzt.
3. **Hinweis (kein Fehler):** Modelle, die im App-Prozess laufen (Sortformer, End-STT, Live-STT), haben kein eigenes Job-Objekt. Sie sind nur über Gate und „eins zur Zeit“ geschützt. Ein Deckel ginge nur über den App-Prozess selbst. So ist es entworfen (FFI, Entwurf M3).

## Grenzen

- Der Testschalter simuliert die **Messung**, nicht den Speicherdruck. Wie sich Windows unter echtem Druck verhält (Auslagerung), ist nicht Teil dieser Messung. Das verbietet die Systemschutz-Regel.
- Der Job-Deckel der Kindprozesse (llama-server) wurde nicht durch Überlauf ausgelöst. Dafür gibt es den Test `process_guard::tests::job_limit_stops_a_runaway_child`. Er ist aber `#[ignore]` und lief in P7b nicht mit (`cargo test --lib process_guard`: 6 ok, 1 ignoriert).
- Parallel lief auf dem Rechner ein fremder `pwsh`-Prozess mit hoher CPU-Last (P5f/anderer Worker). Für QG4 spielt das keine Rolle, hier zählt kein Zeitmaß.
