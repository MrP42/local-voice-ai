# Pakete — Besprechungen auf Granola-Niveau - lokal, ohne Abo (Goal: C:/Users/wolff/local-voice-project/koordination/granola-besprechungen/GOAL.md)

Status: offen | in_arbeit | geliefert | abgenommen | nacharbeit | abgebrochen.
**Nur der Planer schreibt diese Datei** (Zuweisung, Status, Commit). Worker bearbeiten ausschließlich
das ihnen zugewiesene Paket und melden `STATUS: DONE | PARTIAL | BLOCKED` im Report.
Nacharbeit = neues Paket mit Bezug (z. B. P2n), kein Rücksprung. Jede Zeile beginnt mit `| P`.

| ID | M | Paket | Abnahmekriterium | Status | Commit |
|---|---|---|---|---|---|
| P0 | M0 | Analyse-Artefakt (HTML) aus Recherche + Matrix + Roadmap — Sonnet | HTML-Datei ≤ 80 KB, alle 30 Matrixzeilen, Stack-Tabelle, M0–M7; veröffentlicht (Link in GOAL.md) | abgenommen | 91217fa |
| P1 | M1 | Entwurf Notizblock + KI-Notizen + Vorlagen (Datenmodell, Prompt/Schema mit Quellbelegen, UI, Eval, Paketschnitt) — lv-architect | `entwurf/m1-notizen.md` mit 4–6 Coder-Paketen, je Scope + Akzeptanzbefehl | in_arbeit | |
| P2 | M2 | Spike + Entwurf Audio/STT (AEC, VAD, Live-Latenz, Enddurchlauf, Ausfallwächter, Benchmark) mit Messungen auf dieser Maschine — lv-architect | `entwurf/m2-audio-stt.md` mit Messtabelle + 4–6 Coder-Paketen; Spike-Code außerhalb des Repos | in_arbeit | |
| P1a | M1 | Fundament: Migration (Index 2), Modell, Store-Funktionen, 8 Vorlagen (Entwurf §3/§4/§7) — lv-coder-xhigh, wt-m1 | `cargo test --lib meetings::` grün inkl. migration_keeps_legacy_rows, builtin_templates_seed_idempotently, notes_revision_conflict_rejects_write, all_builtins_validate | in_arbeit | |
| P1b | M1 | KI-Notizen-Motor: llm_call.rs, enhance.rs, assemble.rs, Commands + Event (§4-§6) — lv-coder-xhigh | `cargo test --lib meetings::notes::` ≥ 25 Tests grün; `meetings::minutes` grün wie vorher | offen | |
| P1c | M1 | Notizblock + Vorlagen-UI, position_ms (B2), Commands (§5, §9) — lv-coder | tsc ok; Playwright meeting-notes.spec.ts -g "Notizblock|Vorlagen" grün; meetings::recorder grün | offen | |
| P1d | M1 | KI-Notizen-Ansicht: Quelle→Transkript+Audio, Bearbeiten, Anweisung, Checkliste — lv-coder | Playwright -g "KI-Notizen" grün | offen | |
| P1e | M1 | Eval AK3: 3 Fixtures, eval.rs, --eval-notes — lv-coder | Stub-Tests grün; Release-Lauf mit lokalem Modell: user_preserved 1.0, ai_sourced ≥ 0.95 | offen | |
| P1f | M1 | Einstellungen, Auto-Lauf nach Stopp, Systemton-Default, Hinweistext — lv-coder | `cargo test --lib settings` grün; Playwright -g "Hinweis|Systemton" grün | offen | |
