# Handoff – Goal „Besprechungen auf Granola-Niveau“ (30.09.2026, Abnahme-Stand)

## Stand in einem Satz
Goal `koordination/granola-besprechungen/GOAL.md` (Issue #59, Branch `feat/granola-besprechungen`, PR #63) ist BLOCKED nur auf Patricks Abnahme (AK11): alles gebaut und gemergt, AK 10/11, Gates 8/9 (QG9 Budget ~16,6 von 16 MTok, knapp drüber), Installer 0.20.8 inkl. #62 liegt bereit.

## Wo alles steht
- Ziel, AK/QG, Entscheidungen, Verlauf: `koordination/granola-besprechungen/GOAL.md`; Pakete `PAKETE.md`; Befunde `BEFUNDE.md` (alle erledigt, B12 Folge-Goal); Matrix `FEATURE-MATRIX.md` (22/22 Kern).
- Messbelege: `koordination/granola-besprechungen/abnahme/p7b-qg3.md`, `p7b-qg4.md`, `p7b-qg5.md`, `p1i-*.json`.
- Analyse-/Statusseite: https://claude.ai/artifact/JL7yCpwFDTcrvAm9Xj2Dwo (Stand 30.09.)
- Installer: `apps/local-voice/src-tauri/target/release/bundle/nsis/Local Voice AI_0.20.8_x64-setup.exe` (Rückweg: 0.20.6 im selben Ordner).

## Vorfall 30.09.
0.20.7 (Besprechungen OHNE #62) wurde über Patricks 0.20.6 installiert (nicht vom Agenten); UI aus #62 war weg. Behoben: #62 per echtem Merge in `feat/granola-besprechungen` (`988879cf`), Version 0.20.8, 1748 Rust / 306 Playwright grün; 0.20.7-Installer gelöscht. Lehre: Vor jedem Installer-Build prüfen, welche Version installiert ist und ob offene PRs mit höherer Version fehlen.

## Nächste Schritte
1. Patrick installiert 0.20.8 und prüft: Aufnahme mit Notizen → KI-Notizen → Chat; Hinweisfenster; formatierte Zwischenablage; Echo-Test mit Lautsprechern; optional Graph mit eigener Client-ID.
2. Danach: Screenshots nach `abnahme/`, `goal.py resolve-blocker --id B3`, `check --ak 11`, QG9 mit Patricks Einordnung, `goal.py complete`.
3. Merge-Reihenfolge: #58 → #62 → #63 (bei Patrick).
4. Folge-Goal: Stimmprofile (P3d), Nemotron-Diarisierung, Protokoll-Pfad mit P1i-Blocklogik (B12), echte deutsche Testaufnahme; vorbestehend B10 (mp3lame LGPL, Canary NC).

## Empfohlene Skills
- `goal-planner-worker` (Wiederaufnahme: `goal.py status --thema granola-besprechungen`)
- `superpowers:verification-before-completion` vor `goal.py complete`
- `handoff` am Sessionende
