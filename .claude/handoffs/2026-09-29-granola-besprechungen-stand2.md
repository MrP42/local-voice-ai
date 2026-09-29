# Handoff – Goal „Besprechungen auf Granola-Niveau“ (29.09.2026 abends, Stand 2)

## Stand in einem Satz
Goal `koordination/granola-besprechungen/GOAL.md` (Issue #59, Integrationsbranch `feat/granola-besprechungen`, gepusht) läuft (EXECUTING); AK1, AK3, AK4, AK5, AK6, AK8 erfüllt, Feature-Matrix 17/22 Kern belegt (`python koordination/granola-besprechungen/check_matrix.py`), Rust-Suite 1487 grün; drei Worker liefen beim Schreiben: P5b (wt-m1c), P6b (wt-m1), P1h (wt-m2a).

## Wo alles steht (nicht wiederholen, dort lesen)
- Ziel, AK/QG, Entscheidungen E1–E23, Verlauf: `koordination/granola-besprechungen/GOAL.md`
- Pakete mit Status/Commit: `PAKETE.md`; Befunde B1–B9: `BEFUNDE.md`; Matrix: `FEATURE-MATRIX.md`
- Entwürfe: `entwurf/m1-notizen.md`, `m2-audio-stt.md`, `m3-sprecher.md`, `m4-chat-suche.md`, `m5-m6-kalender-export.md`
- Messbelege: `docs/m2-evidence/bench.md`, `koordination/granola-besprechungen/abnahme/*` (Eval-JSONs, Screenshots)
- Analyse-Artefakt: https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo
- Vorheriger Handoff (Pause-Stand mittags): `.claude/handoffs/2026-09-29-granola-besprechungen.md` (überholt)

## Offen bis COMPLETE
- Laufend: P5b (Kalender-Sync/Hinweisfenster, AK9/2), P6b (PDF, AK10/2), P1h (toter llama-server, B4). Ergebnis je Worktree prüfen: `git -C .claude/worktrees/<wt> status`, dann Tests selbst.
- Danach Kern: P6d (Export-UI, AK10/4), P7a (Playwright-Port je Worktree = B9 + bindings.ts einmal generieren), M7: Installer (Patch-Version +1, `dev.ps1 bundle` mit VULKAN_SDK), Offline-Nachweis QG5 (F27), Lizenzen/Third-Party-Notices QG6 (sonora, calcard, Sortformer, BGE-M3, FLEURS/AMI-Attribution), Performance QG3, Doku, PR gegen main, AK11-Screenshots, AK2.
- Komfort (sperrt nicht): P5d Personen, P5e Brief, P5f Graph (braucht Client-ID von Patrick, E14), P6e MCP, P3d Stimmprofile, P1g (Qwen3.5 Denkmodus/Kontext, B3), P2g (Live-Modell Parakeet GGUF, B1).
- Owner offen: **B7** (AK7: AMI-Prüfteil 15,19 % > 15 %; Vorschlag „erfüllt“ + Nemotron als Folge-Goal); E23 echte deutsche Testaufnahme; E14 Graph-Client-ID.
- Budget: Ist ~11,7 MTok von freigegebenen 12, Hochrechnung 15–16, harter Stopp 18 → bei 12 melden.

## Merge-Routine (bewährt)
1. Im Worktree: Abnahme-PNGs verwerfen (`git checkout -- koordination/granola-besprechungen/abnahme/`), `git add -A -- . ':!.superpowers'`, neue Fixtures `git add -f`, commit, push.
2. Haupt-Checkout: `git merge --squash feat/granola-<p>`; Konflikte in lib.rs/cli.rs/bindings.ts/i18n/settingsStore mit `python koordination/granola-besprechungen/tools/reapply_diff.py <datei> $(git merge-base HEAD <branch>) <branch> HEAD` (vorher `git show HEAD:<datei> > <datei>`); nicht verankerte Hunks von Hand (AppSettings-Ende in bindings.ts, i18n-Blöcke, headless-Bedingung in lib.rs `|| cli_args.<flag>...; // Mx-Py` am Kettenende).
3. `cargo test --lib`, `npx tsc --noEmit`, Playwright nur wenn kein Worker Playwright nutzt (B9); commit mit Beschreibung, PAKETE/Matrix/AK per `goal.py check`, push, Build-Cache des Worktrees löschen (`rm -rf <wt>/apps/local-voice/src-tauri/target`).

## Fallen
- Debug-EXE überschreibt `src/bindings.ts` (Regel in `.claude/agents/lv-coder.md`).
- Heredocs mit `\r\n` in Code-Patches erzeugen echte Umbrüche → Edit-Tool nehmen.
- Befehlsketten mit `&&` brechen bei `git add` mit ausgeschlossenem ignoriertem Pfad ab → Commit prüfen.
- Laufwerk C: knapp; nach jedem Merge Worktree-Cache löschen.
- Sonnet-Wochenlimit war mittags erschöpft (Reset 15:00); Worker ggf. mit `model: opus`.

## Empfohlene Skills
- `goal-planner-worker` (Wiederaufnahme: `goal.py status --thema granola-besprechungen`)
- `superpowers:verification-before-completion` vor jeder Abnahme
- `handoff` am Sessionende
