# Handoff — Local Voice AI: Skript-Editor, Skript-Werkstatt, Pakete (16.–21.09.2026)

Datum: 2026-09-21 · Session: Claude Code (VS Code) · Projekt: `C:\Users\wolff\local-voice-project`
Vorheriger Handoff: `2026-09-14-e5-e7-vorlesen-multidevice.md`.

## Stand beim Handoff (Kurzfassung)

- `main` enthält alles bis PR #52 (Stand 0.19.14); Release **app-v0.19.0** wurde am 17.09. veröffentlicht,
  **app-v0.20.0** ist der nächste Schritt (Release-Notes vorbereitet: siehe unten „Zuletzt abgeschlossen").
- Arbeitsweise seit 16.09.: jede Änderung = eigener Zweig → PR → Merge → Patch-Version +1 → lokaler
  Installer (`pwsh -File apps/local-voice/scripts/dev.ps1 bundle`); die App bietet den Installer aus dem
  lokalen Ordner an, wenn er neuer ist als GitHub. Signatur läuft lokal (Schlüssel aus `~/.tauri`).
- Tests: Rust `cargo test --lib` (683), Playwright `tests/script-check.spec.ts` (16), `voices.spec.ts`,
  `workspace.spec.ts` — zusammen 37 grün. Prettier nur auf berührte Dateien; `bindings.ts` von Hand.

## Was seit dem 16.09. entstanden ist (je PR)

| PR | Inhalt |
|---|---|
| #27–#34 | Diktat-Startlatenz, Endtext, Audio-Absenkung, lokale Updates (Helfer mit System32-Pfaden), Systemschutz, Besprechungen (Lücken statt Abbruch, Zeitstempel-Sprung, Live-Detail), Protokoll ohne leere Zusammenfassung |
| #35 | Repo vorzeigbar: READMEs, docs/README.md, Archiv |
| #36 | Update-Vergleich lokal vs. GitHub (neuere gewinnt) |
| #37, #39 | Stimmenliste kompakt, Hörprobe je Stimme/Seed, Export je Stimme, Sammel-Export, Baukasten als Dialog |
| #38, #45 | Fish-Speech startet wieder (kein Job-Speicherdeckel: CUDA reserviert 48 GB Commit; Compile-Threads auf Windows = 1) |
| #40, #43, #44 | Skript-Prüfung: Befunde, Gruppen, Empfehlungen, Prüf-Knopf, live beim Tippen (Debounce 120 ms, Zwei-Zeiger-Mapping, Zeilenindex: 181 → 3 ms bei 100 KB) |
| #47 | Historie (useTextHistory), „Überall ersetzen", Sprung-Fix, Auto-Tagging-Dialog mit Vorlagen (Settings `tts_autotag_presets`/`tts_autotag_last`), Panel dauerhaft |
| #48 | Vorlesen ab Satz / nur dieser Satz (`locate_sentences`), Vorab-Erzeugung (`prewarm`) |
| #49 | Tags auf Deutsch (`resolveTag`, `canonicalizeTags`, Setting `tts_tag_language`), drei Tag-Klassen, Reiter „Alle", Sprecher-Anzeigename = Eingabe |
| #50 | Seiten-Paket `.lvpage` (commands/pages_package.rs) mit Rechtebestätigung für Stimmen |
| #51 | Skript-Werkstatt (`src-tauri/src/books.rs`, `books/ScriptWorkshopDialog.tsx`): Bücher, Figuren, Gedächtnis-Markdown, Vorlagen, Erzeugung per JSON-Schema, `.lvbook` |
| #52 | Startton: dauerhafter Ausgabestream (audio_feedback.rs), README |

## Nicht getestet im echten Betrieb (nur Unit/Playwright mit Attrappe)

- Skript-Werkstatt Ende-zu-Ende mit echtem LLM (Prompt/Schema getestet, UI mit Attrappe).
- Seiten-/Buch-Export und -Import mit echten Dateien (Zip-Code kompiliert, kein Rundlauf-Test — nachholen:
  Rust-Test mit `tempfile` wie in `portable.rs`).
- Vorab-Erzeugung (`prewarm`) parallel zum Vorlesen am Fish-Server.
- Startton-Fix: Hörprobe durch Patrick.

## Offen / bei Patrick

- Release `app-v0.20.0` (Notes liegen bereit).
- NSIS/MSI: `targets: "all"` → nur `["nsis"]`? (Entscheidung offen.)
- Diktat und Besprechung teilen eine Modell-Instanz (Diktat verdrängte Import; jetzt per Retry abgefangen).
- Sprechertrennung aus Mehrsprecher-Audio (Diarisierung) — Plan liegt vor, nicht begonnen.
- PR #32 (Codex macOS) und #21 sind fremd/überholt; 150 `upstream/*`-Zweige aufräumen?

## Werkzeug-Erkenntnisse (siehe Memory)

- Heredocs im Bash-Tool fressen Backslashes → Patches per Write-Tool + `python <datei>`.
- Playwright: Kontextmenü per `evaluate(click)`, Toasts über `[data-sonner-toast]`.
- Während `tauri build` keine Quellen anfassen; neue, noch nicht referenzierte Dateien sind unkritisch.
