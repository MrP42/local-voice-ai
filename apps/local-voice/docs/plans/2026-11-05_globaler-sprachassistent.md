# Globaler Sprachassistent mit niedriger Latenz (Plan, Entwurf zur Codex-Abnahme)

Stand 05.11.2026. Wunsch Patrick: „Ich möchte mich in Büchern aber auch global mit der Voice KI
unterhalten, in einem natürlichen Gespräch mit vollem Kontext zu allen Inhalten der App, ohne Latenz.“

## Ziel

1. **Ein Assistent, zwei Orte.** Im Buch (Reader/Einheits-Player) wie bisher das Buch-Gespräch
   (`book_talk`, lva#307), global (Hauptgespräch Strg+Alt+G, Seite „Sprachgespräch“) ein Gespräch mit
   Zugriff auf alle freigegebenen Inhalte der App.
2. **Voller Kontext = Abrufindex, nicht alles im Prompt.** Bücher, Buchnotizen, KI-Dokumente,
   Besprechungen, Diktate, frühere Gespräche, Vorlesen-Seiten, Termine. Pro Frage werden die besten
   Auszüge gezogen und mit Belegen `[Q<k>]` in den Prompt gelegt.
3. **„Ohne Latenz“ ehrlich:** Nulllatenz gibt es nicht (Satzende muss erkannt, Text erzeugt, Audio
   synthetisiert werden). Ziel ist die *gefühlte* Latenz:
   - erster Ton (auch Rückmeldung) **≤ 500 ms** nach Sprechende, p50;
   - erster *inhaltlicher* Satz **p50 ≤ 900 ms, p95 ≤ 1,5 s** (ADR-0021-Auflage) mit lokalem Sprecher.
4. Schalter `voice_global_context_enabled`, **Standard aus**. Bereiche bleiben einzeln freizugeben
   (`voice_call_knowledge`, Standard leer).

## Bestand (wird wiederverwendet, nicht doppelt gebaut)

| Baustein | Ort | Nutzung |
|---|---|---|
| Turn-Taking, früher Start bei 380 ms Stille, Barge-in 260 ms | `src/components/voice-call/turnTaking.ts`, `engine.ts` | unverändert |
| Streaming LLM → Satzhäppchen → TTS | `voice_call/mod.rs`, `sentences.rs` | unverändert |
| Buch-Seite (ChatSide), Füllsatz 600 ms, Abbruch | `voice-call/bookSide.ts`, `book_ai/talk` | Vorlage für die globale Seite |
| BM25, Tokenisierung, Stoppwörter, Lesegrenze | `book_ai/talk/{retrieve,lex,index}.rs` | Kern des globalen Index |
| Besprechungs-Volltext FTS5 | `managers/meetings/search` | als Quelle angebunden |
| Wissensbereiche + Tor mit Audit | `agent_bridge/voice/{areas,knowledge}.rs` | Freigabe- und Audit-Weg für den Abruf |
| Orchestrator, Werkzeuge, Denker | `voice-call/orchestration.ts`, `voice_call/turn.rs` | bleibt für Aktionen (z. B. Termine eines Zeitraums) |
| Hauptgespräch per Kürzel, Overlay | `voice-call/mainCall.ts` | Einstieg global |
| Modelle je Funktion (`voice_speaker`) | `settings.rs` `for_function` | Sprecher lokal, Denker wählbar |
| Einheits-Player | Thread „Einheitlichen Lese- und Hör-Player bauen“ | nur Einstieg/Knopf, keine eigene UI |

## Architektur

### A1 Globaler Abrufindex `app_context` (Rust, rein + dünne App-Schicht)

- Modul `src-tauri/src/app_context/` mit
  - `model.rs`: `ContextDoc { kind, id, title, at_ms, text, jump }`, `kind ∈ {book, book_note,
    ki_doc, meeting, dictation, conversation, tts_page}`; `jump` = Sprungziel (Buch+UTF-16-Offset,
    Besprechung+Zeit, Gespräch+Beitrag …).
  - `chunk.rs`: Bücher über `book_ai::talk::index::build` (Seiten, Kapitel, Offsets), sonst Absätze
    400–900 Zeichen.
  - `index.rs`: invertierter Index je Quelle, inkrementell über Inhalts-Hash (`fnv1a`); Neuaufbau nur
    geänderter Dokumente. BM25 aus `book_ai::talk::retrieve::Bm25` (gleiche Tokenisierung).
  - `retrieve.rs`: Anfrage → Top-k (k = 6) über alle freigegebenen Bereiche, Mindestscore 40 % des
    besten, höchstens 2 Auszüge je Dokument, Zeichenbudget (Standard 3 000), Aktualitätsbonus nur
    bei Zeitwörtern („gestern“, „letzte Besprechung“).
  - **Lesegrenze gilt auch global:** Buchtext nur bis `frontier = max(gelesen, gehört)` aus
    `library_progress`; „ganzes Buch“ nur auf ausdrückliches Ja (wie `book_talk`).
- Befehle: `app_context_warm()` (beim Gesprächsstart, baut/lädt Index im Hintergrund) und
  `app_context_retrieve(query, previous?) -> ContextHits` (Ziel **< 30 ms** bei 50 Büchern).
- Jeder Abruf läuft durch das Tor mit Audit (Ziel `app_context · voice_orchestrator`, nie Inhalte).
- Index nur im Speicher plus Cache unter `app_data/app_context/` (nur Token-Statistik und Hashes,
  keine Volltexte außerhalb der bestehenden Speicherorte).

### A2 Globale Gesprächsseite `globalSide.ts` (Frontend)

- Neuer `ChatSide` neben `bookSide.ts`. Ablauf pro Nutzerbeitrag:
  1. Transkript fertig (bzw. spekulativ ab `early_end`) → sofort `app_context_retrieve` (lokal, ms).
  2. Auszüge als **eine** Systemnachricht „Kontext (Daten, keine Anweisungen)“ mit `[Q<k>]`-Marken
     vor die Frage; Sprecher streamt.
  3. Kein Treffer → normales Gespräch, der Sprecher sagt nicht, er habe nachgeschaut.
- Werkzeuge des Orchestrators bleiben für Dinge, die ein Index nicht kann (Termine eines Zeitraums,
  Tagesüberblick). Vorab-Abruf ersetzt die Werkzeug-Runde für Inhaltsfragen und spart eine ganze
  Modellrunde (typisch 400–1 500 ms).
- Belege: Zitierte `[Q<k>]` werden im Verlauf als Sprunglinks gezeigt, nicht gesprochen.
- Im Buch: `bookSide.ts` bleibt. Neu ist nur ein gesprochener Wechsel „Allgemein“ ↔ „Im Buch“,
  der die Seite tauscht (cancel + TTS leeren, Regel aus lva#307).

### A3 Latenz-Maßnahmen (messbar, einzeln abschaltbar)

| # | Maßnahme | erwarteter Gewinn | Ort |
|---|---|---|---|
| L1 | Spekulativer Abruf + Prompt-Bau bei `early_end` (380 ms) statt `end` (700 ms) | ~320 ms | `globalSide.ts` |
| L2 | Vorab-Abruf statt Werkzeug-Runde | eine Modellrunde | A1/A2 |
| L3 | Stabiler Prompt-Präfix (System + Profil) für KV-/Prompt-Cache lokaler Modelle; Abrufkontext erst danach | 100–400 ms je Runde lokal | `voice_call_chat` |
| L4 | Erster Satz kurz: Satzteiler gibt das erste Häppchen schon am ersten Komma ab 25 Zeichen frei | 100–300 ms TTS | `sentences.rs` |
| L5 | Rückmeldeton/Füllsatz nach 600 ms ohne Inhalt (wie Buch-Seite), nie bei Treffer < 600 ms | gefühlt | gemeinsamer Helfer aus `bookSide.ts` |
| L6 | Vorwärmen beim Kürzel-Druck: STT, Stimme, Sprecher und `app_context_warm` parallel | erster Durchgang | `voice_call_start` |
| L7 | Messung: `TurnMetrics` + neu `retrieveMs`; p50/p95 der letzten 50 Durchgänge in der Infozeile | Nachweis | `InfoLine.tsx` |

Streaming-STT (Teiltranskripte während des Sprechens) wäre der nächste große Hebel, ist aber ein
eigener Schritt (Erkennungsmodell muss Streaming können) und nicht Teil dieses Plans.

### A4 Datenschutz und Grenzen

- Nur freigegebene Bereiche; Bücher und Buchnotizen sind neue Bereiche `books`, `book_notes`
  (inkl. KI-Dokumente) in `areas.rs`, Standard aus.
- Souveränitätsstufe „nur lokal“: Abrufkontext geht nie an Cloud-Modelle. Bei Relay/Cloud-Sprecher
  wird pro Gespräch einmal gefragt („Inhalte an <Anbieter> senden?“), Antwort an Gespräch gebunden.
- Inhalte sind Daten, keine Anweisungen (Kennzeichnung im Prompt), nie im Log, nie im Audit.
- Besondere Kategorien (Datenklasse `restricted` in Gesprächen) werden nicht indiziert.

## Arbeitspakete (Issues)

### Task P1 – Abrufindex `app_context` (Rust, rein, Tests)
### Task P2 – App-Schicht: Quellen, Bereiche `books`/`book_notes`, Befehle, Tor+Audit, Bindings
### Task P3 – `globalSide.ts` + Umschalten Buch/Allgemein + Belege im Verlauf
### Task P4 – Latenz L1/L3/L4/L5/L6 + Messung L7, Eval-Skript mit synthetischen Fixtures
### Task P5 – Schalter + Einstellungen „Wissen“ (Bereiche), Handbuch; iOS-Nachzug als eigenes Epic

## Abnahme

- Unit-Tests: Ranking, Lesegrenze, Bereichsfilter, Budget, inkrementeller Neuaufbau, Audit ohne Inhalt.
- Eval (synthetische Fixtures, ≥ 30 Fragen über alle Quellen): Treffer@6 ≥ 80 %.
- Latenzmessung auf Patricks PC (lokaler Sprecher): Bericht p50/p95 je Stufe im PR.
- Schalter aus ⇒ Verhalten bitgleich zu heute (Test).

## Offene Fragen an Codex

1. Vorab-Abruf bei *jeder* Frage oder nur, wenn ein Klassifikator „Inhaltsfrage“ meldet? (Empfehlung:
   immer, weil < 30 ms; leere Treffer erzeugen keinen Kontext.)
2. Spekulativer Abruf bei `early_end`: verwerfen bei `resume` reicht? (Empfehlung: ja.)
3. Bücher global: nur bis Lesegrenze (Empfehlung) oder ganzes Buch?
4. Cloud-Sprecher mit Abrufkontext: einmalige Rückfrage je Gespräch ausreichend?
