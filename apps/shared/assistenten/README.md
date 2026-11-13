# Assistenten-Profil (`lva.assistent/1`)

Ein Assistent ist ein versioniertes JSON-Dokument mit **demselben Schema und demselben Verhalten** auf dem iPhone (Swift) und
am PC (TypeScript). Vorbild ist die Einrichtung eines Telefonassistenten bei fonio.ai: Prompt, Begrüßung, Stimme, Wissen,
Werkzeuge, Variablen-Extraktion. Dieses Verzeichnis ist die gemeinsame Quelle der Wahrheit:

| Datei | Inhalt |
|---|---|
| `assistent.schema.json` | JSON Schema (Draft 2020-12), nur Dokumentation; die Kerne prüfen von Hand |
| `vorlagen.json` | die 15 eingebauten Vorlagen (Liste von Profilen, kanonisch geschrieben) |
| `pruefstaende.json` | gemeinsame Fälle, die **beide** Kerne mit identischem Ergebnis bestehen |
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
| `id` | `^[a-z0-9][a-z0-9-]{0,63}$` | – |
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
| `wert` | nicht in der Menge der erlaubten Werte (Enums, `nachfrage_stille_s`, vorbehaltene Id beim Kopieren) |
| `zu_viele` | mehr Einträge als erlaubt (Kontexte 50, Schnittstellen 32, Felder 30, Optionen 20) |
| `doppelt` | Dublette (Bereiche, Schnittstellen, Feldnamen) |
| `schema_unbekannt` | `schema` ist weder die Fassung 1 noch eine neuere |
| `schema_neuer` | `lva.assistent/<n>` mit n>1 |
| `json` | kein gültiges JSON oder kein Objekt (ein Fehler mit Pfad `""`) |

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
- **Vorbehaltene Ids**: die Ids der Vorlagen und alles mit dem Vorsatz `vorlage-`. Sie lassen sich nicht speichern und beim
  Kopieren nicht vergeben (`alsEigenenKopieren` / `copyAsOwn` meldet `id`/`wert`).

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
  vorbehaltener Id (Vorlage) wird ebenfalls abgewiesen (`wert`, Pfad `id`).
- **Die aktive Auswahl liegt in `aktiv.json`** als `{"id": "<id>"}` (gleiche Schreibweise). Fehlt die Datei, ist sie ungültig,
  oder zeigt sie auf ein fehlendes oder gelöschtes Profil, gilt die Vorlage `alles`.
- **Obergrenzen**: 200 Dateien und 256 KiB je Datei. Größere Dateien werden abgewiesen und nicht gelesen (`laenge`, Pfad `""`);
  über 200 Dateien hinaus (nach Dateiname sortiert) werden weitere abgewiesen (`zu_viele`, Pfad `""`).
- **Andere Dateien im Verzeichnis werden ignoriert** (alles, was nicht auf `.json` endet, und `aktiv.json` selbst).
- **Atomar schreiben**: in eine temporäre Datei im selben Verzeichnis, dann ersetzen; nie halb geschriebene Profile.
- **Eigene Ids**: `eigen-` plus 12 Zeichen `[0-9a-z]` aus einem übergebenen Zufallserzeuger.
- Eine abgewiesene Datei wird nie verändert oder gelöscht; sie erscheint in der Liste der Abgewiesenen mit Dateiname und
  Fehlerliste.

## Prüfstände

`pruefstaende.json` hat die Abschnitte `faelle` (Eingabe, erwartet `{ok, kodiert}` oder `{fehler}`; eine Zeichenkette als
`eingabe` ist Rohtext, jeder andere Wert ein JSON-Wert), `platzhalter`, `rollenblock`, `bereiche_iphone`, `bereiche_pc` und
`kontextmodus`. Die erwarteten Ausgaben der gültigen Fälle stammen aus Pythons `json`-Modul, nicht aus einem der Kerne.
Neu erzeugen ist nicht vorgesehen; ein neuer Fall kommt von Hand hinzu und muss in beiden Kernen grün sein.
