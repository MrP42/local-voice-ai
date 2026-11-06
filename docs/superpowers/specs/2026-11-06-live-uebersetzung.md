# Bücher live beim Vorlesen übersetzen (Entwurf)

Stand 06.11.2026, erste Fassung zur Prüfung durch Codex. Wunsch von Patrick (06.11.2026 23:52):
ein englisches Buch auf dem iPhone direkt auf Deutsch vorlesen lassen, Satz für Satz übersetzt,
ohne Wartezeit und ohne das ganze Buch vorher zu übersetzen; Sprachwechsel während des Vorlesens;
bei Bedarf doch das ganze Buch (auch als Kurzbuch) in einer Zielsprache anlegen, einmal erzeugt,
auf allen Geräten gleich, exportierbar als EPUB und wieder importierbar.

Grundlagen: Sprachfassungen #344/#348 (Spez `2026-11-06-sprachfassungen.md`, PC-UI #357),
Stimme je Buchsprache #352/#355, KI-Aufträge vom iPhone #350 (Spez `2026-11-06-ki-auftraege-iphone.md`),
Einheits-Player #326, Export am PC #236/#237, Kurzbuch-Zielsprache #217/#219.

## 1 · Abgrenzung

| Thema | Wo | Diese Spez |
|---|---|---|
| Kurzbuch oder Ganzbuch-Übersetzung **mit KI am PC**, vom iPhone ausgelöst | #350 (A1–A4, B1–B2) | nutzt nur; ergänzt eine Auflage zur Zielsprache (§8) |
| Fassungen, Sprachwechsel mit Positionsübernahme | #344/#348, PC-UI #357, iPhone-PR 3 aus #344 | nutzt; Live-Sprache ist **keine** Fassung |
| Stimme je Sprache | #355 | nutzt für die Zielsprache |
| Export am PC (EPUB u. a.) | #237 | ergänzt nur Metadaten (§9) |
| **Live-Übersetzung beim Vorlesen** | – | **neu** (§3–§6) |
| **Ganzbuch-Übersetzung geräteintern** (ohne Token) | – | **neu** (§7) |
| **Kein Doppel-Erzeugen** über Geräte und Wege | teils #350 (Idempotenz je Auftrag) | **neu**: gemeinsamer Erzeugungsschlüssel und Reservierung für alle Wege (§7.3) |
| **EPUB-Export am iPhone, Re-Import als Fassung** | – | **neu** (§9) |

Nicht doppelt bauen: Alles, was KI am PC ausführt, läuft über die Auftragsobjekte aus #350. Diese Spez baut
keine zweite Auftragsstrecke.

## 2 · Ziel und Grundsätze

1. **Null Token im Normalfall.** Live übersetzt das iPhone selbst mit Apples Übersetzung (Framework
   `Translation`, iOS 26, geräteintern). Kein Cloud-Aufruf, kein Abo-Kontingent, kein Datenabfluss.
2. **Nur was gelesen wird.** Übersetzt wird ein kurzer Vorlauf ab der Leseposition, nie das Buch auf Vorrat.
3. **Keine hörbare Lücke.** Übersetzen und Sprachausgabe laufen als Fließband mit Vorlauf (§4).
4. **Position bleibt in der Quellfassung.** Fortschritt, Abgleich (`apply_progress`) und andere Geräte
   sehen weiter Offsets der Quellfassung. Sprachwechsel und Gerätewechsel setzen satzgenau fort.
5. **Kennzeichnung.** Maschinell übersetzter Text ist immer als solcher erkennbar (Transparenz nach
   Art. 50 KI-VO, auch wenn die Pflicht hier nicht zwingend greift).
6. **Eine Version überall.** Eine Ganzbuch-Übersetzung entsteht je Schlüssel genau einmal und wird
   über den Abgleich verteilt (§7.3).

## 3 · Übersetzer

| Weg | Qualität | Kosten | Daten | Rolle |
|---|---|---|---|---|
| **Apple geräteintern** (`TranslationSession`) | Standard, satzweise | 0 | bleiben auf dem iPhone | **V1, Standard** |
| KI über den PC (Relay) | hoch, mit Kontext | Token je gelesener Stelle | gehen an den gewählten Anbieter | **später (L5)**, nur ausdrücklich |

- Verfügbarkeit je Paar mit `LanguageAvailability.status(from:to:)`: `installed` → sofort; `supported` →
  Sprachpaket einmalig laden (Systemblatt über `.translationTask` + `prepareTranslation()`); `unsupported` →
  Sprache ausgegraut mit Grund.
- Sitzung über `TranslationSession(installedSource:target:)` (iOS 26), eine je Paar, wiederverwendet.
- Quellsprache aus `book.language`; fehlt sie, erkennt der Kern sie aus den ersten Absätzen
  (`NLLanguageRecognizer`) und zeigt sie zur Bestätigung.
- Hinter einem Protokoll `LiveTranslator` (VoiceCore), damit Tests ohne Framework laufen und L5 später
  denselben Platz nutzt.

## 4 · Fließband und Vorlauf

**Einheit:** ein Satz nach `LibraryText.sentences` der Quellfassung. Sehr kurze Sätze (unter 25 Zeichen, etwa
„sagte er.“) werden mit dem folgenden Satz desselben Absatzes zu einer Einheit verbunden. Jede Einheit kennt
ihren Quellbereich (UTF-16, wie `offset`). Daraus folgt eine 1:1-Zuordnung Quelle ↔ Übersetzung.

**Stapel:** Ein Absatz wird als ein Stapel übersetzt (`translations(from:)`, `clientIdentifier` = Einheit),
in Lesereihenfolge.

**Vorlauf (Sollwerte, im Kern als Konstanten):**

| Größe | Wert |
|---|---|
| Mindestvorlauf | 6 Einheiten **und** 30 s geschätzte Sprechzeit (Zeichen ÷ Sprechrate) |
| Höchstvorlauf | 2 Absätze oder 40 Einheiten |
| Vorlauf an Phonon | die nächsten 3 übersetzten Einheiten (`prepare`), wie heute |

Reihenfolge je Einheit: übersetzen → Phonon erzeugt vorab → abspielen. Der Vorlauf entsteht schon beim Öffnen
des Buchs, bei Pause und nach Sprüngen, nicht erst bei „Abspielen“.

**Start:** Nach dem Tippen auf Abspielen wird die erste Einheit vorrangig übersetzt (Ziel: Ton nach unter 1 s,
Messung im Mac-Tor). Der Rest füllt sich im Hintergrund.

**Unterlauf:** `PhononEngine.read(next:)` wird asynchron (`() async -> PhononSentence?`) und wartet auf die
nächste Einheit. Fehlt sie nach 1,5 s: Statuszeile „Übersetzung lädt …“; nach 8 s Pause mit Hinweis und Knopf
„Original weiterlesen“. Niemals Originaltext mit der Zielstimme vorlesen.

**Fehler einer Einheit:** einmal neu versuchen; danach Hinweis an der Stelle, Wiedergabe pausiert. Leere oder
offensichtlich unveränderte Antwort (Ziel = Quelle bei verschiedenen Sprachen) zählt als Fehler.

**Sprünge** (Satz/Absatz/Kapitel, Suche, Fortsetzen von anderem Gerät): Vorlauf ab neuem Cursor neu aufbauen,
laufende Stapel für alte Stellen abbrechen.

**Speicher:** nur im Arbeitsspeicher, Schlüssel (Buch, Quellhash, Zielsprache, Einheit), höchstens 2000
Einheiten (LRU). Nichts auf Platte, nichts im Abgleich. Zurückspulen innerhalb der Sitzung kostet daher nichts.

**Systemstimme** (`AVSpeechSynthesizer`) liest heute absatzweise ab einem Satz. Im Live-Modus liest auch sie
einheitenweise aus demselben Vorlauf; Wortmarkierung entfällt dort.

## 5 · Sprachwechsel während der Wiedergabe

- Im Player ein Sprach-Chip „EN → DE“ (nur Text-Quellen). Liste:
  1. Original,
  2. vorhandene Fassungen nach #348 (Fassungswechsel wie dort),
  3. Live-Sprachen: installiert, ladbar (mit Größe), nicht verfügbar.
- Gibt es für die Zielsprache eine echte Fassung, steht sie oben und ist vorgewählt; „live“ bleibt wählbar.
- Wechsel während der Wiedergabe: Der hörbare Satz endet, ab der nächsten Einheit spricht die neue Sprache.
  Bei Pause greift der Wechsel sofort am aktuellen Satz. Vorlauf der alten Sprache wird verworfen, die erste
  Einheit der neuen vorrangig übersetzt.
- Stimme der Zielsprache nach #355 (Buch → Sprache → bester Ersatz). Testvorgabe: Phonon „Anton“ für Deutsch.
- Die gewählte Live-Sprache wird je Buch **lokal** gemerkt (`lva.player.live.<buchId>`). Abgleich über Geräte
  erst, wenn Codex es verlangt (Frage F3).
- Quelle Hörbuch (Audio): der Chip bietet „Ab hier in Deutsch vorlesen“ an. Übergabe der Position wie
  `ReadAlong.handoverChars`, dann Vorlesen live.

## 6 · Anzeige

- Lesen und Mitlesen zeigen den Absatz in der Zielsprache, sobald seine Einheiten übersetzt sind; noch nicht
  übersetzte Einheiten erscheinen gedämpft im Original.
- Markierung je Einheit. Langes Drücken auf eine Einheit zeigt das Original (Popover, VoiceOver-Aktion
  „Original anzeigen“).
- Kennzeichnung: Chip „Live übersetzt · auf dem iPhone“ im Player und im Mini-Player.
- Mini-Player und Sperrbildschirm zeigen den übersetzten Satz.
- Barrierefreiheit wie #326: Ziele ≥ 44 pt, AX5 bis xxLarge, Large Content Viewer für den Chip.

## 7 · Ganzes Buch in eine Sprache

### 7.1 Einstieg

Im Sprachmenü und auf der Buchseite: „Ganzes Buch in Deutsch anlegen …“. Blatt mit zwei Wegen:

| Weg | Qualität | Kosten | Wo |
|---|---|---|---|
| **Auf dem iPhone** | Standard (Apple) | keine | diese Spez, Paket L3 |
| **Mit KI am PC** | hoch | nach Angebot | #350 B1/B2, hier nur Verweis |

### 7.2 Geräteintern

- Kapitelweise in Abschnitten (gleiche Einheiten wie §4), fortsetzbar: fertige Abschnitte liegen in
  `Library/<buch>/pending/<schlüssel>/` und werden erst bei Abschluss atomar zur Fassung.
- Läuft im Vordergrund und über `BGContinuedProcessingTask` (iOS 26) mit sichtbarem Fortschritt weiter.
  Ob `Translation` dort arbeitet, prüft das Mac-Tor (Frage F2); sonst: Fortsetzen beim nächsten Öffnen.
- Ergebnis: neue Fassung nach #348 (`editions[]`, `files.editions`), Kennzeichnung `translation`:
  `{ engine: "apple-translation", engine_version, source_edition, source_hash, created, device }`.
- Kapitelüberschriften (`#`) und Seitenmarken `{{S. n}}` bleiben erhalten (maskiert wie am PC, #209).
- Abhängigkeit: iPhone-PR 3 aus #344 (Fassungen am iPhone anlegen und hochladen).

### 7.3 Kein Doppel-Erzeugen, eine Version überall

**Erzeugungsschlüssel** (gilt für alle Wege, auch #350 B1):
`sha256(book_id, source_edition, source_hash, target_lang, engine, engine_version, pipeline_version)`.
Für Kurzbücher ergänzt um Vorlage, Lesegrenze und Parameter wie in #350 §4.

Ablauf vor jedem Start:
1. Lokale Fassungen prüfen: gleicher Schlüssel vorhanden → öffnen statt erzeugen.
2. Abgleich ausführen, erneut prüfen.
3. Reservierung im Buch-Register: `pending[schlüssel] = { device, started, heartbeat }` (Register mit
   `updated`/`device` wie #348). Andere Geräte zeigen „Wird auf ‹Gerät› erstellt“ und bieten den Start nicht an.
   Ein Herzschlag je 10 Min; ohne Herzschlag nach 24 h verfällt die Reservierung und „Erneut starten“ erscheint.
4. Gleicher Schlüssel, aber anderer Weg (z. B. Apple vorhanden, KI gewünscht): erlaubt, eigener Schlüssel,
   Hinweis „Es gibt schon eine Standard-Übersetzung“.

Konflikt trotz Reservierung (zwei Geräte offline gestartet): Beide Fassungen bleiben erhalten (Grow-only nach
#348), der Abgleich erkennt gleichen Schlüssel und blendet die später erzeugte aus (Grabstein, Sieger =
früheres `created`, dann kleinere `device`-Kennung). Kein Fortschritt geht verloren, weil die Position an der
Quellfassung hängt bzw. nach #344 übernommen wird.

## 8 · Kurzbuch mit Zielsprache

- Erzeugung bleibt KI-Arbeit am PC (#219 Kern `target_lang`, vom iPhone über #350 A3).
- **Auflage an #350 A3:** Das Bestätigungsblatt fragt die Zielsprache (Standard Deutsch, Liste wie §5) und
  nimmt sie in Angebot, Bindung und Erzeugungsschlüssel auf.
- Vorhandene Kurzbücher lassen sich wie jedes Buch live in eine andere Sprache vorlesen (gleicher Player).
- Kein Kurzbuch auf dem iPhone selbst (Gemma bleibt zurückgestellt).

## 9 · Export und Re-Import

**Export am iPhone:** „Exportieren …“ an Buch und Fassung → EPUB 3 über das Teilen-Blatt (Dateien, Bücher,
Mail, „An Kindle senden“).

- Gültiges EPUB 3 mit EPUB-2-Rückfall: `mimetype` zuerst und unkomprimiert, `container.xml`, OPF, `nav.xhtml`
  **und** `toc.ncx`, Kapitel aus `#`-Überschriften, Cover, `dc:language` der Fassung.
- Metadaten für den Re-Import im OPF:
  `dc:identifier` = `urn:uuid:<edition_id>`, `<meta property="lva:book-id">`, `lva:edition-id`, `lva:source-hash`,
  `lva:generation-key`, `lva:translation` (JSON wie §7.2). Präfix `lva: https://wolffappliedai.de/lva/epub#`.
- Titelseite mit Hinweis „Maschinell übersetzt aus ‹Sprache›, ‹Weg/Modell›, ‹Datum›“.
- Hinweis im Exportblatt: Übersetzungen sind Bearbeitungen (§ 23 UrhG), nur zum privaten Gebrauch.
- **PC:** `library_export/epub.rs` (#237) schreibt dieselben Metadaten. Ein gemeinsames Golden-EPUB prüft
  beide Seiten (Rust und Swift lesen es, beide erzeugen dieselben OPF-Metadaten).

**Re-Import** (iPhone und PC):

| EPUB | Ergebnis |
|---|---|
| `lva:book-id` bekannt, Schlüssel schon vorhanden | „Schon in der Bibliothek“, nichts angelegt |
| `lva:book-id` bekannt, Fassung fehlt | als Fassung an das Buch angehängt (Grabstein für diesen Schlüssel wird nicht überschrieben, Rückfrage) |
| `lva:book-id` unbekannt | neues Buch mit dieser Fassung als Hauptfassung, IDs übernommen |
| ohne `lva:`-Metadaten | normaler Import mit Duplikat-Erkennung nach #344 (Rückfrage, nie still zusammenlegen) |

Sicherung: Der Hub hält Fassungen ohnehin Ende-zu-Ende-verschlüsselt; der Export ist die zusätzliche,
app-unabhängige Kopie.

## 10 · Pakete

| Paket | Inhalt | Abhängig von |
|---|---|---|
| **L1** | VoiceCore `LiveTranslation`: Einheiten, Vorlauf-Planer, LRU, Zuordnung, Protokoll `LiveTranslator`, Tests mit Fake-Übersetzer; Eintrag in `apps/apple/scripts/generate_project.py` | – |
| **L2** | iPhone: `AppleLiveTranslator`, Sprachpakete, `PhononEngine.read` asynchron, Einbau in `LibraryPlayback`, Sprach-Chip, Anzeige, Kennzeichnung | L1 |
| **L3** | Ganzbuch geräteintern → Fassung, Erzeugungsschlüssel, Reservierung im Register, Konfliktregel (Rust + Swift) | L1, #344 PR 3 |
| **L4** | EPUB-Export am iPhone, Metadaten am PC, Re-Import als Fassung (beide), Golden-EPUB | #348 |
| L5 (später) | Live mit KI über PC-Relay (eigene Freigaben und Kosten nach #350 §3/§5) | #350 A1 |
| PC (später) | Live-Übersetzung am PC/Mac mit lokalem Modell; eigenes Issue für Funktionsgleichheit | L1-Muster |

## 11 · Abnahmetests

| Fall | Erwartet |
|---|---|
| Englisches Buch, Ziel Deutsch, Abspielen | Ton nach unter 1 s (iPhone, Mac-Tor misst), über 10 Min keine Lücke über 400 ms zwischen Sätzen |
| Gedrosselter Fake-Übersetzer (Kern-Test) | Planer hält Mindestvorlauf; bei Unterlauf erst Statuszeile, dann Pause, nie Originaltext mit Zielstimme |
| Übersetzte Zeichen je Sitzung | höchstens Gelesenes plus Höchstvorlauf (Kern-Test zählt) |
| Sprachwechsel DE → FR während Wiedergabe | hörbarer Satz endet, nächste Einheit Französisch, Stimme nach #355, Position unverändert |
| Wechsel bei Pause | sofort am aktuellen Satz |
| Sprung Kapitel/Suche | alter Vorlauf verworfen, keine verspäteten Sätze der alten Stelle hörbar |
| Fortschritt nach Live-Lesen | Offset der Quellfassung; PC setzt am selben Satz fort |
| Sehr kurze Sätze | zu Einheiten verbunden, Markierung stimmt |
| Sprachpaket fehlt | Systemblatt zum Laden; „nicht verfügbar“ ohne Absturz |
| Echte Fassung vorhanden | Fassung vorgewählt, Live nur ausdrücklich |
| Ganzbuch geräteintern, App beendet bei 40 % | fortgesetzt ab fertigem Abschnitt, keine halbe Fassung sichtbar |
| Zwei Geräte, gleicher Schlüssel | Reservierung verhindert zweiten Start; offline gestartet: genau eine sichtbare Fassung nach Abgleich |
| Export → Import auf anderem Gerät | „Schon in der Bibliothek“ bzw. als Fassung angehängt, kein neues Buch |
| Export-EPUB | EPUBCheck ohne Fehler; öffnet in Apple Books und Calibre |
| Fremdes EPUB | Duplikat-Rückfrage nach #344 |
| Datenfluss Live | kein Netzwerkaufruf durch die Übersetzung (Test mit gesperrtem Netz) |

## 12 · Fragen an Codex

- **F1** Einheit Satz (genaue Position, schwächerer Kontext) statt Absatz (besserer Kontext, nur absatzgenaue
  Position)? Entwurf: Satz mit Verbinden kurzer Sätze.
- **F2** Ganzbuch geräteintern im Hintergrund über `BGContinuedProcessingTask`, oder nur im Vordergrund mit
  Fortsetzen?
- **F3** Live-Sprache je Buch über Geräte abgleichen (Register wie #355) oder nur lokal?
- **F4** Konfliktregel §7.3 (Grabstein für später erzeugte Fassung) ausreichend, oder Reservierung über ein
  eigenes Hub-Objekt mit Bestätigung vor dem Start?
- **F5** Reihenfolge: L1+L2 sofort, L4 parallel, L3 nach #344 PR 3. Einverstanden?
