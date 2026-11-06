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

### 5.1 Merge des `book`-Objekts je Teil (nicht mehr „neueres Buch gewinnt“)

| Teil | Regel |
| --- | --- |
| Metadaten des Buchs (Titel der Hauptfassung, Autoren, Favorit, Cover) | wie bisher: neueres Buch-`updated` |
| `editions` | **je Kennung**: Vereinigung beider Seiten; gleiche Kennung → neueres `edition.updated` gewinnt (Gleichstand: kleinere Gerätekennung `edition.device`). Fehlt `editions` im Eingang (Altgerät), gelten die lokalen. |
| `edition_tombstones` | Vereinigung; ein Grabstein entfernt die Fassung endgültig (Kennungen werden nie wiederverwendet). |
| `primary_edition` | eigenes Tripel `primary = {edition, updated, device}`, höheres Tripel gewinnt; zeigt es auf eine begrabene Fassung, gilt die erste verbleibende nach `added_ms`. |
| `alignments` | je Fassungspaar, darin je Gruppe (Abschnitt 6); neueres `updated` gewinnt. |
| `reading` | Tripel (Abschnitt 5.2). |

### 5.2 Auswahl `reading`

```json
"reading": {"edition": "ed_…", "updated": 1731000000000, "device": "dev_ios",
            "position": {"key": "audio:book_2", "unit": "ms", "offset": 7385000}}
```

- **Ordnung:** (updated, device, edition) lexikografisch; das größere Tripel gewinnt auf jedem Gerät
  gleich, auch bei gleichem Zeitstempel.
- **Konsistenz:** `reading.position` ist die Stelle, mit der gewechselt wurde. Gewinnt beim Empfang
  das fremde `reading`, wird der Fortschrittsdatensatz `position.key` auf diese Stelle gehoben,
  sofern sein eigenes `updated` älter ist als `reading.updated`; ist er neuer (dort seither weiter
  gehört), bleibt er. Auswahl und Stelle passen danach immer zueinander.
- **Offline / gleichzeitig:** Jedes Gerät arbeitet weiter; nach dem Abgleich gilt das größere
  Tripel. Die verdrängte Fassung behält ihren Datensatz. Spielt das Gerät gerade, wechselt es nicht
  stumm, sondern fragt (sf-11).

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

## 7. Zusammenlegen, Abtrennen, Löschen

- **Erkennen:** sicher nur bei gleichem Open-Library-Werkschlüssel oder gleichem Kerntitel + Autor
  in verschiedenen Sprachen. ISBN belegt keine übersetzungsübergreifende Gleichheit. Unsicher:
  gleicher Autor und je Sprache genau ein Kandidat. **Nie automatisch** zusammenlegen: sichere
  Paare per Hinweisleiste nach Klick, unsichere einzeln im Prüfen-Dialog (standardmäßig aus).
- **Zusammenlegen:** Quellbuch → Fassung im Ziel (`origin_book`, Cover nach `covers/`), Dateien
  umbenannt (Abschnitt 3), Quellordner in den Papierkorb. Undo-Toast + dauerhaft „Abtrennen“.
- **Abtrennen:** Fassung → eigenes Buch (Kennung `origin_book`, falls frei, sonst neu), mit
  Titel, Cover, Dateien, Seite und unverändertem `audio_key`/`page_id` (Fortschritt, Lesezeichen,
  Notizen gehen ohne Umschlüsseln mit). Hauptfassung nur abtrennbar, wenn eine andere als
  Ersatz gewählt wird.
- **Getrennte Aktionen:** „Download entfernen“ (nur Dateien dieses Geräts), „Sprachfassung löschen“
  (Grabstein, Papierkorb 30 Tage; nicht für Haupt-/aktive Fassung ohne Ersatz), „Buch löschen“.

## 8. Sprachwechsel im Player

- PC: Knopf „Sprache: Deutsch, Auswahl öffnen“ in der Spielerleiste, Dropup. iPhone: Knopf im
  Kopf öffnet **dasselbe Blatt wie „Quelle“** (lva#326): oben Sprache/Fassung, darunter Quelle
  (Hörbuch / Vorlesen mit Stimme). Ein Auswahlzustand, eine Optionsliste.
- Nach dem Wechsel Toast mit Stufe (Abschnitt 6) und „Rückgängig“: 10 s, pausiert bei
  Mauszeiger/Fokus, stellt Fassung **und** Position wieder her.
- Fehlt das Hörbuch der Fassung auf dem Gerät: Eintrag zeigt „Laden (412 MB)“, Wechsel startet
  den Download und spielt bis dahin weiter.

## 9. Barrierefreiheit

Ziele ≥ 44 px/pt, Menüs mit `menuitemradio`, Dialoge mit Fokusfalle, Download-Status als Text,
Kontrast aller Texte ≥ 4,5:1. Beleg `belege.json` je Bild: **Messumfang** (Selektor, Anzahl),
jedes Bedienziel mit Element, Beschriftung und Rechteck, jeder sichtbare Textknoten mit
Vorder-/Hintergrund **nach Alpha-Verrechnung** über alle Ebenen bis zur Seitenfläche und Verhältnis.

## 10. Umsetzung

PR 1 Kern (Rust + TS-Bibliothek + Swift-Modell-Durchreichen): Schema 2, Migration, Sync-Erweiterung,
Zusammenlegen/Abtrennen/Löschen, Abgleich-Funktion, Tests. PR 2 PC-Oberfläche. PR 3 iPhone.
