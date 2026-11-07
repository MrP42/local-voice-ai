# Buch-Ordner im Vorlesen und appweite Projekte

Stand: 07.11.2026, Runde 4 nach Codex-Befund Runde 3 · Epic lva#365 ·
Vorschau: `apps/local-voice/docs/vorschau/projektordner/`

## Anlass

Patrick, 07.11.2026: Die Seitenstruktur von „Vorlesen“ ist nicht optimal. Sie soll die Struktur von
„Aufnahmen“ übernehmen. Bücher kommen in ein eigenes Verzeichnis, darin je Buch ein Unterverzeichnis
mit allen zugehörigen Texten und Dokumenten. Darüber hinaus ist eine appweite Struktur gewünscht, die in
Projekten alle Dateien, Aufzeichnungen und Dokumente bündelt, gezielt verarbeitet, der KI als Kontext
dient und auch als Zielverzeichnis.

Nicht Teil dieser Spec (eigener Thread): Transkription fortsetzen, Fortschrittsanzeigen, Knopftext
„Mit KI erstellen“, Symbol des Sprachgesprächs.

## 1 Befund (Ist)

| Bereich | Oberfläche | Ablage |
|---|---|---|
| Vorlesen | Eine flache Liste aller Seiten links (Buchtexte, Übersetzungen, lose Texte gemischt). Darunter die Kapitelliste. Reiter „Original / Übersetzung / KI-Dokumente“ über dem Reader. | `projects/index.json` + `projects/<seite>/state.json`, KI-Dokumente in `projects/<seite>/ki/dokumente/` |
| Bibliothek | Karten je Werk, Buchdetail mit Fassungen und Kurzbuch | `library/<buch>/buch.json` (Schema 2, Sprachfassungen lva#344), `cover.*`, `covers/`, `audio/`, `sync.json` |
| Buch im Ganzen | Ein Werk verteilt sich auf bis zu vier Orte: `library/<buch>/`, je Fassung eine Seite in `projects/`, KI-Dokumente in der Seite, Transkript im Besprechungsauftrag. | — |
| Aufnahmen | Projekte-Leiste (Alle, Ohne Projekt, Ordner; einklappbar zu Kürzeln; Ziehen = verschieben/hinzufügen), Liste, Arbeitsfläche, rechte Spalte. Breiten nach `useRecLayout`. | `meeting_folders`, Spalte `projekt_id` (ADR-0026 PR-7) |
| Suite-Projekte | Projektseite mit Einträgen und Recherche (ADR-0026) | `wai-projekte`, Einträge sind Verweise; Arten `book` und `book_document` gibt es schon |
| Sync | Sammlung `book`: Kern von `buch.json` + `files` (Rollen `text`, `cover`, `audio`, `editions[<fassung>]`, `kurzbuecher`) als Blob-Verweise **ohne Pfade**. Desktop liest den Fassungstext über `pages::page_path(page_id)`. | `sync/books.rs` |

## 2 Begriffe

| Begriff | Bedeutung |
|---|---|
| **Buch-Ordner** | Ein Werk mit allem, was dazugehört: Fassungen je Sprache, Hörbuch, Transkript, KI-Dokumente, Notizen, Anhänge. Physisch ein Verzeichnis. |
| **Texte** | Lose Vorlesen-Seiten ohne Buch. |
| **Projekt** | Suite-Projekt nach ADR-0026. Appweit, bündelt Verweise auf Aufnahmen, Bücher, Texte, Gespräche, Recherchen und eigene Dateien. |
| **Mitgliedschaft** | Ein Eintrag gehört einem Projekt an. Je Mitgliedschaft gibt es den Schalter „Als Kontext“. |
| **Ausgabeziel** | Das Projekt, in dem neue Ergebnisse als Mitglied eingetragen werden. Getrennt von der Kontextauswahl. |

Ein Buch ist kein Projekt. Ein Buch kann in beliebig vielen Projekten stehen (E5).

## 3 Zielbild Vorlesen (Struktur wie Aufnahmen)

1. **Ordnerleiste** (links, einklappbar): Alle · Zuletzt geöffnet · **Bücher** (ein Eintrag je Buch) ·
   **Texte** · **Projekte**.
2. **Inhaltsliste** (Mitte): im Buch-Ordner gruppiert nach *Fassungen*, *Hörbuch*, *KI-Dokumente*,
   *Notizen und Anhänge*. Jedes KI-Dokument zeigt seine Fassung (Sprache + „Hauptfassung“), damit die
   Zuordnung eindeutig ist (E2).
3. **Arbeitsfläche**: Reader wie heute; Reiter „Original / Übersetzung / KI-Dokumente“ entfallen (E2).
   Der Sprachwechsel im Reader (lva#361) bleibt.
4. **Rechte Spalte**: Inhalt/Kapitel, Notizen, Chat zum Buch.

Schmal (≤ 620 px): drei Ebenen Ordner → Liste → Reader, jede mit Zurück-Knopf (44 px). Die rechte
Spalte wird ein Blatt von unten. Keine waagrechte Scrollleiste.

Leere und Fehlerzustände (Vorschau `06-*`):
- Leere Bibliothek: Ordnerleiste zeigt „Bücher“ mit Hinweis und „Buch hinzufügen“; Ziehen bleibt möglich.
- Fehlende Quelle (Seite oder Datei nicht auffindbar): Eintrag bleibt sichtbar, markiert „Datei fehlt“,
  mit „Suchen …“ und „Aus Buch entfernen“. Nichts wird still gelöscht.

Bibliothek bleibt der Katalog. „Öffnen“ führt in den Buch-Ordner. Das Buchdetail zeigt „Dateien im
Buch-Ordner“.

## 4 Ablage: ein Verzeichnis je Buch

```
<Datenordner>/library/
  <buch-id>/
    buch.json, cover.*, covers/, sync.json   wie heute
    fassungen/<seiten-id>/                   je Fassung die bisherige Seite (state.json, Quelldatei, ki/)
    audio/                                   wie heute
    transkript/<fassung-id>.md               lesbarer Text des Hörbuch-Transkripts
    notizen/
    anhaenge/
<Datenordner>/projects/                      Texte (lose Seiten), Ordnername bleibt
```

### 4.1 Identität (Kompatibilität mit Sprachfassungen lva#344)

- **Seiten-ID, Fassungs-ID und Buch-ID ändern sich nicht.** Der Ordner einer Fassung heißt wie ihre
  Seiten-ID; dadurch gibt es keine Namenskollision und keine Umbenennung.
- `buch.json` bleibt Schema 2. Hauptfassung (`primary`), `reading`, `edition_tombstones`,
  `alignments`, `origin`/`origin_book` einer Übersetzung bleiben unverändert.
- Fortschritt (`text:<seite>:<reiter>`, `audio:<buch>`, `doc:<buch>:<dok>`), Lesezeichen,
  Weiterlese-Stellen und Stimmen je Buchsprache (`tts.voice.*`) hängen an IDs, nicht an Pfaden.
- KI-Dokumente bleiben bei ihrer Fassung (`fassungen/<seite>/ki/`). Ein Projektziel ändert diese
  Zuordnung nie (E4).
- Wer eine Seite sucht, fragt `pages::page_path(id)`. Diese Funktion löst über `projects/index.json`
  auf, das je Seite das neue Feld `buch` trägt. Alle Leser (Reader, Sync, KI-Dokumente) nutzen sie schon.

### 4.2 Sicherung (Auflage 1)

- Umfang: alle Dateien, die die Migration verschiebt oder ändert, also jede betroffene Seite vollständig
  (`state.json`, Quelldateien, `ki/` samt Cache), `projects/index.json` und jedes betroffene `buch.json`.
  Audio wird nicht bewegt und nicht gesichert.
- Ort: `<Datenordner>/sicherungen/buch-ordner-<zeit>/` mit `manifest.json` (relativer Pfad, Größe,
  SHA-256 je Datei).
- Die Sicherung wird **vollständig erstellt und gegen das Manifest geprüft, bevor die erste Änderung
  geschieht.** Reicht der Platz nicht oder schlägt die Prüfung fehl, beginnt die Migration nicht.
- Wiederherstellen: „Sicherung zurückspielen“ in Einstellungen › Daten legt den Stand des Manifests
  wieder an (Ziel vorher in den Papierkorb, nie überschreiben). Aufbewahrung 30 Tage, dann Rückfrage.

### 4.3 Migration und Rückfall (Auflage 2)

Dauerhaftes Journal `library/.migration-buch-ordner.json` (atomar geschrieben, fsync), Format:
`{ "version": 1, "phase": "...", "sicherung": "...", "schritte": [{ "seite": "...", "buch": "...", "von": "projects/<seite>", "nach": "library/<buch>/fassungen/<seite>", "status": "offen|verschoben|eingetragen" }] }`.

Feste Reihenfolge:
1. `geplant`: Journal mit allen Schritten schreiben.
2. `gesichert`: Sicherung erstellen und prüfen (4.2).
3. Je Seite: `rename(von, nach)` → Schritt `verschoben` → Index-Eintrag `buch` setzen und Index atomar
   schreiben → Schritt `eingetragen`.
4. `fertig`: Kennzeichen `layout: 2` in `library/.layout.json`, Journal bleibt als Nachweis.

Auflösung jedes Abbruchpunkts beim nächsten Start (vor dem ersten Lesen der Bibliothek):

Vor jeder Zeile der Tabelle gilt die **Doppelungsprüfung**: Für die Seite werden alle verschiedenen
Kandidaten auf Existenz geprüft (Index-Ort, `von`, `nach`, `projects/<seite>`). Existieren zwei oder
mehr, gilt „Seite doppelt vorhanden“: anhalten, nichts verschieben, nichts im Index ändern, Meldung mit
allen Pfaden. Das deckt auch eine Kopie ab, die durch Wiederherstellen aus der Sicherung entstanden ist.
Erst wenn genau ein Kandidat existiert, wird die passende Zeile angewandt.

| Zustand im Journal | Befund auf Platte (genau ein Kandidat) | Auflösung |
|---|---|---|
| beliebig | zwei oder mehr Kandidaten vorhanden | „Seite doppelt vorhanden“: anhalten (hat Vorrang vor allen Zeilen) |
| `geplant` | — | Sicherung neu beginnen |
| Schritt `offen` | Seite unter `von` | normal fortsetzen |
| Schritt `offen` | Seite unter `nach`, nicht unter `von` | als `verschoben` werten |
| Schritt `verschoben` | — | Index-Eintrag setzen, `eingetragen` |
| weder `von` noch `nach` | — | anhalten, Seite aus der Sicherung anbieten |

**Auflösung von `page_path` während Migration und Rückfall.** Solange ein Journal ohne Phase `fertig`
(bzw. ohne abgeschlossenen Rückfall) existiert, lädt `page_path` es einmal je Aufruf unter der
Seitensperre, prüft **alle** verschiedenen Kandidaten auf Existenz und gibt erst danach zurück:
1. den Ort laut Index-Eintrag (`buch` gesetzt → `library/<buch>/fassungen/<seite>`, sonst `projects/<seite>`);
2. hat die Seite einen Journalschritt: dessen `nach` und dessen `von` (beide Richtungen, also auch im
   Rückfall, wo `nach` die Quelle und `von` das Ziel ist);
3. `projects/<seite>`.
Existiert genau einer, wird er zurückgegeben. Existieren zwei oder mehr, liefert `page_path` den Fehler
„Seite doppelt vorhanden“ mit allen Pfaden, statt einen zu wählen; die Reihenfolge oben dient nur der
Meldung. Existiert keiner, „Seite nicht gefunden“. Damit ist das Fenster zwischen `rename` und
Index-Schreiben (Index zeigt noch auf den alten Ort, Seite liegt schon am neuen) abgedeckt.
Ohne Journal entfällt Schritt 2.

Kein automatischer Gesamt-Rückfall: Bei einem Problem hält die Migration an (Zustand bleibt lesbar, siehe Tabelle). Rückfall auf Wunsch: dieselben Schritte rückwärts (`nach` → `von`, Index-Feld leeren),
ebenfalls im Journal. Wiederholung und Rückfall überschreiben nie ein vorhandenes Ziel.

Sperren: Die Migration hält die Seitensperre (`pages::lock`), die Bibliothekssperre und die Laufsperre
des Sync (`run_lock`) über den ganzen Lauf. Sync, Import, Übersetzung und KI-Läufe warten oder werden
vorher sauber beendet; laufende Hintergrundaufträge verschieben den Start mit Hinweis.

Löschkaskade: Ein Buch in den Papierkorb nimmt den ganzen Buch-Ordner mit; Wiederherstellen bringt ihn
zurück. Ein Buch aus einem Projekt entfernen löscht nichts (E5).

### 4.4 Namen und Pfade (Auflage 4)

- Migrierte Fassungsordner heißen wie die Seiten-ID (ASCII, geprüft durch `checked_id`): keine Kollision.
- Anhänge, Notizen, Projektdateien: Dateiname bereinigt (keine `/ \ : * ? " < > |`, keine Steuerzeichen,
  kein `.`/`..`, keine reservierten Windows-Namen `CON`, `NUL`, `COM1` …, keine Endung mit Punkt oder
  Leerzeichen, max. 120 Zeichen). Kollision wird **ohne Beachtung der Groß-/Kleinschreibung** und nach
  Unicode-NFC geprüft; bei Treffer `Name (2).ext`, `Name (3).ext` …
- Ein vorhandener Zielpfad wird nie überschrieben, weder bei Migration noch Import noch Rückfall.

## 5 Appweite Projekte

Grundlage bleibt ADR-0026: Suite-Projekt mit ULID, Einträge sind Verweise, gemeinsame Ablage
`<Datenordner>/WolffAppliedAI/projekte/`.

### 5.1 Bündeln (E3)

- Projekte-Gruppe in der Leiste von Aufnahmen (vorhanden), Vorlesen und Sprachgespräch/Chat.
- App-Inhalte (Aufnahme, Buch, Text, Gespräch) werden verwiesen. **Eine Mitgliedschaft entfernen
  löscht nie die Quelle.** Ein Projekt löschen löscht nur Verweise und die Projektkopien (mit Papierkorb).
- Fremde Dateien werden nach `projekte/<id>/dateien/` kopiert: erst in eine Temp-Datei, dann Größe und
  SHA-256 gegen die Quelle prüfen, dann umbenennen (Regeln 4.4), **erst danach** als Eintrag eintragen.

### 5.2 KI-Kontext (Datenschutz)

- Kontextauswahl (Schalter je Mitgliedschaft) und Ausgabeziel (5.3) sind getrennte Zustände.
- **Alle KI-Wege** im Projekt (Chat, Sprachgespräch, KI-Dokumente, Recherche, Stapelaktionen) bauen
  ihren Kontext über eine gemeinsame Funktion `projekt_kontext(projekt)`, die nur eingeschaltete
  Mitgliedschaften dieses Projekts liefert.
- Ausschalten wirkt sofort auch auf Folgeaufrufe: Der aufgebaute Quellkontext eines laufenden Gesprächs
  wird verworfen und neu gebaut; Caches (Retrieval-Index-Treffer, Zusammenfassungs-Cache) sind nach
  Mitgliedschaft und Schalterstand geschlüsselt und werden nicht mehr benutzt.
- Fehlt die gewählte Form (Kurzbuch, Protokoll), wird der Eintrag **nicht** still durch Rohdaten ersetzt:
  er fehlt im Kontext, mit Hinweis „Kein Kurzbuch – erstellen oder Volltext wählen“.
- Anzeige: Token-Summe der eingeschalteten Einträge und Anzahl, Warnung über dem Modellfenster.
- Datenklasse und Freigaben wie ADR-0026 (Web separat, keine Inhalte ins Log).

### 5.3 Ausgabeziel (E4)

- Ein aktives Projekt je Fenster (Kopfzeile, „Speichert in: …“; mobil als Chip über der Aktion).
- Vor **jeder** Ausgabe ist das Ziel sichtbar und umschaltbar.
- Das Ziel bestimmt nur die **Mitgliedschaft** der Ausgabe. Eine Übersetzung bleibt Fassung ihres
  Buchs, ein KI-Dokument bleibt bei seiner Fassung; das Projekt bekommt einen Verweis.
- Ohne aktives Projekt: Ausgaben entstehen wie heute ohne Mitgliedschaft; die Kopfzeile zeigt
  „Kein Projekt“ mit „Projekt wählen“. Nichts wird nachgefragt oder verschoben.
- Dateiexporte mit aktivem Projekt landen in `projekte/<id>/ausgaben/` (Regeln 4.4).

### 5.4 Gezielt verarbeiten

Mehrfachauswahl in der Projektseite: Zusammenfassen, Übersetzen, Vorlesen/Hörbuch erzeugen,
Recherche mit Auswahl als Quelle, Export als Paket. Jede Aktion nutzt `projekt_kontext` und zeigt ihr Ziel.

## 6 Sync, Übertragungsformat und iPhone (Auflage 3)

### 6.1 Buch-Ordner (P2) ändert das Übertragungsformat nicht

Die Sammlung `book` überträgt Dateien nach **Rolle** (`text`, `cover`, `audio`, `editions[<fassung>]`,
`kurzbuecher`) als Blob-Verweise ohne Pfade. Der Desktop findet den Fassungstext über `page_path`.
Der Umzug ist damit rein lokal: gleiche IDs, gleiche Datensätze, gleiche Blobs. Ältere Desktop- und
iPhone-Versionen sehen keinen Unterschied. Das ist Voraussetzung für P2 und wird mit einem
Sync-Rundlauf-Test belegt (Datensatz vor und nach Migration bytegleich, außer `updated`).

### 6.2 Neue Inhalte (P3): eigene, versionierte Sammlung

Notizen, Anhänge und Transkript-Text reisen **nicht** im Datensatz `book`, weil ein altes Gerät beim
Zurückschreiben unbekannte Felder verlieren könnte. Sie kommen in die neue Sammlung `book_file`:

```json
{ "v": 1, "id": "bf_<40 hex>", "book": "<buch-id>", "edition": "<fassung-id oder leer>",
  "role": "anhang|notiz|transkript", "path": "anhaenge/Cashflow-Tabelle.xlsx",
  "blob": { BlobRef }, "updated": 1731000000000, "device": "dev_pc", "deleted": false }
```

- ID: `bf_` + 40 Hex von SHA-256(`buch-id` + `role` + normalisierter relativer Pfad). Stabil über Geräte.
- `path` ist relativ zum Buch-Ordner, nach 4.4 bereinigt, nie absolut, nie mit `..`.
- Konflikt: neuester `updated` gewinnt; gleiche Zeit → größere Geräte-ID. Die unterlegene Fassung wird
  als `Name (Konflikt Gerät).ext` behalten, nicht verworfen.
- Löschen: Grabstein `deleted: true` 30 Tage, Datei geht auf dem Empfänger in den Papierkorb.
- Datensätze mit `v` > bekannter Version werden gelesen, aber nie geschrieben (nur lesend anzeigen).
- Alte Geräte kennen `book_file` nicht und lassen sie unberührt; sie zeigen die Inhalte nur nicht.

### 6.3 Projekte (P6)

Sammlung `projekt_eintrag` nach demselben Muster (`v`, stabile ID aus Projekt + Eintragsart +
Ziel-ID, `kontext: bool`, `updated`, Grabstein). Projektdateien als `BlobRef` mit relativem Pfad unter
`dateien/` bzw. `ausgaben/`.

### 6.4 Plattformen

| | Windows / macOS (Tauri) | iOS |
|---|---|---|
| Vorlesen-Struktur | P1 | Bibliothek → Buch-Ordner-Ansicht (P3) |
| Buch-Ordner-Ablage | Migration P2 (rein lokal, 6.1) | unverändert lauffähig; eigene Ordneransicht P3 |
| Neue Buchinhalte | P3 `book_file` | P3 `book_file` |
| Projekte | P4, P5 | P6 |

## 7 Entscheidungen (Codex 07.11.2026)

| Nr. | Entscheid | Auflagen umgesetzt in |
|---|---|---|
| E1 | Buchtexte physisch verschieben: BEDINGT JA | 4.2, 4.3, 6.1 |
| E2 | Reiter durch Inhaltsliste ersetzen: JA | 3 (Fassung je KI-Dokument, Sprachwechsel, Zurück mobil) |
| E3 | Hybrid Verweis/Kopie: BEDINGT JA | 5.1, 4.4 |
| E4 | Aktives Projekt als Ausgabeziel: BEDINGT JA | 5.3 |
| E5 | Buch in 0..n Projekten: JA | 2, 4.3, 5.1 (Schalter je Mitgliedschaft) |

## 8 Pakete

| Paket | Inhalt | Abhängig von |
|---|---|---|
| P1 | Vorlesen: Ordnerleiste, Inhaltsliste, Kapitel rechts, Ebenen mobil, Leer-/Fehlzustände. Gruppierung aus `buch.json`, kein Datenumbau. | — |
| P2 | Buch-Ordner-Ablage: Sicherung, Journal, Migration, Rückfall, Sperren, Löschkaskade, Buchdetail „Dateien“. Sync-Rundlauf-Test (6.1). | P1 |
| P3 | Sammlung `book_file`, Notizen/Anhänge/Transkript, iOS Buch-Ordner-Ansicht. | P2 |
| P4 | Projekte in Vorlesen/Gespräch, Projektseite gruppiert, Dateikopie, `projekt_kontext`, Kontext-Schalter. | P1, ADR-0026 |
| P5 | Ausgabeziel, Ausgabeordner, Mehrfachaktionen. | P4 |
| P6 | Sammlung `projekt_eintrag`, iOS Projekte. | P4, P5 |

Jedes UI-Paket mit HTML-Vorschau (390/768/1440/2560, hell/dunkel) und Maß-Belegen zur Codex-Abnahme.

## 9 Abnahmekriterien

- Zwischenfenster: Test mit Abbruch nach `rename` und vor dem Index-Schreiben, in beide Richtungen
  (Migration und Rückfall). `page_path`, Reader und Sync finden die Seite am neuen Ort; der nächste
  Start setzt den Index nach und schließt ab.
- Doppelung: Seite nach Wiederherstellen aus der Sicherung an altem und neuem Ort vorhanden, je einmal
  mit Journalschritt `offen`, `verschoben` und ohne Journal: `page_path` meldet „Seite doppelt
  vorhanden“, der Wiederanlauf hält an, kein Ordner wird verschoben oder überschrieben, der Index bleibt
  unverändert.
- Migration an einer Kopie von Patricks Bestand: jedes Buch vollständig unter `library/<buch>/`;
  Fortschritt, Lesezeichen, Stimmen je Sprache, Hauptfassung, Ausrichtungen unverändert.
- Abbruch an jedem Punkt der Tabelle 4.3 (Test je Zeile): App startet, alle Seiten auffindbar, zweiter
  Lauf schließt ab. Zweiter Lauf nach Erfolg ändert nichts. Rückfall stellt den alten Stand her.
- Sicherung: ohne geprüfte Sicherung keine Änderung (Test mit vollem Datenträger und kaputter Kopie).
- Sync-Rundlauf: Datensätze `book` vor und nach Migration gleich; altes Gerät (aktueller Stand von main)
  synchronisiert weiter ohne Verlust.
- Namen: Test mit `Bericht.pdf` und `bericht.PDF`, `CON.txt`, `..\\x`: kein Überschreiben, kein Ausbruch.
- Kontext: Kontrolltext in einem ausgeschalteten Eintrag erscheint in keinem KI-Weg (Chat, Sprache,
  KI-Dokument, Recherche, Stapel), auch nicht nach Ausschalten mitten im Gespräch; fehlendes Kurzbuch
  führt nicht zu Rohtext im Kontext.
- E4: Übersetzung mit aktivem Projekt bleibt Fassung ihres Buchs und ist zusätzlich Projektmitglied.
- Vorlesen zeigt links nur Ordner. Bei 390 px keine waagrechte Scrollleiste, alle Bedienziele ≥ 44 px
  (Maß-Beleg per Playwright).
