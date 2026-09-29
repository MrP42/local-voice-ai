# Handoff – Goal „Besprechungen auf Granola-Niveau“ (29.09.2026, PAUSED wegen Limit)

## Stand in einem Satz
Goal `koordination/granola-besprechungen/GOAL.md` (Issue #59, Branch `feat/granola-besprechungen`, gepusht) steht auf PAUSED (Grund: limit); M1-Kern (Datenmodell, KI-Notizen-Motor, Notizblock-UI) und M2-Teile (Live-Pfad mit VAD, AEC-Modul) sind abgenommen und gemergt, fünf Pakete lagen beim Pausieren als ungeprüfte WIP-Commits auf eigenen Branches.

## Wiedereinstieg
1. `python ~/.claude/skills/goal-planner-worker/scripts/goal.py resume --thema granola-besprechungen` und `status`.
2. WIP-Branches (je Worktree unter `.claude/worktrees/`, Commit „wip(...) UNGEPRUEFT“) validieren: Report fehlt → Tests selbst laufen lassen (`cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib`, `npx tsc --noEmit`, `npx playwright test <spec>`), ggf. Worker neu briefen (Akzeptanz steht in `PAKETE.md`).
   - `wt-m1` → `feat/granola-p1d` (KI-Notizen-Ansicht)
   - `wt-m1c` → `feat/granola-p1f` (Einstellungen, Auto-Lauf auf TranscriptFinal, Hinweistext)
   - `wt-m2` → `feat/granola-p2f` (Vulkan-GPU-STT; Vulkan-SDK ist installiert `C:\VulkanSDK\1.4.357.0`)
   - `wt-m2b` → `feat/granola-p2b1` (FLEURS-/Synth-Korpus + Satz-Benchmark; Daten in `%LOCALAPPDATA%\lva-bench`)
   - `wt-m4` → `feat/granola-p4a` (Such-Index, Migration Index 3)
3. Merge-Ablauf: im Worktree committen → `git rebase feat/granola-besprechungen` → Konflikte `bindings.ts` mit `koordination/granola-besprechungen/tools/reapply_diff.py` (Hunks neu anwenden), `lib.rs`/`mod.rs` beide Seiten behalten → komplette Suite + tsc → im Haupt-Checkout `git merge --ff-only`.
4. Danach offene Pakete laut `PAKETE.md`: P1e (Eval AK3), P2c2, P2e, P2b2, P2d, M3-Entwurf/Spike (Diarisierung), P4b–P4f, P5a–P5f, P6a–P6e, M7.

## Wichtige Fakten
- Entwürfe: `koordination/granola-besprechungen/entwurf/{m1-notizen,m2-audio-stt,m4-chat-suche,m5-m6-kalender-export}.md`; Analyse-Artefakt https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo.
- Entscheidungen Patrick 29.09.: voller Umfang, Budget 12 MTok (Meldung 6,0/9,6, Stopp 18); Vulkan ja, Claude installiert. Vorschläge E1–E17 in GOAL.md gelten, bis er widerspricht.
- Budget Ist ~4,3 MTok. Real je Coder-Paket 250–410 kTok, je Entwurf 170–240 kTok.
- Worker-Definitionen `.claude/agents/lv-coder(-xhigh).md`, `lv-architect.md`; jeder Worker `CARGO_BUILD_JOBS=8`.
- Laufwerk C: knapp (zuletzt ~40 GB frei; jeder Worktree-target 7–12 GB). Vor Installer-Build ≥ 20 GB frei machen.
- Test-Fixtures unter `src-tauri/tests/fixtures/` sind gitignoriert → `git add -f`.
- Parallel offene PRs #56/#57/#58 (0.20.3) warten auf Patricks Merge.

## Empfohlene Skills
`goal-planner-worker` (Wiederaufnahme), `superpowers:verification-before-completion`, `handoff`.
