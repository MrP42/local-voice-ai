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
- Index nur im Arbeitsspeicher, kein Plattencache (M2).

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

- Nur freigegebene Bereiche; neue Bereiche `books`, `book_notes`, `ki_docs` in `areas.rs`, Standard aus (Matrix: M1).
- Souveränitätsstufe „nur lokal“: Abrufkontext geht nie an Cloud-Modelle. Bei Relay/Cloud-Sprecher
  wird pro Gespräch einmal gefragt („Inhalte an <Anbieter> senden?“), Antwort an Gespräch gebunden.
- Inhalte sind Daten, keine Anweisungen (Kennzeichnung im Prompt), nie im Log, nie im Audit.
- Besondere Kategorien (Datenklasse `restricted` in Gesprächen) werden nicht indiziert.

## Abstimmung Nachbar-Threads (05.11. 23:25)

- Buch-Thread: keine Bedenken gegen Wiederverwendung von `lex`, `retrieve::Bm25`, `index::build`
  (bleiben unverändert; Prompt-Tuning dort nur in `prompt.rs`, feat/324).
- Buchtext global nur bis Lesegrenze. Die Cloud-Freigabe je Verbindung und Modell (`consent_key`)
  gilt auch für globale Anfragen mit Buchauszügen (statt eigener Rückfrage).
- Wechsel „Allgemein“/„Im Buch“: abgebrochene Antworten nie in den Buch-Verlauf schreiben (§3.7).
- iOS-Einstieg: `BookTalkButton`/`BookTalkSheet(book:)` aus `BookPlayer.swift` direkt nutzen.

## Runde 2: Codex-Must-fixes eingearbeitet (05.11. 23:40)

### M1 Freigabematrix (vor Indexierung UND vor Ausgabe geprüft)

| Quelle | Bereich (`voice_call_knowledge`) | Indiziert | Ausschluss |
|---|---|---|---|
| Bücher (Text) | `books` | ja, bis Lesegrenze sichtbar | – |
| Buchnotizen | `book_notes` | ja | Spoilerregel M3 |
| KI-Dokumente (Buch-KI) | `ki_docs` (eigene Freigabe) | ja | Spoilerregel M3 |
| Besprechungen (Notizen, Protokoll) | `meetings` | ja | Datenklasse `restricted`/besondere Kategorie |
| Transkripte | `transcripts` (nur mit `meetings`) | ja | wie oben |
| Diktate | `dictations` | ja | wie oben |
| Frühere Gespräche | `conversations` | ja, ohne laufendes Gespräch | `restricted`, besondereKategorie; Gespräche über ein Buch: M3 |
| Vorlesen-Seiten | `tts` | ja | – |
| Termine, Tagesüberblick | `calendar` | **nein** (bleibt Werkzeug, zeitraumbasiert) | – |
| Profil, Einstellungen, Schlüssel | – | **nie** | – |

`restricted` (auch abgeleitet: Protokoll, Zusammenfassung, Gesprächsbeitrag aus restricted-Quelle)
wird nie aufgenommen; eine Umklassifizierung entfernt das Dokument (`upsert` mit `restricted`).
Vor der Ausgabe prüft `retrieve` erneut den Bereich und die Lesegrenze.

### M2 Löschung und Widerruf

- Index nur im Arbeitsspeicher. **Kein Plattencache** (keine Token-Statistik auf Platte) – damit
  entfällt die Aufbewahrungsfrage; Neuaufbau beim ersten Gespräch nach Start (~1 s bei 50 Büchern,
  im Hintergrund, Abruf wartet nicht, sondern sucht im bis dahin Fertigen).
- Jede Entfernung (Löschen, Ändern, Umklassifizieren, Bereich entzogen, Schalter aus) erhöht
  `epoch`. Abrufergebnisse tragen `epoch`; die Gesprächsseite verwirft Ergebnisse, Prompt-Kontext und
  laufende Modellanfragen mit älterem Stand (`voice_call_cancel`), späte Ereignisse werden verworfen.
- Schalter aus ⇒ `revoke(None)`: Index leer, keine Hintergrundarbeit, kein Vorwärmen, kein Abruf.
- Änderungen kommen über die bestehenden Busse (Bibliothek, Besprechungen, Gespräche, Diktate):
  betroffenes Dokument wird sofort entfernt und erst danach neu aufgenommen.

### M3 Spoilergrenze für alles mit Buchbezug

- Abgeleitete Inhalte tragen `book_id` und `book_upto` (belegte Spanne: Notiz = Ankeroffset,
  KI-Dokument = Ende des zusammengefassten Bereichs, Buchgespräch = höchster zitierter Offset).
- Sichtbar nur, wenn `book_upto ≤ frontier`; unklare Spanne ⇒ unsichtbar (auch der Titel), außer
  bei ausdrücklicher Ganzes-Buch-Freigabe **pro Buch**.
- Buchtext: überspannende Stücke werden gekürzt und nur mit dem gekürzten Text bewertet.
- Vor der Übergabe an den Prompt prüft die App-Schicht die Grenze erneut gegen die aktuelle
  Leseposition (Grenze kann sich nur nach vorne bewegen; bei Rücksprung gilt die neue).

### M4 Belegprüfung und Eval-Tor

- `[Q<k>]` gilt nur, wenn k im Abruf dieses Durchgangs vorkam, `version` (Quellhash) und
  `epoch` noch aktuell sind und das Sprungziel existiert; sonst wird die Marke entfernt und der
  Satz als „ohne Beleg“ angezeigt. Für Bücher zusätzlich die Satzfenster-Prüfung aus `book_ai::talk::cite`.
- Eval-Tor (≥ 30 Fragen je Kategorie, synthetische Fixtures `tooling/fixtures/app-context/`):
  Antworttreue, Belegdeckung ≥ 80 %, **0 Spoiler, 0 Widerrufslecks, 0 Cloud-Lecks unter „nur lokal“,
  0 befolgte Injektionen**. Eine einzige Sicherheitsverletzung blockiert die Freischaltung.
  Ergebnis je (Modell, `PROMPT_VERSION`, Prüfversion).

### M5 Spekulation (L1)

- Spekulativer Abruf nur **lokal**: Abruf + Prompt bauen, keine Modell-, TTS- oder Cloud-Aufrufe.
- Ergebnis gebunden an (Turn-Id, Transkript-Hash, `epoch`, Freigabestand). Verworfen bei `resume`,
  abweichendem Finaltranskript, Abbruch, Seitenwechsel oder neuem Stand.
- Höchstens ein spekulativer Abruf gleichzeitig; Abruf läuft auf dem Blocking-Pool mit Zeitlimit
  50 ms. Gemessen: Treffer/Fehlstart/verworfen, CPU-Zeit.

### M6 Cloud

- „Nur lokal“ sperrt technisch jede Übertragung von Abrufkontext (Sprecher, Denker, Relay,
  Fallbacks), unabhängig von jeder Zustimmung.
- Sonst Zustimmung gebunden an (Anbieter, Modellroute, Bereiche, Gespräch) – baut auf `consent_key`
  des Buch-Gesprächs auf. Jede Änderung ⇒ neue Zustimmung. Laufende Übertragung sichtbar markiert
  (Infozeile „Inhalte gehen an <Anbieter>“).

### M7 Messvorschrift

| Zeitpunkt | Quelle |
|---|---|
| t0 Sprechende | letzter Frame über Schwelle (`lastSpeechAt`, nicht `end`-Ereignis) |
| t1 STT final, t2 Abruf fertig, t3 Prompt fertig, t4 erstes Token, t5 erstes TTS-Audio fertig, t6 hörbarer Inhalt (Wiedergabestart) | `TurnMetrics` erweitert |
| tR Rückmeldung (Ton/Füllsatz) | getrennt; zählt **nicht** als Inhalt |

Kennzahlen: `t6−t0` (Inhalt) und `tR−t0` (Rückmeldung) getrennt. Nenner = abgeschlossene Durchgänge;
Abbrüche, Barge-in, Fehler und Zeitüberschreitungen (> 10 s) werden gezählt und getrennt berichtet,
nicht in p50/p95 gemischt. Stichprobe ≥ 100 Durchgänge je Stufe. Fehlstarts der Spekulation getrennt.
Der Füllsatz nach 600 ms erfüllt das 500-ms-Ziel ausdrücklich nicht; das 500-ms-Ziel für die
Rückmeldung wird nur mit einem kurzen Quittungston (≤ 150 ms Audio, lokal) erreicht.

### M8 Reproduzierbare Freischaltung

- L1–L7 je einzeln gegen dieselbe Basis (alle aus), warm und kalt getrennt, auf Patricks PC.
  Bericht mit Hardware, Modell, Quantisierung, Datenumfang (Bücher/Stücke), Last.
- „Letzte 50 Durchgänge“ in der Infozeile ist nur Betriebsanzeige, kein Nachweis.
- Standard aus: kein Indexaufbau, kein Vorwärmen, kein Abruf (Test). Freischaltung erst nach
  bestandenem Eval-Tor und ausdrücklicher Bereichswahl.

### Antworten Codex auf die offenen Fragen

1. Immer abrufen, sobald Funktion und Bereiche frei sind. „Kein Treffer“ über **Termabdeckung je
   Treffer** (vorläufig ≥ 34 % der Suchterme) statt nur relativer 40 %; Kalibrierung im Eval.
2. Siehe M5. 3. Lesegrenze für alle abgeleiteten Quellen, ganzes Buch nur pro Buch. 4. Siehe M6.

## Stand P1 (Code)

`src-tauri/src/app_context/` (rein): `model`, `chunk`, `index` (inkrementell, `epoch`, `revoke`,
`restricted`), `retrieve` (Bereiche, Lesegrenze inkl. abgeleiteter Inhalte, Termabdeckung, ≤ 2 je
Dokument, Budget, Aktualität), `prompt` (Datenblock, Marken, `cited`, `strip_markers`). 15 Tests grün.
Messung (Release, Cloud-Container, synthetisch, Worst Case: jedes Stück trifft): 50 Bücher /
~20 000 Stücke, Aufbau 0,9 s, Abruf 30 ms je Frage.

## Stand P2/P3 und Codex-Entscheid zu M5 (06.11.)

Codex hat die LLM-Spekulation bei lokalem Sprecher **abgelehnt**: Sprecher und
Denker sind getrennt wählbar, und `voice_call_chat` löst die Route je Anfrage neu
auf. Umgesetzt (lva#328):

- **Bindung im Backend:** `voice_call_chat(…, context)` bekommt nur die Treffer.
  Das Backend baut die Kontextnachricht selbst und nur, wenn unmittelbar vor dem
  Start gilt: Schalter an, Empfänger lokal (eingebautes Modell oder Loopback-
  Endpunkt, kein fremder Ollama-Host), gleicher `epoch`, jede Quelle freigegeben
  und unverändert (`service::admissible`). Sonst antwortet das Modell ohne Kontext.
- **Spekulation nur Abruf:** Mit Kontext startet das Modell erst nach dem Commit;
  davor prüft `take` Turn, Transkript und Stand erneut. Ohne Kontext bleibt der
  bisherige spekulative Start (Bestand, kein Kontext betroffen).
- **Widerruf:** Eine Freigabeänderung, die den Stand erhöht oder den Schalter
  ausschaltet, bricht laufende Antworten mit Kontext ab (`revoked`) und stoppt die
  Sprachausgabe. `app_context_message` prüft den Stand.
- Höchstens eine Spekulation gleichzeitig; Logs ohne Inhalte.
- **Wechsel im Buch-Gespräch:** „allgemein“ bzw. „im Buch/zurück zum Buch“ als
  ganze Äußerung. Er gilt erst hörbar (nicht spekulativ), steht nicht im Verlauf;
  „allgemein“ entstandene Beiträge gehen nicht an die Buch-Seite.
- **Sprung aus Belegen:** Quellenliste klickbar (Seite im Vorlesen bzw. Bereich).
- LLM-Spekulation mit Kontext erst als Folge-PR nach Codex-Auflagen 3–16, falls
  die Messung (P4) sie braucht.

## Arbeitspakete (Issues)

### Task P1 – Abrufindex `app_context` (Rust, rein, Tests)
### Task P2 – App-Schicht: Quellen, Bereiche `books`/`book_notes`, Befehle, Tor+Audit, Bindings
### Task P3 – `globalSide.ts` + Umschalten Buch/Allgemein + Belege im Verlauf
### Task P4 – Latenz L1/L3/L4/L5/L6 + Messung L7, Eval-Skript mit synthetischen Fixtures
### Task P5 – Schalter + Einstellungen „Wissen“ (Bereiche), Handbuch; iOS-Nachzug als eigenes Epic

## Abnahme

- Unit-Tests: Ranking, Lesegrenze, Bereichsfilter, Budget, inkrementeller Neuaufbau, Audit ohne Inhalt.
- Eval-Tor nach M4 (Sicherheitsverletzung blockiert).
- Latenzmessung auf Patricks PC (lokaler Sprecher): Bericht p50/p95 je Stufe im PR.
- Schalter aus ⇒ Verhalten bitgleich zu heute (Test).

## Offene Fragen an Codex

1. Vorab-Abruf bei *jeder* Frage oder nur, wenn ein Klassifikator „Inhaltsfrage“ meldet? (Empfehlung:
   immer, weil < 30 ms; leere Treffer erzeugen keinen Kontext.)
2. Spekulativer Abruf bei `early_end`: verwerfen bei `resume` reicht? (Empfehlung: ja.)
3. Bücher global: nur bis Lesegrenze (Empfehlung) oder ganzes Buch?
4. Cloud-Sprecher mit Abrufkontext: einmalige Rückfrage je Gespräch ausreichend?
