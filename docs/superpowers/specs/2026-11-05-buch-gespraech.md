# Buch-Gespräch – per Sprache und Chat mit der KI über ein Buch sprechen

Stand 05.11.2026 · Epic lva#307 · Pakete G1–G6 (#308–#313) · Branch `feat/307-buch-gespraech`
Grundlagen: Buch-KI (lva#181, `book_ai/`), Besprechungs-Chat (`managers/meetings/chat/`), Sprachgespräch (`components/voice-call/`, `vendor/wai-voice/`), iOS-Reader (`apps/apple/iPhone/Library/`).

## 1. Ziel

Patrick liest oder hört ein Buch und will jederzeit nachfragen können, **vorrangig per Sprache**, alternativ per Text, lokal auf dem Gerät (Windows, Mac, **iPhone**):

- „Warum gibt es zwei Väter, und wo wird das erklärt?“ → kurze Antwort, Stelle („Einleitung, Seite 14“), wörtliches Zitat, gesprochen und als Text, ein Tipp oder „Lies mir die Stelle vor“ springt hin.
- „Was ist in diesem Kapitel bisher passiert?“ / „Was habe ich verpasst?“
- „Erklär mir, was mit Vermögenswerten gemeint ist.“
- Nachfragen im Gespräch („Und was sagt der andere Vater dazu?“).

## 2. Grundentscheidungen

| # | Entscheidung | Grund |
|---|---|---|
| D1 | **Eine Unterhaltung je Buch** für Sprache und Text. Gesprochenes steht als Text im Verlauf, man kann jederzeit tippen. | Kein Moduswechsel; Verlauf bleibt nachlesbar. |
| D2 | **Fester Ablauf ohne Tool-Calling** (Muster `meetings/chat` D4): Absicht → Suche → Auszüge `[Q<k>]` → Antwort → Belege. | Kleine lokale Modelle (iPhone 3–4 B) rufen Werkzeuge unzuverlässig. |
| D3 | **Zitate per Konstruktion** (D5): das Modell nennt nur `[Q<k>]`; Seite, Offset und Zitattext setzt der Code aus dem Buchtext. | Keine erfundenen Zitate oder Seiten. |
| D4 | **Spoilerschutz**: Standard „bis zur Leseposition“ (Suche nur im gelesenen/gehörten Teil), umschaltbar auf „ganzes Buch“ (pro Unterhaltung, gesprochen: „ganzes Buch“). | Fragen beim Lesen sollen nicht vorgreifen. |
| D5 | **Zwei Antwortformen** aus einem Lauf: `text` mit Belegchips, `speech` ohne Marker, Seite und Zitat ausgesprochen („Auf Seite 14 heißt es: …“). | Sprache bevorzugt, aber gleicher Inhalt. |
| D6 | **Modellwissen gekennzeichnet** (Codex-Entscheid B3): Erklärungen über das Buch hinaus beginnen mit „Allgemein:“ und bekommen keinen Beleg. | Transparenz. |
| D7 | **Sprachbefehle lokal**, ohne LLM: „Spring dahin“, „Lies mir die Stelle vor“, „Weiterlesen“, „Ganzes Buch“, „Nur bis hier“, „Stopp“. | Sofortige Reaktion. |
| D8 | **Gemeinsame Regeln, zwei Umsetzungen**: Rust (Desktop) und Swift (iOS) mit denselben Fixtures `tooling/fixtures/buch-gespraech/`. | iOS hat keinen Rust-Kern; Parität über Tests. |
| D9 | Datenschutz: kein Frage-, Antwort- oder Buchtext im Log; nur Codes, Längen, Zähler. Alles lokal; Cloud-Modelle nur, wenn Patrick sie für die Funktion `book_talk` wählt. | Wie `meetings/chat`. |

## 3. Ablauf (beide Plattformen gleich)

### 3.1 Index

- Quelle: Seitentext des Buchs (Desktop: Seite aus `books.rs`; iOS: `LibraryBook`-Text).
- Säubern wie `book_ai/source.rs`: Sprecher `<Anna>` und Stilmarken `[calm]` raus, Seitenmarken `{{S. n}}` werden zur Seitenzuordnung, Überschriften `#` zu Kapiteln.
- Stücke: Absätze, kurze zusammengelegt, Ziel 500–900 Zeichen, nie über Kapitelgrenzen. Je Stück: `id`, `chapter` (Index + Titel), `page` (letzte Seitenmarke davor), `start`/`end` als **UTF-16-Offsets im Originaltext** (für den Sprung im Reader), `text` (gesäubert).
- Index im Speicher, Neuaufbau bei Textänderung (Hash). Keine Datei nötig; Desktop darf ihn in `books/<id>/ki/cache/` ablegen.

### 3.2 Absicht (Regeln, de + en)

| Absicht | Erkennung (Beispiele) | Suche |
|---|---|---|
| `recap` | „bisher“, „was ist passiert“, „verpasst“, „zusammenfassen“, „so far“ | Kapitelzwischenfassungen aus dem B1-Cache bis zur Position (falls vorhanden) + die letzten Stücke vor der Position im aktuellen Kapitel |
| `locate` | „wo“, „an welcher Stelle“, „welche Seite“, „wann zum ersten Mal“ | BM25 + **Erstnennung**: das früheste passende Stück wird immer mitgenommen |
| `explain` | „erklär“, „was bedeutet“, „was ist mit … gemeint“ | BM25 + Begriff aus `begriffe.json` (B3), falls vorhanden |
| `question` | alles andere | BM25 |
| Befehl | D7 | kein LLM |

### 3.3 Suche und Budget

- BM25 über die Stücke im Bereich (D4), Anfrage erweitert um Figurennamen/Varianten aus dem Buchgedächtnis und `begriffe.json`, wenn vorhanden.
- Nachfragen: die Anfrage enthält auch die letzte Nutzerfrage (Pronomen-Auflösung ohne LLM).
- Auswahl: die besten k Stücke + Erstnennung bei `locate`, nach Buchreihenfolge sortiert, nummeriert `Q1…Qk`.
- Budget fest gerechnet:
  - Desktop: `context_budget_tokens` wie `meetings/chat`.
  - iOS Foundation Models (4096 Token gesamt): Anweisung ≤ 350, Verlauf ≤ 400 (höchstens 2 Wechsel), Frage ≤ 150, Antwort 350, **Auszüge ≤ 2.600 Token ≈ 7.500 Zeichen** (Schätzung 3,5 Zeichen/Token, Puffer 10 %).
  - iOS Gemma 4 E2B (G6): Auszüge ≤ 6.000 Token.

### 3.4 Prompt (Kern)

System (Sprache der Frage): Du sprichst mit jemandem über das Buch „{Titel}“. Antworte nur aus den Auszügen. Setze nach jeder Aussage aus dem Buch den Beleg `[Q<k>]`. Steht die Antwort nicht in den Auszügen, sag das. Allgemeines Wissen nur mit „Allgemein:“ und ohne Beleg. Höchstens drei kurze Sätze, wie gesprochen, keine Listen. Bei `recap`: chronologisch, höchstens fünf Sätze. Nicht über die Leseposition hinaus vorgreifen.

### 3.5 Belege und Sprechfassung

- `[Q<k>]` → `Citation { page, chapter, start, end, quote }`. `quote` ist der wörtliche Satz (oder bis zu zwei Sätze, ≤ 240 Zeichen) im Stück, der der belegten Antwortaussage am ähnlichsten ist (Muster `refine_line`). `start`/`end` zeigen genau auf diesen Satz.
- Ungültige IDs werden entfernt; Aussagen ohne Beleg bleiben, die Antwort bekommt `uncited = true`, wenn kein einziger Beleg übrig ist.
- `speech`: Marker raus, Abkürzungen ausgeschrieben, dann **ein** Beleg gesprochen: „Das steht auf Seite 14: ‚…‘“ (Zitat ≤ 25 Wörter, sonst gekürzt mit „…“). Bei `recap` kein Zitat, nur „bis Seite 52“.
- Rückfrage am Ende der Sprechfassung nur bei `locate`: „Soll ich dir die Stelle vorlesen?“ – ein „Ja“ ist dann ein Befehl „Lies mir die Stelle vor“.

### 3.6 Ohne Modell

Fehlt ein Modell oder ist der Speicher knapp: Antwort `found_only` mit den zwei besten Fundstellen („Dazu steht etwas auf Seite 14: …“). Nie ein leerer Fehler.

## 4. Desktop (G1–G3)

### G1 Kern (Rust, `src-tauri/src/book_ai/talk/`)

- Module: `index.rs` (Stücke), `intent.rs`, `retrieve.rs` (nutzt `meetings::chat::context::bm25_rank`, `content_terms`), `prompt.rs`, `cite.rs` (Belege, Sprechfassung), `commands.rs` (Befehlserkennung, gleiche Liste wie iOS), `store.rs` (Verlauf `books/<id>/ki/gespraech.json`, höchstens 200 Wechsel), `mod.rs` (Ablauf, Abbruch, Ereignisse).
- Funktion `FN_BOOK_TALK = "book_talk"` in „Modelle je Funktion“; ohne eigene Wahl das Standardmodell.
- Tauri-Befehle: `book_talk_ask { book_id, request_id, question, position: Option<u32>, scope: "until_position" | "book" }`, `book_talk_cancel`, `book_talk_history`, `book_talk_clear`. Ereignis `book-talk-event`: `Delta { text }` (nur Textform, Marker gefiltert wie `DeltaFilter`), `Done { answer: BookTalkAnswer }`, `Failed { code }`.
- `BookTalkAnswer { intent, text, speech, citations, uncited, found_only, scope }`.
- Tests: rein (Index, Absicht, Belege, Sprechfassung, Befehle) + Eval über die Fixtures mit Fake-LLM.

### G2 Oberfläche (React)

- Reiter „Gespräch“ in der rechten Spalte des Readers; mobil (< 880 px) das vorhandene Bottom Sheet `kidocs/BookSheets.tsx` (ersetzt „Chat folgt“).
- Kopf: Umschalter „Bis hier“ / „Ganzes Buch“, Menü (Verlauf leeren).
- Verlauf: Nutzerbeitrag (mit Mikrofon-Symbol, wenn gesprochen), Antwort mit Belegchips „S. 14“; Chip öffnet die Belegkarte darunter: Zitat, Kapitel, Seite, Knöpfe „Hinspringen“ und „Ab hier vorlesen“.
- Vorschläge, solange der Verlauf leer ist: „Was ist bisher passiert?“, „Wer ist wer?“, „Erklär mir die letzte Seite“.
- Eingabe unten: großer runder **Sprechen-Knopf** (Hauptaktion) + Textfeld + Senden.
- Sprung: markiert den Satz 3 s (vorhandene Live-Hervorhebung), scrollt ihn mittig.

### G3 Sprache

- Eigene `ChatSide` „Buch“ (`components/voice-call/bookSide.ts`): `chat()` ruft `book_talk_ask` und streamt die **Sprechfassung** satzweise an die TTS. STT, Turn-Taking, Barge-in, Füller (E3) bleiben aus der Engine.
- Start: Sprechen-Knopf, Tastenkürzel im Reader (Leertaste halten = Push-to-talk, „G“ = Gespräch an/aus) oder globales Hauptgespräch-Kürzel, wenn ein Buch offen ist (dann Frage an das Buch).
- Vorlesen pausiert beim Start; „Weiterlesen“ oder Ende des Gesprächs setzt an der alten Stelle fort, „Lies mir die Stelle vor“ an der zitierten.
- Stimme: Gesprächsstimme = Phonon Anton (Standard), unabhängig von der Vorlesestimme.
- Messgröße: p95 erster Ton < 1,5 s (lokal), im Test geführt.

## 5. iOS (G4–G6)

### G4 Kern (`VoiceCore/Sources/VoiceCore/BookTalk*.swift`)

- Gleiche Bausteine wie G1 in Swift, ohne Darwin-Abhängigkeit (Linux-Tests): `BookTalkIndex`, `BookTalkIntent`, `BM25`, `BookTalkPrompt`, `BookTalkCitations`, `BookTalkCommands`, `BookTalkBudget`.
- Modellschnittstelle `BookTalkModel { func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> }`, damit Kern und Tests ohne Foundation Models laufen.
- Tests über `tooling/fixtures/buch-gespraech/` (Fake-Modell liefert vorgegebene `[Q<k>]`-Antworten).

### G5 Oberfläche und Sprache

- Im Vorlesen/Reader (`LibraryReadAloudView`, `LibraryPlayerScreen`): Knopf „Fragen“ (Sprechblase mit Welle) in der Steuerleiste → Gesprächsblatt (`.sheet`, mittlere und große Höhe).
- Blatt: Verlauf wie G2, unten großer Sprechen-Knopf (tippen = Freihand bis Stille, halten = Push-to-talk), Tastatur-Knopf für Text.
- Hören: `SpeechAnalyzer` + `SpeechTranscriber` live über `AVAudioEngine` (Modus `.voiceChat` mit Echo-Unterdrückung), Satzende nach 800 ms Stille. Fallback Whisper (vorhanden).
- Sprechen: `PhononEngine` Stimme Anton, satzweise; Barge-in: spricht Patrick, stoppt die Ausgabe.
- AirPods/Sperrbildschirm: Pause-Taste beendet die Antwort; Gespräch läuft nur im Vordergrund (V1).
- Belegkarte: „Hinspringen“ setzt die Leseposition, „Ab hier vorlesen“ startet die Wiedergabe dort.

### G6 Modelle

- Standard Apple Foundation Models (`LanguageModelSession`, `streamResponse`), neue Session je Frage (Verlauf im Prompt, Budget 3.3).
- Optional Gemma 4 E2B (GGUF Q4, Apache-2.0) in der Modellbibliothek, über `LlamaBridge`; Auswahl im Gesprächsblatt-Menü. Qwen 0.5B nur als Notlösung (`found_only` bevorzugt).

## 6. Reihenfolge

1. Dieser PR: Plan, Fixtures, HTML-Vorschau (Codex).
2. Parallel: G1 (Rust) und G4 (Swift) – reine Kerne, keine UI.
3. Nach Codex-JA zur Vorschau: G3+G2 Desktop (ein PR), G5+G6 iOS (ein PR, Mac-Build über den iPhone-Thread).

## 7. Fixtures (`tooling/fixtures/buch-gespraech/`)

- `buch.txt`: synthetisches Sachbuch „Zwei Lehrmeister“ (frei erfunden, kein urheberrechtlich geschützter Text) mit Seitenmarken und Kapiteln; die Erklärung der zwei Lehrmeister steht in der Einleitung (S. 2), spätere Kapitel greifen sie auf.
- `fragen.json`: Fragen mit Absicht, Leseposition, erwarteten Belegseiten und Schlüsselwörtern; dazu Fake-Modellantworten für die Kern-Tests.
- `befehle.json`: Sprachbefehle de/en mit erwartetem Befehl.

## 8. Offene Fragen an Codex

1. Gesprächsspalte als eigener Reiter „Gespräch“ neben Inhalt/Notizen, oder Chat-Reiter umbenennen? Empfehlung: „Chat“ → „Gespräch“ (ein Reiter).
2. Spoilerschutz standardmäßig an? Empfehlung: ja, mit sichtbarem Umschalter.
3. iOS: Gespräch nur im Vordergrund (V1)? Empfehlung: ja; Hintergrund mit Sperrbildschirm später.
