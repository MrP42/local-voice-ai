# Bücher live beim Vorlesen übersetzen (Entwurf)

Stand 06.11.2026, **dritte Fassung** nach Codex R1 und R2 (R1 Must-fixes 1–7, R2 M1–M4 und Testauflage, R3 M1–M3 eingearbeitet, F1–F5 entschieden).
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
- **Echte Fassungen** haben eigene Offsets. Zwischen Quell- und Zielfassung gilt eine **Satzzuordnung** (§4.2).
  Der Quellhash allein ist keine Zuordnung. Ohne gültige Satzzuordnung gelten die Wechselstufen aus #344
  (kapitelgenau/ungefähr) mit ausdrücklich ungenauer Übernahme.

### 4.2 Satzzuordnung und Abfrageregel

- **Format:** `alignment = { v: 1, source_edition, source_hash, target_hash, pairs: [[q_i, z_i], …], eof: [Q, Z] }`.
  Ein Paar je übersetzter Quelleinheit (Satz), `q_i` = Beginn der Einheit im Quelltext, `z_i` = Beginn ihrer
  Übersetzung im Zieltext, beide **UTF-16**. `Q`/`Z` = Länge des Quell- bzw. Zieltexts (EOF-Marker).
- **Gültigkeit:** `q` und `z` je streng monoton steigend; `0 ≤ q_i < Q`, `0 ≤ z_i < Z`; kein Wert mitten in einem
  Surrogatpaar; `source_hash`/`target_hash` passen zu den vorliegenden Texten (§7.3 Hashvertrag); **und** die
  Quellgrenzen `q_0 … q_n` stimmen **vollständig** mit den kanonischen Quelleinheiten überein, also genau den
  Satzbeginnen aus `LibraryText.sentences` (absatzweise, absolut im normalisierten Quelltext), ohne Auslassung
  und ohne Zusatz. Ungültig → Zuordnung wird nicht benutzt. Satzteilung ist Teil von `pipeline_version`; ändert
  sie sich, gelten alte Zuordnungen nur, wenn die Prüfung weiter besteht.
- **Abfrage Quelle → Ziel** für Offset `o`: `o ≥ Q` → `Z` (Buchende). Sonst `i` = größtes mit `q_i ≤ o`
  (binäre Suche); Ergebnis `z_i`. Liegt `o` im Satzinneren, springt die Position **auf den Beginn der Einheit**
  zurück (passt zum Vertrag „Beginn des nächsten ungelesenen Satzes“; der Satz wird wiederholt). Vor `q_0` → `0`.
- **Abfrage Ziel → Quelle** symmetrisch über `z_i`, `o ≥ Z` → `Q`.
- **Zwischen zwei Ergebnissen** derselben Quelle (z. B. nicht-kanonisch → kanonisch, §7.3): Ziel A → Quelle →
  Ziel B, jeweils mit Rücksprung auf den Einheitenbeginn.
- **Kurzbücher** haben keine Satzzuordnung (keine 1:1-Einheiten); dort gilt die ungenaue Stufe.

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
  Pipeline-Version, zuletzt gehaltene Lease-Generation, Liste fertiger Abschnitte mit Hash und Teil-Zuordnung.
  Abschnitt und Manifest werden je atomar geschrieben (Datei schreiben → `fsync` → umbenennen). Passt
  Quellhash oder Pipeline nicht, wird das Manifest verworfen. Eine **neue Lease-Generation** (§7.3, Wiederaufnahme)
  verwirft **keine** Abschnitte: Abschnitte hängen am Schlüssel, nicht an der Generation; das Manifest übernimmt
  nur die neue Generation.
- **Ergebnis:** neue Fassung nach #348 (`editions[]`, `files.editions`) mit
  `translation: { engine: "apple-translation", os_version, pipeline_version, source_edition, source_hash, created }`
  und **Satzzuordnung** `files.editions[id].alignment` (Blob, Format §4.2).
- **Veröffentlichung** ist wiederholbar: Blobs hochladen (inhaltsadressiert, doppelt hochladen ist harmlos) →
  `publish` (§7.3) → erst nach bestätigtem Erfolg `pending/` löschen. Ein Absturz oder verlorene Antwort an
  jeder Stelle wiederholt den Schritt mit derselben `publish_id` und bekommt denselben Erfolg.
- Kapitelüberschriften (`#`) und Seitenmarken `{{S. n}}` bleiben erhalten (maskiert wie am PC, #209).
- Abhängigkeit: iPhone-PR 3 aus #344 und Lease-Protokoll (F5).

### 7.3 Erzeugungsschlüssel, Hashvertrag und Hub-Lease

**Hashvertrag** (Swift, Rust, #350 identisch; Golden-Fixtures):

- **Textnormalisierung** vor jedem Hash: UTF-8, Unicode **NFC**, Zeilenenden `\n`, kein BOM, keine Änderung an
  Leerzeichen im Zeileninneren, genau ein abschließendes `\n` (fehlendes ergänzt, mehrere auf eins gekürzt).
  Offsets (§4.2) beziehen sich auf den **normalisierten** Text.
- `source_hash`, `target_hash`, `content_hash` = `sha256` hex (klein) über die normalisierten Bytes.
- **EPUB-Rückgewinnung** (§9): der Fassungstext liegt zusätzlich unverändert als `lva-text.txt` im Container;
  Hash und Zuordnung beziehen sich auf diese Datei, nie auf das aus XHTML neu gewonnene Lesebild.

**Schlüssel:**

```
key = sha256( canonical_json({
  "v": 1, "kind": "translation" | "kurzbuch",
  "book_id", "source_edition", "source_hash",
  "target_lang": BCP-47 normalisiert (klein, nur Sprache + Schrift, z. B. "de", "zh-hans"),
  "engine": "apple-translation" | "<anbieter>:<modell>",
  "pipeline_version": Ganzzahl,
  "regen": Ganzzahl ≥ 0,
  // nur kurzbuch: "template", "read_limit", "params"
}) )
```

`canonical_json` = RFC 8785 (JCS). Eine Apple-Modellversion ist nicht auslesbar und daher **nicht** Teil des
Schlüssels. **Ersatzregel:** Ein neues `pipeline_version` (bewusste Änderung an Satzteilung, Maskierung, Format)
ergibt einen neuen Schlüssel; OS-Updates nicht. `os_version` steht nur als Information im Ergebnis.

**`regen`** ist immer enthalten, Standard `0`. „Neu erzeugen“ setzt `regen` = (größtes bekannte `regen` für
dieselben übrigen Felder, lokal und im Abgleich) + 1. Ein Wiederholen desselben Laufs (Absturz, Wiederaufnahme)
behält sein `regen`. Zwei Geräte, die offline gleichzeitig „neu erzeugen“, können dasselbe `regen` wählen; das
ist derselbe Schlüssel und fällt unter die Kanonisch-Regel.

**Hub-Lease** (online, F4). Über Gültigkeit und Ablauf entscheidet **ausschließlich die Hub-Uhr**; Clients
rechnen nur mit den vom Hub gelieferten Werten und vergleichen nie eigene Uhren.

| Schritt | Inhalt |
|---|---|
| `lease_acquire(key, owner)` | atomar, nur wenn kein gültiger Lease besteht. Antwort `{generation, expires, expires_in}`; `generation` steigt je Schlüssel monoton (auch über Ablauf hinweg) |
| `lease_renew(key, owner, generation)` | nur aktueller Besitzer und aktuelle Generation, nur vor Ablauf (Hub-Uhr). Antwort: **neues** `expires` und `expires_in` (Restlaufzeit in Sekunden nach Hub-Uhr). Takt: der Client erneuert nach `expires_in / 2`, gemessen mit seiner **monotonen** Uhr ab Empfang der Antwort; `expires` dient nur der Anzeige |
| `lease_status(key)` | `{owner, generation, expires_in}` oder leer; für Anzeige und Wiederaufnahme |
| `publish(key, generation, owner, publish_id, eintrag)` | siehe unten |

**Wiederaufnahme** nach Ablauf, Netzverlust oder Prozessneustart: `lease_status` → gehört der Lease noch mir mit
meiner Generation, weiter mit `renew`; sonst `lease_acquire` → bei Erfolg weiter mit der **neuen** Generation.
Passende Abschnitte (§7.2) bleiben erhalten. Hält ein anderes Gerät den Lease: lokale Arbeit pausiert,
Abschnitte bleiben liegen, Anzeige „Wird auf ‹Gerät› erstellt“.

**`publish`** ist atomar und dauerhaft idempotent:

- In **einer** Hub-Transaktion: Lease-Prüfung (gültig nach Hub-Uhr, Besitzer, Generation = aktuelle) →
  Fassungseintrag schreiben → Veröffentlichungsbeleg `{key, publish_id, generation, eintrag_hash, published_at}`
  dauerhaft speichern → Lease beenden.
- `publish_id` = `sha256(key ‖ generation ‖ eintrag_hash)`. Ein erneuter Aufruf mit vorhandenem Beleg liefert
  **denselben Erfolg** zurück, auch nach Lease-Ablauf und auch wenn inzwischen ein anderer den Lease hält.
- Ohne Beleg und ohne gültigen Lease: abgewiesen (`lease_lost`); der Client geht in die Wiederaufnahme.

Fehlen bedingte Schreiboperation und Transaktion im heutigen Hub, ist das das Vorab-Paket **H1** (Hub-Endpunkte +
Tests); ohne H1 gibt es L3 nicht.

**Offline:** Erzeugung ist erlaubt, aber ohne Beleg als „vorläufig“ markiert. Zugesichert ist nur **ein
kanonisches Ergebnis nach Abgleich**, nicht „genau einmal“. Vorläufige Ergebnisse werden beim nächsten Online-
Abgleich über `lease_acquire` + `publish` nachgereicht, wenn noch kein belegtes Ergebnis existiert; sonst bleiben
sie vorläufig.

**Kanonischer Rang** je Schlüssel, ohne Geräteuhren, dauerhaft:

1. **Belegt** (vom Hub bei `publish` validiert, Beleg gespeichert) vor vorläufig. Ein späterer Lease-Ablauf ändert
   den Rang nicht.
2. Unter belegten: **kleinste Lease-Generation** gewinnt (die erste erfolgreiche Veröffentlichung).
3. Gleichrang (zwei vorläufige oder gleiche Generation): kleinster `target_hash` (Bytevergleich der Hex-Zeichen).
4. Letzter Tie-Breaker bei gleichem `target_hash` (inhaltsgleiche Ergebnisse): kleinste Fassungs-ID (Bytevergleich).
   Damit gibt es immer genau einen Gewinner.

Der Rang hängt nur an gespeicherten Daten, jedes Gerät berechnet denselben Gewinner. **`superseded_by`** im
Fassungsregister zeigt immer **direkt** auf den aktuellen Gewinner (nie auf einen anderen Verlierer, daher
zyklusfrei); ändert sich der Gewinner (z. B. ein vorläufiger wird später belegt und überholt), werden alle
Verweise auf den neuen Gewinner umgeschrieben. Verlierer bleiben **samt ihren Satzzuordnungen** erhalten,
ausgeblendet, nicht gelöscht. Fortschritt und Referenzen auf einen Verlierer werden nach §4.2 über die gemeinsame
Quelle auf den Gewinner umgerechnet.

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
  `lva:target-hash`, sowie `lva-text.txt` (normalisierter Fassungstext, §7.3) und `lva-alignment.json` (§4.2)
  im Container. Gerätenamen und -kennungen werden nicht exportiert.
- **PC:** `library_export/epub.rs` (#237) schreibt dasselbe Schema. Ein gemeinsames Golden-EPUB prüft beide Seiten.
- Weitergabe per Mail, „An Kindle senden“ o. Ä. ist eine Datenweitergabe **außerhalb** des verschlüsselten
  Abgleichs; das Teilen-Blatt sagt das.

### 9.2 Re-Import (iPhone und PC)

Metadaten im EPUB sind **unvertrauenswürdig**.

- **Inhaltsintegrität:** `lva:content-hash` muss zu `lva-text.txt` passen (Hashvertrag §7.3), sonst gelten alle
  `lva:`-Angaben und die Zuordnung als nicht vorhanden.
- **Zuordnung** aus `lva-alignment.json` wird erst aktiviert, wenn die Quellfassung (`lva:source-edition`) auf dem
  Gerät existiert, ihr Hash `source_hash` entspricht, `target_hash` zu `lva-text.txt` passt und die Grenzen nach
  §4.2 gültig sind, einschließlich des vollständigen Abgleichs mit den kanonischen Quelleinheiten. Sonst gilt die ungenaue Stufe (§4.1).
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
| Schlüssel-Fixtures | Swift, Rust und #350 erzeugen dieselben Schlüssel (inkl. `regen`, Sprachnormalisierung) |
| Hash-Golden-Fixtures | NFC, CRLF, BOM, fehlendes/mehrfaches Schluss-`\n` ergeben auf allen Seiten denselben Hash |
| `publish` erfolgreich, Antwort verloren | Wiederholung (auch nach Lease-Ablauf und mit fremdem Lease-Halter) liefert denselben Erfolg, genau ein Fassungseintrag |
| Lease abgelaufen während der Erzeugung, dann Neustart | `lease_acquire` mit neuer Generation, vorhandene Abschnitte weiterverwendet, `publish` mit neuer Generation gelingt |
| Geräteuhren um ±2 h verstellt | Lease-Gültigkeit, Rang und Gewinner unverändert |
| Vorläufiges Ergebnis wird später belegt und gewinnt | `superseded_by` aller Verlierer zeigt direkt auf den neuen Gewinner, keine Kette, kein Zyklus, Verlierer samt Zuordnung erhalten |
| Positionswechsel Quelle → Ziel → Quelle | Satzbeginn bleibt erhalten; Offset im Satzinneren springt auf den Satzbeginn |
| Positionswechsel am Buchende (EOF) | `Q` ↔ `Z`, kein Index außerhalb |
| Umleitung Verlierer → Gewinner | über die gemeinsame Quelle, Satzbeginn |
| Import-Zuordnung mit falschem Quellhash oder nicht monotonen Grenzen | nicht aktiviert, ungenaue Stufe |
| Import-Zuordnung mit ausgelassenem Satz oder `q_i` mitten im Satz | nicht aktiviert (Abgleich mit kanonischen Einheiten scheitert) |
| Zwei vorläufige, inhaltsgleiche Ergebnisse mit verschiedenen Fassungs-IDs | Gewinner = kleinste Fassungs-ID, auf allen Geräten gleich |
| Erneuerungstakt bei verstellter Geräteuhr | Erneuerung nach `expires_in / 2` (monotone Uhr), Lease läuft nicht ab |
| Export → Import auf eigenem Gerät | „Schon in der Bibliothek“ |
| Import mit fremder/manipulierter `lva:`-ID oder falschem Inhalts-Hash | Rückfrage, neue interne IDs, nichts überschrieben |
| Bösartiges EPUB (Pfadtraversierung, Entpackbombe, XXE, Skript) | abgewiesen bzw. entschärft |
| Export-EPUB | EPUBCheck ohne Fehler; öffnet in Apple Books und Calibre; keine Geräte-IDs |
| Rechtehinweis | vor Ganzbuch und Export bestätigt |

## 12 · Entscheidungen von Codex (06.11.2026, R1; R2 bedingt JA, M1–M4 eingearbeitet)

- **F1** Satz als Einheit, kurze Sätze in V1 nicht verbinden; Stapel ohne Kontextannahme.
- **F2** Vordergrund mit Fortsetzen verbindlich; Hintergrund optional nach Gerätetest.
- **F3** Live-Sprache nur lokal; Fortschritt unabhängig davon abgeglichen.
- **F4** Atomarer Hub-Lease statt Register-Reservierung.
- **F5** L1 + L2 zuerst; L4 nach bestätigtem Austauschschema; L3 nach #344 PR 3 und Lease.
