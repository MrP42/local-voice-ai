# Korpus `de-v1`

Deutsche Testsätze für den TTS-Benchmark (Goal tts-provider, Paket P5). Eigene Texte, erstellt am 26.10.2026,
keine Fremdquellen, keine echten personenbezogenen Daten (nur fiktive Namen wie „Max Mustermann“, E-Mail-Adressen
und URLs ausschließlich mit `example.org/.com/.net/.de`, keine Telefonnummern; `validate-corpus` prüft das).

| Datei | Inhalt |
|---|---|
| `de-v1.jsonl` | 123 Sätze, 1424 Wörter, 10 Kategorien, 12 Sätze in der Smoke-Teilmenge |
| `en-func-v1.jsonl` | 6 englische Sätze, nur als Funktionsnachweis für englische Modelle (Phonon 31m-en); kein Qualitätsvergleich |
| `normalization.md` | Normalisierungsregeln für WER/CER (Version `norm-v1`) |

## Format (eine JSON-Zeile je Satz)

| Feld | Bedeutung |
|---|---|
| `id` | `de-v1-<kategorie>-<nnn>`, eindeutig, wird nie wiederverwendet |
| `category` | `alltag`, `lang`, `zahlen`, `datum`, `akronyme`, `codeswitch`, `fachbegriffe`, `email_url`, `fragen`, `ausdruck` |
| `text` | der Satz, wie er an die Sprachausgabe geht |
| `expected_normalized` | so, wie ein ASR den Satz idealerweise ausgibt, nach `norm-v1` normalisiert (kanonische Form, siehe `normalization.md`: Zahlen als Ziffern, Datum/Uhrzeit/Akronyme/Adressen in gesprochener Form) |
| `smoke` | `true` = gehört zur Smoke-Teilmenge (12 Sätze, mind. 1 je Kategorie) |
| `spoken` | nur wo `text` Ziffern/Symbole enthält: die von Hand geschriebene gesprochene Fassung. `expected_normalized` ist `normalize(spoken)`; `validate-corpus` prüft zusätzlich `normalize(text) == expected_normalized`, so ist die automatische Normalisierung unabhängig gegen eine Handlesung geprüft |

Kategorien (Anzahl): alltag 12, lang 11, zahlen 13, datum 13, akronyme 13, codeswitch 12, fachbegriffe 13,
email_url 12, fragen 12, ausdruck 12.

Smoke-Teilmenge: `alltag-001`, `lang-004`, `zahlen-001`, `zahlen-002`, `datum-001`, `datum-002`, `akronyme-001`,
`codeswitch-004`, `fachbegriffe-004`, `email_url-001`, `fragen-006`, `ausdruck-001`.

## Änderungsregel (verbindlich)

**Sätze werden nie geändert, nie gelöscht und nie umsortiert.** Auch Tippfehler bleiben stehen: Ergebnisse früherer
Läufe beziehen sich auf genau diese Texte, und Läufe speichern den SHA-256 des Korpus (über den Inhalt mit
LF-Zeilenenden, damit CRLF-Checkouts denselben Wert liefern).

- Neue oder geänderte Sätze, neue Kategorien, eine andere Smoke-Teilmenge oder eine geänderte Normalisierung
  (`norm-v2`) erfordern eine **neue Korpusdatei `de-v2.jsonl`** mit eigenen IDs (`de-v2-…`).
- Ergebnisse verschiedener Korpusversionen werden nicht verglichen.
- `de-v1.jsonl` und `en-func-v1.jsonl` werden nur gelesen. Der Test `test_corpus_validates_clean_…` bricht, sobald
  Text und `expected_normalized` oder `spoken` auseinanderlaufen.

## Prüfen

```
python scripts/bench/tts/tts_bench.py validate-corpus            # de-v1: >= 100 Sätze, Kategorien >= 10, IDs, Smoke, Normalisierung
python scripts/bench/tts/tts_bench.py validate-corpus en-func-v1 # Basisprüfung
```
