---
thema: tts-provider
titel: Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark
state: REWORK
vorzustand: -
pausengrund: -
issue: 93
repo: MrP42/local-voice-ai
branch: feat/tts-provider
iteration: 1
erstellt: 2026-10-26
aktualisiert: 2026-10-26T21:33
---

# Goal: Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark

Quelle: Auftrag `goal.md` (Patrick, 26.10.2026). Worktree `D:\wt\local-voice-project\tts-provider`, Basis `origin/main` 784d9802.

## Zielzustand
Local Voice AI hat eine dyn-fähige, fähigkeitsgesteuerte TTS-Anbieterschicht (Liste, Fähigkeiten, Lizenz, Health, Stimmen, Synthese, Chunk-Streaming, Abbruch), über die Fish und Piper unverändert weiterlaufen und Phonon (nativ über `ptts.exe`, per OpenAI-kompatiblem Server) sowie Ollama-Speech (versions-/plattformgeprüft) opt-in hinzukommen — sichtbar im Reiter „Ausgabe“, per CLI und Agentenbrücke. Ein wiederverwendbarer Benchmark (≥ 100 deutsche Sätze) erzeugt belegte JSON/CSV/Markdown-Ergebnisse unter `docs/tts/benchmarks/`, ohne erfundene Werte.

## Scope
- Anbietervertrag + Registry in `src-tauri/src/managers/tts/providers/`, Hüllen für Fish/Piper/Mock, Vertragstests
- Adapter: `phonon` (ptts.exe-Subprozess), `openai-speech` (generisch: ptts-openai-server, Kokoro-FastAPI), `ollama-speech` (gated)
- Einbindung in die Vorlese-Pipeline als zusätzliche Engine-Variante (opt-in), Einstellungen mit Validierung
- UI-Gruppe im Reiter „Ausgabe“ (kein neuer Reiter), CLI-Schalter, Agentenbrücken-Werkzeuge mit Lizenz-/Privacy-Sperren
- Benchmark-Harness `scripts/bench/tts/` + Korpus `de-v1` + Hörtest-Paket + gespeicherte Ergebnisse
- Doku `docs/tts/`, Verweise in README/AGENTS

## Non-Scope
- Kein Umbau der Fish-/Piper-Interna, keine Änderung bestehender Defaults (`tts_engine=fish`)
- Keine Installation von XTTS-v2/F5-TTS/Kokoro in den Standardumfang (nur Steckbrief/Status)
- Kein Herunterladen gated/nichtkommerzieller Gewichte ohne Owner-Freigabe; kein Release/Tag
- Fremde Worktrees unter `.claude/worktrees/` werden nicht angefasst

## Akzeptanzkriterien
- [x] AK1 — `cargo test --lib managers::tts::providers` → alle grün, Vertragstest-Satz läuft für jeden registrierten Anbieter (Mock, Fish/Piper-Hülle, Phonon mit Fake-EXE, OpenAI-Speech und Ollama mit Fake-Server)
- [x] AK2 — Regression: `cargo test --lib managers::tts` (bestehende ~265 Tests) grün + Playwright `readaloud-*`/`voice-availability` grün
- [x] AK3 — `local-voice-ai --tts-providers --json` listet ≥ 5 Anbieter mit Stufe, Fähigkeiten, Lizenz, Health; auf diesem PC ist `ollama-speech` „nicht verfügbar“ mit Grund (Version/Plattform)
- [ ] AK4 — Phonon nativ: `--tts-test --tts-provider phonon` erzeugt mit ptts 0.4.2 eine WAV (CPU) — Beleg-Datei; Deutsch entweder belegt oder als Owner-Blocker (gated CC BY-NC) dokumentiert
- [x] AK5 — UI: Playwright-Spec `tts-providers.spec.ts` grün (Liste mit Status/Gerät/Lizenz, Probestimme + Abbrechen, Opt-in-Sperre) + Screenshot
- [x] AK6 — Agentenbrücke: Werkzeuge Anbieter/Fähigkeiten/Stimmen/Health/Auswahl/Benchmark-Ergebnisse; Tests belegen, dass kein MCP-Aufruf Lizenz- oder Privacy-Sperren aufhebt
- [x] AK7 — `pytest scripts/bench/tts` grün; Korpus `de-v1` ≥ 100 Sätze, alle 10 Kategorien, Normalisierungsregeln, Smoke-Teilmenge
- [x] AK8 — Gespeicherte Messung `docs/tts/benchmarks/<datum>/` (JSON + CSV + Markdown) für jede lokal lauffähige Konfiguration mit ≥ 30 warmen Messungen und ≥ 3 Kaltstarts, WER/CER mit ASR-Version; übrige Zeilen `not tested`/`unsupported`/`license restricted`
- [x] AK9 — Blindhörtest-Paket: anonymisierte Reihenfolge, Schlüsseldatei getrennt, Bewertungsbogen; keine erfundenen Bewertungen
- [x] AK10 — Doku: `docs/tts/` (provider-research, implementation-plan, adapter-contract, installation-troubleshooting, benchmark, changelog) + README-Verweis

## Quality Gates
- [x] QG1 — Tests grün: `cargo test --lib managers::tts agent_bridge settings::` + `npx tsc --noEmit` + `pytest scripts/bench/tts`
- [ ] QG2 — Integration läuft: CLI-Ausgabe `--tts-providers` + Phonon-WAV + Benchmark-Lauf aus dem gebauten Binary
- [ ] QG3 — Doku + Handoff aktualisiert: `docs/tts/`, `.claude/handoffs/2026-10-26-tts-provider.md`
- [x] QG4 — Sicherheit/Lizenz: kein Default-Download, NC-Anbieter nur mit Opt-in, Nicht-Loopback-URLs nur mit ausdrücklicher Freigabe, unabhängiges Review ohne kritische Befunde
- [ ] QG5 — Git: Zweig gepusht, PR gegen `main` offen (Regel AGENTS.md), keine fremden Dateien formatiert

## Constraints
- Builds/Tests nur auf D: (`CARGO_TARGET_DIR=D:\lv-build\target-dokumente`), Idle-Priorität, `CARGO_BUILD_JOBS=2`, `memgate.sh` vor rustc; Rust-Lanes nacheinander
- Prettier/Clippy/Translations vorbestehend rot → nur eigene Hunks bewerten
- Kein `git stash`, kein Force-Push, nie direkt auf `main`
- GPU-Messungen nie parallel zu anderen GPU-Läufen

## Architekturprinzipien
- Anbieter-spezifisches bleibt im Adapter; Orchestrierung/Player kennen nur den Vertrag
- Fähigkeiten statt `if engine === …` im Frontend
- Nicht-streamende Anbieter melden `streaming = none|sentence`, nie „chunk“
- Lizenz = Code-Lizenz ≠ Gewichts-Lizenz; `verified` nur mit Quelle + Datum
- Lokal zuerst: nur Loopback-Endpunkte ohne ausdrückliche Freigabe

## Dependencies
- Phonon `ptts` v0.4.2 (gradium-ai/xn-ptts, Release 25.10.2026), Modell `gradium/phonon-31m-en` (Apache-2.0, öffentlich)
- Ollama ≥ 0.40.11 für Speech (laut Release-Notes nur macOS); lokal 0.34.2
- faster-whisper 1.1.1 (lokal) für WER/CER

## Risiken / Owner-Entscheidungen
- O1: Deutsche Phonon-Gewichte (`phonon-40m-de`) sind gated + CC BY-NC 4.0 → Patrick müsste Bedingungen auf HF annehmen und `HF_TOKEN` setzen; Vorschlag: nur interne, nichtkommerzielle Evaluation, kein Ausliefern
- O2: Ollama-Speech unter Windows laut Upstream nicht unterstützt → Test optional auf dem MacBook
- O3: Standard-Engine Fish (Code + s2-pro-Gewichte) steht unter der Fish Audio Research License (nichtkommerziell, lokal belegt 26.10.) → kommerzielle Nutzung braucht Lizenz von Fish Audio oder anderen Standard; Vorschlag: Hinweis in der UI (umgesetzt), Entscheidung bei Patrick
- R1: `ptts.exe` streamt nicht über stdout → nativer Adapter nur satzweise
- R2: Rust-Testbau braucht ~7 GB RAM → memgate

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M0 | `docs/tts/provider-research.md` + `implementation-plan.md` | offen |
| M1 | Vertrag + Registry + Adapter, CLI-Ausgabe `--tts-providers` | offen |
| M2 | UI-Gruppe + Screenshot, Agentenbrücken-Werkzeuge | offen |
| M3 | Benchmark-Harness + Korpus + gespeicherte Ergebnisse | offen |
| M4 | Review, Doku, PR | offen |

## Evidence
- 2026-10-26T15:23 AK7 erfüllt — pytest scripts/bench/tts: 67 passed; validate-corpus de-v1: 123 Saetze, 10 Kategorien, Smoke 12 (Commit 39efdb9c)
- 2026-10-26T17:59 AK1 erfüllt — cargo test --lib managers::tts::providers + Filter: 1361 passed 0 failed (Commit 1ad486f5); contract_suite fuer Mock, Fish-/Piper-Huelle, Phonon-Fake-EXE, OpenAI-/Ollama-Fake-Server
- 2026-10-26T17:59 AK3 erfüllt — debug local-voice-ai.exe --tts-providers --json: 5 Anbieter mit tier/caps/license/health; ollama-speech available=false 'Ollama ist nicht erreichbar' (Version/Plattform-Stufen mit Fake-Server getestet); --tts-voices --tts-provider phonon -> Exit 3 gesperrt (Commit 1ad486f5)
- 2026-10-26T18:05 AK5 erfüllt — LV_DEV_PORT=1638 playwright tts-providers.spec.ts: 12 passed (Attrappe = echte --tts-providers-JSON); Screenshot docs/tts/img/tts-anbieter-gruppe.png (Commit 328d747b)
- 2026-10-26T19:15 AK6 erfüllt — agent_bridge::tools::tts_providers Tests (Werkzeuge ohne Freigabe-Methode, select gesperrt, Pfad-Traversal, Debug-only-Fallback) in Filterlauf 1396 passed; P7-Review: kein Umgehungsweg fuer check_use gefunden (Commit c21d9fab)
- 2026-10-26T21:32 AK2 erfüllt — cargo test Filter managers::tts/commands::tts_providers/mcp/agent_bridge/integrations/tts_cli/settings: 1396 passed 0 failed; Playwright readaloud-*/voice-availability/tts-providers/script-check: 100 passed 12 skipped (Commit 9d4d2a20)
- 2026-10-26T21:32 AK8 erfüllt — docs/tts/benchmarks/2026-10-26/ results.json/csv/report.md: Piper CPU + Fish CUDA je 360 warm + 36 cold (alle ok) + 123 full; WER app-cli Whisper large-v3-turbo Q8; Statuszeilen phonon-31m-en not_tested, 40m-de license_restricted, ollama/kokoro unsupported, xtts/f5 not_tested (Commit 9d4d2a20)
- 2026-10-26T21:32 AK9 erfüllt — listening-kit 40 Proben anonymisiert, Seed 20261026, Schluessel getrennt D:/lv-build/lva-bench/tts/listening-keys/2026-10-26; Vorlage docs/tts/benchmarks/2026-10-26/listening-kit; keine Bewertungen (Commit 9d4d2a20)
- 2026-10-26T21:32 AK10 erfüllt — docs/tts: README, provider-research, implementation-plan, adapter-contract, installation-troubleshooting, licenses, cli-and-automation, benchmark, CHANGELOG; README-Verweise Wurzel + App (Commit 9d4d2a20)
- 2026-10-26T21:33 QG1 erfüllt — cargo test Filter 1396 passed; npx tsc --noEmit ok; pytest scripts/bench/tts 73 passed; Playwright 100 passed (9d4d2a20)
- 2026-10-26T21:33 QG4 erfüllt — P7 unabhaengiges Review: 0 kritisch, 1 hoch + 3 mittel + 5 niedrig + B10 in P7n (c21d9fab) behoben; kein Default-Download, NC-Anbieter nur mit Opt-in+Fingerabdruck, Loopback-Pflicht

## Blocker
- B1 [offen] [P2L] 2026-10-26T14:54 Ursache: Auto-Mode-Klassifikator sperrt das Ausfuehren der heruntergeladenen ptts.exe (Code from External); deutsche 40m-Gewichte zusaetzlich gated + CC BY-NC · Owner: Patrick · entsperrt, wenn: Patrick erlaubt die Ausfuehrung von D:\lv-build\phonon\bin\...\ptts.exe (Permission-Regel oder manueller Lauf) und entscheidet O1 (HF-Bedingungen phonon-40m-de annehmen, HF_TOKEN) · nächste Prüfung: beim nächsten Sessionstart

## Entscheidungen
- 2026-10-26: Basis `origin/main` (Kette #76–#86 ist gemergt).
- 2026-10-26: Neue dyn-Anbieterschicht neben dem bestehenden `TtsEngine`-Trait statt dessen Umbau (Rückwärtskompatibilität, Cache-Tags bleiben).

## Nächste empfohlene Aktion
P6 validieren; AK2 Regression (Playwright readaloud) nach dem Messlauf; Push + PR; Handoff; P2L bleibt Owner-Blocker

## Verlauf
- 2026-10-26T14:42 DISCOVERY — Goal State angelegt
- 2026-10-26T15:20 DISCOVERY — Ist-Stand (Explore + Recherche) erhoben, Goal gefüllt
- 2026-10-26T14:52 DISCOVERY (Runde 0) — Metadaten: issue=93
- 2026-10-26T14:52 READY (Runde 0) — Goal definiert, Issue #93
- 2026-10-26T14:53 PLANNING (Runde 1) — Pakete P1-P7 geplant
- 2026-10-26T14:53 EXECUTING (Runde 1) — P1 (lv-coder-xhigh), P5 (lv-coder) laufen
- 2026-10-26T14:54 EXECUTING (Runde 1) — B1 blockiert Paket P2L: Auto-Mode-Klassifikator sperrt das Ausfuehren der heruntergeladenen ptts.exe (Code from External); deutsche 40m-Gewichte zusaetzlich gated + CC BY-NC – übrige Pakete laufen weiter
- 2026-10-26T16:15 EXECUTING (Runde 1) — P1,P2,P5,P5n abgenommen; P3a + D1 laufen
- 2026-10-26T17:10 EXECUTING (Runde 1) — Budget ~2,4 MTok (69 % von 3,5); Hochrechnung 4-4,5 MTok. P3a abgenommen 630c3793; P3b + P4 laufen
- 2026-10-26T17:59 EXECUTING (Runde 1) — Budget ~3,3 MTok (94 %); Hochrechnung 4,3-4,6 MTok, Stopp bei 5,25 (150 %). P3b abgenommen 1ad486f5; P4n, D2, P7 laufen
- 2026-10-26T18:08 VALIDATING (Runde 1) — P7 Review: 1 hoch, 3 mittel, 5 niedrig (B7-B9)
- 2026-10-26T18:08 REWORK (Runde 1) — P7n Nacharbeit laeuft
- 2026-10-26T19:16 REWORK (Runde 1) — Budget ~4,6 MTok (131 %); P7n abgenommen c21d9fab; P6 Messlauf laeuft exklusiv

