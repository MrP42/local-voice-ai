# Pakete — Modulare TTS-Anbieter, Phonon/Ollama-Speech und reproduzierbarer deutscher TTS-Benchmark (Goal: D:/wt/local-voice-project/tts-provider/koordination/tts-provider/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| P1 | M1 | Anbietervertrag, Registry, Fehlerklassen, Cancel, Mock, Vertragstest-Satz, Hüllen Fish/Piper | `cargo test --lib managers::tts::providers` grün, bestehende `managers::tts` grün | in_arbeit | |
| P5 | M3 | Korpus de-v1 (≥100, 10 Kategorien), Normalisierung, Harness scripts/bench/tts, Hörtest-Paket, pytest | `pytest scripts/bench/tts` grün, Korpus-Schema-Test | in_arbeit | |
| P2 | M1 | Adapter phonon (ptts.exe), openai-speech (HTTP, PCM-Chunks), ollama-speech (Version/Plattform) | Vertragstest-Satz mit Fake-EXE/Fake-Server grün | offen | |
| P3 | M1 | Einstellungen tts_providers + Guardrails, Tauri-Befehle, EngineImpl::Provider, CLI, Agentenbrücke | `--tts-providers --json` ≥5 Anbieter; Guardrail-Tests grün | offen | |
| P4 | M2 | UI-Gruppe im Reiter Ausgabe, Probestimme + Abbrechen, Playwright + Screenshot, i18n de/en | `tts-providers.spec.ts` grün + Screenshot | offen | |
| P6 | M3 | Messlauf seriell + Ergebnisse + Doku docs/tts | docs/tts/benchmarks/<datum>/ JSON/CSV/MD | offen | |
| P7 | M4 | Unabhängiges Review | keine kritischen Befunde offen | offen | |
