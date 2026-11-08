# Notizbuch für den Assistenten – Lesen, Anlegen, Besprechungsnotizen zusammenführen

- Stand: 08.11.2026, Fassung 2 (Codex-Must-fixes 1–9 und Kann-Punkte eingearbeitet, siehe §10)
- Auftrag: Patrick, 08.11.2026 (Thread „Notizbuch für die KI zugänglich machen“)
- Epic: lva#396 (unter lva#384) · Bezug: ADR-0028 (Assistent, Tor, Kontextvertrag E6, Freigabe E10), ADR-0019 (Werkzeuge,
  Wirkungsklassen), ADR-0011 (Eingang), ADR-0018 (Deep-Links), lva#325 (globaler Abrufindex), lva#365 P4 (Projekt-
  kontext), Notizbuch `apps/notizbuch` in wai-sdk (Konnektor `wissen`, `wai-eingang`)
- Zurückgestellt: OCR (wai-sdk#865) und Abschnittspasswort (nur mit Patricks Freigabe). Diese Spezifikation setzt
  beides nicht voraus und legt nur fest, wie der Assistent sich verhält, sobald es sie gibt (§7).

## 1 Ziel

Patrick sagt im Chat oder per Sprache:

- „Was habe ich mir zum Thema Haushaltsplan notiert?“ → Antwort mit Belegen aus dem Notizbuch.
- „Leg mir eine Notiz an: Rückruf Frau Beispiel wegen Förderantrag, bis Freitag.“ → neue Seite im Notizbuch.
- „Schreib die To-dos aus der Besprechung gestern ins Projektnotizbuch.“ → Seite mit Aufgaben, Kategorie Aufgabe.
- „Mach das Protokoll der Besprechung von heute Vormittag.“ → Protokoll aus Aufnahme, Transkript, Notizblock der
  Aufnahme **und** den Notizbuch-Seiten, die Patrick währenddessen geschrieben hat; nichts davon geht verloren.

Heute gilt: Das Notizbuch hat einen lesenden Wissens-Endpunkt (`notizbuch-wissen`, Bereichsfreigabe je Abschnitt),
den Local Voice nicht nutzt. Local Voice hat einen eigenen Notizblock je Besprechung (`meeting_notes`, Blöcke mit
`at_ms`) und baut daraus KI-Notizen und Protokoll. Beide Welten kennen sich nicht.

## 2 Grundsätze

1. **Eine Durchsetzung.** Jeder Zugriff des Assistenten auf das Notizbuch läuft durch das Tor (`assistent::tor`,
   ADR-0028). Kein Weg aus TS, Modell oder Notizbuch-UI an ihm vorbei.
2. **Ein Kontextvertrag.** Notizbuchinhalte gelangen nur über `projekt_kontext` bzw. den globalen Abrufindex
   (lva#325) und die lesenden Werkzeuge in einen Prompt (ADR-0028 E6). Standard: nichts freigegeben.
3. **Das Notizbuch bleibt Herr seiner Daten.** Local Voice liest die Ablage nur lesend und schreibt nie direkt in
   `notizbuch.db`. Schreiben geht als Anfrage in den Eingang des Notizbuchs; Prüfung und Übernahme macht allein der
   Notizbuch-Host (Muster wie Planner-Eingang, ADR-0011).
4. **Nichts geht verloren.** Kein Werkzeug überschreibt oder löscht Nutzertext. Zusammenführen heißt Verweisen und
   Zusammenfassen mit Beleg, nie stilles Weglassen (§5.4, Eval-Tor).
5. **Inhalte sind Daten.** Notizbuchtext bestimmt nie Rechte, Ziele oder Werkzeugwahl (ADR-0028 E10,
   Herkunftsmarke `dokument:notizbuch/<seite>`).
6. **Keine Inhalte in Log, Audit oder Telemetrie.** Nur Kennungen, Längen, Zähler, Codes.

## 3 Lesen

### 3.1 Zugriff (Crate `wai-notizbuch-zugang`, wai-sdk, nach LVA vendort)

- Öffnet `<Datenordner>/de.wolffappliedai.notizbuch/notizbuch.db` **nur lesend** (`mode=ro`, `query_only`), ohne
  Migration. Passt die Schema-Version nicht, meldet der Zugang „Notizbuch-Version wird nicht unterstützt“ und liefert
  nichts (kein Raten am Schema). WAL erlaubt Lesen, während das Notizbuch geöffnet ist.
- Liefert: Notizbücher/Gruppen/Abschnitte (Id, Name, Pfad), Seitenköpfe (Id, Titel, Abschnitt, erstellt,
  geändert, Kategorien), Seitentext als Klartext mit Blockstruktur und Blockzeiten (§5.1), FTS5-Suche (gleiche
  Anfrage-Faltung wie `notizbuch-store::fts_anfrage`), Versionen-Köpfe.
- Was im Papierkorb liegt (Seite oder ein Elternteil), gilt als nicht vorhanden – gleiche Regel wie `wissen.rs`.
- Anhänge (Bilder, Dateien) liefert er in Stufe 1 nur als Platzhalter `[Bild: Alternativtext]` /
  `[Datei: Name, Typ]`; Inhalte von Anhängen erst mit OCR bzw. Dateiextraktion (zurückgestellt).
- Grenzen wie der Wissens-Endpunkt: höchstens 20 Treffer, höchstens 32 000 Zeichen je Seite.

Warum nicht der HTTP-Endpunkt `notizbuch-wissen`: Er braucht einen laufenden Begleitprozess mit Token und Port; der
Assistent soll auch antworten, wenn das Notizbuch geschlossen ist. Der Endpunkt bleibt für Automation (ADR-0019)
bestehen; beide nutzen dieselbe Sichtbarkeitsregel (gemeinsame Funktion im Crate).

### 3.2 Freigabe

- **Wissensbereich „Notizbuch“** (neu in `agent_bridge/voice/areas.rs` und im Assistenten): Freigabe je
  **Abschnitt** (wahlweise ganzes Notizbuch = alle heutigen und künftigen Abschnitte darin). Standard leer.
  Einstellungen › Assistent › Wissensbereiche › Notizbuch zeigt den Baum aus dem Zugang mit Schaltern.
- Ein nicht freigegebener Abschnitt und eine nicht vorhandene Seite ergeben dieselbe Antwort (kein Existenz-Orakel).
- **Projekt:** Eine Notizbuch-Seite oder ein Abschnitt kann Mitglied eines Suite-Projekts werden
  (`wai://notizbuch/seite/<id>`, wai-links kennt den Typ; Abschnitt neu `wai://notizbuch/abschnitt/<id>`).
  Für den Kontext gilt dann der Schalter „Als Kontext“ aus lva#365 P4 (fehlender Stand = aus). Projektmitgliedschaft
  ist keine Freigabe für den globalen Bereich und umgekehrt.
- **Eine Sichtbarkeitsregel für alles (M1).** Baum, Pfade, Titel, Trefferzahlen, Auszüge, Zuordnungskandidaten und
  Rückfragen entstehen nur aus Abschnitten, die für **diese Runde** freigegeben sind. Gezählt wird nach dem Filtern;
  ein verborgener Abschnitt verändert keine Zahl, keinen Pfad und keine Rückfrage.
- **Kein projektübergreifender Zugriff (M1).** In einer Projektrunde sind nur die Notizbuch-Mitglieder **dieses**
  Projekts mit Schalter „Als Kontext“ sichtbar; die globale Bereichsfreigabe gilt dort nicht zusätzlich. Ohne Projekt
  gilt nur die globale Bereichsfreigabe. Ein Wechsel des Projekts ist ein Bereichswechsel (ADR-0028 E6).
- **Widerruf und Prüfung bei Ausgabe (M2).** Der Freigabestand ist ein Hash über Freigabeliste **und** Gliederungs-
  zustand (je Abschnitt: Notizbuch, Gruppe, Papierkorb, Sperre; je Seite: Abschnitt, Papierkorb). Papierkorb,
  Verschieben und künftiges Sperren ändern ihn auch ohne neuen Seitenzeitstempel. Ein neuer Stand verwirft
  Index-Chunks und Zwischenergebnisse; laufende Runden mit betroffenen Inhalten brechen ab (ADR-0028 E6, eine Regel).
  Unmittelbar vor jeder Ausgabe an Modell oder Oberfläche prüft das Tor jede zitierte Seite erneut. Gespeicherte
  Links, Auszüge und Belege in Unterhaltungen geben keine Dauerberechtigung: beim Öffnen oder Wiederverwenden wird neu
  geprüft, sonst steht dort „nicht mehr freigegeben“.

### 3.3 Werkzeuge (S1, lesend)

| Werkzeug | Ergebnis |
|---|---|
| `notizbuch_suchen {anfrage, grenze?}` | Treffer `{seite_id, titel, pfad, auszug, geaendert}` nur aus freigegebenen Abschnitten |
| `notizbuch_seite {seite_id}` | Titel, Pfad, Text mit Blockzeiten (gekürzt), Kategorien |
| `notizbuch_zeitraum {von, bis}` | Seiten, die im Zeitraum angelegt oder geändert wurden (für „was habe ich heute notiert“, §5) |
| `notizbuch_baum {}` | freigegebene Notizbücher/Abschnitte mit Seitenzahl (für Rückfragen „in welches Notizbuch?“) |

Antworten folgen der Belegregel aus ADR-0028 E4.2: Jede Aussage nennt die Seite (Deep-Link `wai://notizbuch/seite/<id>`
öffnet sie im Notizbuch, ADR-0018).

### 3.4 Globaler Abrufindex

Neue Quelle `notizbuch` im Index aus lva#325 (Chunks je Seite, Index nur im RAM mit Epoch, Neuaufbau bei
geändertem `geaendert` oder Freigabewechsel). Dieselbe Abschnittsfreigabe wie §3.2.

## 4 Anlegen und Verwalten

### 4.1 Weg: Eingang des Notizbuchs

Local Voice schreibt eine Anfragedatei nach `wai/eingang/notizbuch/` (`wai-eingang`: UUID-Name, atomar, Größengrenze).
Der Notizbuch-Host übernimmt sie beim Start und, wenn er läuft, sofort (Dateiwächter wie beim Planner-Eingang).

Anfrage v1 (`contracts`, Fixtures Rust/TS):

```json
{ "v": 1, "app": "local-voice", "id": "<ulid>", "aktion": "seite_anlegen",
  "ziel": { "abschnitt_id": "<id>" }, "titel": "…", "bloecke": [ { "art": "absatz|ueberschrift|aufzaehlung|aufgabe",
  "text": "…", "kategorie": "aufgabe|wichtig|frage|idee|nachfassen|entscheidung"?, "erledigt": false? } ],
  "herkunft": { "art": "assistent", "unterhaltung": "<id>", "quellen": ["wai://local-voice/besprechung/<id>"] } }
```

Aktionen v1: `seite_anlegen`, `bloecke_anhaengen {seite_id}`, `abschnitt_anlegen {notizbuch_id, name}` (nur für den
Abschnitt „Assistent“, §4.2) und `abbrechen {anfrage_id}` für eine noch nicht übernommene Anfrage. Kein Überschreiben,
kein Löschen von Blöcken.
Idempotenz über `(app, id)`. Ergebnis legt der Host als Statusdatei ab (`uebernommen` mit Seiten-Id oder `abgelehnt`
mit Code); Local Voice zeigt es an der Auftragskarte.

Prüfung im Notizbuch-Host (maßgeblich): Der **tatsächliche** Ziel-Abschnitt (bei `bloecke_anhaengen` der aktuelle
Abschnitt der Seite zum Übernahmezeitpunkt, nicht eine Angabe des Absenders) existiert, ist aktiv und für
**Schreiben** freigegeben (§4.2); die Schreibfreigabe liest der Host aus seiner eigenen Ablage, die Local Voice nur über
eine bestätigte Eingangsaktion ändern kann; Text nur Klartext (kein HTML, kein Markdown-Link mit `javascript:`, Steuer- und Bidi-Zeichen entfernt);
höchstens 200 Blöcke, 64 000 Zeichen; Seitenlinks nur auf existierende Seiten. Vor `bloecke_anhaengen` legt der Host
eine Version der Seite an (bestehende Versionierung), damit jede Änderung rückgängig zu machen ist.

Ist die Zielseite gerade im Editor offen und ungespeichert, hängt der Host die Blöcke über eine Editor-Transaktion an
(Ereignis an das Webview), nicht an der gespeicherten Fassung vorbei. Die Statusdatei `uebernommen` schreibt der Host
erst, wenn die Seite mit den neuen Blöcken gesichert in der Ablage steht (M3); scheitert das Sichern, lautet der Status
`abgelehnt` mit Code und nichts gilt als angelegt.

Läuft das Notizbuch nicht, sagt der Assistent: „Die Notiz liegt bereit und erscheint, sobald das Notizbuch geöffnet
ist.“ Optional (Codex E3): Local Voice startet das Notizbuch minimiert über den Deep-Link.

### 4.2 Schreibfreigabe

- Getrennt von der Lesefreigabe, je Abschnitt, Standard aus.
- Beim ersten Schreibwunsch ohne Ziel schlägt der Assistent einen neuen Abschnitt **„Assistent“** vor. Anlegen und
  Schreibfreigabe sind eine eigene Aktion `abschnitt_anlegen` mit **ausdrücklicher Freigabe per Karte** (nie per
  Sprachfrist, nie gebündelt mit dem Inhalt). Das Notizbuch wählt Patrick auf der Karte aus den lesefreigegebenen
  Notizbüchern; sonst gibt es nur „Im Notizbuch freigeben“ (Deep-Link in die Notizbuch-Einstellung).
- Wirkungsklasse (ADR-0019-Nachtrag aus ADR-0028): **lokal, umkehrbar** (Papierkorb, Versionen). Ausführung nur, wenn
  die strukturierte Freigabe der Runde (E10) die Aktion enthält – und die enthält sie nur, wenn die **Nutzereingabe**
  das Anlegen verlangt. Anweisungen in Mails, Transkripten oder Notizen lösen nie ein Schreiben aus.
- Bestätigung (M4): Die Sprachfrist gilt **nur** für `seite_anlegen` und `bloecke_anhaengen`, die Patrick in derselben
  Äußerung ausdrücklich beauftragt hat, und nur in schreibfreigegebene Abschnitte. Der Assistent sagt Ziel und Titel an;
  die Frist (Standard 3 s, einstellbar 2–10 s) beginnt erst, wenn die Ansage **vollständig abgespielt** ist; „Stopp“,
  „Nein“ oder „Ändern“ wirken während der Ansage und der Frist. Die Übergabe an den Eingang erfolgt erst nach Ablauf;
  bis zur Übernahme kann „Abbrechen“ die Anfrage zurückziehen. Wird die Ansage unterbrochen, ist die Spracheingabe aus
  oder nicht verfügbar (Mikrofon belegt, Ausgabe stumm), gibt es **keine** automatische Übergabe, sondern die Karte. Im
  Chat zeigt immer eine Karte Ziel, Titel und Blöcke mit „Anlegen“ und „Ändern“. Wiederholungen begründen keinen
  pauschalen Verzicht: jede Anfrage hat ihre eigene Ansage oder Karte.
- Herkunft: Seiten und Blöcke aus dem Assistenten tragen `herkunft: "assistent"` (Notizbuch zeigt ein kleines Symbol
  und in der Seiteninfo „Angelegt vom Assistenten, Unterhaltung …“).

### 4.3 Verwalten (Stufe N5)

Umbenennen, Verschieben zwischen schreibfreigegebenen Abschnitten, Kategorie setzen, Aufgabe abhaken, Wiedervorlage
setzen, Seite in den Papierkorb legen. Jeweils als Eingangs-Aktion und **immer mit Karte**; die Sprachfrist aus §4.2
gilt hier nie (Auflösung des Widerspruchs zu E4). Endgültig löschen und Papierkorb leeren gibt es für den Assistenten nicht.

## 5 Besprechung: Notizen zusammenführen

### 5.1 Blockzeiten im Notizbuch

Damit Notizbuch-Notizen auf der Zeitachse einer Aufnahme liegen, bekommt jeder Block im Notizbuch zwei Attribute
(neu in `domain/sicherung.ts` eingetragen):

- `bid`: stabile Block-ID (ULID), vergeben beim Anlegen, bleibt beim Bearbeiten, Verschieben und Kopieren in die
  Zwischenablage nicht erhalten (Einfügen erzeugt eine neue). Alle Belege und Zuordnungen verweisen auf
  `(seite_id, bid)`, nie auf eine Position (M6).
- `zeit`: ISO-Zeit der **ersten** Eingabe in den Block, `zeit_geaendert`: der letzten inhaltlichen Änderung.

Jede Blockzeit ist ein **Intervall** `[frueh, spaet]` mit Güte:

| Güte | Herkunft | Intervall |
|---|---|---|
| `genau` | Attribut `zeit` | `[zeit, zeit]` |
| `version` | Altseite: Block erscheint zuerst in Version *n* | `[zeit(n−1), zeit(n)]` |
| `seite` | sonst | `[erstellt, geaendert]` der Seite |

Eine Regel für die Zeitachse: Liegt das Intervall vollständig außerhalb der Aufnahme, ist `at_ms = None` („vor/nach der
Besprechung“). Sonst wird es auf die Aufnahme abgebildet und an den Rändern abgeschnitten. Protokoll und Belege nennen
eine Minute nur bei Güte `genau`; sonst „zwischen 14:02 und 14:40“ bzw. „während der Besprechung“, nie eine
Scheingenauigkeit.

### 5.2 Zuordnung

Kandidaten sind Seiten aus lese- **oder** projektfreigegebenen Abschnitten, deren Blockzeiten im Fenster
`[Beginn − 15 min, Ende + 30 min]` der Besprechung liegen. Bewertung (deterministisch im Tor, kein Modellurteil):

| Merkmal | Gewicht |
|---|---|
| ausdrücklicher Seitenlink auf genau diese Besprechung (`wai://local-voice/besprechung/<id>`) | hoch |
| gleiches Projekt oder Herkunft Local Voice (beweist keine Zugehörigkeit, M5) | gering |
| Anteil der Blöcke mit Zeit im Fenster | hoch |
| Begriffsüberlappung Block ↔ Transkriptabschnitt derselben Minuten (BM25 über gefaltete Wörter, Namen, Zahlen) | mittel |
| Titel enthält Datum, Teilnehmende oder Kalendertitel | gering |

- Punktzahl ≥ Schwelle A: automatisch zugeordnet; die Besprechung zeigt „Notizen aus dem Notizbuch: *Seite*
  (während der Besprechung)“ mit Zuordnungsgrund (Zeit, gemeinsame Begriffe, Link) und „Lösen“.
- Zwischen B und A: Vorschlag mit Grund und „Zuordnen“. Unter B: nichts.
- **Automatik nur mit bestandenem Tor (M5):** Schwellen werden auf einem Kalibriersatz gewählt und auf einem davon
  getrennten Prüfsatz gemessen, der die Sonderpfade (Altseiten, Güte `seite`, Link vorhanden, gleiches Projekt ohne
  Bezug, zwei parallele Besprechungen) enthält. Erreicht die gemessene Präzision dort nicht 0,95, gibt es nur
  Vorschläge. Die Präzision ist ein Messwert über den Satz, keine Garantie je Zuordnung; deshalb bleibt „Lösen“
  immer sichtbar.
- Kandidaten nur aus freigegebenen Abschnitten (§3.2, eine Regel). Gibt es keine, zeigt die Besprechung nichts: keine
  Rückfrage, keine Zahl, kein Hinweis, der auf verborgene Abschnitte schließen lässt (M1). Freigeben geht nur über die
  Einstellung.
- Zuordnungen werden in `meeting_note_links (meeting_id, seite_id, art: auto|bestaetigt|geloest, stand)` gespeichert;
  „gelöst“ wird nie wieder automatisch zugeordnet.

### 5.3 Abbildung auf die Zeitachse

`at_ms = blockzeit − besprechungsbeginn` (Wanduhr der Aufnahme `started_at`), geklemmt auf `[0, Dauer]`, sonst
`None` („vor/nach der Besprechung“). Die Blöcke gehen als zweite Notizquelle neben den Notizblock der Aufnahme in
KI-Notizen und Protokoll ein (`notes/enhance`, `minutes`): `NoteSource::Notizbuch {seite_id, block_id, stand}`. Sie
werden **nicht** in `meeting_notes` kopiert (keine zweite Wahrheit, kein Auseinanderlaufen); geändert wird im
Notizbuch, die Besprechung sieht beim nächsten Lauf den neuen Stand.

### 5.4 Verlustfreiheit (deterministisch geprüft)

- Jeder Nutzerblock aus beiden Quellen ist im Ergebnis entweder einem Eintrag zugeordnet (`note_ref`, wie heute im
  Notizblock) oder steht im Abschnitt „Weitere Notizen“. `assemble` prüft das; fehlt ein Block, wird er angehängt,
  nie verworfen.
- **Vollständig lesen (M7):** Für die Zusammenführung lädt der Zugang die zugeordneten Seiten ohne die Grenzen der
  Werkzeuge (20 Treffer, 32 000 Zeichen); passt ein Teil nicht ins Modellbudget, wird nach dem bestehenden
  Map-Reduce-Verfahren aufgeteilt. Ist eine Seite nicht vollständig lesbar (Größe über 1 MB Text, Leserfehler), steht
  im Protokoll „Notizen unvollständig: *Seite*“ und die Verlustprüfung meldet es.
- **Duplikate (M7):** Zusammengelegt werden nur Blöcke, deren gefalteter Text gleich ist **und** deren Zahlen,
  Datums-/Zeitangaben, Verneinungen („nicht“, „kein“, „no“, „not“) und Aufgabenstand übereinstimmen. Ähnliche, aber
  nicht gleiche Blöcke bleiben getrennt; sie stehen nebeneinander mit Kennzeichen „ähnlich notiert“.
- Aufgaben (Notizbuch-Kategorie Aufgabe, Aufgabenliste) werden zu Aufgaben im Protokoll mit Verweis zurück; erledigte
  bleiben erledigt.
- **Widersprüche (M8).** Erkannt werden deterministisch nur diese Fälle: In einem Blockpaar (Notizblock ↔
  Notizbuch oder Notiz ↔ Transkriptabschnitt im gleichen Zeitintervall, Begriffsüberlappung ≥ Schwelle) unterscheiden
  sich (a) Zahlen oder Beträge, (b) Datum oder Uhrzeit, (c) Verneinung, (d) Aufgabenstand. Dann steht im Protokoll
  beides mit Beleg und „abweichend notiert“; das Modell entscheidet nicht still. Andere Widersprüche werden nicht
  behauptet. Unabhängig davon sind die Originalnotizen immer über ihre Belege (`seite_id`, `bid` bzw. Notizblock-ID)
  aus dem Protokoll heraus erreichbar.
- Anhänge und Zeichnungen erscheinen als Verweis „[Zeichnung auf Seite …]“.

### 5.5 Rückweg (Stufe N4b, braucht N1 und N3)

„Protokoll ins Notizbuch“ (Knopf in der Besprechung und Assistent-Aktion): legt über den Eingang eine Seite mit dem
Protokoll in den zugeordneten oder gewählten Abschnitt; Aufgaben mit Kategorie Aufgabe (und „An Planner senden“ wie
gewohnt im Notizbuch).

### 5.6 Eval-Tor (synthetische Daten, `example.*`)

Kalibriersatz 40 DE + 40 EN, davon getrennter Prüfsatz 40 DE + 40 EN, je mit Notizbuch-Seiten (richtig, zeitnah aber
themenfremd, Altseite ohne Blockzeiten, gleiches Projekt ohne Bezug, parallele Besprechungen, sehr lange Seite,
Duplikate mit abweichender Zahl/Verneinung, Widersprüche, Prompt-Injection in Notizen, verborgener Abschnitt):

- 0 verlorene Nutzerblöcke (§5.4), 0 fälschlich verschmolzene Blöcke, 0 ausgeführte Schreibaktionen aus Inhalten,
  0 Inhalte, Titel, Pfade oder Zählungen aus nicht freigegebenen Abschnitten, 0 Ausgaben nach Widerruf.
- Zuordnung auf dem Prüfsatz: Präzision ≥ 0,95 bei „automatisch“ (sonst nur Vorschläge, §5.2), Trefferquote ≥ 0,9
  über automatisch + Vorschlag.

## 6 Plattformen und Sync

- Windows und macOS: Notizbuch und Local Voice auf demselben Gerät; vollständiger Umfang.
- iPhone: kein Notizbuch auf dem Gerät. Lesende Notizbuch-Werkzeuge und Schreiben laufen wie Konnektor-Werkzeuge
  über den gekoppelten PC (ADR-0028 E2); ohne PC sagt der Assistent das. Kein eigener Sync des Notizbuchs in dieser
  Stufe.
- Assistent-Unterhaltungen speichern nur Verweise und Auszüge mit Beleg, nie ganze Seiten (Sync-Datenklasse).

## 7 Zurückgestellt und Folgen

- **Abschnittspasswort:** Sobald es existiert, sind gesperrte Abschnitte für den Assistenten nie lesbar oder
  beschreibbar, auch wenn freigegeben; entsperrt Patrick ihn, gilt das nur für die Sitzung des Notizbuchs, nicht für
  den Assistenten. Umsetzung erst nach Patricks Freigabe.
- **OCR / Text aus Anhängen:** später als zusätzliche Blöcke mit Herkunft `ocr`; bis dahin Platzhalter.
- Ablage „Dokumente“ (RAG/Cloud) ist ein eigener Thread; der Zugang hier ist so geschnitten, dass das Notizbuch
  später eine Quelle dieser Ablage werden kann.

## 8 Stufen

| Stufe | Repo | Inhalt | Voraussetzung |
|---|---|---|---|
| N0 | wai-sdk | Crate `wai-notizbuch-zugang` (lesend, Sichtbarkeit, Blocktext, Suche), Eingangsvertrag v1 + Fixtures; `wissen.rs` nutzt dieselbe Sichtbarkeitsfunktion | – |
| N1 | wai-sdk | Notizbuch: Blockzeit `zeit`, Eingang `notizbuch` (anlegen, anhängen, Statusdatei, Version vor Änderung, Editor-Transaktion), Herkunftssymbol, Abschnitt „Assistent“ | N0 |
| N2 | lva | Vendoring; Wissensbereich Notizbuch mit Abschnittsfreigabe; Werkzeuge §3.3; Index-Quelle; Projektmitglied Seite/Abschnitt | N0, ADR-0028 A4 für Projektkontext |
| N3 | lva | Schreibwerkzeuge über Eingang, Freigabe E10, Bestätigung (Sprache/Karte), Statusanzeige | N1, N2, ADR-0028 A3 (Auftragskarte) |
| N4a | lva | Zuordnung Besprechung ↔ Seiten (lesend), zweite Notizquelle in KI-Notizen/Protokoll, Verlustprüfung, Eval-Tor | N2; Blockzeiten aus N1 für Güte `genau` |
| N4b | lva | Rückweg „Protokoll ins Notizbuch“ (§5.5), eigene Freigabe | N1, N3, N4a |
| N5 | wai-sdk + lva | Verwalten (§4.3) | N3 |

Reihenfolge N0 → N2 → N4a (Lesen und Protokoll zuerst, größter Nutzen) parallel zu N1 → N3 → N4b. N4a funktioniert vor
N1 mit Güte `version`/`seite` und nur mit Vorschlägen; automatische Zuordnung erst nach bestandenem Tor.
UI-Teile (Einstellungen Notizbuch-Freigabe, Bestätigungskarte, Besprechungs-Zuordnung, Herkunftssymbol) bekommen je
eine HTML-Vorschau unter `apps/local-voice/docs/vorschau/notizbuch/` zur Codex-Abnahme.

## 9 Fragen an Codex

- **E1 Zugang:** Direktes Lesen von `notizbuch.db` (nur lesend, Schema-Prüfung) statt HTTP-Endpunkt. Empfehlung: ja.
- **E2 Schreiben über Eingang** statt gemeinsamer Schreibzugriff. Empfehlung: ja.
- **E3 Notizbuch starten:** Darf Local Voice das Notizbuch für eine Eingangsanfrage minimiert starten? Empfehlung:
  nein in N3, Anfrage wartet; als Einstellung in N5.
- **E4 Bestätigung:** Sprachansage mit 3 s Einspruchsfrist für „Anlegen“ (umkehrbar) statt Pflicht-Klick; Karte im
  Chat. Empfehlung: ja; Papierkorb und Verschieben immer mit Karte.
- **E5 Zuordnung automatisch** ab Schwelle A mit „Lösen“, statt immer Vorschlag. Empfehlung: ja, Patrick wünscht
  automatisch; Präzisionstor 0,95.
- **E6 Blockzeiten** als Editor-Attribut plus Ableitung aus Versionen für Altseiten. Empfehlung: ja.

## 10 Codex-Entscheid (08.11.2026) und Einarbeitung

E1–E3 ja. E4, E5, E6 ja mit Bedingungen. Eingearbeitet:

| Punkt | Wo |
|---|---|
| M1 Freigabe: keine Projektgrenze überschreiten, keine verräterischen Rückfragen/Zählungen | §3.2, §5.2 |
| M2 Widerruf über Gliederungszustand, Prüfung bei Ausgabe, keine Dauerberechtigung durch Links | §3.2 |
| M3 `abschnitt_anlegen` mit ausdrücklicher Freigabe, tatsächlicher Zielabschnitt, Erfolg erst nach Sicherung | §4.1, §4.2 |
| M4 Frist ab vollständiger Ansage, keine Auto-Übergabe ohne Sprachkanal, kein Pauschalverzicht, §4.3 nur Karte | §4.2, §4.3 |
| M5 Projekt/Herkunft kein Beweis, getrennter Prüfsatz mit Sonderpfaden, sonst nur Vorschläge | §5.2, §5.6 |
| M6 stabile Block-ID, Unsicherheitsintervalle, eine Regel für die Zeitachse | §5.1 |
| M7 vollständiges Nachladen oder „unvollständig“; Duplikate nur bei gleichen Zahlen/Verneinungen/Stand | §5.4 |
| M8 definierte Widerspruchsfälle, Originale über Belege erreichbar | §5.4 |
| M9 N4a lesend getrennt von N4b Rückweg | §5.5, §8 |
| Kann: Frist einstellbar, Zuordnungsgrund, Abbrechen ausstehender Anfragen | §4.2, §5.2, §4.1 |

