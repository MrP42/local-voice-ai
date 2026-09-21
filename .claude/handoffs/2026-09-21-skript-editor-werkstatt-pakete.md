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

## Laufender Faden beim Handoff (21.09., Sessionende)

- Tag `app-v0.20.0` gepusht (PR #53 gemergt, `main` = 0.20.0). Release-Workflow „Release (Windows)"
  Lauf 35556342298 lief beim Handoff noch; danach: `gh release edit app-v0.20.0 --title "Local Voice AI 0.20.0"
  --notes-file <Notes>` — die Notes stehen unten unter „Release-Notes 0.20.0" (auch in
  `scratchpad/release-notes-0.20.0.md`, Temp wird gelöscht). Prüfen, dass `latest.json`, `.exe` und `.sig`
  im Release liegen; macOS-Lauf hängt seine DMGs später an.
- Lokaler Installer 0.19.14 liegt im Bundle-Ordner; 0.20.0 kommt über GitHub (App zeigt die neuere).
- Goal-Hook „alles umsetzen testen und optimieren" war aktiv; alle Blöcke sind umgesetzt, offen nur die
  Punkte unter „Offen / bei Patrick".

## Empfohlene Skills für die nächste Session

- `superpowers:verification-before-completion` vor jeder Erfolgsmeldung (Tests wirklich laufen lassen).
- `fragGPT` für Architekturfragen (hat am 21.09. die Live-Prüfung sauber entschieden).
- `code-review` (Level high) über `src-tauri/src/books.rs` und `commands/pages_package.rs` — beide ohne
  Rundlauf-Test im Echtbetrieb.
- `feature-dev:feature-dev` für die Sprechertrennung (Diarisierung), falls Patrick sie freigibt.

## Release-Notes 0.20.0 (Vorlage)

Alle Arbeiten vom 17. bis 21. September 2026 seit 0.19.0 (PR #36 bis #52).

## Vorlesen: Skript-Editor

- **Skript-Prüfung live beim Tippen:** unbekannte Sprecher und Tags werden rot gewellt unterstrichen, mit Randmarke je Zeile. Das Korrekturpanel öffnet per „Skript prüfen" (mit Zähler), zeigt eine Gruppe nach der anderen (ein Problem, alle Stellen, durchklickbar) und macht Empfehlungen: Tippfehler bei Sprechern, Synonyme bei Tags. Ein Klick wirkt auf alle Stellen. Bleibt offen, bis man es schließt. 100 KB Text werden in 3 ms geprüft.
- **Historie:** Rückgängig/Wiederherstellen per Knopf und Strg+Z/Strg+Y — auch für Auto-Tagging, Aufbereiten und Ersetzen.
- **Überall ersetzen:** Rechtsklick auf Wort, Sprecher oder Tag → Suchen-und-Ersetzen im ganzen Text (Trefferzahl, Groß/Klein, ganze Wörter).
- **Vorlesen ab Satz / nur diesen Satz** aus dem Kontextmenü; der Sprecherkontext bleibt erhalten.
- **Änderungen vorab erzeugen:** geänderte Sätze im Hintergrund in den Cache, auch während des Vorlesens; danach spielt der Text bei beendetem Server.
- **Tags auf Deutsch:** `[ruhig]`, `[calm]` und `[Relaxed]` sind dasselbe Tag; Einstellung „Sprache der Tags im Text" (auto/de/en). Drei Klassen einheitlich: dokumentiert (gelb), erweitert (bernstein), unbekannt (rot) — mit Legende und Reiter „Alle" in der Palette.
- **Auto-Tagging-Dialog:** Umfang, bevorzugte Tags je Kategorie, Stil-Hinweis; je Seite gespeichert, Vorlagen app-weit, zuletzt benutzt als Vorbelegung.
- **Sprecher:** der eingetippte Name (mit Umlauten) ist der Anzeigename; die technische Kennung bleibt im Hintergrund.

## Skript-Werkstatt (neu)

- **Bücher** bündeln Seiten als Teile einer Geschichte; **Figuren** mit festen Stimmen; **Gedächtnis** (Welt, Figuren, Verlauf, Stil) als Markdown im Anwendungsordner, nach jedem erzeugten Teil fortgeschrieben — erst nach Bestätigung.
- **Vorlagen** mit Platzhaltern: vier eingebaute (Geschichte, Hörspiel, Fortsetzung, Sachtext), anpassbar, zurücksetzbar, eigene.
- **Erzeugen** über den eingestellten KI-Anbieter mit Gedächtnis-Kontext; Vorschau mit editierbarem Skript und Gedächtnis-Vorschlag; „Übernehmen" legt die Seite als nächsten Teil an.
- **Bücher exportieren/einspielen** (`.lvbook`) mit Seiten, Gedächtnis und Stimmen.

## Seiten und Stimmen

- **Seiten als Paket** (`.lvpage`) exportieren und einspielen: Arbeitsstand, Projektdateien, optional die verwendeten Stimmen — mit Rechtebestätigung für die Weitergabe von Stimmen; Import nie überschreibend.
- **Stimmen:** kompakte Liste mit Hörprobe je Stimme, Hörprobe für den Seed, Export je Stimme und „Alle exportieren…" (Auswahl, Ort, Name mit Zeitstempel, gepackt oder Ordner). „Neue Stimme erschaffen" als Dialog.

## Updates und Stabilität

- Update-Anzeige bietet immer die neuere Version (lokal oder GitHub).
- Fish-Speech startet wieder (kein Job-Speicherdeckel für den GPU-Prozess; Compile-Threads auf Windows = 1).
- Startton beim Diktat kommt zuverlässig (dauerhafter Ausgabestream).
- Sprachserver beenden friert das Fenster nicht mehr ein; abgebrochene Downloads verschwinden sofort aus der Fußleiste.
