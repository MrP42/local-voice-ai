---
thema: ui-vorlesen-kompakt
titel: Vorlesen-Oberflaeche: einheitlich, kompakt, anpassbar
state: PAUSED
vorzustand: EXECUTING
pausengrund: limit
issue: 61
repo: MrP42/local-voice-ai
branch: feat/ui-vorlesen-kompakt
iteration: 1
erstellt: 2026-09-29
aktualisiert: 2026-09-29T12:50
---

# Goal: Vorlesen-Oberflaeche: einheitlich, kompakt, anpassbar

## Zielzustand
Die Vorlesen-Seite hat rechts neben dem Text nur noch EINE Spalte: oben eine kompakte Bedienung (Transport, Stimme, eine Zeile reiner Symbol-Knöpfe mit Tooltip, Seltenes hinter einem Menü), darunter Dateien/Hilfe – wahlweise wie bisher nebeneinander; Spaltenbreiten sind ziehbar und bleiben erhalten, erzeugte Audios heißen kurz und unterscheidbar, und „Ausdruck & Sprechstil“ scrollt statt den Text zu überdecken. Erkennbar an den Playwright-Belegen unten, an Vorher/Nachher-Screenshots (Artefakt) und am Abnahme-Installer 0.20.4.

## Scope
- UI-Audit der Vorlesen-Seite (Seitenliste, Editor mit Reitern, Ausdruck & Sprechstil, Bedienspalte, Dateien/Hilfe): Abweichungen vom App-Standard (Knopfgrößen, Symbole, Schriftgrößen, Abstände, Tab-Stile) mit Fundstelle → `UI-AUDIT.md`; Befunde der Schwere „hoch“ und „mittel“ auf dieser Seite beheben.
- Bedienspalte: Aktionen als Symbol-Knöpfe in einer Zeile (je Reiter Original/Übersetzung/Zusammenfassung), Seltenes im Menü „☰“, Tooltip mit Name + Kurzerklärung (Maus und Tastaturfokus), je Aktion ein eigenes Symbol, einheitliche Größe.
- Layout: rechte Spalte gestapelt (Standard) oder nebeneinander (bisher), Umschalter in der Spalte selbst; linke und rechte Spalte per Ziehgriff in der Breite verstellbar, dauerhaft gespeichert.
- Dateiname erzeugter Audios kürzer und eigenständig; Dateiliste zeigt bei langen Namen Anfang UND Zeitstempel.
- Fehler: „Ausdruck & Sprechstil“ → „Alle“ überlagert den Text.
- Tests (Playwright gegen die Tauri-Attrappe, Unit), Hilfe-Text der Seite, Abnahme-Installer.

## Non-Scope
- Andere Seiten (Verlauf, Besprechungen, Modelle, Einstellungen): Audit-Befunde dort nur als Folge-Goal notieren.
- Rust-Backend (Export-Logik, Satz-Cache) – nur der vorgeschlagene Dateiname ändert sich.
- Granola-Goal (`koordination/granola-besprechungen`, Worktrees wt-m*), offene fremde PRs #21, #32, #56–#58, #60.
- Umbenennen bestehender Audiodateien auf der Platte.
- Neuer Seitenleisten-Eintrag oder neuer Einstellungsreiter (Memory „Einstellungen am richtigen Ort“).

## Akzeptanzkriterien
- [x] AK1 — Audit: `koordination/ui-vorlesen-kompakt/UI-AUDIT.md` listet jede Abweichung als *Element · Ist · Standard · Fundstelle · Schwere · Status*; jede Zeile „hoch“/„mittel“ trägt Status „behoben (Paket/Commit)“, Rest „Folge-Goal“.
- [x] AK2 — Einheitliche Aktionen: Playwright-Test `tests/readaloud-toolbar.spec.ts` → alle Aktionsknöpfe der Bedienspalte (`[data-testid^="tts-action-"]`) haben gleiche Höhe und Breite (±1 px), kein sichtbarer Beschriftungstext, jedes Symbol (`svg.lucide-*`-Klasse) kommt genau einmal vor; Auto-Tagging ist nicht mehr niedriger als der Rest.
- [x] AK3 — Eine Zeile + Menü: im Reiter Original stehen in einer Zeile (gleiche `top` ±2 px) Hinzufügen (zuerst), Diktieren, Als Audio speichern, Änderungen vorab erzeugen, Menü; das Menü enthält Skript-Werkstatt, Text aufbereiten, Skript prüfen, Auto-Tagging und löst jede davon aus; der Fehlerzähler der Skriptprüfung ist am Menüknopf sichtbar. Übersetzung/Zusammenfassung zeigen ihre Aktion ebenfalls als Symbol.
- [x] AK4 — Tooltip: Hover UND Tastaturfokus auf jeden Aktionsknopf zeigen Name + Kurzerklärung (`role="tooltip"`, per `aria-describedby` verbunden); `aria-label` trägt den Namen.
- [x] AK5 — Kompakt gestapelt: Viewport 1920×1050 und 1366×768 im Layout „gestapelt“ → Höhe des Bedienblocks `[data-testid="tts-controls"]` ≤ 50 % von `window.innerHeight`, Dateien/Hilfe liegen darunter in derselben Spalte; es gibt rechts vom Editor genau eine Spalte.
- [x] AK6 — Umschaltbar: Umschalter gestapelt/nebeneinander in der rechten Spalte; Wahl übersteht Neuladen (localStorage); „nebeneinander“ entspricht dem bisherigen Aufbau.
- [x] AK7 — Ziehbare Spalten: Griffe zwischen Seitenliste|Editor und Editor|rechter Spalte (`role="separator"`, Pfeiltasten, Doppelklick = Standard) ändern die Breite in Grenzen; Breite übersteht Neuladen; Test belegt beides.
- [x] AK8 — Dateiname: `tests/exportName.spec.ts` → neuer Name `<Stimme>[-<Zusatz>]_<JJJJ-MM-TT_HHMM>.<ext>` (z. B. `Patrick_2026-09-29_1736.wav`, `Skript_…` bei Skript-Stimmen, `-EN` im Reiter Übersetzung, `-Zusammenfassung`), Stamm ohne Zeitstempel ≤ 24 Zeichen, Windows-sicher; der Speichern-Dialog schlägt ihn vor.
- [x] AK9 — Dateiliste: ein langer Altname (`CASE-GESPRÄCH-IE2S-…_2026-09-28_1736.wav`) zeigt sichtbar Anfang, „…“ und `2026-09-28_1736`; voller Name im Tooltip.
- [x] AK10 — Palette-Fehler: Playwright → „Ausdruck & Sprechstil“ öffnen, „Alle“ wählen → Klappbereich ist in der Höhe begrenzt und scrollt (`scrollHeight > clientHeight`), Editor-Box ≥ 160 px hoch und überschneidet sich nicht mit dem Klappbereich (Screenshot-Beleg).
- [ ] AK11 — Anfassbar: Vorher/Nachher-Screenshots (gestapelt, nebeneinander, Menü offen, Tooltip, Palette „Alle“) als Artefakt-Link; Installer `Local Voice AI_0.20.4_x64-setup.exe` gebaut.

## Quality Gates
- [x] QG1 — Typen: `cd apps/local-voice && pnpm exec tsc --noEmit` → Exit 0.
- [x] QG2 — Gesamte Playwright-Suite: `cd apps/local-voice && pnpm exec playwright test --reporter=line` → keine neuen Fehlschläge gegenüber der Basislinie (Basislinie in Evidence).
- [x] QG3 — Lint/Format nur berührte Dateien: `pnpm exec eslint <Dateien>` 0 Fehler, `pnpm exec prettier --check <Dateien>` grün (vorbestehendes Rot anderer Dateien bleibt, AGENTS.md).
- [x] QG4 — i18n: neue Schlüssel in `de` und `en` vorhanden (Parität der neuen Schlüssel per Skript), keine hartcodierten deutschen/englischen UI-Texte.
- [x] QG5 — Rust unberührt außer Versionsdateien: `git diff --stat origin/chore/0.20.3-abnahme..HEAD -- apps/local-voice/src-tauri` zeigt nur `Cargo.toml`/`tauri.conf.json`(/`Cargo.lock`).
- [x] QG6 — Git: Commit je abgenommenem Paket, Branch gepusht, PR gestapelt auf #58 (Basis `chore/0.20.3-abnahme`), kein Push auf `main`, keine Formatierläufe über fremde Dateien.
- [x] QG7 — Doku + Handoff: Hilfe-Abschnitt „vorlesen“ passt zur neuen Bedienung; Handoff `.claude/handoffs/2026-09-29-ui-vorlesen-kompakt.md`.
- [x] QG8 — Budget: ≤ 1,8 MTok geschätzt; Zwischenstand bei 50 %/80 %, harter Stopp bei 150 % (2,7 MTok).

## Constraints
- Geteilter Baum: eigener Worktree `.claude/worktrees/wt-ui`, Paket-Worktrees `wt-ui-p*`; nie `git stash`, nie `git add -A` am Repo-Root, nie `reset --hard` auf fremde Zweige.
- Tests je Worktree auf eigenem Port (`PW_PORT`), sonst testet Playwright per `reuseExistingServer` den Vite-Server eines anderen Worktrees.
- Design-System der App: Tokens/Klassen aus `App.css` (`.mbtn`, `Button`, `text-text/..`), lucide-Symbole, gelber Primärton nur für die eine Hauptaktion (Abspielen).
- Tag-Einfügen per Cursor bleibt: Palette gehört unter das Textfeld (Entscheidung Patrick 14.09.).

## Architekturprinzipien
- Kleinster Diff: Bedienleiste als eigene Komponente aus `TtsSettings.tsx` herauslösen statt die 2.400-Zeilen-Datei weiter aufzublähen; Verhalten (Handler, Zustände) bleibt, nur die Darstellung ändert sich.
- Persistenz über den vorhandenen `usePersistentState` (localStorage), keine neuen Backend-Settings.
- Barrierefrei: Tastatur (Tab, Enter, Pfeiltasten, Esc), `aria-*`, Fokus sichtbar.

## Dependencies
- Basis `origin/chore/0.20.3-abnahme` (PR #58 = #56 + #57); PR dieses Goals wird darauf gestapelt.
- `node_modules` je Worktree per `pnpm install --frozen-lockfile --prefer-offline`.

## Risiken / Owner-Entscheidungen
- E1 Dateiname (Vorschlag gewählt, änderbar): `<Stimme>[-Zusatz]_<Zeitstempel>` statt Projekttitel – der Ordner gehört ohnehin zur Seite.
- E2 Standard-Layout (Vorschlag gewählt, änderbar): „gestapelt“; Umschalter in der rechten Spalte, kein Einstellungsreiter.
- E3 Menüinhalt (Vorschlag gewählt): Werkstatt, Text aufbereiten, Skript prüfen, Auto-Tagging ins Menü; Hinzufügen, Diktieren, Speichern, Vorab erzeugen in der Zeile.
- R1 Konflikte in `TtsSettings.tsx` zwischen parallelen Paketen → Paketgrenzen je Codebereich festgelegt, Merge durch den Planer.

## Meilensteine
| M | Ergebnis (anfassbar) | Status |
|---|---|---|
| M1 | UI-AUDIT.md + Vorher-Screenshots | offen |
| M2 | Symbolleiste mit Menü/Tooltips, Palette-Fix, Dateinamen – Screenshots | offen |
| M3 | Gestapeltes Layout + ziehbare Spalten – Screenshots | offen |
| M4 | Installer 0.20.4 + Artefakt Vorher/Nachher, PR offen | offen |

## Evidence
- 2026-09-29T12:09 AK8 erfüllt — exportName.spec.ts (11 Tests inkl. Backslash, Stamm<=24) gruen; readaloud-files.spec.ts: Speichern schlaegt Patrick_<stempel>.wav / Skript_... vor; Integration 86 passed, c0da046
- 2026-09-29T12:09 AK9 erfüllt — readaloud-files.spec.ts: Altname CASE-GESPRAECH-...: Schwanz _2026-09-28_1736.wav sichtbar innerhalb der Zeile, title=voller Name; 86 passed, c0da046
- 2026-09-29T12:09 AK10 erfüllt — readaloud-palette.spec.ts 4 passed (1920x1050: Editor 488px, Klappbereich 356px=40%, scrollt; 1366x768 ebenso), Screenshot screens/p3/palette-alle-1920.png; 86 passed, 534f66c
- 2026-09-29T12:12 AK6 erfüllt — readaloud-layout.spec.ts: Umschalter gestapelt->nebeneinander, Neuladen behaelt Wahl; Integration 96 passed, cb9501d
- 2026-09-29T12:12 AK7 erfüllt — readaloud-layout.spec.ts: resize-pages/resize-right ziehen, Pfeiltaste +-16, Neuladen behaelt Breite, Doppelklick=Standard, Grenzen; Editor>=358px bei 1280; Integration 96 passed, cb9501d
- 2026-09-29T12:22 AK2 erfüllt — readaloud-toolbar.spec.ts (a)(b): alle tts-action-Knoepfe 36x36 +-1 ohne Text, jede lucide-Klasse hoechstens einmal in allen 3 Reitern; Integration 110 passed/6 skipped, 2c9f6fd
- 2026-09-29T12:22 AK3 erfüllt — readaloud-toolbar.spec.ts (c)(d)(f): add,dictate,save,prewarm,menu eine Zeile, add zuerst; Menue loest Werkstatt/Pruefen/Auto-Tag/tidy aus; Badge am Menue; Uebersetzen/Zusammenfassen als Symbol; screens/p2/menue-offen.png; 110 passed, 2c9f6fd
- 2026-09-29T12:22 AK4 erfüllt — readaloud-toolbar.spec.ts (e): Tooltip bei Hover (400 ms) und Tastaturfokus, role=tooltip, aria-describedby, Esc schliesst; screens/p2/tooltip.png; 110 passed, 2c9f6fd
- 2026-09-29T12:34 AK5 erfüllt — readaloud-layout.spec.ts AK5-Test (P8): gestapelt 1920x1050 und 1366x768 mit Text: tts-controls <= 0,5*innerHeight UND scrollHeight <= clientHeight+1, tts-files darunter; Bild p5/gestapelt nach P2: Bedienung ~175 px; 115 passed, c8176e0
- 2026-09-29T12:34 AK1 erfüllt — UI-AUDIT.md: 24 Befunde mit Messwert+Fundstelle, Statusspalte: alle 2 hoch + 12 mittel behoben (P2-P8 mit Commit), A18 (niedrig) teilweise -> Folge-Goal; 3448e46
- 2026-09-29T12:48 QG1 erfüllt — pnpm exec tsc --noEmit -> Exit 0 auf fb3c79f/fdf153d
- 2026-09-29T12:48 QG2 erfüllt — playwright test (LV_DEV_PORT=1610) -> 119 passed, 10 skipped, 0 failed (Basislinie 72 passed); mit SCREENS_DIR 129 passed
- 2026-09-29T12:48 QG3 erfüllt — prettier --end-of-line auto --check auf 28 beruehrte Dateien gruen; eslint beruehrte src-Dateien Exit 0
- 2026-09-29T12:48 QG4 erfüllt — 23 neue de-Schluessel, alle auch in en (Paritaetsskript); Texte ueber i18n
- 2026-09-29T12:48 QG5 erfüllt — git diff --stat origin/chore/0.20.3-abnahme -- src-tauri: nur Cargo.lock, Cargo.toml, tauri.conf.json (Version)
- 2026-09-29T12:48 QG6 erfüllt — Commit je Paket (P1-P9), Branch feat/ui-vorlesen-kompakt gepusht, PR #62 gestapelt auf #58, kein Push auf main
- 2026-09-29T12:48 QG8 erfüllt — Worker ~1,23 MTok + Planer ~0,6 MTok = ~1,85 MTok bei Schaetzung 1,8 (103 %), unter hartem Stopp 2,7; Meldungen bei ~47 % und 86 % abgegeben
- 2026-09-29T12:49 QG7 erfüllt — Hilfe vorlesen.de/en.md an Menue angepasst (P2, 2c9f6fd); Handoff .claude/handoffs/2026-09-29-ui-vorlesen-kompakt.md; GLOBAL.md fortgeschrieben

## Blocker
-

## Entscheidungen
- 2026-09-29 E1–E3 als Vorschlag gewählt (siehe Risiken), Basis `chore/0.20.3-abnahme`.

## Nächste empfohlene Aktion
Installer pruefen: .claude/worktrees/wt-ui/apps/local-voice/src-tauri/target/release/bundle/nsis/Local Voice AI_0.20.4_x64-setup.exe; falls fehlt: PowerShell $env:CARGO_BUILD_JOBS='6'; PATH+=~/.cargo/bin; pnpm exec tauri build --bundles nsis. Dann goal.py check --ak 11 --done, goal.py complete

## Verlauf
- 2026-09-29T11:49 DISCOVERY — Goal State angelegt
- 2026-09-29T11:55 DISCOVERY (Runde 0) — Metadaten: issue=61
- 2026-09-29T11:58 READY (Runde 0) — Goal definiert, Issue #61, Basislinie 72/72
- 2026-09-29T11:58 PLANNING (Runde 1) — P1-P5 geschnitten
- 2026-09-29T11:58 EXECUTING (Runde 1) — P1 (Opus) und P2-P5 (Sonnet) laufen parallel in wt-ui-p1..p5
- 2026-09-29T12:14 EXECUTING (Runde 1) — P1,P3,P4,P5 abgenommen (Integration 96 passed). P2 laeuft; P6/P7 aus Audit gestartet. Budget ca. 0,85 MTok (~47 %)
- 2026-09-29T12:34 EXECUTING (Runde 1) — P1-P8 integriert (115 passed). B4 Menue abgeschnitten -> P9 laeuft. Budget ~1,55/1,8 MTok (86 %)
- 2026-09-29T12:50 PAUSED (Runde 1) — Alles gemergt/gepusht, PR #62, Artefakt veroeffentlicht; nur AK11 (Installer) offen - Build lief (Log scratchpad/build-0.20.4.log) [Pause: limit]

