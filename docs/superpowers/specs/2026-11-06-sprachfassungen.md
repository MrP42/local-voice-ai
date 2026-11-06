# Sprachfassungen eines Buchs (lva#344)

Stand 06.11.2026, Runde 2 nach Codex-Befund. Ein Werk steht einmal in der Bibliothek; jede
**Sprachfassung** (Objekt) hat Text und/oder Hörbuch in einer Sprache. Im Player wählt man die
**Sprache** (Auswahl). „Ausgabe“ bleibt der bibliografische Begriff (Open Library).

## 1. Datenmodell `buch.json` (Schema 2)

```json
{
  "id": "book_1730000000000",
  "schema": 2,
  "title": "Der Weg zur Gelassenheit",          // Anzeige = Hauptfassung
  "editions": [
    {"id": "ed_book_1730000000000", "lang": "de", "title": "Der Weg zur Gelassenheit",
     "isbn": "978…", "work_key": "/works/OL1W", "text": {…TextVersion}, "audio": {…AudioVersion},
     "audio_key": "audio:book_1730000000000",
     "origin": "migration", "added_ms": 1730000000000, "updated": 1730000000000, "device": ""},
    {"id": "ed_1730900000000_3f2a", "lang": "en", "title": "The Way to Serenity",
     "audio": {"file": "ed_1730900000000_3f2a__teil_001.m4b", …},
     "audio_key": "audio:book_1730500000000",
     "origin": "merge", "origin_book": "book_1730500000000", "added_ms": 1730900000000,
     "updated": 1730900000000, "device": "dev_pc"}
  ],
  "primary": {"edition": "ed_book_1730000000000", "updated": 1730000000000, "device": ""},
  "edition_tombstones": [{"id": "ed_…", "removed_ms": 1731000000000}],
  "reading": {"edition": "ed_1730900000000_3f2a", "updated": 1731000000000, "device": "dev_pc",
              "position": {"key": "audio:book_1730500000000", "unit": "ms", "offset": 7385000}},
  "alignments": [ … Abschnitt 6.1 … ],
  "text": {…}, "audio": {…}, "language": "de"   // Spiegel der Hauptfassung (Schema 1)
}
```

- **Gleichwertig:** Alle Fassungen stehen in `editions`, jede mit stabiler `id`. Die Hauptfassung
  ist nur ein Verweis (`primary.edition`): sie liefert Titel, Cover und Sortierung.
- Mehrere Fassungen derselben Sprache sind erlaubt (z. B. zwei englische Hörbücher); Auswahl,
  Fortschritt und Zuordnung hängen immer an der `id`, nie an der Sprache.
- **Spiegel:** `text`, `audio`, `language` werden beim Schreiben aus der Hauptfassung gesetzt.
  Ältere Stände (Desktop vor lva#344, iPhone) lesen weiter ein gültiges Buch. Schema-2-Geräte lesen
  den Spiegel nie als Quelle, außer bei der Migration.
- KI-Übersetzungen bleiben abgeleitete Inhalte der Vorlesen-Seite einer Fassung; sie sind keine
  Sprachfassung und werden überall als „KI-Übersetzung“ gekennzeichnet.

## 2. Migration Schema 1 → 2 (versioniert, wiederholbar)

- Auslöser: `schema` fehlt oder < 2 beim Lesen. Ergebnis wird erst beim nächsten Schreiben
  gespeichert; Lesen ändert keine Datei.
- Fehlen `editions`: eine Fassung aus `text`/`audio`/`language`, Kennung **deterministisch**
  `ed_<buchkennung>`, `audio_key` = `audio:<buchkennung>`, `updated` = `added_ms`, `device` leer.
  Jedes Gerät erzeugt dieselbe Fassung, mehrfaches Migrieren ergibt dasselbe.
- Medien bleiben, wo sie sind: Dateinamen, Seitenkennungen, Fortschrittsschlüssel
  (`audio:<buch>`, `text:<seite>:…`), Lesezeichen (`tts.bookmarks.<seite>`) unverändert.
- Ein Schema-2-Buch, das ein Altgerät ohne `editions` zurückschreibt, verliert nichts: Beim
  Empfang gilt Feld-Merge (Abschnitt 5); fehlende `editions` im Eingang heißen „unverändert“,
  nicht „gelöscht“.

## 3. Dateien über Kennungen

- Hörbuchteile liegen flach in `<buch>/audio/`. Bestehende Namen bleiben. Neu angelegte oder beim
  Zusammenlegen übernommene Teile heißen `<editionId>__<name>`; Kollisionen sind damit
  ausgeschlossen.
- Jede Fassung hat ihre eigene Vorlesen-Seite (`text.page_id`). Cover einer übernommenen Fassung:
  `<buch>/covers/<editionId>.<ext>` (für „Abtrennen“).
- Sync-`files` (Erweiterung von `2026-11-02-bibliothek-sync.md`):

```json
"files": {
  "text": {…}, "cover": {…}, "audio": [{"file": "teil_001.mp3", …}],      // Hauptfassung, wie bisher
  "editions": {
    "ed_1730900000000_3f2a": {"text": {…}, "cover": {…},
                               "audio": [{"file": "ed_1730900000000_3f2a__teil_001.m4b", …}]}
  }
}
```

  Altgeräte ignorieren `files.editions`. Das iPhone lädt Hörbuchteile je Fassung erst bei Bedarf.

## 4. Fortschritt, Lesezeichen, Notizen: dauerhafte Referenzen

Jede Fassung trägt ihre Fortschrittsreferenzen **selbst**; sie werden einmal vergeben und nie
umgeschlüsselt, auch nicht beim Zusammenlegen, Abtrennen oder Wechsel der Hauptfassung.

| Was | Referenz | Vergabe |
| --- | --- | --- |
| Hörbuch | `edition.audio_key` | Migration: `audio:<buch>` (bisheriger Schlüssel). Neue Fassung: `audio:<buch>:<editionId>`. Zusammenlegen: Schlüssel der Quelle bleibt (z. B. `audio:book_2`). |
| Text | `text:<seite>:<tab>` | an `text.page_id` gebunden, die Seite gehört genau einer Fassung |
| Lesezeichen, Notizen, Hervorhebungen | `tts.bookmarks.<seite>` usw. | seitengebunden = fassungsgebunden |

- Die Oberfläche liest den Hörbuch-Fortschritt immer über `edition.audio_key`, nie über die
  Buchkennung. Abtrennen nimmt `audio_key` mit; das neue Buch findet seinen Fortschritt also auch
  unter neuer Kennung. Rückweg verlustfrei, weil nichts umbenannt wird.
- Zusammenlegen bildet **kein** Maximum: jede Fassung behält ihren Datensatz. „Zuletzt aktiv“ =
  `reading` (Abschnitt 5). Kartenfortschritt = Fortschritt der zuletzt aktiven Fassung.

## 5. Synchronisieren und Konflikte

### 5.1 Zustandsbasierter Merge (im Code: `library/editions.rs`, `merge_state`)

Der Fassungsteil eines Buchs ist ein Zustand aus Registern und Mengen. `merge_state(x, y)` ist eine
reine Funktion; **kommutativ, assoziativ, idempotent** und absorbierend sind als Property-Test
belegt (`merge_kommutativ_assoziativ_idempotent`, 5000 zufällige Tripel je Lauf, inkl. gleicher
Zeitstempel, gleicher `added_ms`, verschiedener Inhalte bei gleicher Version). Jede Reihenfolge des
Empfangs ergibt denselben Stand.

| Teil | Typ | Ordnung / Regel |
| --- | --- | --- |
| `editions` | Map Kennung → Register | größtes (`updated`, `device`, Inhaltshash FNV-1a der kanonischen JSON); Hash entscheidet gleiche Version mit verschiedenem Inhalt |
| `edition_tombstones` | Grow-only-Menge | je Kennung größtes `removed_ms`; eine begrabene Kennung ist in jedem Ergebnis entfernt |
| `primary` | Register | größtes (`updated`, `device`, `edition`) |
| `reading` | Register | größtes (`updated`, `device`, `edition`, Hash der Stelle) |
| `alignments` | Map Fassungspaar (ungerichtet) → Register über die **ganze** Gruppenliste | größtes (`updated`, `device`, Hash der Gruppen); ungültige Listen (Lücke, Doppelzuordnung) werden verworfen |
| Metadaten (Autoren, Favorit, Beschreibung) | wie bisher | neueres Buch-`updated` |

**Wirksame Sicht** (nach dem Merge, verändert die Register nicht, damit die Assoziativität hält):
- Hauptfassung = `primary`, falls die Fassung lebt; sonst die älteste lebende nach (`added_ms`, Kennung).
- **Titel, Untertitel, ISBN, Werkschlüssel, Sprache und Schema-1-Spiegel** (`text`, `audio`,
  `language`) kommen immer aus der wirksamen Hauptfassung (`apply_state`), nie aus dem
  Buch-Zeitstempel. Das Buchcover ist das Cover der Hauptfassung; beim Wechsel tauschen
  `cover.*` und `covers/<editionId>.*`.
- Zuordnungen gelten nur zwischen lebenden Fassungen.
- Ein Altgerät ohne `editions` liefert nur eine Änderung der Arbeitskopie der Hauptfassung; sie
  wird als neue Version dieser Fassung eingespielt, alles andere bleibt lokal.
- Ist der lokale Buchstand neuer und ungesichert, gewinnen die lokalen Metadaten, der Fassungsteil
  wird trotzdem zusammengeführt und mit dem nächsten Push verteilt.

### 5.2 Auswahl `reading`

```json
"reading": {"edition": "ed_…", "updated": 1731000000000, "device": "dev_ios",
            "position": {"key": "audio:book_2", "unit": "ms", "offset": 7385000}}
```

- Wirksam nur, wenn die Fassung lebt. Zeigt die Auswahl auf eine gelöschte oder abgetrennte
  Fassung, gilt die Hauptfassung; die Stelle entfällt.
- `position.key` muss der Fortschrittsschlüssel **dieser** Fassung sein (`audio_key` oder
  `text:<eigene Seite>:…`), sonst entfällt die Stelle (`effective_reading`).
- Gewinnt beim Empfang ein fremdes `reading`, wird der Datensatz `position.key` auf die Stelle
  gehoben, wenn sein `updated` **echt kleiner** als `reading.updated` ist. Gleicher Zeitstempel:
  der lokale Datensatz bleibt (seine eigene Merge-Regel `mergeRecordText` gilt). Gehörte/gelesene
  Bereiche und `finished` bleiben immer erhalten (`lift_progress`).
- Spielt das Gerät gerade, wechselt es nicht stumm, sondern fragt (sf-11).

## 6. Positionsabgleich, Kapitelzuordnung

### 6.1 Modell

```json
"alignments": [{"a": "ed_de", "b": "ed_en", "updated": 1731000000000, "device": "dev_pc",
  "groups": [
    {"a": [0], "b": [0], "confirmed": true},
    {"a": [2], "b": [2, 3], "confirmed": true},        // 1:n
    {"a": [3, 4], "b": [4], "confirmed": false},       // n:1, noch offen
    {"a": [17], "b": [19, 20], "confirmed": true}
  ]}]
```

- Die Gruppenliste eines Fassungspaars ist **ein** Register (keine Gruppenkennungen): Teilen,
  Verbinden, Grenzen verschieben und Bestätigen erzeugen eine neue, vollständig geprüfte Liste mit
  neuem `updated`. Damit kann der Merge weder Lücken noch Doppelzuordnungen erzeugen. Grenze:
  bearbeiten zwei Geräte offline verschiedene Gruppen desselben Paars, gilt die jüngere Liste
  ganz; die andere Bearbeitung ist sichtbar als unbestätigte Gruppe neu zu machen.
- Gruppen sind zusammenhängende Kapitelbereiche, lückenlos und in Reihenfolge; jedes Kapitel beider
  Fassungen gehört genau einer Gruppe (1:1, 1:n, n:1, n:m). Eine vollständige Zuordnung deckt
  18 ↔ 21 Kapitel ab.
- Bearbeiten: Gruppen teilen, mit der nächsten verbinden, Grenze verschieben, je Gruppe bestätigen.
  „Alle bestätigen“ gibt es nur, wenn jede Gruppe 1:1 ist **und** beide Kapitelzahlen gleich sind;
  sonst muss jede Gruppe mit mehr als einem Kapitel einzeln bestätigt werden.

### 6.2 Wechselregel und Zusagen

| Stufe | Wann | Zielstelle | Anzeige |
| --- | --- | --- | --- |
| **genau** | gleiche Fassung, Text ↔ Hörbuch (Zeitachse/Segmente) | Segmentstelle | „an derselben Stelle“ |
| **kapitelgenau** | Quellkapitel liegt in einer **bestätigten** Gruppe | Anteil innerhalb der Gruppe (Summe ihrer Kapitel) auf die Zielgruppe übertragen | „Kapitel 7, etwa gleiche Stelle im Kapitel“ bzw. bei 1:n „Kapitel 3–4, etwa gleiche Stelle“ |
| **ungefähr** | Gruppe unbestätigt oder keine Zuordnung | Anteil am ganzen Buch | „ungefähr bei 34 %“ + „Kapitel zuordnen“ |

- Unbestätigte Gruppen werden nie als „kapitelgenau“ angezeigt, auch wenn der Vorschlag plausibel ist.
- Kapitelquellen: Inhaltsverzeichnis/Überschriften (Text), Kapitelmarken/Teile (Hörbuch), Hörbuch
  mit Transkript über die Zeitachse.

## 7. Zusammenlegen, Abtrennen, Löschen, Wiederherstellen

- **Erkennen:** sicher nur bei gleichem Open-Library-Werkschlüssel oder gleichem Kerntitel + Autor
  in verschiedenen Sprachen. ISBN belegt keine übersetzungsübergreifende Gleichheit. Unsicher:
  gleicher Autor und je Sprache genau ein Kandidat. **Nie automatisch** zusammenlegen: sichere
  Paare per Hinweisleiste nach Klick, unsichere einzeln im Prüfen-Dialog (standardmäßig aus).
- **Zusammenlegen** (`join_in`): **alle** Fassungen des Quellbuchs (auch mehrfassige) werden mit
  **neuen** Kennungen Teil des Ziels; `audio_key`, `page_id` und Lesezeichen bleiben, Hörbuchteile
  heißen `<neueKennung>__<name>`, das Cover der Quell-Hauptfassung liegt unter `covers/`. Das
  Quellbuch geht als Ganzes in den Papierkorb und bekommt im Sync den Buch-Grabstein. Eine Kennung
  lebt so immer in genau einem Buch.
- **Abtrennen** (`split_in`, zugleich „Zusammenlegen rückgängig“): Fassung → eigenes Buch (Kennung
  `origin_book`, falls frei, sonst neu), mit Titel, Cover, Dateien und unverändertem
  `audio_key`/`page_id`. Im alten Buch bekommt die Fassung einen Grabstein. Hauptfassung nur
  abtrennbar, wenn eine andere als Ersatz gewählt wird.
- **Löschen** (`remove_in`): Grabstein, Dateien 30 Tage unter `.papierkorb/<buch>~<fassung>/`.
  Haupt- oder gewählte Fassung nur mit Ersatz.
- **Wiederherstellen** (`restore_in`): neue, **deterministische** Kennung `<alt>_r` mit denselben
  Referenzen. Zwei Geräte, die dieselbe Fassung wiederherstellen, erzeugen dieselbe Kennung;
  gleichzeitiges Löschen (alte Kennung) und Wiederherstellen (neue Kennung) konvergiert zur
  wiederhergestellten Fassung (Test `wiederherstellen_deterministisch`).
- **Getrennte Aktionen:** „Download entfernen“ (nur Dateien dieses Geräts), „Sprachfassung löschen“,
  „Buch löschen“.
- **Grenze (dokumentiert):** Buchübergreifende Operationen auf zwei Geräten gleichzeitig offline
  (A trennt Fassung E ab, B legt dasselbe Buch mit einem dritten zusammen) können zwei Bücher mit
  denselben Referenzen (`audio_key`/`page_id`) ergeben. Es geht nichts verloren; die Bibliothek
  erkennt gleiche Referenzen in zwei Büchern und bietet „Zusammenlegen“ an.

## 8. Sprachwechsel im Player

- PC: Knopf „Sprache: Deutsch, Auswahl öffnen“ in der Spielerleiste, Dropup. iPhone: Knopf im
  Kopf öffnet **dasselbe Blatt wie „Quelle“** (lva#326): oben Sprache/Fassung, darunter Quelle
  (Hörbuch / Vorlesen mit Stimme). Ein Auswahlzustand, eine Optionsliste.
- Nach dem Wechsel Toast mit Stufe (Abschnitt 6) und „Rückgängig“: 10 s, pausiert bei
  Mauszeiger/Fokus, stellt Fassung **und** Position wieder her.
- Fehlt das Hörbuch der Fassung auf dem Gerät: Eintrag zeigt „Laden (412 MB)“, Wechsel startet
  den Download und spielt bis dahin weiter.

## 9. Barrierefreiheit

Ziele ≥ 44 px/pt (einschließlich Navigation `div.n`), Menüs mit `menuitemradio`, Dialoge mit Fokusfalle, Download-Status als Text,
Kontrast aller Texte ≥ 4,5:1. Beleg `belege.json` je Bild: **Messumfang** (Selektor, Anzahl),
jedes Bedienziel mit Element, Beschriftung und Rechteck, jeder sichtbare Textknoten mit
verrechneter Vorder- und Hintergrundfarbe (Alpha über alle Ebenen bis zur ersten deckenden Fläche,
bei Verläufen die ungünstigste Stufe, Cover eingeschlossen) und Verhältnis. Kurzfassung in
`kennzahlen.json`.

## 10. Umsetzung

PR 1 Kern (Rust + TS-Bibliothek + Swift-Modell-Durchreichen): Schema 2, Migration, Sync-Erweiterung,
Zusammenlegen/Abtrennen/Löschen, Abgleich-Funktion, Tests. PR 2 PC-Oberfläche. PR 3 iPhone.
