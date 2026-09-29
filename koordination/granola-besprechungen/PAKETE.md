# Pakete — Besprechungen auf Granola-Niveau - lokal, ohne Abo (Goal: C:/Users/wolff/local-voice-project/koordination/granola-besprechungen/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| P0 | M0 | Analyse-Artefakt (HTML) aus Recherche + Matrix + Roadmap — Sonnet | HTML-Datei ≤ 80 KB, alle 30 Matrixzeilen, Stack-Tabelle, M0–M7; veröffentlicht (Link in GOAL.md) | abgenommen | 91217fa |
| P1 | M1 | Entwurf Notizblock + KI-Notizen + Vorlagen (Datenmodell, Prompt/Schema mit Quellbelegen, UI, Eval, Paketschnitt) — lv-architect | `entwurf/m1-notizen.md` mit 4–6 Coder-Paketen, je Scope + Akzeptanzbefehl | abgenommen | 4b12c80 |
| P2 | M2 | Spike + Entwurf Audio/STT (AEC, VAD, Live-Latenz, Enddurchlauf, Ausfallwächter, Benchmark) mit Messungen auf dieser Maschine — lv-architect | `entwurf/m2-audio-stt.md` mit Messtabelle + 4–6 Coder-Paketen; Spike-Code außerhalb des Repos | abgenommen | ffcbf4e |
| P1a | M1 | Fundament: Migration (Index 2), Modell, Store-Funktionen, 8 Vorlagen (Entwurf §3/§4/§7) — lv-coder-xhigh, wt-m1 | `cargo test --lib meetings::` grün inkl. migration_keeps_legacy_rows, builtin_templates_seed_idempotently, notes_revision_conflict_rejects_write, all_builtins_validate | abgenommen | 13fa4d3 |
| P1b | M1 | KI-Notizen-Motor: llm_call.rs, enhance.rs, assemble.rs, Commands + Event (§4-§6) — lv-coder-xhigh | `cargo test --lib meetings::notes::` ≥ 25 Tests grün; `meetings::minutes` grün wie vorher | in_arbeit | |
| P1c | M1 | Notizblock + Vorlagen-UI, position_ms (B2), Commands (§5, §9) — lv-coder | tsc ok; Playwright meeting-notes.spec.ts -g "Notizblock/Vorlagen" grün; meetings::recorder grün | in_arbeit | |
| P1d | M1 | KI-Notizen-Ansicht: Quelle→Transkript+Audio, Bearbeiten, Anweisung, Checkliste — lv-coder | Playwright -g "KI-Notizen" grün | offen | |
| P1e | M1 | Eval AK3: 3 Fixtures, eval.rs, --eval-notes — lv-coder | Stub-Tests grün; Release-Lauf mit lokalem Modell: user_preserved 1.0, ai_sourced ≥ 0.95 | offen | |
| P1f | M1 | Einstellungen, Auto-Lauf nach Stopp, Systemton-Default, Hinweistext — lv-coder | `cargo test --lib settings` grün; Playwright -g "Hinweis/Systemton" grün | offen | |
| P2a | M2 | DSP-Thread, VAD-Segmentierer (Silero v4), Stille-Gate/Halluzinationsfilter; neues Event TranscriptFinal am Ende von stop() (Entwurf M2 §3.1/§3.4) — lv-coder-xhigh | `cargo test --lib meetings::` grün, ≥ 12 neue Tests; Stille-Fixture per --import-meeting unverändert | in_arbeit | |
| P2b1 | M2 | Korpus: FLEURS-de ≥ 220 Sätze + synthetischer Mehrsprecher-Korpus (SAPI, Echo/Überlappung) + Satz-Benchmark über vorhandene CLI → bench.md (Modellwahl) — lv-coder, wt-m2b | `python scripts/bench/make_corpus.py --check` ok; `docs/m2-evidence/bench.md` mit WER je Modell (≥ 220 Sätze) | in_arbeit | |
| P2b2 | M2 | Harness --simulate-meeting (replay.rs), m2-bench.ps1 -Quick/-Full, Latenz p95 (AK5) — lv-coder | `m2-bench.ps1 -Quick` → JSON mit latency_p95_ms/wer_live/wer_final; AK5 latency_p95_ms ≤ 5000 | offen | |
| P2c1 | M2 | AEC-Modul echo.rs (sonora =0.2.0, Aligner mit QPC-Versatz/Drift, rein testbar) + Fixtures m2_echo_* — lv-coder-xhigh, wt-m2 | `cargo test --lib meetings::echo` grün (ERLE ≥ 20 dB nach 5 s auf Fixture, Aligner ±Versatz/Drift/Pause) | in_arbeit | |
| P2c2 | M2 | AEC-Integration: QPC-Anker in loopback.rs/mic_capture.rs, AEC im DSP-Thread, mic_aec.wav, Setting meeting_echo_cancellation — lv-coder-xhigh | --simulate-meeting Echo-Fixture: ich_far_word_leak ≤ 0.10, Baseline --no-aec ≥ 0.5 (AK6) | offen | |
| P2d | M2 | Enddurchlauf, replace_segments (Epoche+1), transcript_live.json, words, remap_sources, Recovery processing, TranscriptFinal nach Enddurchlauf — lv-coder-xhigh | `cargo test --lib meetings::` grün inkl. Altdaten; --simulate-meeting --final-model → epoch live+1; Orphan processing → ready | offen | |
| P2e | M2 | Ausfallwächter signal_watch.rs + Health-Event + Warnleiste — lv-coder | `cargo test --lib meetings::signal_watch` ≥ 6 Zustandsfolgen; Playwright meeting-health grün | offen | |
| P2f | M2 | GPU-Backend STT im Windows-Release (Vulkan) — lv-coder; Owner-Entscheidung E5 | `--list-devices` zeigt Vulkan RTX 4090; turbo rtf ≥ 30 mit bound_backend Vulkan | blockiert | |
| P4 | M4 | Entwurf + Spike Chat/Suche/Ordner/Recipes (FTS5, sqlite-vec, Embeddings über llama-server, Hybrid-RRF, Chat mit Zitaten, Eval AK8) — lv-architect | `entwurf/m4-chat-suche.md` mit Messwerten + 4–6 Coder-Paketen | in_arbeit | |
