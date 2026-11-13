# Assistenten-Profil (`lva.assistent/1`)

Ein Assistent ist ein versioniertes JSON-Dokument mit **demselben Schema und demselben Verhalten** auf dem iPhone (Swift) und
am PC (TypeScript). Vorbild ist die Einrichtung eines Telefonassistenten bei fonio.ai: Prompt, Begrüßung, Stimme, Wissen,
Werkzeuge, Variablen-Extraktion. Dieses Verzeichnis ist die gemeinsame Quelle der Wahrheit:

| Datei | Inhalt |
|---|---|
| `assistent.schema.json` | JSON Schema (Draft 2020-12): beschreibt die **Struktur**, nur Dokumentation. Maßgeblich ist die Prüfung im Kern (von Hand); sie verlangt zusätzlich: keine Steuerzeichen im Namen, Trimmen des Namens, ganzzahlige Darstellung (`5.0` wird abgewiesen), vorbehaltene Ids, Gerätenamen als Id, vollständig lesbare unbekannte Felder |
| `vorlagen.json` | die 15 eingebauten Vorlagen (Liste von Profilen, kanonisch geschrieben) |
| `pruefstaende.json` | gemeinsame Fälle, die **beide** Kerne mit identischem Ergebnis bestehen |
| `auswertung.schema.json` | JSON Schema des Ergebnisses `lva.auswertung/1` (Dokumentation), siehe Abschnitt „Auswertung“ |
| `auswertung-pruefstaende.json` | gemeinsame Fälle der Auswertung (Erdung, Fristauflösung, Prompt) |
| `README.md` | diese Beschreibung |

Kerne: `apps/apple/VoiceCore/Sources/VoiceCore/Assistant/AssistantProfile*.swift` und
`apps/local-voice/src/components/voice-call/assistenten/` (`profil.ts`, `text.ts`, `vorlagen.ts`). Die eingebetteten Fassungen
von `vorlagen.json` (`AssistantProfileTemplates.swift`, `vorlagenJson.ts`) erzeugt
`python3 apps/apple/scripts/generate_assistant_templates.py` (`--check` prüft, ob sie noch passen). Vite lädt nicht aus Ordnern
außerhalb von `apps/local-voice`, darum liegt der JSON-Text für den PC als Zeichenkette in `vorlagenJson.ts`.

## Felder

Schlüssel deutsch, ASCII. Pflicht sind `schema`, `id`, `name`; fehlende optionale Felder und Objekte bekommen die Standardwerte
und werden beim Schreiben ausgegeben. Längen zählen in **Unicode-Codepunkten** (Swift `unicodeScalars.count`, TS `[...s].length`).

| Feld | Regel | Standard |
|---|---|---|
| `schema` | genau `"lva.assistent/1"` | – |
| `id` | `^[a-z0-9][a-z0-9-]{0,63}$`, kein Gerätename von Windows (siehe unten) | – |
| `name` | nach Entfernen von Leerzeichen an den Enden 1..60 Codepunkte, kein Steuerzeichen/Zeilenumbruch | – |
| `beschreibung` | 0..200 | `""` |
| `anweisungen` | 0..8000, Zeilenumbrüche erlaubt | `""` |
| `begruessung` | 0..300 | `""` |
| `sprache` | `"auto"` oder `^[a-z]{2}(-[A-Z]{2})?$` | `"de"` |
| `stimme` | `null` oder Text 1..120 | `null` |
| `verhalten` | `{stil: natuerlich\|woertlich, sprechpause: kurz\|normal\|lang, unterbrechen: bool, nachfrage_stille_s: 0 oder 5..120}` | natuerlich, normal, true, 0 |
| `wissen` | `{alle_bereiche: bool, bereiche: [Schlüssel], modus: alles\|zuerst\|nur, kontexte: [{art, id, titel}]}` | false, [], alles, [] |
| `schnittstellen` | Liste von Kennungen `^[a-z0-9][a-z0-9._-]{0,63}$`, höchstens 32, keine Dubletten | `[]` |
| `auswertung` | `{zusammenfassung: bool, aufgaben: bool, felder: [{name, beschreibung, typ, optionen}]}` | false, false, [] |

- **Bereiche** (`wissen.bereiche`): bekannt sind `buecher, buchnotizen, ki_dokumente, besprechungen, sprachnotizen, ablage,
  gespraeche, kalender, vorlesen, notizbuch`. Unbekannte Schlüssel, die `^[a-z][a-z0-9_]{0,39}$` erfüllen, bleiben erhalten und
  werden ignoriert (kein Fehler). Dubletten sind ein Fehler (`doppelt`, am späteren Eintrag).
- **Kontexte**: höchstens 50; `art` ∈ `buch, besprechung, sprachnotiz, datei, ordner, projekt, abschnitt` (Pflicht); `id` 1..200
  (Pflicht); `titel` 0..200 (Standard `""`). Modus `zuerst`/`nur` mit leeren `kontexte` ist gültig und wirkt wie `alles`
  (`kontextModusWirksam` / `effectiveContextMode`).
- **Auswertungsfelder**: höchstens 30; `name` `^[a-z][a-z0-9_]{0,39}$` und eindeutig (Pflicht); `beschreibung` 0..200; `typ` ∈
  `text, zahl, datum, auswahl` (Pflicht); `optionen` (Texte 1..60, Standard `[]`) ist bei `auswahl` Pflicht mit 2..20 Einträgen.
- **Unbekannte Felder bleiben erhalten**, auf der obersten Ebene und in `verhalten`, `wissen`, `auswertung`, in Kontext- und
  Feld-Objekten: Der Kern liest in ein typisiertes Modell, das je Ebene die unbekannten Felder (`extra`) behält; beim Schreiben
  liegen die typisierten Felder darüber.

## Prüfung und Fehler

`parse` liefert das Profil **oder** die vollständige Fehlerliste `[{pfad, code}]`, nach `pfad`, dann `code` sortiert (nach
Codepunkten, gleich der UTF-8-Reihenfolge; `[10]` steht also vor `[2]`), ohne Wiederholungen. Es wird nie stillschweigend repariert.
Pfade: `name`, `wissen.kontexte[3].art`, `auswertung.felder[0].optionen`.

| Code | Bedeutung |
|---|---|
| `fehlt` | Pflichtfeld fehlt |
| `typ` | falscher JSON-Typ (auch `null` für Felder außer `stimme`) |
| `laenge` | Länge außerhalb der Grenzen (Codepunkte); bei `optionen` einer Auswahl: weniger als 2 |
| `muster` | passt nicht zum Muster (bei `name`: Steuerzeichen) |
| `wert` | nicht in der Menge der erlaubten Werte (Enums, `nachfrage_stille_s`, Gerätename als `id`, vorbehaltene Id beim Kopieren) |
| `zu_viele` | mehr Einträge als erlaubt (Kontexte 50, Schnittstellen 32, Felder 30, Optionen 20) |
| `doppelt` | Dublette (Bereiche, Schnittstellen, Feldnamen) |
| `schema_unbekannt` | `schema` ist weder die Fassung 1 noch eine neuere |
| `schema_neuer` | `lva.assistent/<n>` mit n>1 |
| `json` | kein gültiges JSON oder kein Objekt (ein Fehler mit Pfad `""`); bei einem Baum: ein Wert, den der Leser nicht wieder läse (siehe „Prüfung des Baums“); bei einer Datei: nicht lesbar, Symlink oder andere nicht reguläre Datei (Pfad `""`) |

## Vergleichsregel: genau

Alle Vergleiche und Schlüssel sind **genau**: Zwei Texte sind gleich, wenn ihre Folge der Unicode-Skalare (UTF-8-Bytes) gleich ist.
Es gibt keine Normalisierung und keine kanonische Gleichheit: `é` (U+00E9) und `e` + U+0301 sind zwei verschiedene Texte, zwei
Schlüssel, zwei Platzhalter. Das gilt für Platzhalter samt Zusatzwerten und der Liste der Unbekannten, Dubletten (Bereiche,
Schnittstellen, Feldnamen), JSON-Objektschlüssel und deren Dubletten, Ids, Vorlagen und Dateinamen. TypeScript vergleicht Zeichenketten
mit `===` (Codeeinheiten, für wohlgeformte Texte dasselbe). Swift vergleicht `String` dagegen kanonisch (`==`, `Set<String>`,
`Dictionary<String, _>`, `contains`; sogar U+212A ist dort gleich `K`); der Swift-Kern vergleicht darum über die UTF-8-Bytes
(`AssistantExact`, `AssistantExactKey`), auch in den `Equatable`-Umsetzungen seiner öffentlichen Typen. Deren `==` zählt die Reihenfolge der Mitglieder eines JSON-Objekts nicht (geschrieben wird sortiert), die einer Liste schon. Ein Swift-`[String: String]`
(Zusatzwerte) führt kanonisch gleiche Schlüssel schon beim Einfügen zusammen; das ist die Grenze des Typs, nicht des Kerns.

## Gerätenamen und vorbehaltene Ids

- **Gerätenamen von Windows** taugen nicht als Dateiname und sind als `id` abgewiesen (Pfad `id`, Code `wert`): `con`, `prn`, `aux`,
  `nul`, `com0` bis `com9`, `lpt0` bis `lpt9`. Mit Nummer oder Zusatz bleiben andere Ids gültig (`con1`, `com10`, `nul-1`, `console`).
  Im Schema steht das als `not`.
- **Vorbehaltene Ids**: `aktiv` (der Dateiname `aktiv.json` gehört der aktiven Auswahl), die Ids der Vorlagen und alles mit dem
  Vorsatz `vorlage-`. Speichern, Löschen, Kopieren-Ziel und Auswählen mit einer vorbehaltenen Id schlagen fehl (Swift
  `reservedId`; das Auswählen nur für vorbehaltene Ids, die keine Vorlage sind). Eine vorhandene `aktiv.json` wird nie als Profil gelesen
  und nie als Profil überschrieben.

## Prüfung des Baums

Grundsatz: **Was die Prüfung besteht, schreibt `encode`/`kodiere` und liest `parse` wieder, mit bytegleicher zweiter Ausgabe.**
Darum prüft `parse(tree:)`/`profilAusBaum`/`pruefeProfil` (also auch alles, was nicht aus Text, sondern von Hand gebaut wurde) die
übernommenen unbekannten Felder vollständig: Zahlentext nach der JSON-Grammatik (`-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`),
dieselbe Tiefengrenze wie der Leser (die Wurzel liegt bei Tiefe 0, Werte bis Tiefe 64 sind lesbar), doppelte Schlüssel in
jedem Objekt (auch in den typisierten Ebenen), in TypeScript zusätzlich einzelne Surrogate in Zeichenketten und Schlüsseln (typisierte
Felder eingeschlossen; Swift-Strings können keine enthalten). Befund: Code `json`, Pfad des Feldes. Bei unbekannten Feldern ist das
der Pfad des obersten unbekannten Feldes, nie tiefer (`x`, `verhalten.neu`, `wissen.kontexte[0].neu`, `auswertung.felder[1].neu`); bei
doppelten Schlüsseln in einer typisierten Ebene der Pfad des Objekts (`""`, `verhalten`, `wissen.kontexte[0]`). Ein Wert in zu großer
Tiefe wird nicht weiter durchlaufen. Ein typisiertes Ganzzahlfeld nimmt nur `-?(0|[1-9][0-9]*)` (`007` ist `typ`).

## Auslegungen (wo die Vorgabe mehrdeutig war, gilt die einfachste)

Jede Auslegung ist durch mindestens einen Fall in `pruefstaende.json` festgehalten.

- **JSON-Leser**: strenges RFC 8259, ein Wert, nichts danach. Ein BOM, Kommentare, `NaN`, führende Nullen, Komma am Ende, rohe
  Steuerzeichen in Zeichenketten, einzelne Surrogate in `\u`-Folgen und mehr als 64 Ebenen Tiefe sind `json`. Doppelte
  Schlüssel: der letzte gilt. Zahlen bleiben im Baum als **Originaltext** (`1.50` bleibt `1.50`); ein typisiertes Ganzzahlfeld
  nimmt nur `-?(0|[1-9][0-9]*)` bis 15 Ziffern (Bruch/Exponent: `typ`, mehr Ziffern: `wert`).
- **Schreiber**: Schlüssel nach Codepunkten sortiert, 2 Leerzeichen Einzug, leere Listen und Objekte als `[]` und `{}`, `": "`
  zwischen Schlüssel und Wert, Escapes `\" \\ \n \r \t \b \f` und `\u00xx` (klein) für übrige Zeichen unter U+0020, alles
  andere unverändert als UTF-8, genau ein `\n` am Ende. Die Ausgabe ist auf beiden Plattformen bytegleich.
- **`schema`**: eine falsche Fassung bricht die Prüfung ab (nur dieser eine Fehler, weil der Rest nach einer anderen Fassung
  nichts bedeutet). `lva.assistent/<n>` zählt als neuer, wenn `n` ohne führende Null aus Ziffern besteht und nicht 1 ist.
- **`name`**: Es wird nur U+0020 an den Enden entfernt, und gespeichert wird der getrimmte Name. Steuerzeichen sind
  U+0000–U+001F, U+007F–U+009F, U+2028, U+2029 (Fehler `muster`; Länge und Muster können beide anschlagen).
- **Listen über dem Höchstmaß** melden `zu_viele` und werden trotzdem Eintrag für Eintrag geprüft. `optionen` ohne `auswahl`
  sind frei (nur Typ und Länge der Einträge zählen).
- **`bereiche` mit Musterfehler** melden kein `doppelt` für denselben Eintrag.
- **Wirksame Bereiche** = (`alle_bereiche` ? alle Bereiche der Plattform : die auf die Plattform abgebildeten bekannten
  Schlüssel) ∩ freigegebene Bereiche; das Profil öffnet nie mehr, als freigegeben ist. iPhone (`AssistantArea`): `buecher`→`books`,
  `besprechungen`→`transcripts`, `ablage`→`files`, `sprachnotizen`→`notes`, alle übrigen Schlüssel ohne Wirkung. PC
  (`ALL_AREAS` in `bereiche.ts`): `buecher`→`books`, `buchnotizen`→`book_notes`, `ki_dokumente`→`ki_docs`,
  `besprechungen`→`meetings`, `sprachnotizen`→`dictations`, `gespraeche`→`conversations`, `kalender`→`calendar`,
  `vorlesen`→`tts`, `notizbuch`→`notebook`; `ablage` hat am PC noch keine Wirkung. Das Ergebnis steht in der Reihenfolge der
  Plattformliste (`AssistantArea.allCases` bzw. `ALL_AREAS`), ohne Wiederholung.
- **Vorbehaltene Ids**: `aktiv`, die Ids der Vorlagen und alles mit dem Vorsatz `vorlage-` (siehe oben). Sie lassen sich nicht
  speichern und beim Kopieren nicht vergeben (`alsEigenenKopieren` / `copyAsOwn` meldet `id`/`wert`); die Profilprüfung selbst
  lässt sie zu.

## Platzhalter

In `begruessung` und `anweisungen`: `{{name}}`, `{{tageszeit}}`, `{{datum}}`, `{{wochentag}}`, `{{uhrzeit}}`; weitere per
übergebener Tabelle (für spätere Schnittstellen; die eingebauten gehen vor). Die Werte kommen aus festen Listen, nicht aus
Locale-Schnittstellen: Sprache `en` und `en-XX` → Englisch, alles andere (auch `auto`) → Deutsch.

| Platzhalter | Deutsch | Englisch |
|---|---|---|
| `tageszeit` | Stunde 5–10 „Morgen“, 11–17 „Tag“, sonst „Abend“ | morning, afternoon, evening |
| `datum` | „13. November 2026“ | „November 13, 2026“ |
| `wochentag` | Montag … Sonntag (1 = Montag) | Monday … Sunday |
| `uhrzeit` | „08:05“ | „08:05“ |

Eingabe sind Kalenderfelder (Jahr, Monat, Tag, Wochentag, Stunde, Minute); keine Zeitzonenlogik im Kern. Ein Wert außerhalb
des Bereichs (Monat 13, Stunde 24 …) gilt als leer.

**Erkennung**: `{{` + höchstens 40 Zeichen ohne `{`, `}` und Zeilenumbruch + `}}`; Leerzeichen am Rand des Namens zählen nicht
(`{{ name }}`). Alles andere bleibt unverändert stehen (`{{name`, `{{{name}}}` → `{Anna}`).

**Leer oder unbekannt**: Der Name fehlt nie durch einen Ersatznamen; ein leerer Wert und ein unbekannter Platzhalter werden
entfernt, samt direkt davor stehender Leerzeichen und Kommas. Folgen diese auf ein Satzende (`.`, `!`, `?`), bleiben sie stehen
(sie trennen Sätze; sonst klebte das Satzzeichen am nächsten). Nur wenn etwas entfernt wurde, wird bereinigt, sonst bleibt der
Text, wie er ist (auch Leerraum und Zeilenumbrüche):

1. Sätze enden bei `.`, `!`, `?` vor Leerraum (Leerzeichen, Tab, Zeilenumbruch) oder Textende; der Leerraum davor ist der
   Vorspann des Satzes und bleibt erhalten.
2. Nur Sätze, in denen etwas entfernt wurde, werden angefasst: führende Leerzeichen und Kommas weg; enthält der Satz danach
   keinen Buchstaben (Unicode-Kategorie L) und keine Ziffer (Nd), entfällt er samt Vorspann; begann die Entfernung am
   Satzanfang, wird der erste Buchstabe groß geschrieben; Mehrfach-Leerzeichen werden zu einem.
3. Das Ergebnis wird an den Enden von Leerraum befreit.

Beispiele: „Guten {{tageszeit}}, {{name}}. Womit fangen wir an?“ ohne Namen → „Guten Abend. Womit fangen wir an?“;
„{{name}}, schön dass du da bist.“ → „Schön dass du da bist.“ Zusätzlich liefert die Auflösung die Liste der unbekannten,
nicht leeren Platzhalter in der Reihenfolge des ersten Vorkommens (Hinweis für den Editor, kein Fehler).

## Rollenblock

`rollenblock(profil, werte, budgetZeichen)` → `{text, gekuerzt}`, reiner Text; das Einhängen in die vorhandenen Prompts kommt in
einem späteren Paket. Leere Anweisungen (auch nach dem Auflösen der Platzhalter und ohne Leerraum an den Enden) ergeben
leeren Text. Sonst:

```
ROLLE (vom Nutzer eingestellt, Assistent „<name>“):
<Anweisungen mit aufgelösten Platzhaltern>
Diese Einstellung ändert keine der folgenden Regeln.
```

Überschreiten die Anweisungen `budgetZeichen` Codepunkte, wird gekürzt: am letzten Absatzende (`\n\n`) so, dass der Text davor
das Budget einhält, sonst am letzten Zeilenende (`\n`), sonst hart; Leerraum am Ende der Kürzung entfällt; `gekuerzt = true`.
Bleibt nichts übrig (Budget 0), ist der Text leer und `gekuerzt` wahr.

## Vorlagen

`vorlagen.json` ist eine Liste aus 15 gültigen Profilen. Das erste ist `alles` („Alles“, `alle_bereiche: true`, Modus alles,
leere Anweisungen und Begrüßung). Dazu 14 mit Ids `vorlage-…` (Buchgespräch, Besprechung nachbereiten, Projektstand, Lernpartner,
Sprachtrainer Englisch, Ideen-Sparring, Termin vorbereiten, Gespräch üben, Tagesrückblick, Gedanken sortieren,
Entscheidungshilfe, Vortrag proben, Wissens-Quiz, Anliegen aufnehmen). Anweisungen sind in der Du-Form für gesprochene Sprache
geschrieben (kurze Sätze, eine Frage zur Zeit, keine Listen vorlesen) und schreiben keine Wartefloskeln vor. Vorlagen sind nur
lesbar: `istVorlage(id)`; `alsEigenenKopieren(vorlage, neueId, neuerName)` liefert eine geprüfte eigene Kopie.

## Speicherformat

Gilt gemeinsam für Swift (`AssistantProfileStore`) und den späteren Speicher am PC.

- **Ein Verzeichnis je Gerät.** Es enthält nur eigene Assistenten, **nie Vorlagen**.
- **Je eigenem Assistenten eine Datei `<id>.json`** mit exakt der Ausgabe von `encode` (UTF-8, ohne BOM, sortierte Schlüssel,
  2 Leerzeichen, `\n` am Ende).
- **Der Dateiname muss zur `id` im Inhalt passen**, sonst wird die Datei abgewiesen (Code `muster`, Pfad `id`). Eine Datei mit
  vorbehaltener Id (Vorlage, `vorlage-…`, `aktiv`) wird ebenfalls abgewiesen (`wert`, Pfad `id`).
- **Die aktive Auswahl liegt in `aktiv.json`** als `{"id": "<id>"}` (gleiche Schreibweise). Fehlt die Datei, ist sie ungültig,
  oder zeigt sie auf ein fehlendes oder gelöschtes Profil, gilt die Vorlage `alles`. `aktiv` ist deshalb als Id vorbehalten; das
  Auswählen schreibt nur `aktiv.json`, nie ein Profil.
- **Gelesen wird nur von regulären Dateien.** Die Art des Eintrags kommt von `attributesOfItem` (Symlinks werden nicht verfolgt,
  Typ `.typeRegular`). Ein Symlink `x.json` (auch ein hängender) und jede andere nicht reguläre Datei (FIFO, Gerät) gilt als
  abgewiesen: Code `json`, Pfad `""`; sie wird nicht geöffnet. Ein **Verzeichnis** `x.json` wird ignoriert. Das gilt für Auflisten
  und direkten Abruf gleich; auch `aktiv.json` als Symlink ist ungültig (Rückfall `alles`).
- **Obergrenzen**: 256 KiB je Datei (größere werden abgewiesen und nicht gelesen: `laenge`, Pfad `""`). Höchstens **200 Dateien**:
  Die Grenze gilt für das **Auflisten** (über 200 Dateien hinaus, nach Dateiname sortiert, werden weitere abgewiesen: `zu_viele`,
  Pfad `""`, ohne sie zu lesen) und für die **Neuanlage** (`tooMany`; das Überschreiben einer vorhandenen Datei bleibt möglich).
  Der direkte Abruf per Id liest unabhängig davon.
- **Andere Dateien im Verzeichnis werden ignoriert** (alles, was nicht auf `.json` endet, und `aktiv.json` selbst).
- **Atomar schreiben**: in eine temporäre Datei im selben Verzeichnis, dann ersetzen; nie halb geschriebene Profile.
- **Eine abgewiesene Datei wird nie verändert oder gelöscht**; sie erscheint in der Liste der Abgewiesenen mit Dateiname und
  Fehlerliste. **Speichern** (`save`) schreibt nur, wenn die Zieldatei fehlt oder ein gültiges eigenes Profil mit genau dieser Id
  ist; sonst Fehler `occupied` („belegt“: beschädigt, neueres Schema, falsche Id im Inhalt, zu groß, Symlink, Verzeichnis, keine
  reguläre Datei) und es wird nichts geschrieben. **Löschen** (`delete`) entfernt nur ein gültiges eigenes Profil; eine abgewiesene
  Datei ist `occupied`, ein Verzeichnis, Symlink oder anderes Nicht-Reguläres (und eine fehlende Datei) ist `notFound`, nichts wird
  entfernt.
- **Ein Schreiber je Verzeichnis.** Kapazitätsprüfung, die Prüfung der Zieldatei und `newOwnId` sind nicht atomar gegen parallele
  Prozesse; innerhalb der App läuft das Schreiben seriell. Zwei Prozesse, die dasselbe Verzeichnis beschreiben, sind nicht vorgesehen.
- **Eigene Ids**: `eigen-` plus 12 Zeichen `[0-9a-z]` aus einem übergebenen Zufallserzeuger.

## Prüfstände

`pruefstaende.json` hat die Abschnitte `faelle` (Eingabe, erwartet `{ok, kodiert}` oder `{fehler}`; eine Zeichenkette als
`eingabe` ist Rohtext, jeder andere Wert ein JSON-Wert), `ids` (je Id: Ergebnis der Profilprüfung `ok`/`muster`/`wert` und ob sie
vorbehalten ist), `baeume` (von Hand gebaute Profilbäume mit den Sonderformen `{"$zahl": Rohtext}`, `{"$paare": [[Schlüssel,
Wert], …]}` für doppelte Schlüssel und `{"$tief": n}` für Tiefe; erwartet wie bei `faelle`), `platzhalter`, `rollenblock`,
`bereiche_iphone`, `bereiche_pc` und `kontextmodus`. Die erwarteten Ausgaben der gültigen Fälle stammen aus Pythons `json`-Modul,
nicht aus einem der Kerne. Der Swift-Test und der Node-Test lesen **dieselbe Datei** und prüfen dieselben erwarteten Bytes (Swift
vergleicht über `Array(text.utf8)`, nicht über `String ==`). Neu erzeugen ist nicht vorgesehen; ein neuer Fall kommt von Hand
hinzu und muss in beiden Kernen grün sein. Nur in TypeScript prüfbar (Swift-Strings kennen keine einzelnen Surrogate und das
JSON hier keine ungültigen Zahlen): einzelne Surrogate und sehr große Eingaben (`push(...feld)`) stehen im Node-Test.

## Auswertung (`lva.auswertung/1`)

Am Gesprächsende wertet ein Modell das Gespräch nach dem Profil (`auswertung`) aus. Der gemeinsame Kern (Paket G5, #559) ist
**reine Logik ohne Oberfläche, ohne Modellaufruf, ohne Speicher und ohne Folgeaktionen**: Er baut den Prompt, liest die Rohantwort
tolerant, **erdet jede Angabe im Gespräch** und kodiert das Ergebnis. Kerne: `apps/apple/VoiceCore/Sources/VoiceCore/Assistant/
AssistantOutcome.swift` (Typen, Kodierung, Prüfung, Erdung), `AssistantOutcomePrompt.swift` (Prompt, Gesprächsfassung),
`AssistantOutcomeDates.swift` (Normalisierung, Zahlen, Fristauflösung) und am PC `assistenten/auswertung.ts`,
`auswertungFrist.ts`. Dateien dieses Verzeichnisses dazu: `auswertung.schema.json` (Dokumentation) und
`auswertung-pruefstaende.json` (gemeinsame Fälle; beide Kerne vergleichen bytegenau).

### Ablauf (drei reine Funktionen)

| Schritt | Swift | TypeScript |
|---|---|---|
| Prompt | `AssistantOutcome.prompt(profile:turns:today:budgetChars:)` → `{text, truncated}` | `auswertungPrompt(profil, verlauf, heute, budgetZeichen)` → `{text, gekuerzt}` |
| Auswerten | `AssistantOutcome.evaluate(profile:turns:raw:today:created:truncated:)` → `Result<{outcome, hints}, .json>` | `auswerten(profil, verlauf, rohantwort, heute, {erstellt, gekuerzt})` → `{ok, ergebnis, hinweise}` oder `{ok: false, fehler: "json"}` |
| Kodieren / Lesen | `outcome.encode()` / `AssistantOutcome.parse(json:)` | `kodiereAuswertung(e)` / `parseAuswertung(text)` |
| Frist | `AssistantOutcomeDates.deadline(_:today:)` | `frist(ausdruck, heute)` |

`verlauf` ist die Liste der Beiträge `{rolle: nutzer|assistent, text}`; die Nummer eines Beitrags ist seine Position ab 1. `heute`
sind Jahr, Monat, Tag des Gesprächs (keine Zeitzonenlogik). `erstellt` (ISO-8601 UTC `JJJJ-MM-TTThh:mm:ssZ`) und `gekuerzt` (aus
dem Prompt) setzt der Aufrufer, der Kern kennt weder Uhr noch Prompt-Aufruf; ohne Angabe bleibt `erstellt` leer und `gekuerzt`
falsch. Kein auffindbares JSON-Objekt in der Rohantwort ist der Fehler `json` (der Aufrufer kann einmal neu fragen).

### Prompt

Eine Zeichenkette für EIN Modell: Anweisung, dann `Gespräch:` und die Beiträge als `[1] Nutzer: …`, `[2] Assistent: …` (Leerraum
zu einem Leerzeichen zusammengefasst, ein Beitrag je Zeile). Die Anweisung verlangt genau ein JSON-Objekt, nur für das, was das
Profil einschaltet (`felder`, `zusammenfassung`, `aufgaben`), dazu immer `offene_fragen`. Ihre Regeln: nichts erfinden; je Feld
Status `klar`, `unklar` oder `nicht_genannt`; bei widersprüchlichen oder später korrigierten Angaben gilt die letzte eindeutige,
sonst `unklar`; Beleg ist ein wörtliches Zitat mit Beitragsnummer; Fristen als wörtlicher Ausdruck (`frist_text`), nicht als
errechnetes Datum; Zusammenfassung zwei bis fünf Sätze; nur Namen aus dem Gespräch, keine Floskeln. Das heutige Datum steht mit
Wochentag in der Anweisung (fehlt bei ungültigem `heute`). Der Wortlaut ist in den Prompt-Fällen der Prüfstände festgehalten.

**Kürzung**: `budgetZeichen` zählt Codepunkte des ganzen Textes. Passt er nicht, bleiben die ersten zwei Beiträge und so viele
der neuesten wie möglich; dazwischen steht eine Zeile `[… n Beiträge ausgelassen …]` (bei n = 1 `[… 1 Beitrag ausgelassen …]`),
`gekuerzt` ist wahr. Die Nummern bleiben die des ungekürzten Gesprächs. Ein Beitrag wird nie geschnitten; das Mindeste (zwei
erste Beiträge und die Auslassung) bleibt auch über dem Budget stehen. Mit höchstens zwei Beiträgen ist nichts auszulassen
(`gekuerzt` falsch). Ein negatives Budget zählt als 0.

### Ergebnis

Schlüssel deutsch, kanonisch kodiert wie das Profil (sortiert, 2 Leerzeichen, `\n` am Ende, bytegleich in beiden Kernen).

| Feld | Regel |
|---|---|
| `schema` | genau `"lva.auswertung/1"` (sonst `schema_unbekannt` / `schema_neuer`) |
| `assistent` | Id des Profils |
| `erstellt` | `""` oder `JJJJ-MM-TTThh:mm:ssZ` |
| `gekuerzt` | bool |
| `zusammenfassung` | 0..1200; leer, wenn das Profil keine will |
| `felder` | alle Felder des Profils in dessen Reihenfolge: `{name, typ, status, wert, beleg, beitrag}`; `wert`, `beleg` Text oder `null` (je ≤ 500), `beitrag` Zahl ab 1 oder `null` |
| `aufgaben` | höchstens 20: `{text 1..300, frist, frist_text ≤ 100, beleg 1..500, beitrag}`; `frist` `JJJJ-MM-TT` oder `null` |
| `offene_fragen` | höchstens 10 Texte je ≤ 300 |

Werte stehen kanonisch als Text: `zahl` mit Punkt als Dezimaltrenner ohne Tausenderzeichen, `datum` als `JJJJ-MM-TT`, `auswahl` als
genau eine der `optionen`, `text` höchstens 500. `nicht_genannt` hat `wert`, `beleg` und `beitrag` `null`; `klar` hat einen `wert`.
`parse` prüft dieses Format mit Pfad und Code wie das Profil (`fehlt`, `typ`, `laenge`, `muster`, `wert`, `zu_viele`, `doppelt`,
`schema_unbekannt`, `schema_neuer`, `json`); unbekannte Schlüssel werden verworfen, nichts wird repariert.

### Normalisierung für Vergleiche

Alle Vergleiche mit dem Gespräch laufen auf dem **normalisierten** Text, gemeinsam festgelegt:

1. Kleinschreibung, je Codepunkt (`ΑΣ` wird `ασ`, `İ` wird `i̇`; keine Kontextregeln).
2. Anführungszeichen überall entfernen: `" ' ` ´ „ “ ” ‚ ‘ ’ « » ‹ ›`.
3. Leerraum (U+0009–U+000D, U+0020, U+00A0) zu einem Leerzeichen zusammenfassen.
4. Leerraum und Satzzeichen an den Rändern entfernen: `. , ; : ! ? … ( ) [ ] { } - – —`. Innen bleiben Satzzeichen stehen.

„Steht im Beitrag“ heißt: Der normalisierte Beleg ist ein zusammenhängender Teil (Codepunkte) des normalisierten Beitrags. Ein
Beleg, der normalisiert leer ist, steht nirgends.

**Unicode: kein Zusammensetzen, keine kanonische Gleichheit.** Verglichen wird immer Codepunkt für Codepunkt (Swift: UTF-8-Bytes
bzw. Skalar-Folgen, nie `String ==`, das kanonisch äquivalente Texte gleichsetzt). `é` (U+00E9) und `e` + U+0301 sind **verschiedene**
Texte: Ein Beleg in der einen Form steht nicht in einem Beitrag in der anderen, ein Feldname oder eine Auswahloption in der einen
Form ist nicht die der anderen Form (Feld: `unbekannt`, Auswahl außerhalb der Optionen: `unklar`, Hinweis `wert`), ebenso `ü` und `u` + U+0308
in der Fristauflösung (`übermorgen` gilt, `u` + U+0308 + `bermorgen` nicht). Das ist gewollt: Ein Beleg, der nicht bytegleich im
Gespräch steht, ist kein Beleg. Fälle in `auswertung-pruefstaende.json` (Profil P7, Gespräch C7, `normalisierung`, `frist`).

### Rohantwort lesen

Das **erste ausbalancierte, gültige JSON-Objekt** im Text zählt, auch in Code-Zäunen oder mit Text davor und danach. Von jedem `{`
aus wird die schließende Klammer gesucht (Zeichenketten und Escapes beachtet). Gibt es keine, geht die Suche beim nächsten `{`
weiter. Gibt es eine, der Text dazwischen ist aber kein gültiges JSON (strenger Leser wie beim Profil), geht die Suche **hinter**
der Klammer weiter (innere Objekte eines kaputten äußeren zählen nicht). Nichts gefunden: Fehler `json`.

**Obergrenzen gegen manipulierte Rohantworten** (gleich in beiden Kernen):

- Eine Rohantwort mit mehr als **262144 Codepunkten** (`MAX_ROHANTWORT` / `AssistantOutcome.maxRaw`; gezählt in Codepunkten, nicht in
  UTF-16-Einheiten) ist der Fehler `json`, ohne dass weiter gesucht oder gelesen wird. Genau 262144 werden gelesen.
- **Tiefe:** der strenge Leser (wie beim Profil) erlaubt höchstens 64 Ebenen; ein Objekt mit 65 ist ungültig, die Suche geht
  dahinter weiter.
- **Nicht quadratisch:** Ein `{`, zu dem es keine schließende Klammer gibt, zählt als offener Start. Der 17. offene Start beendet die
  Suche mit `json` (`MAX_OFFENE_STARTS` / `AssistantOutcome.maxOpenStarts` = 16 sind erlaubt). Ausbalancierte, aber ungültige Objekte
  werden überlesen, die Suche geht hinter ihnen weiter. 200000 `{` oder 80000 mal `{,}` enden unter 2 Sekunden als `json`
  (Test in beiden Kernen); mit echtem Objekt dahinter wird das echte gefunden.
- Große Gespräche und lange Zeichenketten lösen keine Ausnahme aus: Ketten werden in Blöcken zu Texten zusammengesetzt
  (`ausCodepunkten`, 8192 je Block), nie über `fromCodePoint(...x)` oder `push(...x)` mit datenabhängiger Länge. Belege, Werte,
  Aufgabentexte usw. halten ihre Längenregeln auch bei Millionen Zeichen im Gespräch oder 500000 Beiträgen ein.

### Erdung im Code (Abnahme: erfundene Werte = 0)

Gelesen werden je Feld `name`, `status`, `wert` (Text oder Zahl als Originaltext, an den Enden von Leerraum befreit),
`beleg`, `beitrag` (Zahl oder Ziffernfolge). Dann, in dieser Reihenfolge:

1. **Status.** Fehlt er oder ist er ungültig, gilt `klar` mit Wert, sonst `nicht_genannt` (Hinweis `fehlt` bzw. `wert`/`typ`).
   `nicht_genannt` verwirft Wert und Beleg. `klar` ohne Wert wird `nicht_genannt`. `unklar` ohne Wert und ohne Beleg bleibt `unklar`.
2. **Beleg.** Jeder Wert braucht einen Beleg. Ein **Beitragspräfix** am Anfang des Belegs wird vor dem Vergleich abgestreift und
   nicht gespeichert: optional `[N]` (1 bis 6 Ziffern), optional ein Rollenwort `Nutzer`, `Assistent`, `User`, `Assistant` (Groß-/Kleinschreibung
   gleich) mit Doppelpunkt, Leerraum dazwischen frei (`[3] Nutzer: …`, `Nutzer: …`, `[3] …`, `Assistant: …`). Ein anderes Wort vor dem
   Doppelpunkt (`Chef: …`) ist kein Präfix. Nennt die Antwort keinen `beitrag`, gilt die Nummer des Präfixes. Steht der Beleg nicht (normalisiert) im angegebenen Beitrag, wird der erste Beitrag
   gesucht, der ihn enthält, und `beitrag` korrigiert (kein Hinweis). Steht er in keinem Beitrag des **ungekürzten** Gesprächs
   oder fehlt er: Feld `nicht_genannt`, `wert`, `beleg`, `beitrag` `null`, Hinweis `beleg_fehlt`. Belege länger als 500
   Codepunkte werden hart gekürzt (Hinweis `laenge`; ein Anfang bleibt ein Teil des Originals).
3. **Typ.** `zahl`: `-?\d+`, `-?\d+[.,]\d+`, `1.250,50`, `1,250.50` oder ein Zahlwort null bis zwölf; kanonisch mit Punkt, ohne
   Tausenderzeichen. `datum`: `JJJJ-MM-TT` mit gültigem Kalendertag, sonst wird der Wert als Ausdruck durch die Fristauflösung
   gelesen. `auswahl`: genau eine Option, sonst gleich nach Normalisierung (die Option in ihrer Schreibweise gilt). `text`: höchstens
   500 Codepunkte (sonst hart gekürzt, Hinweis `laenge`). Nicht lesbare Zahl oder ungültiges Datum: Status `unklar`, Wert als Text
   erhalten (≤ 500), Hinweis `typ`. Auswahl außerhalb der Optionen: `unklar`, Wert erhalten, Hinweis `wert`.
4. **Deckung.** Ist der Wert durch den Beleg nicht gedeckt, wird der Status `unklar` (Wert bleibt, Hinweis
   `wert_nicht_im_beleg`). Gedeckt heißt: `text` der normalisierte Wert ist Teil des normalisierten Belegs; `zahl` der Beleg
   enthält diese Zahl (Ziffernfolgen, durch ein einzelnes `.` oder `,` zwischen Ziffern verbunden, als Dezimalzahl oder mit
   Tausenderpunkt gelesen; Vorzeichen zählen nicht; `1.250,50` deckt `1250.5`) **oder** ein Zahlwort null bis zwölf (de/en, die
   `ein`-Formen als 1) steht für genau diesen Wert; `datum` eine Folge von bis zu 6 Wörtern des Belegs löst die Fristauflösung auf
   genau diesen Tag auf; `auswahl` die Option (normalisiert) steht im Beleg.
5. **Status am Ende.** `klar` nur, wenn das Modell `klar` meldete und Typ und Deckung stimmen; sonst `unklar`.

**Felder der Antwort.** Nur Felder des Profils zählen; der erste Eintrag je Name gilt (weitere: `doppelt`, unbekannte
Namen: `unbekannt`). Fehlende Felder sind `nicht_genannt` (kein Hinweis). Das Ergebnis hat immer alle Felder in Profilreihenfolge.

**Aufgaben.** Nur wenn das Profil sie will (sonst Hinweis `unbekannt` am Schlüssel, falls die Liste nicht leer ist). Eine Aufgabe
ohne Text (`fehlt`) oder ohne Beleg im Gespräch (`beleg_fehlt`) wird verworfen; `beitrag` wird wie bei Feldern korrigiert. Der
`frist_text` (≤ 100) muss normalisiert im Gespräch stehen, sonst wird er geleert (Hinweis `beleg_fehlt` am `frist_text`; die Aufgabe
bleibt). `frist` ist das Ergebnis der Fristauflösung für `frist_text`, sonst `null`. Mehr als 20: die ersten 20, Hinweis `zu_viele`.
Text über 300: am Satzende gekürzt, sonst hart (`laenge`).

**Zusammenfassung** nur, wenn das Profil sie will (sonst `unbekannt`); über 1200 Codepunkte am letzten Satzende (`.`, `!`, `?` vor
Leerraum oder Textende) gekürzt, sonst hart (`laenge`). **Offene Fragen:** leere entfallen, über 300 gekürzt wie oben, mehr als 10
(`zu_viele`) abgeschnitten.

**Hinweise** sind `[{pfad, code}]`, nach Pfad und Code sortiert (Codepunkte), ohne Wiederholung, Codes `beleg_fehlt`,
`wert_nicht_im_beleg`, `typ`, `wert`, `unbekannt`, `doppelt`, `laenge`, `fehlt`, `zu_viele`. Pfade: `felder[2].wert`,
`aufgaben[1].beleg`, `offene_fragen`; Indizes bei Feldern des Profils sind die des Profils, bei `unbekannt`/`doppelt`/`typ` eines
Roheintrags die der Rohliste. Unbekannte Schlüssel der Antwort (oberste Ebene, Feld- und Aufgabeneinträge) werden verworfen
(`unbekannt`). Ein Schlüssel, der nicht `^[a-z][a-z0-9_]{0,39}$` entspricht, erscheint nie im Pfad (Inhalt des Modells). Die
Auswertung schreibt **nie in Logs**; Hinweise tragen nur Pfad und Code.

### Fristauflösung

`frist(ausdruck, heute)` → `JJJJ-MM-TT` oder `null`, aus einer festen Liste, deutsch und englisch. Der Ausdruck wird normalisiert;
führende Vorsätze `am`, `bis`, `zum`, `zur`, `spätestens`, `by`, `until`, `on` entfallen (solange mehr als ein Wort bleibt).
`heute` muss ein gültiger Kalendertag der Jahre 1000–9999 sein, sonst `null`. Wochentag zählt Montag = 1 wie bei den Platzhaltern.

| Ausdruck | Ergebnis |
|---|---|
| `heute`, `today` / `morgen`, `tomorrow` / `übermorgen` (`uebermorgen`), `day after tomorrow` | heute / +1 / +2 Tage |
| Wochentag allein, `nächsten`/`nächster`/`kommenden`/`next` + Wochentag | der nächste solche Tag **nach** heute (1–7 Tage) |
| `übernächsten`/`übernächster` + Wochentag | eine Woche später |
| `in N Tagen/Wochen`, `in N days/weeks` | N Zahl (höchstens 3 Ziffern) oder Zahlwort null bis zwölf (`ein`-Formen = 1) |
| `nächste Woche`, `next week` | Montag der Folgewoche |
| `Ende der Woche`, `end of the week` (auch `dieser`/`this`) | Freitag der laufenden Woche; am Samstag und Sonntag der der folgenden |
| `Ende des Monats`, `end of the month` (auch `dieses`/`this`) | letzter Tag des Monats von heute |
| `14.11.`, `14.11`, `14.11.2026` | Tag.Monat ohne Jahr: nächstes Vorkommen ab heute (heute zählt); mit vierstelligem Jahr genau dieser Tag |
| `14. November`, `14 November [2026]`, `14th of November`, `November 14[th][, 2026]` | wie oben, Monatsnamen deutsch und englisch (ausgeschrieben, auch `maerz`) |
| `2026-11-14` | genau dieser Tag |

Alles andere ist `null` (`gestern`, `nächsten Monat`, `Nov 14`, `in dreizehn Tagen`, zweistellige Jahre …); dann bleibt `frist_text`
stehen und `frist` ist `null`. Ungültige Tage (`31.4.`, `29.2.2027`) sind `null`; `29.2.` ohne Jahr sucht das nächste Schaltjahr
(höchstens 8 Jahre voraus). Bleiben nach den Vorsätzen mehr als 6 Wörter, ist das Ergebnis `null`. `fristenIn(text, heute)` /
`AssistantOutcomeDates.deadlines(in:today:)` löst jede Folge von bis zu 6 Wörtern eines Textes auf (für die Deckung bei `datum`),
und zwar nur in den ersten 1024 Wörtern (`MAX_FRIST_WOERTER`). Datumsangaben ohne Jahr meinen „ab heute“; `heute` sollte darum der Tag
des Gesprächs sein.

### Prüfstände

`auswertung-pruefstaende.json` hat die Abschnitte `auswerten` (Profil, Verlauf, `heute`, Rohantwort, optional `erstellt` und
`gekuerzt`; erwartet `{ok, kodiert, hinweise}` oder `{ok: false, fehler: "json"}`; statt `roh` darf `roh_teile` stehen, eine Liste
aus `{text, anzahl}`, deren Teile `anzahl`-mal wiederholt und hintereinandergesetzt die Rohantwort ergeben, damit Fälle an den Grenzen
klein bleiben), `frist` (Ausdruck, `heute`, erwartet Datum oder
`null`), `normalisierung`, `lesen_fehler` (Eingabe für `parse`, erwartete Fehlerliste) und `prompt` (Profil, Verlauf, `heute`,
Budget, erwartet Text und `gekuerzt`, bytegleich). Alle Gespräche sind erfunden. Die erwarteten `kodiert`-Texte der
`auswerten`-Fälle stammen aus Pythons `json`-Modul (die Erwartungen sind von Hand als Daten geschrieben); die Prompt-Texte stammen
aus der TypeScript-Fassung und wurden von der Swift-Fassung unabhängig bestätigt. Ein neuer Fall kommt von Hand hinzu und muss in
beiden Kernen grün sein.
