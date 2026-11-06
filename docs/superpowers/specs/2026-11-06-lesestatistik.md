# Lesestatistik in der Bibliothek (lva#354)

Stand: 06.11.2026 · Auftrag Patrick 06.11.2026 19:32 + 19:33 · Gilt für Windows, Mac und iPhone

## Ziel

Eine ruhige Übersicht, wie viel Patrick liest und hört und womit er sich beschäftigt:

- Zeit und Seiten je Tag, je Buch, je Format (Lesen, Hören, Vorlesen) und je Sprache,
- Einordnung jedes Buchs in eine Kategorie und Themen, damit Schwerpunkte je Monat und Jahr sichtbar werden,
- ein Leseziel (z. B. 30 Minuten am Tag) mit „heute erreicht“, Serie, Verlauf und Durchschnitt.

Kein eigener Menüpunkt: Einstieg aus der Bibliothek. Der Name in der Oberfläche ist **„Lesestatistik“**
(kurz, sachlich; „Tagebuch“ klingt nach Schreiben).

## Nicht-Ziele

- Keine Ranglisten, keine Gamification über Serie und Ziel hinaus, keine Benachrichtigungen ohne Zustimmung.
- Keine Weitergabe an Dritte, kein Server-Auswerten. Keine Lesedaten an Sprachmodelle.
- Keine Erfassung außerhalb der App (Audible, Kindle).

## Datenschutz

| Punkt | Festlegung |
|---|---|
| Speicherort | `<appdata>/library/lesestatistik/` auf dem Gerät |
| Abgleich | Nur über den bestehenden Geräte-Sync (Ende-zu-Ende verschlüsselt, nur eigene gekoppelte Geräte). Schalter „Lesestatistik zwischen meinen Geräten abgleichen“ (`reading_stats_sync`), **Vorgabe aus**, je Gerät; die Statistikseite fragt einmal, wenn der Geräte-Sync aktiv ist. Aus = dieses Gerät sendet nichts; Stände, die andere Geräte mit Zustimmung geteilt haben, werden angenommen |
| KI-Einordnung | Gesendet werden nur Titel, Autor, Klappentext und die ersten 2.000 Zeichen des Buchs an das für „Bibliothek“ gewählte Modell (lokal bevorzugt). Hinweiszeile zeigt, wohin |
| Zweck | Persönliche Übersicht für den Nutzer selbst. Nichts davon fließt in andere Funktionen (Buch-Gespräch, Vorschläge) |
| Löschen | „Statistik löschen …“: alles oder ein Zeitraum; wirkt über den Sync auf allen Geräten (Grabstein je Gerät und Tag) |
| Export | CSV (Tag, Buch, Format, Sprache, Minuten, Seiten) |
| Pause | „Erfassung pausieren“ – solange aus, wird nichts gezählt |

## Erfassung

### Formate

| Format | Wann | Zählt |
|---|---|---|
| `hoeren` | Hörbuch spielt (AudioBar, iPhone-Player), auch Mitlesen | Wandzeit, solange nicht pausiert |
| `vorlesen` | Sprachausgabe liest einen Buchtext vor | Wandzeit, solange die Ausgabe läuft |
| `lesen` | Reader zeigt den Text ohne Wiedergabe | Wandzeit mit Aktivität (siehe unten) |

Hören und Mitlesen gleichzeitig zählt **einmal** als `hoeren` (kein Doppelzählen).

### Aktive Zeit

- Ein Taktgeber (`readingClock`) läuft je Gerät genau einmal. Jede Quelle meldet `start(sitzung)` / `stop()`;
  eine neue Quelle beendet die alte.
- Wiedergabe: Takt alle 5 s; Lücke > 15 s (Schlaf, Hänger) zählt nicht.
- Lesen: zählt nur, solange das Fenster sichtbar ist **und** in den letzten 3 Minuten geblättert, gescrollt,
  getippt oder eine Taste gedrückt wurde. Danach ruht die Zählung, ohne die Sitzung zu verwerfen.
- Tagesgrenze: Ortszeit des Geräts; eine Sitzung über Mitternacht wird geteilt.
- Unter 20 s je Buch und Tag wird nicht angezeigt (bleibt gespeichert).

### Seiten

- Text: Fortschritt in Zeichen (neu erreichte Bereiche, nicht Wiederholungen). Umrechnung mit den
  **Buchseiten**, wenn die Seitenzahl bekannt ist (`Zeichen × Seiten / Textlänge`), sonst **Normseite**
  (1.800 Zeichen). Die Anzeige sagt, welche Umrechnung gilt.
- Hörbuch: Anteil des Hörbuchs × Seitenzahl, wenn bekannt; mit Transkript/Mitlese-Abgleich über die Textstelle.
  Ohne beides **nur Zeit** (Codex 06.11.), keine Schätzung über Wörter je Minute.
- Seiten sind Schätzwerte und werden gerundet angezeigt.

### Sprache

Sprache der gerade gelesenen bzw. gehörten **Fassung** (lva#344), sonst Sprache des Buchs, sonst „unbekannt“.

## Datenmodell

### Tagebuch je Gerät

```
<appdata>/library/lesestatistik/<geraet>.json
{
  "version": 1,
  "device": "<sync-geraete-id oder lokale id>",
  "days": {
    "2026-11-06": {
      "<buch-id>|<format>|<sprache>": { "sec": 1820, "chars": 9100, "audio_ms": 0 }
    }
  },
  "cleared": { "2026-11-01": 1762000000000 }
}
```

- Jedes Gerät schreibt **nur seine eigene** Datei. Werte wachsen nur (Zähler je Gerät).
  Zusammenführen = je Schlüssel Maximum. Dadurch gibt es keine Konflikte und kein Doppelzählen.
- Fremde Geräte liegen als `<andere-id>.json` daneben, nur lesend.
- `cleared` = Grabstein je Tag mit Zeitpunkt: Werte dieses Tages von vor dem Zeitpunkt zählen nicht.
- Schreiben gebündelt (höchstens alle 10 s und beim Verbergen/Schließen), atomar (temp + rename),
  defekte Datei wird beiseitegelegt wie `fortschritt.json`.

### Sync

Neue Sammlung `readlog`, ein Objekt je Gerät und Monat (`r_` + 40 Hex von SHA-256(gerät|JJJJ-MM)).
Ein Monat mit täglichem Lesen von 20 Büchern bleibt weit unter 100 KB.

### Einordnung am Buch

Neue Felder an `LibraryBook` (reisen mit der Sammlung `book`, alte Geräte ignorieren sie):

```
"category": "persoenlichkeit",          // Schlüssel aus den Vorlagen oder "eigen:<name>"
"topics": ["Rhetorik", "Selbstführung"], // höchstens 5 freie Themen
"classified_by": "ki" | "hand"           // "ki" = Vorschlag, noch nicht bestätigt
```

Ein Buch mit `classified_by = "hand"` wird nie automatisch neu eingeordnet.

### Leseziel

Synchronisierte Einstellungen (`SYNCED_SETTINGS`): `reading_goal_minutes` (0 = aus, Vorschlag 30),
`reading_goal_days` (Bitmaske Mo–So, Vorgabe alle 7), `reading_stats_ai` (KI-Einordnung, Vorgabe an).
Je Gerät: `reading_stats_sync` (Vorgabe aus), `reading_stats_paused`. Erfassung, Abgleich und KI-Einordnung
sind getrennt schaltbar (Codex A3).

## Kategorien (Vorlagen)

| Schlüssel | Anzeige | Gruppe |
|---|---|---|
| `persoenlichkeit` | Persönlichkeitsentwicklung | Sachbuch |
| `kommunikation` | Kommunikation & Rhetorik | Sachbuch |
| `fuehrung` | Führung & Management | Sachbuch |
| `business` | Business & Unternehmertum | Sachbuch |
| `finanzen` | Finanzen & Wirtschaft | Sachbuch |
| `psychologie` | Psychologie | Sachbuch |
| `gesundheit` | Gesundheit & Sport | Sachbuch |
| `wissenschaft` | Wissenschaft & Natur | Sachbuch |
| `technik` | Technik, IT & KI | Sachbuch |
| `fachbuch` | Fachbuch (Beruf, Recht, Verwaltung) | Sachbuch |
| `geschichte` | Geschichte & Politik | Sachbuch |
| `gesellschaft` | Gesellschaft & Kultur | Sachbuch |
| `philosophie` | Philosophie & Spiritualität | Sachbuch |
| `ratgeber` | Ratgeber & Alltag (Familie, Erziehung, Haus) | Sachbuch |
| `biografie` | Biografie & Erinnerungen | Sachbuch |
| `roman` | Roman & Erzählung | Belletristik |
| `spannung` | Krimi & Thriller | Belletristik |
| `fantastik` | Fantasy & Science-Fiction | Belletristik |
| `kinder` | Kinder- & Jugendbuch | Belletristik |

19 Vorlagen; eigene Kategorien lassen sich ergänzen, umbenennen und ausblenden.
Themen sind frei; die KI schlägt 2–4 vor, die Eingabe vervollständigt aus bereits verwendeten Themen.

### KI-Einordnung

- Auslöser: beim Import (wenn ein Modell bereitsteht) und per „Einordnen lassen“ (Buch) bzw.
  „Alle ohne Einordnung einordnen“ (Statistik). Nie ohne Modell, nie wiederholt für „hand“.
- Antwort als JSON `{ "category": "<schlüssel>", "topics": ["…"], "confidence": 0..1 }`;
  unbekannter Schlüssel → `null` (bleibt „Nicht eingeordnet“).
- Anzeige als Vorschlag mit KI-Marke und „Übernehmen“ / „Ändern“. Übernehmen oder Ändern setzt `hand`.

## Oberfläche

### Einstieg

In der Bibliotheksleiste vor „Auswählen“ ein kleiner Knopf: Ring mit Tagesfortschritt + „18 / 30 Min.“
(ohne Ziel: Diagrammsymbol + „Lesestatistik“), Zugriffsname „Lesestatistik, heute 18 von 30 Minuten“.
Ergänzend ein Eintrag im ⋮-Menü der Bibliothek (Codex 06.11.). Unter 640 px nur Ring als 44-px-Ziel mit Beschriftung im Tooltip/aria-label.
Öffnet die Statistik **in der Bibliothek** wie die Buchansicht (Zurück-Pfeil, gleiche Kopfzeile, `Seitenkopf`
bleibt „Bibliothek“, Untertitel „Lesestatistik“).

### Statistikseite (von oben)

1. **Heute & Ziel**: Ring (heute / Ziel), Serie in Tagen, Ø der letzten 7 und 30 Tage, Ziel erreicht an x von y Tagen.
   „Ziel ändern“ öffnet eine kleine Fläche (Minuten 10–180 als Stufen, Wochentage).
2. **Zeitraum**: Segmentschalter Woche / Monat / Jahr + Pfeile ‹ › + „Heute“.
3. **Kennzahlen** (4 Kacheln): Zeit, Seiten, Bücher, beendet.
4. **Verlauf**: Balken je Tag (Jahr: je Woche), gestapelt nach Format, Ziellinie gestrichelt, Ø-Linie.
   Tabelle als Alternative für Screenreader („Als Tabelle“).
5. **Schwerpunkte**: horizontale Balken nach Kategorie (Anteil der Zeit), darunter die häufigsten Themen als Chips
   mit Minuten. Klick auf Kategorie/Thema filtert die Bücherliste.
6. **Sprachen**: Balken je Sprache mit Zeit und Anteil.
7. **Bücher im Zeitraum**: Cover, Titel, Kategorie, Formate (Symbole), Zeit, Seiten, Stand (beendet / x %).
   Klick öffnet das Buch.
8. **Jahr** (nur bei „Jahr“): Kalender-Raster wie Beitragskalender, Farbstufe nach Minuten, Ziel erreicht mit Punkt.
9. Fuß: Datenschutzzeile („Nur auf deinen Geräten gespeichert …“), „Exportieren (CSV)“, „Statistik löschen …“, „Erfassung pausieren“.

Leerer Zustand: Erklärung in zwei Sätzen und „Ziel festlegen“.

### Buchansicht

Unter den Metadaten eine Zeile **Einordnung**: Kategorie-Auswahl + Themen-Chips (bearbeitbar), KI-Vorschlag mit Marke.
Daneben „Gelesen: 6 h 40 Min. · 212 Seiten · zuletzt gestern“ als Link in die Statistik (gefiltert auf das Buch).

### Barrierefreiheit und Layout

- Fluent 2 Tokens, hell/dunkel, Kontrast ≥ 4,5:1 auch für Diagrammfarben (Formate: Lesen = Markenblau,
  Hören = Grün, Vorlesen = Violett; zusätzlich Muster/Legende, nie nur Farbe).
- Ziele ≥ 44 px, Tastatur vollständig, Diagramme mit Tabellen-Alternative.
- Responsive: 390 / 768 / 1440 / 2560 px; ab 1280 px zweispaltig (Verlauf + Schwerpunkte nebeneinander),
  ab 1920 px dreispaltig. Nur der Inhalt scrollt.

## iPhone

- Erfassung im `LibraryPlayback` (Hören, Vorlesen) und im Reader (Lesen) mit denselben Regeln.
- Einstieg: Bibliothek > Symbol „Statistik“ in der Navigationsleiste; gleiche Abschnitte als Liste, Diagramme mit Swift Charts.
- Tagebuch-Datei im App-Container, gleiche JSON-Form; Sync über `LibrarySync`.
- Einordnung am iPhone nur anzeigen und von Hand ändern (KI-Einordnung läuft am Desktop).

## Stufen

| PR | Inhalt | Prüfung |
|---|---|---|
| 1 | Kern Rust: Tagebuch, Zusammenführen, Grabsteine, Sync `readlog`, Buchfelder, Befehle, Einstellungen | Rust-Tests (Merge, Mitternacht, Grabstein, Max-Regel) |
| 2 | Desktop: Taktgeber + drei Quellen, Statistikseite, Einordnung (Vorlagen + KI), Leseziel, Export/Löschen | Vitest (Aggregation, Seiten), Playwright + Vorschau-Bilder |
| 3 | iPhone: Erfassung, Seite, Sync | XCTest Merge-Gleichheit mit Rust-Vektoren |

Gemeinsame Testvektoren `docs/superpowers/specs/2026-11-06-lesestatistik-vektoren.json` sichern, dass Rust,
TypeScript und Swift gleich zusammenführen und rechnen.

## Entscheidungen und Auflagen (Codex, 06.11.2026, bedingt JA auf d2c8591a)

1. Abgleich nur nach ausdrücklicher Zustimmung je Gerät (Vorgabe aus).
2. Hörbuch ohne Seitenzahl: nur Zeit.
3. Ring-Knopf in der Leiste mit Zugriffsname, ⋮-Menü ergänzend.

- **A1** Zähler je Gerät/Buch/Tag/Format, Merge komponentenweise per Maximum, danach Geräte summieren.
  Gleichzeitiges Lesen und Hören zählt nicht doppelt zur Zielzeit: ein Taktgeber je Gerät, Mitlesen zählt als Hören.
- **A2** Löschen mit neuer Epoche (`since`); Grabsteine schlagen späte Offline-Daten (Eimer mit `since` ≤ Grabstein
  zählen nie). Der Dialog nennt den Geltungsbereich („auf allen deinen Geräten, Tage x–y“).
- **A3** Erfassung, Abgleich und KI-Einordnung getrennt schaltbar; vor externer Einordnung Anbieter und gesendete
  Felder zeigen, lokale Modelle kennzeichnen; „Pause“ erklärt ihre Wirkung.
- **A4** Ziele ≥ 44 px, Kontrast gemessen, Tastatur/Fokus/Schließen des Ziel-Popovers (Esc, Fokus zurück),
  Diagrammwerte auch als Tabelle.
- **A5** „insgesamt“ und „im Zeitraum“ klar benennen; Jahresliste als Ausschnitt kennzeichnen („7 von 41“, „Alle zeigen“).
- **A6** Leerzustände (Erstnutzung, leerer Zeitraum, pausiert); Export/Löschen ohne Daten deaktiviert; Ziel aus
  und Buch ohne Seitenzahl sichtbar darstellen.

## Umsetzungsnotizen (Stufe 1 und 2, 06.11.)

- Die KI-Einordnung sendet nur Titel, Autor und Klappentext. Ein Textanfang (`sample`) wird nicht gesendet (Feld bleibt `null`).
- „Lesen“ zählt Zeichen nur für vorwärts geblätterte Seiten; Zurückblättern und erneutes Öffnen zählen nicht doppelt.
- Ein ⋮-Menü auf Ebene der Bibliothek gibt es nicht (nur je Karte). Einstieg ist deshalb allein der Ring-Knopf in der Leiste, Trefferfläche 44 px (sichtbare Pille 32 px).
- In der zweispaltigen Ansicht entfällt die Kategorie-Spalte der Büchertabelle (Container-Abfrage), die Kategorie steht in den Schwerpunkten.
- Gemeinsame Testvektoren für Rust, TypeScript und Swift folgen mit Stufe 3 (iPhone).
