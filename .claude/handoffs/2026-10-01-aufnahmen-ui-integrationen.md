# Handoff – Local Voice AI: Aufnahmen-UI (#64), Integrationen/YouTube (#66), Granola (#59) – 01.10.2026

## Stand in einem Satz
Aufnahmen-Oberfläche (#64, M1–M6) ist fertig (Installer 0.20.10 gebaut), YouTube-Bündel 1 (#66: A1/A1n/A2/A3) ist fertig und committet; laufend: U7 (Warteschlange + Metadaten, wt-u7) und U8 (Sprechernamen, wt-u8); danach Integrationsschritt I1 → Installer 0.20.11, dann U9 (Projekt-Protokoll).

## Laufende Worker (Ergebnisse abwarten, dann selbst prüfen)
- U7 `.claude/worktrees/wt-u7` Branch `feat/aufnahmen-u7`: Import-Warteschlange, „Gleichzeitige Transkriptionen 1–3“ mit RAM/VRAM-Gate, Metadaten (Titel, Beschreibung, Datum, Teilnehmende, Projekte). Eigene Migration → beim Merge umnummerieren.
- U8 `.claude/worktrees/wt-u8` Branch `feat/aufnahmen-u8` (basiert auf u7-Docs): Sprecher benennen überall, Namensvorschlag aus Anrede, „Mein Name“.

## Branches / Stände
- `feat/granola-besprechungen` (PR #63 gegen `chore/0.20.3-abnahme`, enthält #56–#58 und #62): Granola fertig, Installer 0.20.9; Goal #59 BLOCKED nur auf Patricks Abnahme (AK11). Merge-Reihenfolge #58 → #62 → #63.
- `feat/aufnahmen-ui` (wt-aui): M2–M6 + Version 0.20.10 (Installer `wt-aui/apps/local-voice/src-tauri/target/release/bundle/nsis/Local Voice AI_0.20.10_x64-setup.exe`).
- `feat/integrationen` (wt-int): enthält aufnahmen-ui M2–M6 + A1/A1n/A2/A3 (Migration 5 Register/Provenienz, 6 Fassungen).
- `fix/ram-vram-beenden` (wt-m1, Commit 1ab37a90): Leerlauf-Stopp llama-server, Piper im Job-Objekt, App-Anteil RAM/VRAM in Fußleiste (neues Feld `GpuMemory.luid`).

## Nächste Schritte
1. U7/U8 abnehmen (Diff lesen, Tests selbst).
2. Integrationspaket I1 (Worker): `feat/integrationen` + `feat/aufnahmen-u7` + `feat/aufnahmen-u8` + `fix/ram-vram-beenden` zusammenführen; Migrationen umnummerieren; B5 reparieren (Flaky: `search::index unfiled_filter_lists_meetings_without_a_living_folder`, `llm::server … restart_is_allowed_again_after_the_cooldown`); bindings.ts regenerieren (`cd apps/local-voice/src-tauri && ./target/debug/local-voice-ai.exe --list-models`); volle Tests; Version 0.20.11; Installer (`pwsh apps/local-voice/scripts/dev.ps1 bundle` mit VULKAN_SDK=C:\VulkanSDK\1.4.357.0). Vorher installierte Version prüfen (0.20.9 bzw. 0.20.10) – nie zurückstufen, nie selbst installieren.
3. Budget-Meldung an Patrick vor U9: Rahmen 5,55 MTok, verbraucht ~4,85, Hochrechnung ~6,1.
4. U9 Projekt-Protokoll (mehrere Aufnahmen eines Projekts per Häkchen gemeinsam protokollieren).
5. Offene Befunde: #66 B4 Farbkontrast app-weit (axe color-contrast, eigenes Paket mit Freigabe), #59 B10 Lizenzfunde (mp3lame LGPL, Canary NC).
6. Danach: #66 Bündel 2 (Seite Integrationen, Rechte-Matrix, Mail/OneDrive/Obsidian/RAG, MCP/CLI steuernd), #67, #68, #69 (Bild/Video-Inhalte).
7. Frage an Patrick offen: ältere Issues (#3, #5, #6, #8–#11, #15, #29) durchgehen?

## Wo alles steht
- Goals: `koordination/{granola-besprechungen,aufnahmen-ui,integrationen,workflow-automation,lokaler-agent}/GOAL.md, PAKETE.md, BEFUNDE.md` (je auf ihrem Branch; neueste aufnahmen-ui-Docs auf `feat/aufnahmen-u8`).
- Prototyp Aufnahmen: https://claude.ai/artifact/3RaBLbuyHDX1WnQtAvMQJS · Granola-Analyse: https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo
- Issues: #59, #61, #64–#69 am 01.10. aktualisiert.
- Worker-Regeln: `.claude/agents/lv-coder.md`.

## Fallen
- Installer nie über Patricks App installieren; vor Build installierte Version und offene höhere PR-Versionen prüfen (Vorfall 30.09. mit 0.20.7).
- Plattenplatz C: nach Builds prüfen; Build-Caches fertiger Worktrees löschen (heute 116 GB freigeräumt).
- `--list-models` nur aus `apps/local-voice/src-tauri` (sonst entsteht `apps/src/bindings.ts`).
- Tauri fängt Datei-Drops fensterweit ab (onDragDropEvent statt HTML5).
- `agent-*`-Worktrees gehören anderen Sessions – nicht anfassen.

## Empfohlene Skills
- `goal-planner-worker` (Wiederaufnahme: `python ~/.claude/skills/goal-planner-worker/scripts/goal.py status --thema aufnahmen-ui` bzw. `integrationen`)
- `superpowers:verification-before-completion` vor Abnahmen
- `artifact-design` bei neuen Statusseiten
