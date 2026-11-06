# Bücher live beim Vorlesen übersetzen (Entwurf)

Stand 06.11.2026, **zweite Fassung** nach Codex R1 (bedingt JA, Must-fixes 1–7 eingearbeitet, F1–F5 entschieden).
Wunsch von Patrick (06.11.2026 23:52): ein englisches Buch auf dem iPhone direkt auf Deutsch vorlesen lassen,
Satz für Satz übersetzt, möglichst ohne Wartezeit und ohne das ganze Buch vorher zu übersetzen; Sprachwechsel
während des Vorlesens; bei Bedarf doch das ganze Buch (auch als Kurzbuch) in einer Zielsprache anlegen, möglichst
nur einmal erzeugt, auf allen Geräten gleich, exportierbar als EPUB und wieder importierbar.

Grundlagen: Sprachfassungen #344/#348 (Spez `2026-11-06-sprachfassungen.md`, PC-UI #357),
Stimme je Buchsprache #352/#355, KI-Aufträge vom iPhone #350 (Spez `2026-11-06-ki-auftraege-iphone.md`),
Einheits-Player #326, Export am PC #236/#237, Kurzbuch-Zielsprache #217/#219.

## 1 · Abgrenzung

| Thema | Wo | Diese Spez |
|---|---|---|
| Kurzbuch oder Ganzbuch-Übersetzung **mit KI am PC**, vom iPhone ausgelöst | #350 (A1–A4, B1–B2) | nutzt nur; Auflage zur Zielsprache (§8) |
| Fassungen, Sprachwechsel mit Positionsübernahme | #344/#348, PC-UI #357, iPhone-PR 3 aus #344 | nutzt; Live-Sprache ist **keine** Fassung |
| Stimme je Sprache | #355 | nutzt für die Zielsprache |
| Export am PC (EPUB u. a.) | #237 | ergänzt das Austauschschema (§9) |
| **Live-Übersetzung beim Vorlesen** | – | **neu** (§3–§6) |
| **Ganzbuch-Übersetzung geräteintern** (ohne Token) | – | **neu** (§7) |
| **Erzeugungsschlüssel und Hub-Lease** gegen Doppel-Erzeugung | Schlüsselform aus #350 A1 | **neu**: Lease-Protokoll für alle Wege (§7.3) |
| **EPUB-Export am iPhone, sicherer Re-Import** | – | **neu** (§9) |

Alles, was KI am PC ausführt, läuft über die Auftragsobjekte aus #350. Keine zweite Auftragsstrecke.

## 2 · Ziel und Grundsätze

1. **Null Token im Normalfall.** Live übersetzt das iPhone selbst mit Apples Übersetzung (Framework
   `Translation`). **Der Buchinhalt wird geräteintern übersetzt.** Apple kann Nutzungs- und Leistungsmetriken
   ohne Textinhalt erheben; das steht so im Hinweis, nicht „kein Netzwerkaufruf“.
2. **Nur was gelesen wird.** Übersetzt wird ein kurzer Vorlauf ab der Leseposition, nie das Buch auf Vorrat.
3. **Möglichst keine hörbare Lücke.** Übersetzen und Sprachausgabe laufen als Fließband mit Vorlauf (§4).
   Zeiten sind Messziele unter festgelegten Bedingungen (§11), keine Zusagen.
4. **Position nach festem Vertrag** (§4.1). Fortschritt bezieht sich immer auf eine Quellfassung.
5. **Kennzeichnung.** Maschinell übersetzter Text ist immer als solcher erkennbar.
6. **Ein kanonisches Ergebnis.** Online verhindert ein Hub-Lease parallele Erzeugung; offline ist doppelte
   Arbeit möglich, nach dem Abgleich gilt genau ein kanonisches Ergebnis, die anderen bleiben verlustfrei (§7.3).

## 3 · Übersetzer

| Weg | Qualität | Kosten | Rolle |
|---|---|---|---|
| **Apple geräteintern** (`TranslationSession`) | Standard, satzweise | 0 | **V1, Standard** |
| KI über den PC (Relay) | hoch, mit Kontext | Token je gelesener Stelle | **später (L5)**, nur ausdrücklich |

**Systemvoraussetzung:** iOS 26 (Projektziel `IPHONEOS_DEPLOYMENT_TARGET` 26.0). Der direkte Sitzungsweg
`TranslationSession(installedSource:target:)` braucht iOS 26 und keine View. Sinkt das Projektziel später unter
26, ist Live dort ausgeblendet mit Hinweis „ab iOS 26“ (`#available`-Prüfung im Code bleibt trotzdem).

**Sprachpakete**, getrennt behandelt:

| Lage | Verhalten |
|---|---|
| `LanguageAvailability.status(from:to:)` = `installed` | sofort nutzbar, ohne UI |
| `supported` (Paket fehlt) | Knopf „Sprache laden“. Nur dieser Weg braucht eine per `.translationTask` bereitgestellte Sitzung und `prepareTranslation()` mit Systemzustimmung |
| Nutzer lehnt ab | Sprache bleibt „nicht geladen“, kein erneutes Nachfragen ohne Tippen |
| kein Netz beim Laden | Hinweis „Zum Laden ist eine Verbindung nötig“, erneut versuchen |
| Paket später entfernt | Fehler beim Übersetzen → Status neu prüfen, Wiedergabe pausiert mit „Sprache laden“ |
| `unsupported` | ausgegraut mit Grund |

Paketgrößen werden nur angezeigt, wenn das System sie verlässlich liefert; sonst keine Angabe.

Quellsprache aus `book.language`; fehlt sie, erkennt der Kern sie aus den ersten Absätzen
(`NLLanguageRecognizer`) und zeigt sie zur Bestätigung. Sprachkennungen werden normalisiert (§7.3).

Der Übersetzer liegt hinter dem Protokoll `LiveTranslator` (VoiceCore), damit Tests ohne Framework laufen und L5
denselben Platz nutzt.

## 4 · Fließband und Vorlauf

**Einheit (F1):** genau ein Satz nach `LibraryText.sentences` der Quellfassung. Kurze Sätze werden in V1
**nicht** verbunden, damit „Wechsel am Satzende“ (§5) und die Position eindeutig bleiben. Jede Einheit kennt
ihren Quellbereich in UTF-16.

**Stapel:** Mehrere Sätze eines Absatzes gehen als ein Aufruf (`translations(from:)`, `clientIdentifier` je Satz).
Das dient nur dem Durchsatz; ein gemeinsamer Absatzkontext wird **nicht** angenommen.

**Vorlauf:** Ziel sind 6 Einheiten und 30 s geschätzte Sprechzeit (Zeichen ÷ Sprechrate). Die **Höchstgrenze
von 40 Einheiten hat Vorrang**; sehr lange Absätze werden in Stapel zu höchstens 12 Sätzen geteilt. An Phonon
gehen wie heute die nächsten 3 übersetzten Einheiten (`prepare`). Reihenfolge je Einheit: übersetzen →
Phonon erzeugt vorab → abspielen. Der Vorlauf entsteht schon beim Öffnen, bei Pause und nach Sprüngen.

**Start:** Die erste Einheit wird vorrangig übersetzt; der Rest füllt sich im Hintergrund.

**Unterlauf:** `PhononEngine.read(next:)` wird asynchron (`() async -> PhononSentence?`) und wartet auf die
nächste Einheit. Fehlt sie nach 1,5 s: Statuszeile „Übersetzung lädt …“; nach 8 s Pause mit Hinweis und Knopf
„Original weiterlesen“. Niemals Originaltext mit der Zielstimme vorlesen.

**Antworten:** Leere Antwort = Fehler. Eine unveränderte Antwort (Namen, Zahlen, Zitate) ist **kein** Fehler;
nur bei Sätzen ab 4 Wörtern löst sie genau einen erneuten Versuch aus, danach wird sie übernommen.
Fehler nach einem erneuten Versuch: Hinweis an der Stelle, Wiedergabe pausiert.

**Speicher:** nur im Arbeitsspeicher, Schlüssel (Buch, Quellfassung, Quellhash, Zielsprache, Satzbereich),
höchstens 2000 Einheiten (LRU). Nichts auf Platte, nichts im Abgleich.

**Systemstimme** (`AVSpeechSynthesizer`) liest im Live-Modus satzweise aus demselben Vorlauf; Wortmarkierung
entfällt dort.

### 4.1 Positionsvertrag

- **Live-Fortschritt** = `(source_edition, source_hash, offset)`, `offset` in UTF-16 der Quellfassung, Bedeutung:
  **Beginn des nächsten ungelesenen Satzes**. Ein Satz gilt erst als gelesen, wenn sein Ton vollständig
  abgespielt ist (`played`). Bei Unterbrechung mitten im Satz bleibt der Fortschritt an dessen Beginn; Weiter
  wiederholt ihn.
- Übersetzungs- und Audiovorlauf ziehen den Fortschritt **nie** vor.
- Der Abgleich (`apply_progress`) überträgt den Fortschritt unabhängig von der Live-Sprache (F3). Andere Geräte
  setzen am selben Satz der Quellfassung fort, im Original oder live.
- **Wiedergabegeneration:** Jeder Sprung, Sprachwechsel, Quellen- oder Stimmwechsel erhöht sie. Übersetzungs-
  und Audioergebnisse tragen die Generation, unter der sie bestellt wurden; ältere werden verworfen, nie
  abgespielt oder angezeigt.
- **Echte Fassungen** haben eigene Offsets. Zwischen Quell- und Zielfassung wird eine **Satzzuordnung** in beide
  Richtungen gespeichert (§7.2, §9). Der Quellhash allein ist keine Zuordnung. Ohne Satzzuordnung gelten die
  Wechselstufen aus #344 (kapitelgenau/ungefähr) mit ausdrücklich ungenauer Übernahme.

## 5 · Sprachwechsel während der Wiedergabe

- Im Player ein Sprach-Chip „EN → DE“ (nur Text-Quellen). Liste: Original; vorhandene Fassungen nach #348
  (Fassungswechsel wie dort); Live-Sprachen nach Status aus §3.
- Gibt es für die Zielsprache eine echte Fassung, steht sie oben und ist vorgewählt; „live“ bleibt wählbar.
- Wechsel während der Wiedergabe: Der hörbare Satz endet, ab dem nächsten Satz spricht die neue Sprache
  (neue Generation). Bei Pause greift der Wechsel sofort am aktuellen Satz.
- Stimme der Zielsprache nach #355 (Buch → Sprache → bester Ersatz). Testvorgabe: Phonon „Anton“ für Deutsch.
- Die gewählte Live-Sprache wird je Buch **nur lokal** gemerkt (`lva.player.live.<buchId>`) (F3).
- Quelle Hörbuch (Audio): der Chip bietet „Ab hier in Deutsch vorlesen“ an. Übergabe über
  `ReadAlong.handoverChars` (Satzbeginn), dann Vorlesen live.

## 6 · Anzeige

- Lesen und Mitlesen zeigen den Absatz in der Zielsprache, sobald seine Sätze übersetzt sind; noch nicht
  übersetzte erscheinen gedämpft im Original.
- Markierung je Satz. Langes Drücken zeigt das Original (Popover, VoiceOver-Aktion „Original anzeigen“).
- Kennzeichnung: Chip „Live übersetzt · auf dem iPhone“ im Player und im Mini-Player; Info-Text nach §2.1.
- Mini-Player und Sperrbildschirm zeigen den übersetzten Satz.
- Barrierefreiheit wie #326: Ziele ≥ 44 pt, AX5 bis xxLarge, Large Content Viewer für den Chip.

## 7 · Ganzes Buch in eine Sprache

### 7.1 Einstieg

Im Sprachmenü und auf der Buchseite: „Ganzes Buch in Deutsch anlegen …“. Vor dem Start der Rechtehinweis aus §9.3.

| Weg | Qualität | Kosten | Wo |
|---|---|---|---|
| **Auf dem iPhone** | Standard (Apple) | keine | Paket L3 |
| **Mit KI am PC** | hoch | nach Angebot | #350 B1/B2, hier nur Verweis |

### 7.2 Geräteintern

- **Verbindlich (F2): Vordergrund mit Fortsetzen.** Satzweise wie §4, in Abschnitten. Unterbrechung (App im
  Hintergrund, Sperre, Prozessende) verliert höchstens den laufenden Abschnitt; beim nächsten Öffnen geht es
  ohne abschließenden Rückruf am letzten gesicherten Abschnitt weiter.
- `BGContinuedProcessingTask` ist **optional** nach einem Test auf einem echten iPhone (Sperrbildschirm, Ablauf,
  Prozessabbruch); eine Fertigstellung im Hintergrund wird nicht zugesichert.
- **Abschnittsmanifest** `Library/<buch>/pending/<schlüssel>/manifest.json`: Erzeugungsschlüssel, Quellhash,
  Pipeline-Version, Lease-Generation, Liste fertiger Abschnitte mit Hash. Abschnitt und Manifest werden je
  atomar geschrieben (Datei schreiben → `fsync` → umbenennen). Passt Quellhash oder Pipeline nicht, wird das
  Manifest verworfen.
- **Ergebnis:** neue Fassung nach #348 (`editions[]`, `files.editions`) mit
  `translation: { engine: "apple-translation", os_version, pipeline_version, source_edition, source_hash, created }`
  und **Satzzuordnung** `files.editions[id].alignment` (Blob, Liste `[quell_offset, ziel_offset]` je Satzbeginn,
  aufsteigend; Abfrage in beide Richtungen per binärer Suche).
- **Veröffentlichung** ist wiederholbar: Blob hochladen (inhaltsadressiert) → Fassungseintrag mit Lease-Generation
  einreichen (§7.3) → erst danach `pending/` löschen. Ein Absturz dazwischen wiederholt denselben Schritt.
- Kapitelüberschriften (`#`) und Seitenmarken `{{S. n}}` bleiben erhalten (maskiert wie am PC, #209).
- Abhängigkeit: iPhone-PR 3 aus #344 und Lease-Protokoll (F5).

### 7.3 Erzeugungsschlüssel und Hub-Lease

**Schlüssel** (alle Wege; Swift, Rust und #350 identisch, gemeinsame Golden-Fixtures aus #350 A1):

```
key = sha256( canonical_json({
  "v": 1, "kind": "translation" | "kurzbuch",
  "book_id", "source_edition", "source_hash",
  "target_lang": BCP-47 normalisiert (klein, nur Sprache + Schrift, z. B. "de", "zh-hans"),
  "engine": "apple-translation" | "<anbieter>:<modell>",
  "pipeline_version": Ganzzahl,
  // nur kurzbuch: "template", "read_limit", "params"
}) )
```

`canonical_json` = RFC 8785 (JCS). Eine Apple-Modellversion ist nicht auslesbar und daher **nicht** Teil des
Schlüssels. **Ersatzregel:** Ein neues `pipeline_version` (bewusste Änderung an Satzteilung, Maskierung,
Format) erzeugt einen neuen Schlüssel; OS-Updates nicht. `os_version` steht nur als Information im Ergebnis.
Wer eine frischere Übersetzung will, wählt „Neu erzeugen“ (neuer Schlüssel mit `"regen": n`).

**Hub-Lease** (online, F4):

| Schritt | Inhalt |
|---|---|
| Erwerb | `lease_acquire(key, owner = Geräte-ID, ttl = 30 Min)` am Hub, **atomar** (Bedingung: kein gültiger Lease). Antwort: `generation` (monoton steigend je Schlüssel), `expires` (Serverzeit) |
| Verlängern | alle 10 Min `lease_renew(key, owner, generation)`; nur der aktuelle Besitzer, nur vor Ablauf |
| Veröffentlichen | `publish(key, generation, ergebnis)` prüft am Hub: gültiger Lease, gleicher Besitzer, gleiche Generation. Ein abgelaufener oder überholter Besitzer wird abgewiesen |
| Ablauf | serverseitig; danach kann ein anderes Gerät mit höherer Generation erwerben |
| Anzeige | anderes Gerät liest den Lease: „Wird auf ‹Gerät› erstellt“, kein Start; nach Ablauf „Erneut starten“ |

Der Hub braucht dafür eine bedingte Schreiboperation. Fehlt sie im heutigen Hub, ist das ein eigenes
Vorab-Paket **H1** (Hub-Endpunkt + Tests); ohne H1 gibt es L3 nicht.

**Offline:** Erzeugung ist erlaubt, aber ohne Lease als „vorläufig“ markiert. Zugesichert ist nur **ein
kanonisches Ergebnis nach Abgleich**, nicht „genau einmal“. Doppelte Ergebnisse bleiben verlustfrei erhalten.

**Kanonisch** bei gleichem Schlüssel, ohne Geräteuhren: das Ergebnis mit gültiger Lease-Generation vor
vorläufigen; unter gleichrangigen das mit dem kleinsten Inhalts-Hash des Text-Blobs. Nicht-kanonische werden
ausgeblendet, nicht gelöscht; Fortschritt und Referenzen auf sie werden über die Satzzuordnung auf das
kanonische Ergebnis umgeleitet (Umleitungseintrag `superseded_by` im Fassungsregister).

## 8 · Kurzbuch mit Zielsprache

- Erzeugung bleibt KI-Arbeit am PC (#219 Kern `target_lang`, vom iPhone über #350 A3).
- **Auflage an #350 (von Codex bestätigt):** Zielsprache im Kurzbuch-Blatt wählbar (Standard Deutsch) und Teil
  von Angebot, Bindung und Erzeugungsschlüssel; `target_lang` in `jobquote`, `joboffer`, `job` (A1-Fixtures).
- Vorhandene Kurzbücher lassen sich wie jedes Buch live in eine andere Sprache vorlesen.
- Kein Kurzbuch auf dem iPhone selbst (Gemma bleibt zurückgestellt).

## 9 · Export und Re-Import

### 9.1 Export

„Exportieren …“ an Buch und Fassung → EPUB 3 über das Teilen-Blatt.

- Gültiges EPUB 3 mit EPUB-2-Rückfall: `mimetype` zuerst und unkomprimiert, `container.xml`, OPF, `nav.xhtml`
  und `toc.ncx`, Kapitel aus `#`-Überschriften, Cover, `dc:language` der Fassung. Lizenz- und Rechtehinweise der
  Quelle (falls im Import vorhanden) werden übernommen.
- Titelseite: „Maschinell übersetzt aus ‹Sprache›, ‹Weg/Modell›, ‹Datum›“.
- **Austauschschema** im OPF (Präfix `lva: https://wolffappliedai.de/lva/epub#`), nur Daten, keine Geräte:
  `lva:schema` = 1, `lva:book-id`, `lva:edition-id`, `lva:source-edition`, `lva:source-hash`, `lva:generation-key`,
  `lva:translation` (JSON wie §7.2 **ohne** Gerätenamen/-kennungen), `lva:content-hash` (SHA-256 des Fassungstexts),
  sowie `lva-alignment.json` im Container (Satzzuordnung aus §7.2). Gerätenamen und -kennungen werden nicht exportiert.
- **PC:** `library_export/epub.rs` (#237) schreibt dasselbe Schema. Ein gemeinsames Golden-EPUB prüft beide Seiten.
- Weitergabe per Mail, „An Kindle senden“ o. Ä. ist eine Datenweitergabe **außerhalb** des verschlüsselten
  Abgleichs; das Teilen-Blatt sagt das.

### 9.2 Re-Import (iPhone und PC)

Metadaten im EPUB sind **unvertrauenswürdig**.

- **Inhaltsintegrität:** `lva:content-hash` muss zum tatsächlichen Text passen, sonst gelten alle `lva:`-Angaben
  als nicht vorhanden.
- **Herkunftsnachweis:** Wiedererkennung ohne Rückfrage nur, wenn das Gerät die `lva:edition-id` mit **genau diesem**
  Inhalts-Hash selbst kennt (lokal oder im eigenen Abgleich). Dann: „Schon in der Bibliothek“, nichts angelegt.
- Alles andere (fremde oder unbekannte IDs, passende `book-id` mit neuem Inhalt): **Rückfrage** „Als Fassung an
  ‹Buch› anhängen / Als neues Buch / Abbrechen“. Angelegt wird immer mit **neuen internen IDs**; die alten stehen
  nur als Herkunftsangabe dabei.
- Ein Import überschreibt nichts und ändert keine Grabsteine oder Leases.
- Ohne `lva:`-Metadaten: normaler Import mit Duplikat-Erkennung nach #344.
- **Härtung:** ZIP-Pfadtraversierung (`..`, absolute Pfade) sperren; Entpackgrenzen (Gesamtgröße, Dateizahl,
  Kompressionsverhältnis); XML ohne externe Entitäten/DTD; Skripte und externe Ressourcen werden nicht
  ausgeführt oder geladen; `lva:`-Werte und `lva-alignment.json` mit Größen- und Formatgrenzen geprüft. Gilt
  für den vorhandenen EPUB-Import mit (`BookText.epub`, PC `ebook`).

### 9.3 Rechte

- Eine Übersetzung ist eine Bearbeitung (§ 23 UrhG); eine vollständige Kopie eines Buchs ist auch privat nur
  eingeschränkt zulässig (§ 53 Abs. 4 UrhG). „Nur privat“ ist **keine** pauschale Erlaubnis.
- Vor Ganzbuch-Übersetzung und Export: Hinweis „Nur bei eigenen Rechten, passender Lizenz (z. B. gemeinfrei,
  CC) oder gesetzlicher Erlaubnis“ mit Bestätigung je Buch. DRM wird nie umgangen (§ 95a UrhG); DRM-Bücher
  kommen ohnehin nicht in die Bibliothek.
- Live-Vorlesen erzeugt keine dauerhafte Kopie (nur Arbeitsspeicher).

## 10 · Pakete (F5)

| Paket | Inhalt | Abhängig von |
|---|---|---|
| **L1** | VoiceCore `LiveTranslation`: Einheiten, Vorlauf, LRU, Fließband mit Generation, Protokoll `LiveTranslator`, Tests mit Fake-Übersetzer; Eintrag in `apps/apple/scripts/generate_project.py` | – |
| **L2** | iPhone: `AppleLiveTranslator`, Sprachpakete nach §3, `PhononEngine.read` asynchron, Positionsvertrag §4.1, Sprach-Chip, Anzeige, Kennzeichnung | L1 |
| **L4** | EPUB-Export am iPhone, Austauschschema am PC, sicherer Re-Import, Golden-EPUB | Austauschschema von Codex bestätigt |
| **H1** | Hub: atomarer Lease (acquire/renew/publish mit Generation) | – |
| **L3** | Ganzbuch geräteintern, Manifest, Satzzuordnung, Lease, Kanonisch-Regel (Swift + Rust) | L1, #344 PR 3, H1, #350 A1-Fixtures |
| L5 (später) | Live mit KI über PC-Relay (Freigaben und Kosten nach #350 §3/§5) | #350 A1 |
| PC (später) | Live-Übersetzung am PC/Mac mit lokalem Modell; eigenes Issue | L1-Muster |

## 11 · Abnahmetests

Messziele (keine Zusagen) unter: iPhone 15 oder neuer, iOS 26, Sprachpaket installiert, Phonon „Anton“, Tempo 1,
Flugmodus, englisches Prosa-Buch:

| Messziel | Wert |
|---|---|
| Tippen auf Abspielen bis Ton | Median < 1 s |
| Lücke zwischen Sätzen über 10 Min | 95. Perzentil < 400 ms |

Verbindliche Tests:

| Fall | Erwartet |
|---|---|
| Gedrosselter Fake-Übersetzer (Kern) | Vorlauf nach §4; bei Unterlauf erst Statuszeile, dann Pause, nie Originaltext mit Zielstimme |
| Übersetzte Zeichen je Sitzung (Kern) | höchstens Gelesenes plus Vorlauf |
| Kurze Absätze (Kern) | Höchstgrenze 40 hat Vorrang |
| Unveränderte Antwort „Harry.“ / „1984“ | übernommen, kein Fehler |
| Unveränderter langer Satz | ein erneuter Versuch, danach übernommen |
| Sprachwechsel DE → FR während Wiedergabe | hörbarer Satz endet, nächster Satz Französisch, Stimme nach #355, Fortschritt unverändert |
| Wechsel bei Pause | sofort am aktuellen Satz |
| Sprung während laufender Übersetzung | Ergebnis alter Generation weder hörbar noch sichtbar |
| Unterbrechung mitten im Satz | Fortschritt am Satzbeginn, Weiter wiederholt den Satz |
| Vorlauf | zieht den Fortschritt nicht vor |
| Fortschritt nach Live-Lesen | `(source_edition, source_hash, offset)`; PC setzt am selben Satz fort |
| Sprachpaket: fehlt / abgelehnt / kein Netz / später entfernt | je eigener Zustand nach §3, kein Absturz |
| Echte Fassung vorhanden | Fassung vorgewählt, Live nur ausdrücklich |
| Ganzbuch: Prozessabbruch bei 40 % | Fortsetzen am letzten Abschnitt, keine halbe Fassung sichtbar |
| Ganzbuch: Absturz zwischen Upload und Einreichen | Wiederholung ohne Doppel-Blob, eine Fassung |
| Ganzbuch: Quellhash geändert | Manifest verworfen |
| Zwei Geräte online gleichzeitig | genau ein Lease; der zweite startet nicht |
| Lease abgelaufen, alter Besitzer veröffentlicht | vom Hub abgewiesen |
| Zwei Geräte offline | beide Ergebnisse erhalten, eines kanonisch nach §7.3, Fortschritt umgeleitet |
| Schlüssel-Fixtures | Swift, Rust und #350 erzeugen dieselben Schlüssel |
| Export → Import auf eigenem Gerät | „Schon in der Bibliothek“ |
| Import mit fremder/manipulierter `lva:`-ID oder falschem Inhalts-Hash | Rückfrage, neue interne IDs, nichts überschrieben |
| Bösartiges EPUB (Pfadtraversierung, Entpackbombe, XXE, Skript) | abgewiesen bzw. entschärft |
| Export-EPUB | EPUBCheck ohne Fehler; öffnet in Apple Books und Calibre; keine Geräte-IDs |
| Rechtehinweis | vor Ganzbuch und Export bestätigt |

## 12 · Entscheidungen von Codex (06.11.2026, R1)

- **F1** Satz als Einheit, kurze Sätze in V1 nicht verbinden; Stapel ohne Kontextannahme.
- **F2** Vordergrund mit Fortsetzen verbindlich; Hintergrund optional nach Gerätetest.
- **F3** Live-Sprache nur lokal; Fortschritt unabhängig davon abgeglichen.
- **F4** Atomarer Hub-Lease statt Register-Reservierung.
- **F5** L1 + L2 zuerst; L4 nach bestätigtem Austauschschema; L3 nach #344 PR 3 und Lease.
