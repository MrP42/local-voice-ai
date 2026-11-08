# Notizbuch für den Assistenten – Lesen, Anlegen, Besprechungsnotizen zusammenführen

- Stand: 08.11.2026, Entwurf zur Codex-Abnahme
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
- Widerruf wirkt sofort im Tor (ADR-0028 E6, eine Regel): laufende Runden mit widerrufenen Inhalten brechen ab, der
  Index-Epoch wechselt.

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

Aktionen v1: `seite_anlegen`, `bloecke_anhaengen {seite_id}`. Kein Überschreiben, kein Löschen von Blöcken.
Idempotenz über `(app, id)`. Ergebnis legt der Host als Statusdatei ab (`uebernommen` mit Seiten-Id oder `abgelehnt`
mit Code); Local Voice zeigt es an der Auftragskarte.

Prüfung im Notizbuch-Host (maßgeblich): Ziel-Abschnitt existiert, ist aktiv und für **Schreiben** freigegeben
(§4.2); Text nur Klartext (kein HTML, kein Markdown-Link mit `javascript:`, Steuer- und Bidi-Zeichen entfernt);
höchstens 200 Blöcke, 64 000 Zeichen; Seitenlinks nur auf existierende Seiten. Vor `bloecke_anhaengen` legt der Host
eine Version der Seite an (bestehende Versionierung), damit jede Änderung rückgängig zu machen ist.

Ist die Zielseite gerade im Editor offen und ungespeichert, hängt der Host die Blöcke über eine Editor-Transaktion an
(Ereignis an das Webview), nicht an der gespeicherten Fassung vorbei.

Läuft das Notizbuch nicht, sagt der Assistent: „Die Notiz liegt bereit und erscheint, sobald das Notizbuch geöffnet
ist.“ Optional (Codex E3): Local Voice startet das Notizbuch minimiert über den Deep-Link.

### 4.2 Schreibfreigabe

- Getrennt von der Lesefreigabe, je Abschnitt, Standard aus.
- Beim ersten Schreibwunsch ohne Ziel schlägt der Assistent einen neuen Abschnitt **„Assistent“** in einem von
  Patrick gewählten Notizbuch vor; nach Bestätigung legt der Host ihn an und er ist schreibfreigegeben.
- Wirkungsklasse (ADR-0019-Nachtrag aus ADR-0028): **lokal, umkehrbar** (Papierkorb, Versionen). Ausführung nur, wenn
  die strukturierte Freigabe der Runde (E10) die Aktion enthält – und die enthält sie nur, wenn die **Nutzereingabe**
  das Anlegen verlangt. Anweisungen in Mails, Transkripten oder Notizen lösen nie ein Schreiben aus.
- Bestätigung: Bei Sprache sagt der Assistent Ziel und Titel an und legt nach 3 s ohne Einspruch an („Stopp“ bricht
  ab); im Chat zeigt eine Karte Ziel, Titel und Blöcke mit „Anlegen“ und „Ändern“. Bei Wiederholung in derselben
  Unterhaltung mit gleichem Ziel entfällt die Karte (Codex E4).
- Herkunft: Seiten und Blöcke aus dem Assistenten tragen `herkunft: "assistent"` (Notizbuch zeigt ein kleines Symbol
  und in der Seiteninfo „Angelegt vom Assistenten, Unterhaltung …“).

### 4.3 Verwalten (Stufe N5)

Umbenennen, Verschieben zwischen schreibfreigegebenen Abschnitten, Kategorie setzen, Aufgabe abhaken, Wiedervorlage
setzen, Seite in den Papierkorb legen. Jeweils als Eingangs-Aktion, immer mit Bestätigung, Papierkorb nie ohne
Karte. Endgültig löschen und Papierkorb leeren gibt es für den Assistenten nicht.

## 5 Besprechung: Notizen zusammenführen

### 5.1 Blockzeiten im Notizbuch

Damit Notizbuch-Notizen auf der Zeitachse einer Aufnahme liegen, bekommt jeder Block im Notizbuch das Attribut
`zeit` (ISO-Zeit der letzten inhaltlichen Änderung des Blocks, gesetzt vom Editor; neu in `domain/sicherung.ts`
eingetragen). Ältere Seiten ohne Blockzeiten: Der Zugang leitet die Zeit deterministisch aus den Versionen ab (ein
Block, der zuerst in Version *n* auftaucht, liegt zwischen den Zeiten von *n−1* und *n*); sonst gilt die Seitenzeit
mit Kennzeichen „ungefähr“.

### 5.2 Zuordnung

Kandidaten sind Seiten aus lese- **oder** projektfreigegebenen Abschnitten, deren Blockzeiten im Fenster
`[Beginn − 15 min, Ende + 30 min]` der Besprechung liegen. Bewertung (deterministisch im Tor, kein Modellurteil):

| Merkmal | Gewicht |
|---|---|
| ausdrücklicher Bezug: Seitenlink auf die Besprechung, Seite aus Local Voice angelegt, gleicher Projekt-Verweis | sicher |
| Anteil der Blöcke mit Zeit im Fenster | hoch |
| Begriffsüberlappung Block ↔ Transkriptabschnitt derselben Minuten (BM25 über gefaltete Wörter, Namen, Zahlen) | mittel |
| Titel enthält Datum, Teilnehmende oder Kalendertitel | gering |

- **Sicher** oder Punktzahl ≥ Schwelle A: automatisch zugeordnet; die Besprechung zeigt „Notizen aus dem Notizbuch:
  *Seite* (14:02–14:47)“ mit „Lösen“.
- Zwischen B und A: Vorschlag mit „Zuordnen“. Unter B: nichts.
- Schwellen werden am Eval-Satz (§5.5) festgelegt; Zuordnung nur in Besprechungen, deren Notizbuch-Abschnitt für den
  Assistenten freigegeben ist. Ohne Freigabe fragt die Besprechung einmal, ob der Abschnitt freigegeben werden soll.
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
- Doppelte Blöcke (gefalteter Text gleich oder Ähnlichkeit ≥ 0,9) werden zu einem Eintrag mit beiden Belegen.
- Aufgaben (Notizbuch-Kategorie Aufgabe, Aufgabenliste) werden zu Aufgaben im Protokoll mit Verweis zurück; erledigte
  bleiben erledigt.
- Widersprechen sich Notiz und Transkript (Zahl, Datum, Name), steht im Protokoll beides mit Beleg und Hinweis
  „abweichend notiert“; das Modell entscheidet nicht still.
- Anhänge und Zeichnungen erscheinen als Verweis „[Zeichnung auf Seite …]“.

### 5.5 Rückweg

„Protokoll ins Notizbuch“ (Knopf in der Besprechung und Assistent-Aktion): legt über den Eingang eine Seite mit dem
Protokoll in den zugeordneten oder gewählten Abschnitt; Aufgaben mit Kategorie Aufgabe (und „An Planner senden“ wie
gewohnt im Notizbuch).

### 5.6 Eval-Tor (synthetische Daten, `example.*`)

40 Besprechungen DE + 40 EN mit Notizbuch-Seiten (richtig, zeitnah aber themenfremd, alte Seite ohne Blockzeiten,
Duplikate, Widersprüche, Prompt-Injection in Notizen):

- 0 verlorene Nutzerblöcke (§5.4), 0 ausgeführte Schreibaktionen aus Inhalten, 0 Inhalte aus nicht freigegebenen
  Abschnitten.
- Zuordnung: Präzision ≥ 0,95 bei „automatisch“, Trefferquote ≥ 0,9 über automatisch + Vorschlag.

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
| N4 | lva | Zuordnung Besprechung ↔ Seiten, zweite Notizquelle in KI-Notizen/Protokoll, Verlustprüfung, Rückweg, Eval-Tor | N2 |
| N5 | wai-sdk + lva | Verwalten (§4.3) | N3 |

Reihenfolge N0 → N2 → N4 (Lesen und Protokoll zuerst, größter Nutzen) parallel zu N1 → N3.
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
