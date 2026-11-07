# Buch-Ordner im Vorlesen und appweite Projekte

Stand: 07.11.2026 · Status: Entwurf zur Codex-Abnahme · Vorschau: `apps/local-voice/docs/vorschau/projektordner/`

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
| Vorlesen | Eine flache Liste aller Seiten links (Buchtexte, Übersetzungen, lose Texte, Audible-Reste gemischt). Darunter die Kapitelliste. Reiter „Original / Übersetzung / KI-Dokumente“ über dem Reader. | `projects/index.json` + `projects/<seite>/state.json`, KI-Dokumente in `projects/<seite>/ki/dokumente/` |
| Bibliothek | Karten je Werk, Buchdetail mit Fassungen und Kurzbuch | `library/<buch>/buch.json`, `cover.*`, `audio/` |
| Buch im Ganzen | Ein Werk verteilt sich auf bis zu vier Orte: `library/<buch>/`, je Fassung eine Seite in `projects/`, KI-Dokumente in der Seite, Transkript im Besprechungsauftrag. | — |
| Aufnahmen | Drei Ebenen: Projekte-Leiste (Alle, Ohne Projekt, Ordner; einklappbar zu Kürzeln; Ziehen = verschieben/hinzufügen), Liste, Arbeitsfläche, rechte Spalte (Transkript). Breiten nach `useRecLayout` (breit ab 1000 px, gestapelt bis 620 px). | `meeting_folders`, Spalte `projekt_id` (ADR-0026 PR-7) |
| Suite-Projekte | Projektseite mit Einträgen und Recherche (ADR-0026, PR-5/PR-7) | `wai-projekte`, Einträge sind Verweise; Arten `book` und `book_document` gibt es schon |

Folge: Wer ein Buch bearbeitet, sucht Original, Übersetzung, Kurzbuch und Notizen an verschiedenen
Stellen. Die KI bekommt keinen sauber abgegrenzten Kontext, und es gibt keinen Ort, an dem Ergebnisse
landen.

## 2 Begriffe

| Begriff | Bedeutung |
|---|---|
| **Buch-Ordner** | Ein Werk mit allem, was dazugehört: Fassungen je Sprache, Hörbuch, Transkript, Kurzbuch, KI-Dokumente, Notizen, Anhänge. Physisch ein Verzeichnis. |
| **Texte** | Lose Vorlesen-Seiten ohne Buch (eingefügter Text, Webseite, Dokument). |
| **Projekt** | Suite-Projekt nach ADR-0026. Appweit, bündelt Verweise auf Aufnahmen, Bücher, Texte, Gespräche, Recherchen und eigene Dateien. |

Ein Buch ist kein Projekt. Ein Buch kann in beliebig vielen Projekten stehen (Verweis).

## 3 Zielbild Vorlesen (Struktur wie Aufnahmen)

Vorlesen bekommt dasselbe Drei-Ebenen-Muster wie Aufnahmen und nutzt dieselben Breiten und Regeln
(`useRecLayout`, einklappbare Leiste mit Kürzeln, Ziehen und Ablegen).

1. **Ordnerleiste** (links, einklappbar):
   - Alle · Zuletzt geöffnet
   - **Bücher** (aufklappbar, ein Eintrag je Buch mit Mini-Cover und Fortschritt)
   - **Texte** (lose Seiten)
   - **Projekte** (Suite-Projekte, wie in Aufnahmen)
2. **Inhaltsliste** (Mitte): Inhalt des gewählten Ordners. Im Buch-Ordner gruppiert nach
   *Fassungen* (je Sprache: Text, Hörbuch, Fortschritt), *KI-Dokumente* (Kurzbuch, Zusammenfassungen,
   Begriffe), *Notizen*, *Anhänge*. Kopf mit Cover, Titel, „Weiterlesen“ und Menü (Hinzufügen,
   Im Explorer zeigen, Zu Projekt hinzufügen).
3. **Arbeitsfläche**: der Reader wie heute. Die Reiter „Original / Übersetzung / KI-Dokumente“
   entfallen, weil die Auswahl in der Inhaltsliste geschieht. Der Sprachwechsel im Reader bleibt
   (Sprachfassungen lva#361).
4. **Rechte Spalte** (einklappbar): Inhalt/Kapitel, Notizen, Chat zum Buch. Die Kapitelliste wandert
   damit aus der linken Spalte nach rechts; links stehen nur noch Ordner.

Schmal (≤ 620 px, Smartphone): Ordner, Inhaltsliste und Reader sind drei Ebenen mit Zurück-Pfeil;
die rechte Spalte wird ein Blatt von unten. Touch-Ziele ≥ 44 px, keine waagrechte Scrollleiste.

Bibliothek bleibt der Katalog (Karten, Cover, Weitermachen). „Öffnen“ führt in den Buch-Ordner
im Vorlesen. Das Buchdetail zeigt zusätzlich „Dateien im Buch-Ordner“ mit „Im Explorer zeigen“.

Ziehen und Ablegen wie bisher (lva#200): auf einen Buch-Ordner = Fassung oder Anhang anhängen,
auf „Texte“ = neuer Text, auf ein Projekt = Verweis hinzufügen.

## 4 Ablage: ein Verzeichnis je Buch

```
<Datenordner>/library/                      Bücher
  <buch-id>/
    buch.json                               Metadaten, Fassungen, Fortschrittsschlüssel (wie heute)
    cover.jpg
    fassungen/
      <sprache>-<fassung-id>/               je Sprachfassung: bisherige Seite (state.json, Quelldatei)
        state.json
        quelle.epub | quelle.pdf | …
        ki/dokumente/<ulid>.md + index.json KI-Dokumente dieser Fassung (Kurzbuch usw.)
        ki/cache/
    audio/                                  Hörbuch-Teile (wie heute)
    transkript/<sprache>.md                 lesbarer Text des Hörbuch-Transkripts (Zeitachse bleibt im Auftrag)
    notizen/
    anhaenge/                               beliebige Dateien zum Buch
    sync.json                               wie heute
<Datenordner>/projects/                     Texte (lose Seiten), Ordnername bleibt aus Kompatibilität
```

- **Seitenkennungen bleiben gleich.** `projects/index.json` bekommt je Seite ein Feld `buch`
  (Buch-ID oder leer). `page_path()` löst darüber auf; alle Aufrufer (7 Dateien) laufen ohnehin über
  diese Funktion. Fortschritt, Lesezeichen und Sync-Schlüssel ändern sich nicht.
- **Migration** (einmalig, wiederholbar, Schema 3), Auflagen aus ADR-0026 PR-8b übernommen:
  Sicherung vor dem Lauf (`library/.sicherung-<datum>/index.json` + Liste aller verschobenen Pfade),
  Verschieben per atomarem `rename` je Seite, Sperre gegen gleichzeitige Läufe, Prüfung nach jedem
  Schritt, bei Fehler Rückfall auf den alten Ort, Abbruch hinterlässt einen lesbaren Zustand.
  Nicht beschreibbarer Pfad wird verständlich gemeldet. Löschkaskade: Buch in den Papierkorb nimmt den
  ganzen Buch-Ordner mit (heute bleiben Seiten liegen).
- Ältere Version der App nach der Migration: findet die Buchseiten nicht mehr unter `projects/`.
  Deshalb Schema-Sprung mit Hinweis „Neuere Bibliothek, bitte aktualisieren“ statt stillem Leerlauf.

## 5 Appweite Projekte

Grundlage bleibt ADR-0026: ein Suite-Projekt mit ULID, Einträge sind Verweise, gemeinsame Ablage
`<Datenordner>/WolffAppliedAI/projekte/`. Neu sind drei Funktionen.

### 5.1 Bündeln

- Die Projekte-Gruppe steht in der Leiste jeder Arbeitsseite: Aufnahmen (gibt es), Vorlesen (neu),
  Sprachgespräch/Chat (neu). Ein Projekt sieht in jeder Seite gleich aus.
- Die Projektseite zeigt alle Einträge gruppiert: Aufnahmen, Bücher, Texte, Gespräche, Recherchen,
  Dateien, Ausgaben.
- App-Inhalte (Aufnahme, Buch, Text, Gespräch) werden **verwiesen**, nicht kopiert.
- **Eigene Dateien** (PDF, DOCX, Bilder, Tabellen), die ins Projekt gezogen werden, werden in den
  Projektordner `projekte/<id>/dateien/` **kopiert**, damit das Projekt vollständig ist.

### 5.2 KI-Kontext

- Je Eintrag ein Schalter „Als Kontext“ (Standard: an für Texte, Bücher als Kurzbuch, Aufnahmen als
  Protokoll; aus für große Rohdaten).
- Chat, Sprachgespräch, KI-Dokumente und Recherche im Projekt nutzen nur markierte Einträge.
- Anzeige des Kontextumfangs (geschätzte Token) und Warnung über dem Fenster des gewählten Modells.
- Datenklasse und Freigaben wie ADR-0026 (Web separat freigeben, keine Inhalte ins Log).

### 5.3 Zielverzeichnis

- Ein **aktives Projekt** (Kopfzeile, Profil-unabhängig, je Fenster) ist das Ziel für neue Ausgaben:
  KI-Dokumente, Protokolle, Exporte, Recherche-Berichte, neue Aufnahmen, Übersetzungen.
- Das Ziel ist vor jeder Ausgabe sichtbar („Speichert in: Projekt Finanzwissen“) und umschaltbar.
- Dateiexporte landen in `projekte/<id>/ausgaben/`; App-Inhalte bekommen den Verweis.

### 5.4 Gezielt verarbeiten

Mehrfachauswahl in der Projektseite mit Aktionen: Zusammenfassen, Übersetzen, Vorlesen/Hörbuch
erzeugen, Recherche mit Auswahl als Quelle, Export als Paket.

## 6 Plattformen und Sync

| | Windows / macOS (Tauri) | iOS |
|---|---|---|
| Vorlesen-Struktur | gleich (P1) | Bibliothek → Buch-Ordner als Liste der Inhalte (P3) |
| Buch-Ordner-Ablage | Migration (P2) | eigene Ablage je Buch, gleiche Unterteilung (P3) |
| Projekte | volle Funktion (P4, P5) | Projekte lesen, Einträge öffnen, Kontext-Schalter, Ziel wählen (P6) |
| Sync | Sammlung `book` trägt Dateien je Unterordner; Seiten der Fassungen reisen mit dem Buch. Neue Sammlung `projekt_eintrag` (Mitgliedschaft + Kontext-Schalter), Projektdateien über die vorhandene Dateiablage (Blobs). | gleich |

## 7 Entscheidungen für Codex

| Nr. | Frage | Empfehlung | Alternative |
|---|---|---|---|
| E1 | Buchtexte physisch nach `library/<buch>/` verschieben? | **Ja**, mit Migration nach Abschnitt 4 | Nur logisch gruppieren (kein Datenumbau, aber Patricks Wunsch „eigenes Verzeichnis“ nicht erfüllt) |
| E2 | Reiter „Original / Übersetzung / KI-Dokumente“ ersetzen? | **Ja**, durch die Inhaltsliste | Reiter behalten und Liste zusätzlich |
| E3 | Wie kommen Dateien ins Projekt? | **Hybrid**: App-Inhalte als Verweis, fremde Dateien als Kopie in `dateien/` | Alles Verweis (Projekt bricht, wenn Datei verschoben wird) / alles Kopie (Doppelte) |
| E4 | Zielverzeichnis | **Aktives Projekt** ist Ziel, sichtbar und umschaltbar | Bei jeder Ausgabe fragen |
| E5 | Buch in mehreren Projekten? | **Ja**, 0..n Verweise | 0..1 wie Planner-Plan |

## 8 Pakete

| Paket | Inhalt | Abhängig von |
|---|---|---|
| P1 | Vorlesen: Ordnerleiste, Inhaltsliste, Kapitel rechts, schmale Ebenen. Gruppierung aus vorhandenen Daten (`buch.json` → Seiten), noch ohne Datenumbau. | — |
| P2 | Buch-Ordner-Ablage, `index.json` Feld `buch`, Migration mit Sicherung/Rückfall, Löschkaskade, Buchdetail „Dateien im Buch-Ordner“. | P1 |
| P3 | Sync der Buch-Ordner, iOS Buch-Ordner-Ansicht. | P2 |
| P4 | Projekte in Vorlesen- und Gesprächsleiste, Projektseite gruppiert, Dateien-Kopie, Kontext-Schalter mit Tokenanzeige. | P1, ADR-0026 |
| P5 | Aktives Projekt als Ziel, Ausgabeordner, Mehrfachaktionen. | P4 |
| P6 | iOS Projekte, Sync `projekt_eintrag`. | P4, P5 |

Jedes UI-Paket mit HTML-Vorschau (390/768/1440/2560, hell/dunkel) zur Codex-Abnahme vor dem Merge.

## 9 Abnahmekriterien

- Ein Buch mit zwei Fassungen, Hörbuch und Kurzbuch liegt nach der Migration vollständig unter
  `library/<buch>/`; Fortschritt, Lesezeichen, Stimme je Sprache und Sync bleiben erhalten (Test mit
  Kopie von Patricks Bestand, 66 Einträge).
- Migration zweimal hintereinander: zweiter Lauf ändert nichts. Abbruch mitten im Lauf: App startet,
  alle Seiten auffindbar.
- Vorlesen zeigt links nur Ordner; keine Kapitelliste mehr in der linken Spalte.
- Bei 390 px keine waagrechte Scrollleiste, alle Touch-Ziele ≥ 44 px.
- Projekt-Chat nutzt nachweislich nur Einträge mit „Als Kontext“ (Test mit Kontrolltext).
- Jede KI-Ausgabe zeigt vor dem Start ihr Ziel.
