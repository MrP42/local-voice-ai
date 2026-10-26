---
thema: tts-provider
titel: Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark
state: EXECUTING
vorzustand: -
pausengrund: -
issue: 93
repo: MrP42/local-voice-ai
branch: feat/tts-provider
iteration: 1
erstellt: 2026-10-26
aktualisiert: 2026-10-26T14:53
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
- [ ] AK1 — `cargo test --lib managers::tts::providers` → alle grün, Vertragstest-Satz läuft für jeden registrierten Anbieter (Mock, Fish/Piper-Hülle, Phonon mit Fake-EXE, OpenAI-Speech und Ollama mit Fake-Server)
- [ ] AK2 — Regression: `cargo test --lib managers::tts` (bestehende ~265 Tests) grün + Playwright `readaloud-*`/`voice-availability` grün
- [ ] AK3 — `local-voice-ai --tts-providers --json` listet ≥ 5 Anbieter mit Stufe, Fähigkeiten, Lizenz, Health; auf diesem PC ist `ollama-speech` „nicht verfügbar“ mit Grund (Version/Plattform)
- [ ] AK4 — Phonon nativ: `--tts-test --tts-provider phonon` erzeugt mit ptts 0.4.2 eine WAV (CPU) — Beleg-Datei; Deutsch entweder belegt oder als Owner-Blocker (gated CC BY-NC) dokumentiert
- [ ] AK5 — UI: Playwright-Spec `tts-providers.spec.ts` grün (Liste mit Status/Gerät/Lizenz, Probestimme + Abbrechen, Opt-in-Sperre) + Screenshot
- [ ] AK6 — Agentenbrücke: Werkzeuge Anbieter/Fähigkeiten/Stimmen/Health/Auswahl/Benchmark-Ergebnisse; Tests belegen, dass kein MCP-Aufruf Lizenz- oder Privacy-Sperren aufhebt
- [ ] AK7 — `pytest scripts/bench/tts` grün; Korpus `de-v1` ≥ 100 Sätze, alle 10 Kategorien, Normalisierungsregeln, Smoke-Teilmenge
- [ ] AK8 — Gespeicherte Messung `docs/tts/benchmarks/<datum>/` (JSON + CSV + Markdown) für jede lokal lauffähige Konfiguration mit ≥ 30 warmen Messungen und ≥ 3 Kaltstarts, WER/CER mit ASR-Version; übrige Zeilen `not tested`/`unsupported`/`license restricted`
- [ ] AK9 — Blindhörtest-Paket: anonymisierte Reihenfolge, Schlüsseldatei getrennt, Bewertungsbogen; keine erfundenen Bewertungen
- [ ] AK10 — Doku: `docs/tts/` (provider-research, implementation-plan, adapter-contract, installation-troubleshooting, benchmark, changelog) + README-Verweis

## Quality Gates
- [ ] QG1 — Tests grün: `cargo test --lib managers::tts agent_bridge settings::` + `npx tsc --noEmit` + `pytest scripts/bench/tts`
- [ ] QG2 — Integration läuft: CLI-Ausgabe `--tts-providers` + Phonon-WAV + Benchmark-Lauf aus dem gebauten Binary
- [ ] QG3 — Doku + Handoff aktualisiert: `docs/tts/`, `.claude/handoffs/2026-10-26-tts-provider.md`
- [ ] QG4 — Sicherheit/Lizenz: kein Default-Download, NC-Anbieter nur mit Opt-in, Nicht-Loopback-URLs nur mit ausdrücklicher Freigabe, unabhängiges Review ohne kritische Befunde
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
-

## Blocker
-

## Entscheidungen
- 2026-10-26: Basis `origin/main` (Kette #76–#86 ist gemergt).
- 2026-10-26: Neue dyn-Anbieterschicht neben dem bestehenden `TtsEngine`-Trait statt dessen Umbau (Rückwärtskompatibilität, Cache-Tags bleiben).

## Nächste empfohlene Aktion
Pakete P1 (Vertrag) und P5 (Benchmark-Korpus/Harness) parallel briefen.

## Verlauf
- 2026-10-26T14:42 DISCOVERY — Goal State angelegt
- 2026-10-26T15:20 DISCOVERY — Ist-Stand (Explore + Recherche) erhoben, Goal gefüllt
- 2026-10-26T14:52 DISCOVERY (Runde 0) — Metadaten: issue=93
- 2026-10-26T14:52 READY (Runde 0) — Goal definiert, Issue #93
- 2026-10-26T14:53 PLANNING (Runde 1) — Pakete P1-P7 geplant
- 2026-10-26T14:53 EXECUTING (Runde 1) — P1 (lv-coder-xhigh), P5 (lv-coder) laufen

