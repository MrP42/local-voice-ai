# Normalisierung für WER/CER (`norm-v1`)

Implementierung: `scripts/bench/tts/ttsbench/normalize.py`. Dieselbe Funktion läuft auf der Referenz
(`expected_normalized`) und auf der ASR-Ausgabe; ein Fehler entsteht also nur dort, wo sich die **gesprochenen
Wörter** nach der Normalisierung unterscheiden, nicht wegen Schreibweisen (Ziffer/Zahlwort, Groß-/Kleinschreibung,
Satzzeichen). Die Normalisierung ist idempotent (`normalize(normalize(t)) == normalize(t)`, getestet).

## Regeln (in dieser Reihenfolge)

1. **Falten:** Unicode NFC, Kleinschreibung, `ä ö ü ß` → `ae oe ue ss` (identisch zu `benchlib.normalize_word`),
   übrige Akzente entfernt (`café` → `cafe`).
2. **E-Mail/URL → gesprochene Form:** `@` → `at`, `.` → `punkt`, `/` → `slash`, `_` → `unterstrich`, `-` → Leerzeichen.
   URLs nur mit bekannter Endung (`org com de net eu io info at ch dev app`); `https://` entfällt (`://` wird nicht
   mitgesprochen erwartet).
   `info@example.org` → `info at example punkt org`.
3. **Abkürzungen:** `z. B.` → `zum beispiel`, `d. h.` → `das heisst`, `u. a.`, `usw.`, `ca.`, `bzw.`, `ggf.`, `evtl.`,
   `inkl.`, `vgl.`, `Nr.` werden ausgeschrieben.
4. **Datum:** `26.10.2026` → `26te oktober 2026` (Tag als Ordinalzahl, Monatsname; Jahr als Zahl). `3. November`,
   `vom 5. bis 7. Juni` und Ordinalzahl + `Jahrhundert/Platz/Stock/…` ergeben `<n>te`.
5. **Uhrzeit:** `14:30 Uhr`, `14.30 Uhr`, `14:30` → `14 uhr 30`; volle Stunde ohne Minuten (`8:00 Uhr` → `8 uhr`);
   führende Null der Minuten entfällt (`22:05` → `22 uhr 5`, gesprochen „zweiundzwanzig Uhr fünf“).
6. **Währung/Dezimalzahlen:** `12,50 €` → `12 euro 50` (gesprochen „zwölf Euro fünfzig“), `0,99 €` → `0 euro 99`;
   `3,5` → `3 komma 5`; `1.250,50` → `1250 komma 50`; Tausenderpunkte entfallen (`2.300.000` → `2300000`);
   Versionsnummern `2.0` → `2 punkt 0`.
7. **Einheiten/Symbole:** `%` → `prozent`, `€` → `euro`, `$` → `dollar`, `&` → `und`, `§` → `paragraf`,
   `°C` → `grad celsius`, `km/h` → `kilometer pro stunde`, hinter einer Ziffer `kg km cm mm` → ausgeschrieben.
8. **Satzzeichen:** `. , ; : ! ? " „ “ » « ( ) [ ] … '` trennen Satz- und Zahlläufe (sie verschmelzen nie), `- – — /`
   trennen Wörter (`E-Mail` → `e mail`, `Fine-Tuning` → `fine tuning`). Apostroph im Wort entfällt (`it's` → `its`).
9. **Zahlwörter → Ziffern:** zusammengeschriebene und getrennt geschriebene deutsche Zahlwörter 0 bis 999 999
   (`dreiundvierzig`, `zweihundert dreiundvierzig`, `drei und zwanzig`, `neunzehnhundertneunundachtzig`,
   `siebenundzwanzigtausendeins`) werden zu **einer** Zifferfolge (`43`, `243`, `23`, `1989`, `27001`).
   `<n> Million(en)/Milliarde(n) [<m>]` wird zu einer Zahl (`zwei Millionen dreihunderttausend` = `2,3 Mio.` = `2300000`).
   Eine Zahl zählt immer als **ein** Wort, egal wie sie geschrieben oder gesprochen wurde.
10. **Ordinalzahlen:** Wortformen (`dritten`, `sechsundzwanzigste`, `ersten`, `achte`) und Ziffern vor Monatsnamen/
    Ordinal-Substantiven werden zu `<n>te` (`3te`, `26te`, `1te`, `8te`); Endung und Fall sind egal.
11. **Ziffernweise Lesung:** Folgen von mindestens drei Einzelziffern (`zwei sieben null null eins`) und Einzelziffern nach
    `komma` (`komma eins neun fünf`) werden zu einer Zifferfolge (`27001`, `195`). Damit sind „ISO 27001“ und
    „ISO zwei-sieben-null-null-eins“ gleich, ebenso Postleitzahlen.
12. **`ein`/`eine` vor Maß-/Mengenwort** (`euro, stunde, jahr, prozent, million, …`) ist die Zahl 1 (`ein Euro` = `1 Euro`).
13. **Akronyme:** Folgen aus mindestens zwei Einzelbuchstaben (`D S G V O`, `D-S-G-V-O`, `K I`) und Folgen von mindestens
    drei deutschen Buchstabennamen (mind. zwei eindeutige: `de es ge vau o`) werden ein Token (`dsgvo`, `ki`).
    Als Wort gesprochene Akronyme (`NATO`, `UNO`) sind ohnehin ein Token (`nato`).

## Grenzen (bekannt, bewusst nicht gelöst)

| Thema | Verhalten |
|---|---|
| Zahl oder Ziffernfolge? | Zahl und ziffernweise Lesung ergeben dieselbe Ziffernfolge; ob die Sprachausgabe „viertausendsiebenhundertelf“ oder „vier sieben eins eins“ sagt, wird **nicht** unterschieden. Genau zwei Einzelziffern in Folge (`zwei drei`) werden nicht verschmolzen. |
| `ein`/`eine`/`einen` | Nur `ein`, `eine` (vor Maßwörtern) und `eins` werden zu `1`; sonst bleibt `ein` Artikel. `einen/einem/einer/eines` bleiben unverändert (`für einen Euro` ≠ `für 1 Euro`). |
| Ordinalzahl-Ziffer | Ein Punkt hinter einer Ziffer gilt nur vor Monatsnamen, vor `bis … Monat` und vor `jahrhundert, jahrtausend, platz, stock, etage, klasse, geburtstag, advent, auflage, mal, bezirk, liga, weltkrieg` als Ordinalzahl; sonst ist `3.` die Zahl 3. |
| Datum | `26.10.2026` wird als „sechsundzwanzigster Oktober“ erwartet. Liest die Sprachausgabe „sechsundzwanzigster zehnter“, entsteht ein Fehler. Zweistellige Jahre (`26.10.26`) und ISO-Daten (`2026-10-26`) sind nicht behandelt. |
| Zahlwörter | Bis 999 999 sowie Vielfache von Million/Milliarde. Nicht behandelt: `halb`, `anderthalb`, `Dutzend`, Brüche, römische Zahlen, englische Zahlwörter im Codeswitch. Ein Wort, das zufällig ein Zahlwort ist (`acht geben`, `eins`), wird wie eine Zahl behandelt, auf beiden Seiten gleich (kein Fehler, aber Verlust der Unterscheidung). |
| Währung/Einheiten | Nur `€`, `$`, `Euro` und die Tabelle unter Regel 7; `€ 20` (Symbol vorn) → `euro 20`. Keine anderen Währungen/Einheiten. |
| Akronyme | Phonetisch geschriebene Buchstabennamen („Dee Es Gee“) werden nur über die Tabelle deutscher Namen erkannt; zwei Buchstabennamen allein nicht. `E-Mail` (`e mail`) und `Email` (`email`) sind verschieden. Wörter mit Ziffern (`5G`, `CO2`, `B2B`) bleiben ein Token. |
| Adressen | Nur Endungen aus Regel 2; erwartet werden deutsche Sprechwörter `at`, `punkt`, `slash`. Sagt die Sprachausgabe „dot“ oder „Klammeraffe“, entsteht ein Fehler. Schema (`https://`) wird nicht erwartet. |
| Fremdwörter | Englische Wörter bleiben unverändert (`feedback`, `review`); unterschiedliche Schreibung durch das ASR (`Pull-Request` / `Pullrequest`) bleibt ein Fehler (Bindestriche werden getrennt, Zusammenschreibung nicht). |
| CER | Auf dem normalisierten Text einschließlich Leerzeichen; eine Zahl zählt mit ihren Ziffern. CER ist daher bei Zahlen/Adressen streng. |

WER/CER sind ein **Verständlichkeits-Proxy**: Sie messen, wie gut ein ASR die Synthese zurücklesen kann, nicht wie
natürlich sie klingt; ASR-Fehler (Eigennamen, Fremdwörter, Halluzinationen bei Stille) fließen mit ein.

## Versionierung

`norm-v1` ist an `de-v1` gebunden: `expected_normalized` ist das gespeicherte Ergebnis der Regeln. Eine Änderung der
Regeln ist eine neue Normalisierungsversion und verlangt ein neues Korpus (siehe `README.md`, Änderungsregel). Die
Version steht in `results.json`, `asr.json` und im Bericht.
