# Handoff – Local Voice AI: Vorlesen-Oberfläche einheitlich, kompakt, anpassbar (29.09.2026)

## Stand in einem Satz
Goal `ui-vorlesen-kompakt` (Issue #61) ist umgesetzt und belegt: **PR #62** (<https://github.com/MrP42/local-voice-ai/pull/62>) auf Branch `feat/ui-vorlesen-kompakt`, gestapelt auf #58 (`chore/0.20.3-abnahme`); Version 0.20.4; Vorher/Nachher-Seite <https://claude.ai/artifact/USSYtsBp5gUgs2eUxUPyRw>; Installer `Local Voice AI_0.20.4_x64-setup.exe` unter `.claude/worktrees/wt-ui/apps/local-voice/src-tauri/target/release/bundle/nsis/`.

## Wo alles steht (nicht wiederholen)
- Goal State: `koordination/ui-vorlesen-kompakt/{GOAL,PAKETE,BEFUNDE,UI-AUDIT}.md`, Bilder `screens/vorher|nachher`. `python ~/.claude/skills/goal-planner-worker/scripts/goal.py status --thema ui-vorlesen-kompakt` (aus dem Worktree `wt-ui`).
- Pakete P0–P9 mit Commit in PAKETE.md; Befunde B1–B4 (alle erledigt) in BEFUNDE.md.

## Was Patrick tun muss
1. Installer 0.20.4 installieren, Vorlesen-Seite durchklicken (Symbolzeile + ☰, Tooltips, Umschalter gestapelt/nebeneinander im Kopf der Dateileiste, Ziehgriffe, neue Aufnahme → Name `Stimme_Datum_Uhrzeit`, Palette „Alle“).
2. In PR #62 die „Human Written Description“ ausfüllen; Merge-Reihenfolge: #56, #57 → #58 → #62 (GitHub setzt die Basis von #62 nach dem Merge von #58 auf `main`, sonst von Hand umstellen).
3. Entscheidungen E1–E3 (GOAL.md) bestätigen oder ändern: Dateiname aus Stimme, Standard „gestapelt“, Menüinhalt.

## Offen / Folge-Goal
- A18 (niedrig): Transport 34 px vs. Aktionen 36 px vs. Select 44 px in der Bedienspalte.
- Andere Seiten (UI-AUDIT.md, Abschnitt „Folge-Goal“): Einstellungen-Knöpfe 25 px/400, Modelle-Suchfeld 19 px, Verlauf „Aufnahmeordner öffnen“ als `sm`.
- Palette: Favoriten-Sterne ragen 12 px über die Chip-Ecke in die Zeile darüber (kosmetisch).
- Paket-Worktrees `wt-ui-p1..p9` und lokale Zweige `feat/ui-p*` können nach dem Merge weg (`git worktree remove`, nur eigene!).

## Werkzeug-Fallen (neu in dieser Session)
- Playwright/Vite: `LV_DEV_PORT=<port>` je Worktree (sonst testet `reuseExistingServer` fremde Bäume); **Ports 1448–1547 sind von Windows reserviert** (EACCES) → 16xx nehmen.
- Screenshot-Tests schreiben nur mit `SCREENS_DIR`; immer `animations: "disabled"` (Übergänge verfälschen den aktiven Zustand).
- `toBeVisible()` erkennt kein Abschneiden durch overflow-Container → `elementFromPoint` prüfen.
- Installer-Build: `cargo` nicht im PATH; `set X=6 && …` in cmd liefert „6 “ → Umgebungsvariablen in PowerShell setzen und `Start-Process` erben lassen. Kaltbau im Worktree ~20 min.

## Empfohlene Skills
- `goal-planner-worker` (Wiederaufnahme nur bei Nacharbeit aus der Abnahme), `superpowers:finishing-a-development-branch` für die Merge-Reihenfolge, `handoff` am Sessionende.
