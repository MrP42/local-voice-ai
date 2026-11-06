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
     "origin": "migration", "added_ms": 1730000000000},
    {"id": "ed_1730900000000_3f2a", "lang": "en", "title": "The Way to Serenity",
     "audio": {"file": "ed_1730900000000_3f2a__teil_001.m4b", …},
     "origin": "merge", "origin_book": "book_1730500000000", "added_ms": 1730900000000}
  ],
  "primary_edition": "ed_book_1730000000000",
  "edition_tombstones": [{"id": "ed_…", "removed_ms": 1731000000000}],
  "reading": {"edition": "ed_1730900000000_3f2a", "updated": 1731000000000, "device": "dev_pc"},
  "alignments": [{"a": "ed_book_…", "b": "ed_1730…", "chapters": [[0,0],[1,1],…],
                  "confirmed": true, "updated": 1731000000000}],
  "text": {…}, "audio": {…}, "language": "de"   // Spiegel der Hauptfassung (Schema 1)
}
```

- **Gleichwertig:** Alle Fassungen stehen in `editions`, jede mit stabiler `id`. Die Hauptfassung
  ist nur ein Verweis (`primary_edition`): sie liefert Titel, Cover und Sortierung.
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
  `ed_<buchkennung>`. Jedes Gerät erzeugt dieselbe Kennung, mehrfaches Migrieren ergibt dasselbe.
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

## 4. Fortschritt, Lesezeichen, Notizen

| Was | Schlüssel | Bemerkung |
| --- | --- | --- |
| Text je Fassung | `text:<seite>:<tab>` | unverändert, die Seite gehört genau einer Fassung |
| Hörbuch Hauptfassung (migriert) | `audio:<buch>` | unverändert |
| Hörbuch weiterer Fassungen | `audio:<buch>:<editionId>` | neu |
| Lesezeichen, Notizen, Hervorhebungen | `tts.bookmarks.<seite>` usw. | seitengebunden = fassungsgebunden |

- Zusammenlegen verschiebt keine Schlüssel und bildet **kein** Maximum: jede Fassung behält ihren
  eigenen Datensatz. „Zuletzt aktiv“ ist `reading` + Datensatz dieser Fassung; die Buchseite zeigt
  „Zuletzt: English, Kapitel 7, iPhone, vor 2 Std.“.
- Kartenfortschritt = Fortschritt der zuletzt aktiven Fassung.

## 5. Auswahl synchronisieren, Konflikte

- `reading` reist im `book`-Objekt (verschlüsselt wie alles). Beim Empfang wird es **feldweise**
  zusammengeführt: neueres `reading.updated` gewinnt, bei Gleichstand die kleinere `device`.
  Das übrige Buch folgt der bisherigen Regel (neueres `updated`).
- Positionen bleiben in den Fortschrittsdatensätzen je Fassung (bisherige Merge-Regel
  `mergeRecordText`). Ein Wechsel schreibt in einem Schritt: Zielposition in den Datensatz der
  Zielfassung, dann `reading`. Beides trägt denselben Zeitstempel.
- **Gleichzeitiger Wechsel / offline:** Jedes Gerät arbeitet mit seinem `reading` weiter. Nach dem
  Abgleich gewinnt der neuere Wechsel. Die verdrängte Fassung verliert nichts (eigener Datensatz).
  Spielt das Gerät gerade, wechselt es nicht stumm, sondern zeigt
  „Auf dem iPhone zuletzt English gehört (Kapitel 7). Dorthin wechseln?“.
- `alignments` und `edition_tombstones` werden vereinigt (Grabstein schlägt Fassung mit älterem
  `added_ms`), damit gelöschte Fassungen nicht wieder auftauchen.

## 6. Positionsabgleich und Zusagen

| Stufe | Wann | Anzeige |
| --- | --- | --- |
| **genau** | gleiche Fassung, Text ↔ Hörbuch über die Zeitachse (`audio.sync`) bzw. Satz-/Segmentabgleich | „an derselben Stelle“ |
| **kapitelgenau** | andere Fassung, Kapitelzuordnung **bestätigt** | „Kapitel 7, etwa gleiche Stelle im Kapitel“ |
| **ungefähr** | sonst (Anteil am ganzen Buch) | „ungefähr bei 34 %“ |

- Gleiche Kapitelzahl → Zuordnung 1:1 wird **vorgeschlagen**, gilt aber erst nach Bestätigung
  („Kapitel zuordnen …“). Ohne Bestätigung immer „ungefähr“.
- Kapitel: Inhaltsverzeichnis/Überschriften (Text), Kapitelmarken/Teile (Hörbuch), bei Hörbuch mit
  Transkript über die Zeitachse.

## 7. Zusammenlegen, Abtrennen, Löschen

- **Erkennen:** sicher nur bei gleichem Open-Library-Werkschlüssel oder gleichem Kerntitel + Autor
  in verschiedenen Sprachen. ISBN belegt keine übersetzungsübergreifende Gleichheit. Unsicher:
  gleicher Autor und je Sprache genau ein Kandidat. **Nie automatisch** zusammenlegen: sichere
  Paare per Hinweisleiste nach Klick, unsichere einzeln im Prüfen-Dialog (standardmäßig aus).
- **Zusammenlegen:** Quellbuch → Fassung im Ziel (`origin_book`, Cover nach `covers/`), Dateien
  umbenannt (Abschnitt 3), Quellordner in den Papierkorb. Undo-Toast + dauerhaft „Abtrennen“.
- **Abtrennen:** Fassung → eigenes Buch (Kennung `origin_book`, falls frei, sonst neu), mit
  Titel, Cover, Dateien, Seite und Fortschritt. Hauptfassung nur abtrennbar, wenn eine andere als
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

Ziele ≥ 44 px/pt (gemessen, `belege.json`), Menüs mit `menuitemradio`, Dialoge mit Fokusfalle,
Download-Status als Text, Kontrast kleiner Texte ≥ 4,5:1 (gemessen).

## 10. Umsetzung

PR 1 Kern (Rust + TS-Bibliothek + Swift-Modell-Durchreichen): Schema 2, Migration, Sync-Erweiterung,
Zusammenlegen/Abtrennen/Löschen, Abgleich-Funktion, Tests. PR 2 PC-Oberfläche. PR 3 iPhone.
