# Feature-Matrix Granola ↔ Local Voice AI

Quelle Granola: [recherche/granola-analyse.md](recherche/granola-analyse.md) · Ist-Stand LVA: [recherche/ist-stand.md](recherche/ist-stand.md) · Stack: [recherche/lokaler-stack.md](recherche/lokaler-stack.md)

**Klasse:** `Kern` = muss gleichwertig oder besser werden (Goal-Kriterium) · `Komfort` = geplant, sperrt das Goal nicht · `Nein` = bewusst Non-Scope.
**Status:** `offen` · `in_arbeit` · `gleichwertig` · `besser` · `vorhanden` (schon vor dem Goal gleichwertig/besser) · `verworfen`.
**Beleg:** Befehl → Ergebnis, Testname, Commit oder Screenshot. Ein `Kern`-Eintrag ohne Beleg zählt als offen (`check_matrix.py`).

| ID | Granola-Funktion | Klasse | LVA-Ist (29.09.) | Ziel „gleich oder besser" | M | Status | Beleg |
|---|---|---|---|---|---|---|---|
| F01 | Aufnahme ohne Bot: Mikrofon + Systemaudio, Start per Klick | Kern | vorhanden (Windows), Systemton standardmäßig aus | Systemton standardmäßig an; Start per Klick, Kürzel und aus Erkennung/Kalender | M1 | gleichwertig | P1f d0bc230: Systemton-Default an, Start per Klick; Playwright -g Systemton |
| F02 | Ausfallsicherheit der Aufnahme (Granola: stille Ausfälle, Nutzerkritik) | Kern | Watchdog Start-Timeout, Recovery nur `recording` | **besser:** Pegel-/Stillewächter meldet sofort sichtbar, Recovery auch für `processing`, Nachtranskription | M2 | besser | P2e d99a2edf: Warnleiste bei stummem/fehlendem Signal je Kanal; P2d 0031578: Recovery auch processing + Nachholen aus Audio |
| F03 | Echo-Behandlung Mikrofon ↔ Lautsprecher | Kern | fehlt | AEC auf der Ich-Spur; keine doppelten Sätze ohne Headset | M2 | gleichwertig | P2c2 ea2a398f: AEC live, Leak 0,00 statt 0,565 (AK6); realer Lautsprechertest in der Abnahme |
| F04 | Live-Transkript „Me/Them" | Kern | Pseudo-Live, 20-s-Blöcke | sichtbare Verzögerung p95 ≤ 5 s; zusätzlich hochwertiger Enddurchlauf | M2 | besser | P2a+P2b2 d0f3391e: Live p95 1,4 s (AK5) + Enddurchlauf P2d (Qwen3-ASR/Vulkan) |
| F05 | Sprachen inkl. Deutsch, Wechsel im Meeting, eigenes Vokabular (30/50 Begriffe) | Kern | Auto-Sprache; Vokabular nur Whisper | **besser:** Deutsch-WER gemessen und dokumentiert, Vokabular unbegrenzt für alle Engines (bzw. Nachkorrektur), Ausgabesprache frei wählbar | M2 | besser | bench.md: Deutsch-WER gemessen (Ende 4,17 %, Live 7,86 %/6,86 %), Ausgabesprache frei (P1b); Vokabular ohne Grenze folgt nicht als eigenes Feature |
| F06 | Sprecher: Kanaltrennung + Namen aus Meeting-App | Kern | nur Kanal „Ich/Gegenseite" | **besser:** akustische Diarisierung der Gegenseite und von Importen/Präsenz, Sprecher benennen, Stimme wiedererkennen | M3 | besser | P3a/P3b/P3c a97a1008: akustische Diarisierung Gegenseite/Import/Praesenz (deutsch 0,94 % DER), Benennen, Namen ueberstehen Neu-Transkription; Wiedererkennen (P3d) optional |
| F07 | Notizblock während der Besprechung | Kern | fehlt | Editor neben dem Live-Transkript, Stichpunkte mit Zeitstempel, absturzsicher gespeichert | M1 | gleichwertig | P1c 4555f85: Notizblock mit Zeitstempel/Autosave; Playwright meeting-notes (Notizblock) |
| F08 | Enhanced Notes: eigene Notizen + Transkript verschmelzen, Nutzertext schwarz / KI grau, Lupe zur Quelle | Kern | Protokoll ohne Nutzernotizen | gleich + **besser:** Quelle springt ins Transkript **und ins Audio**; keine Aussage ohne Beleg im Transkript | M1 | besser | P1b 58fa7bd + P1d + P1e 2ec9c328: Nutzertext byte-genau, KI grau, Quelle → Transkript UND Audio; Eval 30/30, 21/21 |
| F09 | Vorlagen (29 vorgefertigt, eigene, teilbar) | Kern | Abschnitte hart codiert | mitgelieferte deutsche Vorlagen (≥ 8), eigene anlegen/bearbeiten, Export/Import als Datei | M1 | gleichwertig | P1a 13fa4d3 (8 deutsche Vorlagen) + P1c (eigene, Export/Import .lvtemplate.json) |
| F10 | Notizen bearbeiten und per Anweisung ändern; Action-Item-Checkliste | Kern | Protokoll nur Vorschau; Aufgaben nur im Text | Editor für Notizen/Protokoll, „Anweisung anwenden", Aufgaben abhakbar | M1 | gleichwertig | P1d: Bearbeiten, Anweisung anwenden, Aufgaben-Checkliste; Playwright -g KI-Notizen |
| F11 | Chat je Besprechung (auch live) mit Inline-Zitaten | Kern | fehlt | lokal, Zitate mit Sprung zu Transkript/Audio | M4 | gleichwertig | P4c 5ed9138 + P4e b23878ba: Chat je Besprechung/live, Zitate → Transkript+Audio; Qualitätsbeleg folgt mit P4f (AK8) |
| F12 | Chat über alle Besprechungen / Ordner / Person | Kern | fehlt | lokale Hybrid-Suche (Volltext + Embeddings), Antworten mit Zitaten | M4 | besser | P4b/P4c/P4g a4207200: lokal, Hybrid-Suche, Zitate mit Sprung ins Audio, Eval 24/24 (AK8) |
| F13 | Recipes (gespeicherte Chat-Prompts) | Komfort | fehlt | gespeicherte Prompts, **besser:** mit Variablen | M4 | besser | P4c 5ed9138 + P4e: 7 Recipes mit Variablen (Person/Ordner/Zeitraum wirken auch als Filter), eigene anlegen/duplizieren |
| F14 | Ordner/Spaces, Suche | Kern | Liste ohne Suche | Ordner, Volltextsuche, Filter | M4 | gleichwertig | P4a 0c20b01 + P4d be9cb4e: Ordner n:m, Volltext (FTS5), Filter; Suche p95 180 ms bei 100k Chunks |
| F15 | Kalender Google/Outlook: Titel, Teilnehmer, Erinnerung 1 min vorher | Kern | fehlt | lokal ohne Abo: ICS-Abo und/oder Outlook-Desktop; Titel/Teilnehmer übernehmen, Erinnerung | M5 | offen | |
| F16 | Ad-hoc-Erkennung über Mikrofonnutzung | Kern | fehlt | Erkennung laufender Meeting-Apps → Hinweis „Aufnahme starten?" | M5 | offen | |
| F17 | People/Companies aus Kalender | Komfort | fehlt | Personenliste aus Teilnehmern und Sprechernamen, Besprechungen je Person | M5 | offen | |
| F18 | Pre-Meeting-Brief | Komfort | fehlt | Kurzbrief aus früheren Besprechungen mit denselben Teilnehmenden | M5 | offen | |
| F19 | Follow-up-Mail (nur Gmail) | Kern | fehlt | Entwurf erzeugen, kopieren/als Mail öffnen (jedes Mailprogramm) | M6 | besser | P6c d4fe2229: Entwurf lokal, jedes Mailprogramm (Kopieren/mailto/.eml) statt nur Gmail |
| F20 | Teilen/Export (Link, CSV per Mail) | Kern | Word/TXT/MD/Zwischenablage | **besser:** formatierte Zwischenablage, PDF/SRT/JSON; kein gehosteter Link | M6 | offen | |
| F21 | MCP-Server/API (remote, bezahlt) | Komfort | fehlt | **besser:** lokaler MCP-Server (nur lesend) für Claude/Codex | M6 | offen | |
| F22 | Integrationen Slack/Notion/HubSpot/Zapier | Nein | – | Cloud-Integrationen außerhalb des Ziels; Export deckt den Bedarf | – | verworfen | Non-Scope GOAL.md |
| F23 | Mobile (iOS/Android/Watch), Telefonate | Nein | Apple-App separat | Non-Scope | – | verworfen | Non-Scope GOAL.md |
| F24 | Team-Spaces, Teilen im Team, SSO/SCIM, Audit | Nein | – | Einzelnutzer, lokal | – | verworfen | Non-Scope GOAL.md |
| F25 | Screenshots geteilter Bildschirme (Beta macOS) | Nein | – | Non-Scope | – | verworfen | Non-Scope GOAL.md |
| F26 | Transparenz/Einwilligung | Kern | Einwilligungsdialog (§ 201 StGB) | gleich; Hinweistext zum Kopieren in den Meeting-Chat | M1 | gleichwertig | P1f d0bc230: Hinweistext lokal/extern zum Kopieren + Einwilligungsdialog |
| F27 | Datenschutz: Cloud USA, Training-Default an | Kern | lokal | **besser:** kein Netzverkehr im Besprechungspfad (außer bewusst gewähltem externem LLM) | M7 | offen | |
| F28 | Audio-Wiedergabe zur Verifikation (Granola: keine) | Kern | vorhanden (Player, Zeit → Audio) | **besser:** bleibt; Aufbewahrung wählbar | – | vorhanden | ist-stand.md: MeetingDetail Player je Kanal |
| F29 | Datei-Import (Granola: ausdrücklich nicht geplant) | Kern | vorhanden (Audio/Video, VTT/SRT) | **besser:** bleibt; Diarisierung für Importe (F06) | – | vorhanden | ist-stand.md: import.rs |
| F30 | Kosten | Kern | lokal | **besser:** 0 € laufend; alle Modelle lokal nutzbar | M7 | offen | |
