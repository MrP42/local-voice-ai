# Inhalte-Modul: Vorlesen wird Arbeitsfläche für Inhalte (Epic #530)

Stand 12.11.2026, **Fassung 2** (Codex Runde 1 auf cb5addd: NEIN mit 8 Blockern; diese Fassung arbeitet B1–B8
und die Hinweise ein, Zuordnung in §14). Auftrag Patrick, 12.11.2026 (diktiert): Das Modul „Vorlesen“ soll Inhalte
verwalten, öffnen, bearbeiten und erstellen, angebunden an Projekte, Ablage (RAG) und Aufnahmen, mit Baumansicht,
formatierter Darstellung, bereinigtem Import (Text, E-Mail, Dokument, URL), KI-Auswertung („das für mich Relevante,
ohne alles zu lesen“) und Routinen (z. B. täglich 6 Uhr neue Intranetbeiträge prüfen, relevante als Seite anlegen
und sofort zusammenfassen). Vorschau: `apps/local-voice/docs/vorschau/inhalte-i0/`.

## 1 Bestand (wird wiederverwendet, nicht neu gebaut)

| Baustein | Ort | Nutzen |
|---|---|---|
| Seiten des Vorlesens (`projects/<seite>/state.json`, Dateien, KI-Dokumente) | `commands/pages.rs`, `settings/tts/*` | bleiben die Dokumente; bekommen Lesefassung und Quelle (§3) |
| Ordnerleiste, Buch-Ordner | `ReaderFolders.tsx`, `useReaderFolders.ts` | wird zum Baum (§2) |
| Markdown-Untermenge der Buchansicht | `readerDoc.ts` | Grundlage der Lesefassung (§3.2) |
| Bereinigungsregeln | `lib/reader/textCleanup.ts`, `lib/reader/pasteCleanup.ts` (I1) | Bereinigung beim Einfügen (§4.1) |
| Kopf-/Fußzeilen, Seitenzahlen, Silbentrennung bei PDF | `ebook.rs`, `ablage/textlauf.rs` (#476) | Dateiimport (§4.2) |
| Extraktoren HTML (`scraper`), E-Mail (`mail-parser`), DOCX/PDF, Abrufschutz | `ablage/extrakt/*`, `docs/ablage-abrufschutz.md` | Dateiimport (§4.2) |
| Projekte mit Einträgen und Gruppen, Kennungen `wai://…` | `projekte/*`, `lib/projekteKontext.ts` | Baum gruppiert danach, Einträge bleiben Verweise (§2.2) |
| KI-Freigabe je Ablage-Ordner, Projekt-Kontextfreigabe (P4c) | `ablage/freigabe.rs`, `projekte/kontext.rs` | Freigaben (§6) |
| Belegprüfung (Satz ↔ Quelle), JSON-Schema-Prüfung | `assistent::belege_pruefen`, `assistent/tor.rs`, `book_ai/talk/cite.rs` | Kurzfassung (§5) |
| Verdichtung langer Texte | `book_ai/kurzbuch.rs`, `text.summarize` (`app_actions.rs`) | Kurzfassung (§5.4) |
| Workflow-Engine: Zeitplan (15 min Karenz), RSS-Muster `youtube_channel` mit Ledger, `page.create`, `knowledge.rate`/`reconcile`, Vorlage „Kanal → Wissen“, Trockenlauf, Rechte, Wiederholung | `managers/workflows/*`, `docs/AUTOMATIONEN.md` | Routinen (§7) |
| Lokales Modell (llama-server), Speicher-Tor | `managers/llm/*`, `queue_gate` | KI ohne Anbieter-CLI (§8) |

Neu entsteht nur, was fehlt: Lesefassung mit Blockkennungen und Revisionen, Quellenablage je Seite, Import Datei/URL
in Seiten, Baum, Kurzfassung als Stil des vorhandenen Zusammenfassens mit Belegprüfung, ein Feed-Auslöser und drei
dünne Aktionen.

## 2 Oberfläche und Navigation

### 2.1 Aufbau

- **Name:** Menüpunkt „Inhalte“ statt „Vorlesen“ (E1). Vorlesen bleibt Funktion jeder Seite (Wiedergabeleiste).
- **Links: Baum** (Fluent-2-Tree; Tastatur: Pfeile, Pos1/Ende, `*`, Typeahead; `aria-expanded`, `aria-selected`):
  1. *Neu für dich* – Seiten mit Eingangsmarke `neu` (aus Routinen und Importen), ungelesen fett, Zähler,
  2. *Projekte* → je Projekt die Gruppen aus `gruppiere()`: Seiten, Bücher, Aufnahmen (mit Besprechungsdokumenten
     und Protokollen als Kinder der Aufnahme), Dateien aus der Ablage, Recherchen, KI-Dokumente,
  3. *Ohne Projekt* (heutige Seiten- und Buch-Ordner),
  4. *Routinen* – Abläufe, die mindestens eine Aktion `content.*` enthalten (Auslöser beliebig), mit Zustand
     (aus / Trockenlauf / scharf) und Unterknoten *Aussortiert* (§7.3).
- **Mitte: Dokument** mit Reitern oben (wie #440): *Kurzfassung*, *Lesefassung*, *Übersetzung*, *KI-Dokumente*;
  Schalter Lesen/Bearbeiten wie heute. Reiter mit `aria-controls`, Pfeiltasten wechseln.
- **Quelle und Datenschutz**: ab 1280 px rechte Spalte, darunter Knopf „Quelle“ (Symbol ⓘ) im Dokumentkopf, der
  dasselbe als Blatt öffnet (unten, 66 % Höhe; bei 390 px Vollbild mit Zurück). Inhalt: Herkunft (aus `quelle.json`,
  nie vom Modell), Quellrevisionen mit „Rohquelle ansehen“, Datenklasse, KI-Freigabe, Modell der Kurzfassung,
  Projekt, Notizen und Anhänge.
- **Listenansicht** als Alternative für einen Knoten (Name, Art, Quelle, Geändert, Relevanz), sortier- und filterbar.
- Responsiv: 390 px Baum als eigene Ebene, 768 px Baum einklappbar, ≥1280 px drei Spalten, Lesespalte ≤ 70ch.
  Keine doppelten Scrollleisten. Meldungen zum Dokument (z. B. nach dem Einfügen) stehen im Fluss
  direkt unter den Reitern und verdecken keine Inhalte; sie schließen sich nach 10 s oder per Knopf.

### 2.2 Einträge anderer Module (E2, Lebenszyklus)

- Der Baum zeigt **Verweise** mit der vorhandenen Kennung (`wai://lva/seite/<id>`, `wai://lva/buch/<id>`,
  `wai://lva/besprechung/<id>`, `wai://lva/ablage/<dat>`, `wai://projekte/…`). Das Original bleibt im
  besitzenden Modul; es wird nichts kopiert.
- **Schreibrecht:** nur Seiten sind im Inhalte-Modul bearbeitbar. Bücher, Aufnahmen, Ablage-Dateien und Recherchen
  öffnen **lesend** mit Banner „Gehört zu Aufnahmen – In Aufnahmen öffnen“ (bzw. Bibliothek, Ablage, Projekte).
- **Löschen/Verschieben** geschieht im besitzenden Modul; der Baum folgt über dessen Ereignisse bzw. beim nächsten
  Laden. Fehlt das Original, zeigt der Eintrag „Original nicht mehr vorhanden“ mit „Aus Projekt entfernen“ (nur der
  Verweis) – nie stilles Verschwinden.
- Ablage-Dateien werden über den Abrufschutz gelesen (§6.3).

## 3 Seiten: Fassungen, Revisionen, Quelle (E3)

### 3.1 Datenordner einer Seite (neu, additiv)

```
projects/<seite>/
  state.json                 bisher: Sprechtext, Einstellungen (unverändert lesbar für alte Apps)
  inhalt/aktuell.json        Lesefassung (führend, §3.2)
  inhalt/rev/<rev>.json      Revisionen, unveränderlich (Journal wie P4a: .teil → SHA-256 → umbenennen)
  quelle/quelle.json         Quellrevisionen: {rev, art, url|datei|absender, abgerufen, sha256, groesse, titel, datum}
  quelle/<rev>.<ext>         Rohquelle unverändert (HTML, EML, PDF, TXT der Einfügung), nie überschrieben
  kurzfassung/<rev>.json     Kurzfassung je Lesefassungs-Revision (§5)
```

### 3.2 Lesefassung (führend)

- Format: Blockliste `{"version":1,"rev":"…","bloecke":[{"id":"b_<ULID>","typ":"h2|p|li|zitat|tabelle|code|bild","md":"…"}]}`
  mit Markdown-Untermenge (fett, kursiv, Link, Code) innerhalb des Blocks. Kein rohes HTML, keine Skripte; Links nur
  `http(s)`/`mailto`, Öffnen mit Rückfrage. Bilder nur als Verweis auf `quelle/`-Datei.
- **Blockkennungen sind stabil:** Bearbeiten eines Blocks behält seine `id`; neue Blöcke bekommen neue; Teilen
  vergibt die alte `id` an den ersten Teil; Verbinden behält die erste. Kennungen werden nie wiederverwendet.
- Jede Speicherung erzeugt eine Revision (Hash über die Blockliste). Aufbewahrt werden alle Importbasen (erste
  Revision nach jedem Import) und die letzten 50 übrigen Revisionen. „Versionen“ im Menü zeigt sie mit
  „Wiederherstellen“ (legt eine neue Revision an, löscht nichts).

### 3.3 Sprechtext und TTS-Marken

- **Neue Seiten mit Lesefassung:** Der Sprechtext wird **nur** aus der Lesefassung abgeleitet (Überschriften als
  eigene Sätze, Tabellen zeilenweise „Spalte: Wert“, Links als Linktext). Es gibt keine Rückableitung
  Sprechtext → Lesefassung; Tabellen, Links und Struktur können dadurch nicht verloren gehen.
- Der Chip-Editor bleibt für Sprecher, Tags und Pausen. Diese **TTS-Marken** werden als Anker gespeichert
  (`block_id`, Zeichenposition, Prüfsumme des Blocktexts). Ändert sich der Blocktext, wird eine Marke mit nicht
  passender Prüfsumme als „verwaist“ angezeigt (übernehmen/verwerfen), nie stumm verschoben.
- Textänderungen an Seiten mit Lesefassung geschehen im Blockeditor (Bearbeiten). Der Chip-Editor sperrt dort die
  Eingabe von Text und erlaubt nur Marken; Hinweis „Text in der Lesefassung bearbeiten“.
- **Alte Seiten ohne Lesefassung** bleiben unverändert (Sprechtext führend, wie heute). „In Lesefassung umwandeln“
  ist eine ausdrückliche Aktion: Sprechtext wird Importbasis (Quellrevision `art: "sprechtext"`), Marken werden
  Anker. Keine automatische Migration.

### 3.4 Export und Sicherung

Seitenpaket (`pages_package.rs`) und Datensicherung nehmen `inhalt/`, `quelle/`, `kurzfassung/` mit (I2). Ältere
App-Versionen lesen ein solches Paket weiter über `state.json` (Sprechtext), verlieren aber Lesefassung und Quelle;
das Paket trägt dafür eine Mindestversion und die alte App meldet „Paket aus neuerer Version, nur Text übernommen“.

### 3.5 Sync

In I2–I5 sind Lesefassung, Quelle und Kurzfassung **nur auf diesem Gerät** (Hinweis in „Quelle und Datenschutz“).
Der Bibliothek-Sync überträgt weiter nur den Sprechtext. Sync der neuen Teile ist eigene Stufe I8 (E2E über den Hub,
Quelle als Blob, Kurzfassung abgeleitet und neu erzeugbar).

## 4 Import (Quelle bleibt immer erhalten)

### 4.1 Einfügen (I1, nur Text)

- Regeln (deterministisch, ≥ 20 Vorlagen getestet): unsichtbare Zeichen/Steuerzeichen/NBSP, Zeilen nur mit
  Seitenzahl, wiederholte Kopf-/Fußzeilen (≥ 3×, ≥ 8 Zeilen Abstand, Text ≥ 1500 Zeichen, keine Sätze), Zeilen nur
  aus festen Bedienresten (abschließende Liste in `pasteCleanup.ts`, nur ganze Zeilen), Silbentrennung, abgerissene
  Zeilen, Leerzeichen, Leerzeilen. **Keine** Zeitstempel- und Absatzregel (Agenden wie „(10:30) Begrüßung“ bleiben).
- **Rohfassung dauerhaft:** Vor dem Bereinigen wird der eingefügte Rohtext gespeichert – bis I2 in `state.json` der
  Seite als `einfuegungen[]` (höchstens 20, je ≤ 2 MiB, Zeit, SHA-256, Anzahl je Regel), ab I2 als Quellrevision
  `quelle/<rev>.txt`. Editor-Menü „Letzte Einfügungen“ zeigt je Einfügung die Liste der Änderungen und bietet
  „Rohfassung wiederherstellen“ (fügt den Rohtext an der Stelle ein, sofern der bereinigte Text dort unverändert
  steht; sonst „Rohfassung kopieren“). Das gilt auch nach einem Neustart.
- Meldung nach dem Einfügen: „Beim Einfügen bereinigt: N Änderungen“ mit „Anzeigen“ und „Rückgängig“.
- Schalter „Beim Einfügen automatisch bereinigen“ (Standard an, gerätelokal).
- Ab I2: liegt `text/html` in der Zwischenablage, wird es nach Erlaubnisliste in Blöcke übertragen; Rohquelle
  `quelle/<rev>.html`.

### 4.2 Datei und E-Mail (I3)

- EML, HTML, DOCX, PDF, TXT/MD per Drag-and-drop oder Dialog. Der Extraktor der Ablage liefert zusätzlich Blöcke
  (Überschriften, Listen, Tabellen). Die Datei wird unverändert als Rohquelle abgelegt.
- E-Mail: Kopf (Von, Datum, Betreff) aus dem Parser als Herkunft, Text- oder HTML-Teil als Blöcke, Anhänge als
  Seiten-Dateien. MSG nur über Outlook-Export (keine neue Abhängigkeit).
- Aus verknüpften Ordnern gilt der Abrufschutz mit seiner Zusage (§6.3). Mehrere Dateien → mehrere Seiten;
  Fehlschläge einzeln gemeldet (§9).

### 4.3 URL (I3)

- Erlaubt: `https` (und `http` nur für ausdrücklich freigegebene Hosts). Freigabe privater Ziele je
  **Schema + Host + Port** (z. B. `https://intranet.example.org:443`), gerätelokal.
- Auflösen und Prüfen **beim Verbindungsaufbau** (eigener Resolver im Connector): jede aufgelöste IP wird geprüft;
  Loopback, Link-Local, Multicast, Metadatenadressen immer gesperrt; RFC1918/ULA nur für freigegebene Hosts. Kein
  zweites Auflösen nach der Prüfung (DNS-Rebinding).
- 10 s, ≤ 5 MiB, ≤ 3 Umleitungen, jede Umleitung wie ein neuer Abruf geprüft; Anmeldedaten (§7.4) nur an genau den
  freigegebenen Ursprung, bei Umleitung auf einen anderen Ursprung entfernt.
- Bilder werden nicht abgerufen (Platzhalter mit Alt-Text); „Bilder laden“ je Seite optional, gleiche Regeln.
- Hauptinhalt per Lesbarkeits-Heuristik (`article`/`main`, Textdichte, Navigation/Fuß/Kommentare weg). Rohquelle
  `quelle/<rev>.html` wird **nie** als HTML ausgeführt: „Rohquelle ansehen“ zeigt sie als Quelltext (nur Text).

## 5 Kurzfassung „Das Wichtigste für mich“ (E5)

### 5.1 Schema (jedes Feld belegt)

```json
{"version": 1, "inhalt_rev": "<rev der Lesefassung>", "modell": {"id": "…", "lokal": true},
 "relevanz": {"stufe": "hoch|mittel|gering", "themen": ["KI"], "warum": {"text": "…", "beleg": ["b_…"]}},
 "in_einem_satz": {"text": "…", "beleg": ["b_…"]},
 "kernaussagen": [{"text": "…", "beleg": ["b_…"]}],
 "fuer_mich": [{"text": "…", "art": "aufgabe|frist|termin|entscheidung|info", "datum": "2026-12-01", "beleg": ["b_…"]}],
 "zahlen": [{"wert": "…", "bedeutung": "…", "beleg": ["b_…"]}],
 "betroffen": [{"text": "…", "beleg": ["b_…"]}]}
```

- Quellenangaben (Titel, URL, Datum, Abrufzeit) kommen aus `quelle.json`, **nie** aus der Modellausgabe.

### 5.2 Prüfung vor Anzeige

1. JSON-Schema-Prüfung (`assistent/tor.rs`), unbekannte Felder verworfen.
2. Jede `beleg`-Kennung muss in der Lesefassung der Revision `inhalt_rev` existieren; unbekannte → Aussage verworfen.
3. Belegprüfung `assistent::belege_pruefen` (Aussage ↔ Blocktext); nicht gedeckt → verworfen.
4. `datum` muss im belegten Block vorkommen (gleiches Datum, beliebige Schreibweise), sonst ohne Datum angezeigt.
5. Bleibt `in_einem_satz` ohne Beleg, entfällt der Satz; bleiben weniger als eine Kernaussage, gilt die Kurzfassung
   als „ohne belastbare Belege“ (§9).
Verworfene Aussagen werden gezählt („2 Aussagen ohne Beleg ausgeblendet“), nicht angezeigt.

### 5.3 Veraltung

Die Kurzfassung gehört zu genau einer Revision der Lesefassung. Ändert sich die Lesefassung, zeigt die Kurzfassung
„Veraltet: Text wurde nach der Kurzfassung geändert“ mit „Neu erstellen“; Belege, deren Block entfernt wurde, sind
nicht mehr anklickbar. Belege sind Knöpfe (fokussierbar, Name „Beleg: Abschnitt ‚…‘ anzeigen“) und springen in die
Lesefassung, die den Block kurz hervorhebt.

### 5.4 Erzeugung ohne neuen Generator

`content.brief` und der Knopf „Kurzfassung erstellen“ sind ein neuer **Stil `brief`** des vorhandenen
Zusammenfassens (`text.summarize`/`summary_options`) mit Verdichtung langer Texte nach `book_ai/kurzbuch.rs`
(abschnittsweise, dann zusammenführen; Blockkennungen bleiben in jedem Abschnitt erhalten) und der Belegprüfung oben.
Weitere Erzeugnisse (Vermerk, E-Mail-Entwurf, Aufgaben, Präsentation) über die vorhandenen KI-Dokumente.

### 5.5 Relevanzprofil

Einstellungen → Ausgabe → Vorlesen und Inhalte → „Was ist für mich relevant“: Rolle, Themen mit Gewicht,
Ausschlüsse. **Dasselbe Profil** nutzt die Vorlage „Kanal → Wissen“ (Variable `profil` wird mit dem Profil
vorbelegt statt eigenem Text); kein zweites Profil. Gerätelokal, kein Sync in I5.

## 6 Freigaben und Datenklassen

### 6.1 Wann Inhalte an ein Modell gehen dürfen

| Inhalt | Lokales Modell | Cloud-Modell |
|---|---|---|
| eigene Seite ohne Projekt | erlaubt | nur Datenklasse ≤ INTERNAL **und** Anbieter-Freigabe je Anbieter (wie Buch-Gespräch D9) |
| Seite/Eintrag in einem Projekt | nur mit Projekt-Kontextfreigabe (P4c) | zusätzlich Anbieter-Freigabe |
| Ablage-Datei | nur bei KI-Freigabe des Ordners „an“ | zusätzlich Cloud-Ausnahme des Ordners für diesen Anbieter (ADR-0029) |
| Datei/Seite mit Klasse PERSONAL oder höher | erlaubt | nie |

Fehlt eine Freigabe, erscheint „Für KI nicht freigegeben“ mit „Freigabe ansehen“; Kurzfassung, Relevanzprüfung und
KI-Dokumente laufen nicht. Eine Routine hält dann an diesem Schritt an (§9), sie überspringt ihn nicht stumm.

### 6.2 Vererbung

Kurzfassung, KI-Dokumente, Übersetzungen, Kopien und von Routinen angelegte Seiten erben die **höchste** Datenklasse
ihrer Quellen und des Zielprojekts. Eine Routine darf nur in ein Projekt schreiben, dessen Klasse mindestens der
konfigurierten Quellklasse entspricht (Intranet: INTERNAL).

### 6.3 Abrufschutz

Dateiimport aus verknüpften Ordnern und lesende Vorschau fremder Ablage-Einträge nutzen den Abrufschutz und zeigen
dessen Zusage wörtlich: „Local Voice überspringt erkannte, nur online gespeicherte Dateien. Durch gleichzeitige
Änderungen des Synchronisationsdienstes kann beim Lesen dennoch ein automatischer Download ausgelöst werden.“
(akzeptiertes Restfenster, `docs/ablage-abrufschutz.md`).

## 7 Routinen (E6, Workflow-Engine)

### 7.1 Auslöser

- **`feed.new_item`** (neu, nach `youtube_channel`): RSS/Atom einer freigegebenen Adresse (§4.3), Abruf zur Uhrzeit
  (`at`, Karenz 15 min wie `schedule`) oder im Intervall. Lauf-Schlüssel `feed:<feed>:<guid|link-hash>`. Erster Abruf
  markiert vorhandene Einträge als bekannt (`backfill` 0–20). Ein Eintrag gilt erst als bekannt, wenn das Einreihen
  gelungen ist; scheitert es, bleibt er unbekannt und kommt beim nächsten Abruf wieder.
- **`web.new_links`** (neu): HTML-Listenseite mit CSS-Auswahl für Beitragslinks, gleiche Regeln.
- Kein eigener Planer, keine eigenen Zeitgeber: beide hängen am vorhandenen Takt.
- **Nachholen:** nur innerhalb der Karenz von 15 min nach der Uhrzeit (Vertrag `schedule`). War der Rechner um
  6:00 aus, holt der nächste Abruf die neuen Einträge, weil der Feed sie weiter listet (Feed nennt 15–50 Einträge);
  was aus dem Feed gefallen ist, ist verloren – das zeigt der Lauf als „Lücke möglich“, wenn der älteste Feedeintrag
  jünger ist als der letzte bekannte.

### 7.2 Aktionen

| Aktion | Wirkung | Recht |
|---|---|---|
| `content.import_url` (neu) | ruft ab (§4.3), legt Seite mit Eingangsmarke `pruefung` an (unsichtbar außer im Lauf) | „Inhalte anlegen“ |
| `content.relevance` (neu, Hülle um `knowledge.rate` + Relevanzprofil) | stuft ein; setzt Eingangsmarke `neu` oder `aussortiert` | „Inhalte anlegen“ |
| `content.brief` (neu, Stil `brief`, §5.4) | erzeugt Kurzfassung zur Seite | „KI nutzen“ + Freigaben §6 |
| `notify.local` (vorhanden) | Hinweis ohne Inhalt („Neuer Beitrag in Inhalte“) | vorhanden |

- **Reihenfolge Relevanz/Import:** Ein optionaler Vorfilter prüft nur Feed-Titel und -Teaser (Stichworte „muss
  enthalten“/„darf nicht enthalten“, ohne Modell). Danach Import, dann Relevanz auf dem Volltext. Nicht relevante
  Seiten landen unter *Routinen › Aussortiert* (14 Tage, wiederherstellbar, dann Papierkorb der Seiten) – nie
  stilles Löschen.
- **Dubletten:** Schlüssel kanonische URL (ohne Tracking-Parameter); gleiche URL mit neuem Inhalt (anderer SHA-256)
  ergibt eine neue Quellrevision derselben Seite, Kurzfassung wird veraltet und neu erzeugt; keine zweite Seite.
- **Fehler/Wiederholung:** Vertrag der Engine (Versuche, Wartezeit, „Lauf beenden“/„Weitermachen“). Eine Seite
  mit Marke `pruefung` aus einem abgebrochenen Lauf bleibt im Lauf sichtbar und wird beim Wiederholen weiterverwendet
  (Schlüssel), nicht doppelt angelegt.
- **Trockenlauf** (Vertrag `AUTOMATIONEN.md`): „Trockenlauf anzeigen“ berechnet den Plan je Schritt mit Parametern,
  Recht (Aus/Fragen/Erlaubt) und Bedingung; Auslöserdaten sind Beispieldaten; es wird nichts abgerufen, geschrieben,
  gesendet oder an ein Modell gegeben. Scharfschalten erst danach; Änderung → zurück in den Trockenlauf.
- Vorlagen (alle ausgeschaltet im Trockenlauf): „Intranet: neue Beiträge zu einem Thema“, Newsletter per E-Mail,
  Pressemitteilungen, Amtsblatt, Ausschreibungen, Fachportal, Förderaufrufe, Gesetzesänderungen, Verbandsnews,
  Stellenausschreibungen intern u. a. (10–20).

### 7.3 Abgrenzung zu Wissen

Seite = Arbeitsdokument mit Lesefassung, Quelle und Kurzfassung. Wissensnotiz (Vault, `knowledge.reconcile`) =
verdichtetes Dauerwissen; eine Routine kann sie zusätzlich erzeugen (Schritte der Vorlage „Kanal → Wissen“), ändert
bestehende Notizen nur nach Freigabe. Ablage-Index = Suche über Ablage-Dateien; Seiten liegen nicht darin, die Suche
im Baum nutzt den vorhandenen Seiten-Suchweg. Gemeinsam genutzt: Relevanzprofil, `knowledge.rate`, Dublettenschutz
über Ledger + kanonische URL.

### 7.4 Anmeldung an Intranet-Seiten

Betrifft URL-Import (I3) und Routinen (I6). Offen bis Patricks Antwort (§11). Reihenfolge der Wege: (1) ohne
Anmeldung im Behördennetz; (2) Windows-integriert (Kerberos/NTLM über das System, kein gespeichertes Passwort);
(3) M365 über die vorhandene Integration (SharePoint-News per Graph); (4) Sitzung aus einer einmaligen Anmeldung im
eingebetteten Fenster, gerätelokal im Tresor. Keine Passwörter im Klartext, keine Inhalte und Cookies im Log.

## 8 Autarker Betrieb und Modellfehler

- **Ohne Claude-/Codex-Installation vollständig nutzbar.** KI-Schritte des Inhalte-Moduls nutzen standardmäßig das
  lokale Modell (llama-server). Die CLI-Wege `cli://claude`/`cli://codex` werden **nie** automatisch gewählt, auch
  nicht als Ausweichweg; ein Cloud-Anbieter nur nach §6.1. Kein stiller Wechsel von lokal zu Cloud oder CLI.
- Import, Lesen, Bearbeiten, Vorlesen, Baum, Suche und Routinen ohne KI-Schritte funktionieren ohne jedes Modell.
- Modellbereitstellung: Download und Auswahl unter „Modelle“ (Thread „App ohne Claude-Installation lauffähig
  machen“); Mindestbedarf wird beim Einrichten der Kurzfassung genannt.
- Offline: lokales Modell, Lesen, Vorlesen gehen; URL-Import und Feed-Auslöser nicht (§9 „Offline“).

| Fall | Verhalten |
|---|---|
| kein lokales Modell eingerichtet | KI-Knöpfe zeigen „Kein lokales Modell eingerichtet“ + „Modell einrichten“; Routine hält am KI-Schritt |
| Modell startet nicht | „Lokales Modell startet nicht“ + „Erneut versuchen“, Fehlercode im Log ohne Inhalt |
| Arbeitsspeicher knapp | Schritt wartet am Speicher-Tor: „Wartet auf freien Arbeitsspeicher“, abbrechbar |
| ungültige JSON-Ausgabe | ein Reparaturversuch mit demselben Modell, dann „Kurzfassung konnte nicht erstellt werden“ |
| Kontext zu groß | Verdichtung in Abschnitten (§5.4); reicht das nicht: „Text zu lang für das Modell“ + Hinweis auf größeres Modell |
| Abbruch durch Nutzer | nichts gespeichert, vorherige Kurzfassung bleibt |

## 9 Zustände (Texte, Folgeaktionen)

| Zustand | Text | Aktionen |
|---|---|---|
| keine Kurzfassung | „Noch keine Kurzfassung.“ | Kurzfassung erstellen |
| ohne belastbare Belege | „Keine Aussage ließ sich im Text belegen.“ | Lesefassung öffnen, Neu erstellen |
| Aussagen ausgeblendet | „N Aussagen ohne Beleg ausgeblendet.“ | – |
| veraltet | „Veraltet: Text wurde nach der Kurzfassung geändert.“ | Neu erstellen |
| Modellfehler | siehe §8 | je Fall |
| nicht freigegeben | „Für KI nicht freigegeben.“ | Freigabe ansehen |
| Quelle geändert | „Die Quelle hat sich geändert (neue Fassung vom …).“ | Unterschiede ansehen, Übernehmen |
| Anmeldung abgelaufen | „Anmeldung bei intranet.example.org abgelaufen.“ | Neu anmelden; Routine wartet |
| offline | „Keine Verbindung. Abruf beim nächsten Lauf.“ | – |
| Import teilweise fehlgeschlagen | „3 von 4 Dateien importiert. ‚x.pdf‘: kein Text gefunden.“ | Details, Erneut versuchen |
| Freigabe ausstehend (Recht „Fragen“) | „Routine wartet auf deine Freigabe.“ | Ansehen, Freigeben, Ablehnen |
| Original fehlt | „Original nicht mehr vorhanden.“ | Aus Projekt entfernen |
| Text enthält Anweisungen an KI | „Der Text enthält Aufforderungen an eine KI. Sie wurden nicht befolgt.“ | Stellen anzeigen |
| leerer Baumknoten | „Noch keine Inhalte. Text einfügen, Datei ablegen oder Adresse importieren.“ | Neu |

## 10 Unvertrauenswürdige Inhalte

Inhalte aus Web, E-Mail, Dateien und Feeds sind **Daten, nie Anweisungen** (AUTOMATIONEN.md „Lokaler Agent“,
Buch-Gespräch D10): im Prompt als abgegrenzter Datenblock mit Hinweis; KI-Schritte des Inhalte-Moduls haben keine
Werkzeuge; Ziele, Empfänger, Projekte, Freigaben und Folgeschritte bestimmt nur der Ablauf bzw. die Person am Gerät, nie
das Modell oder der Text; Ausgaben werden nur über das Schema (§5.2) übernommen; erkannte Aufforderungen an die KI
werden angezeigt (§9). `content.relevance` liefert nur Stufe und Begründung, keine Aktion.

## 11 Offene Fragen

An Patrick (im Thread am 12.11.2026 gestellt; betreffen I3 bei angemeldeten Intranetseiten und I6): Intranet-System,
Feed oder Newsletter-Mail, Netz/VPN und Anmeldeart, Gerät und Modell.

Zur Abnahme (Architektur): E1–E6 (§12), Aufbewahrung der Revisionen (50 + Importbasen), Aussortiert-Frist (14 Tage),
Sync erst in I8.

## 12 Entscheidungen (mit Folgen)

| | Empfehlung | Folge | Alternative | Folge |
|---|---|---|---|---|
| E1 | „Inhalte“ ersetzt „Vorlesen“ | Umbenennung in Navigation, Hilfe, Tour; Vorlesen bleibt Funktion | Vorlesen bleibt, nur Baum dazu | Import/KI/Routinen unter falschem Namen |
| E2 | Verweise, Original im Modul, fremde Einträge lesend | kein Doppelbestand; Lebenszyklus §2.2 | Kopien | Doppelbestand, Abweichungen |
| E3 | Blockliste als führende Lesefassung, Sprechtext abgeleitet, Marken als Anker, Revisionen | kein Strukturverlust; Chip-Editor bei neuen Seiten nur Marken | HTML als Format | Sanitizer-Last, schwer zu belegen |
| E4 | automatische Bereinigung mit dauerhafter Rohfassung | eingefügter Text sofort lesbar, jederzeit umkehrbar | Vorschau mit Bestätigung | jeder Einfügevorgang ein Dialog |
| E5 | Schema mit Pflichtbeleg je Feld, Prüfung vor Anzeige | weniger, aber belegte Aussagen | freier Text | nicht prüfbar |
| E6 | vorhandene Engine + `feed.new_item`/`web.new_links` + `content.*` | Trockenlauf, Rechte, Ledger, Neustart geerbt | eigener Planer | zweiter Scheduler |

## 13 Ausbaustufen (je eigener PR)

| Stufe | Inhalt | hängt ab von |
|---|---|---|
| I0 | Konzept + Vorschau | – |
| I1 | Einfügen bereinigen + dauerhafte Rohfassung (`einfuegungen[]`) | I0-Abnahme |
| I2 | Lesefassung (Blöcke, Revisionen, Marken als Anker), Darstellung, HTML-Einfügen, Paket/Sicherung | I0 |
| I3 | Import Datei/E-Mail/URL mit Quelle, URL-Schutz | I2; angemeldete Intranetseiten zusätzlich §7.4 |
| I4 | Baum, Liste, „Neu für dich“, Quelle-Blatt, Menüpunkt „Inhalte“ | I0 |
| I5 | Kurzfassung (Stil `brief`, Belegprüfung) + Relevanzprofil | I2 |
| I6 | Routinen: `feed.new_item`, `web.new_links`, `content.*`, Vorlagen | I3, I5, Patricks Antwort |
| I7 | iOS nachziehen (Lesen, Kurzfassung, „Neu für dich“) | I4, I5, I8 |
| I8 | Sync von Lesefassung, Quelle, Kurzfassung | I2, I5 |

## 14 Fixrunde 1 (Codex-Befunde → Abschnitt)

B1 autark → §8; B2 Freigaben/Vererbung/Abrufschutz → §6; B3 Fassungen/Rohquelle/Revisionen/Marken → §3, §4.1;
B4 Belege → §5.1–5.3; B5 unvertrauenswürdig → §10; B6 Engine-Vertrag → §7.1–7.2, Vorschau 03; B7 Zustände → §9,
Vorschau 05; B8 mobil → §2.1, Vorschau 04/06. Hinweise: Generator → §5.4; Wissen → §7.3; Lebenszyklus → §2.2;
Sync → §3.5; URL → §4.3; Barrierefreiheit → §2.1, §5.3, Vorschau; Toast → §2.1; Etappen → §13.

## 15 Tests (je Stufe)

Rust: Revisionen/Journal, Blockkennungen (Teilen/Verbinden), Anker/Verwaisen, Belegprüfung (unbekannte Kennung,
Datum, leere Kurzfassung), URL-Schutz (DNS-Rebinding, private IP, Umleitung auf fremden Ursprung, Größe, Zeit),
Feed-Ledger (Einreihen scheitert, Dublette, neue Quellrevision), Freigaben (ohne Ordnerfreigabe kein Modellaufruf),
kein CLI-Weg. Node: Bereinigung ≥ 20 Vorlagen, Ableitung Sprechtext. Playwright: Baum, Quelle-Blatt 390 px, Zustände
§9, Tastatur (Baum, Reiter, Belege) bei 390/768/1440/2560 hell/dunkel. Nur synthetische Inhalte (`example.org`).
