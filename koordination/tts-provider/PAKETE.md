# Pakete — Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark (Goal: D:/wt/local-voice-project/tts-provider/koordination/tts-provider/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| P1 | M1 | Anbietervertrag, Registry, Fehlerklassen, Cancel, Mock, Vertragstest-Satz, Hüllen Fish/Piper | `cargo test --lib managers::tts::providers` grün, bestehende `managers::tts` grün | abgenommen | 3c2fefa9 |
| P5 | M3 | Korpus de-v1 (≥100, 10 Kategorien), Normalisierung, Harness scripts/bench/tts, Hörtest-Paket, pytest | `pytest scripts/bench/tts` grün, Korpus-Schema-Test | abgenommen | 39efdb9c |
| P2 | M1 | Adapter phonon (ptts.exe), openai-speech (HTTP, PCM-Chunks), ollama-speech (Version/Plattform) | Vertragstest-Satz mit Fake-EXE/Fake-Server grün | abgenommen | 732cb1bd |
| P3a | M1 | Einstellungen tts_providers + Lizenz-/Privacy-Sperren, Registry-Anbindung, EngineImpl::Provider, Tauri-Befehle inkl. Probestimme/Abbruch, bindings.ts | cargo test gruen, Guardrail-Tests, tsc gruen | abgenommen | 630c3793 |
| P3b | M1 | CLI (--tts-providers, --tts-voices, --tts-provider, --tts-bench nach row.schema) + Agentenbruecke | `--tts-providers --json` >=5 Anbieter; Bridge-Tests | abgenommen | 1ad486f5 |
| P4 | M2 | UI-Gruppe im Reiter Ausgabe, Probestimme + Abbrechen, Playwright + Screenshot, i18n de/en | `tts-providers.spec.ts` grün + Screenshot | abgenommen | 2b527389 |
| P6 | M3 | Messlauf seriell + Ergebnisse + Doku docs/tts | docs/tts/benchmarks/<datum>/ JSON/CSV/MD | abgenommen | 9d4d2a20 |
| P7 | M4 | Unabhängiges Review | keine kritischen Befunde offen | abgenommen | review-only |
| P2L | M1 | Phonon live: ptts.exe 0.4.2 (D:\lv-build\phonon, SHA256 geprueft) + phonon-31m-en@92741c5 real ausfuehren, WAV-Beleg, Benchmark-Zeile | WAV + rows.jsonl aus echtem Lauf | blockiert | |
| P5n | M3 | ASR-Engine app-cli (Whisper large-v3-turbo Q8 ueber App-Binary, ohne Download) | pytest scripts/bench/tts gruen, asr --engine app-cli mit Attrappe | abgenommen | 3a035ed4 |
| D1 | M4 | Doku adapter-contract, installation-troubleshooting, licenses (docs/tts) | Dateien vorhanden, Fundstellen stimmen | abgenommen | 178d18e0 |
| P4n | M2 | Playwright-Attrappe aus echter `--tts-providers --json`-Ausgabe ableiten (Phonon CPU/satzweise/0.4.2, Piper unverifiziert), Screenshot neu | Spec gruen, Screenshot zeigt reale Deskriptoren | abgenommen | 328d747b |
| D2 | M4 | Doku: CLI-Schalter, Einbindung (adapter-contract Abschnitt 11), README-Verweis, docs/tts/CHANGELOG.md | Platzhalter <!-- P3 --> aufgeloest, README verlinkt | abgenommen | 14327484 |
| P7n | M4 | Nacharbeit Review: B6, B7, B8, B9 (Widerruf live, Instanzbindung, Ollama-Probe, Phonon-Stimme offline, Cache-Tag, Fish-Bench-Zuschlag, Fristen, Ergebnis-Fallback) | Regressionstests je Befund gruen, Filter gruen | abgenommen | c21d9fab |
