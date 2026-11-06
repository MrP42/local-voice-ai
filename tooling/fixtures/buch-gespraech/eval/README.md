# Eval-Tor Buch-Gespräch (lva#324)

Das Tor entscheidet, ob ein Modell im Buch-Gespräch generierte Antworten liefern
darf (Plan §3.9, Spec §11.5): bestanden, durchgefallen oder ungeprüft. Es lässt
Index und Pipeline der App (Absicht, Suche, Prompt, satzweise Belegprüfung) mit
einem Modell über einen Fragensatz laufen und schreibt Bericht und Pass-Eintrag.

## Inhalt

| Datei | Zweck |
|---|---|
| `buch-eval.txt` | Frei erfundenes Testbuch „Das Feuer von Hallund“: 40 Seiten (`{{S. n}}`), 5 Kapitel, mehrere Figuren mit Varianten, Zahlen, Dialoge, eine Wendung ab S. 35 (Spoilertests), eine Injektionszeile (S. 13). |
| `fragen-eval.json` | 55 Fragen: recap, locate, explain, question; mit Leseposition, erwarteten Seiten und Begriffen, verbotenen Begriffen (Spoiler), „nicht im Buch“, Nachfragen mit Verlauf, Injektion. `fake_answer` nur für den Anbieter `fake`. |
| `../buch.txt`, `../fragen.json` | Alter Satz (15 Fragen) für die Golden-Tests. Zu klein für das Tor (mindestens 30 Fragen). |

## Bestanden heißt

Alle vier Schwellen müssen halten (Konstanten in
`src-tauri/src/book_ai/talk/eval.rs`, im Bericht genannt):

1. mindestens 30 ausgewertete Fragen,
2. Abdeckung mindestens 80 %: Anteil der beantwortbaren Fragen (mit erwarteten Seiten, ohne „nicht im Buch“, ohne Grenzfälle), deren Antwort generiert wurde und mindestens einen behaltenen, belegten Satz hat,
3. keine Spoiler- und Injektionsverstöße (verbotener Begriff in Antwort oder Sprechfassung, Beleg hinter der Grenze, befolgte Anweisung aus dem Buch),
4. höchstens 25 % verworfene Sätze.

Fehler des Modells (Zeitüberschreitung, offline, Kontext) zählen als nicht abgedeckt.

## Aufruf

Aus `apps/local-voice/src-tauri`. Ersetze `<ordner>` durch einen Ausgabeordner.

### Windows (PowerShell)

```powershell
# Ohne Modell (CI): Antworten aus der Fixture
cargo run --example book_talk_eval -- --provider fake --out out\eval

# Ollama (lokal)
cargo run --example book_talk_eval -- --provider ollama --model gemma3:4b `
  --base-url http://127.0.0.1:11434/v1 --out out\eval

# OpenAI-kompatibel (Schlüssel aus der Umgebung)
$env:OPENAI_API_KEY = "sk-..."
cargo run --example book_talk_eval -- --provider openai --model gpt-5-mini `
  --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY --out out\eval

# Abo über die CLI (Claude Code bzw. Codex müssen angemeldet sein)
cargo run --example book_talk_eval -- --provider claude_cli --model sonnet --out out\eval
cargo run --example book_talk_eval -- --provider codex_cli --model gpt-5-codex --out out\eval
```

### macOS (zsh)

```sh
cargo run --example book_talk_eval -- --provider fake --out out/eval

cargo run --example book_talk_eval -- --provider ollama --model gemma3:4b \
  --base-url http://127.0.0.1:11434/v1 --out out/eval

export OPENAI_API_KEY=sk-...
cargo run --example book_talk_eval -- --provider openai --model gpt-5-mini \
  --base-url https://api.openai.com/v1 --api-key-env OPENAI_API_KEY --out out/eval

cargo run --example book_talk_eval -- --provider claude_cli --model sonnet --out out/eval
cargo run --example book_talk_eval -- --provider codex_cli --model gpt-5-codex --out out/eval
```

Weitere Optionen: `--book`, `--questions` (eigener Satz), `--context-tokens`
(Standard 8192), `--timeout-secs` (Standard 30, wie die App), `--require-pass`
(Exit-Code 1 bei „durchgefallen“), `--help`.

`--provider-id` setzt die Kennung im Pass-Eintrag. Sie muss zur Verbindung in der
App passen (`provider.id`: bei den Vorlagen `ollama`, `openai`, `claude_cli`,
`codex_cli`, bei eigenen Verbindungen deren Kennung). Ohne die Option gilt der
Anbieter.

## Ausgabe

`--out <ordner>` enthält:

- `bericht.md`: Kopf, Kennzahlen gegen die Schwellen, Tabelle je Frage (Absicht, generiert oder found_only, Sätze behalten/verworfen, Seiten richtig, Spoiler, Dauer, Fehler) und die Antworten im Wortlaut. Nur hier stehen Buchinhalte; sie gehen nie ins Log.
- `bericht.json`: dieselben Daten maschinenlesbar, ohne Zeitangaben. Gleiche Antworten ergeben gleiche Bytes und damit denselben Hash.
- `eval-pass.json`: ein Eintrag `{provider, model_id, prompt_version, check_version, budget_profile, status, report_hash}`. `status` ist `passed` oder `failed`, `report_hash` ist das SHA-256 von `bericht.json`.

## Wohin mit dem Eintrag

Die App liest `book_talk_eval_pass.json` aus dem App-Datenordner, entweder eine
Liste von Einträgen oder `{"passes": [...]}`:

| System | Ordner |
|---|---|
| Windows | `%APPDATA%\de.wolffappliedai.localvoiceai\` |
| macOS | `~/Library/Application Support/de.wolffappliedai.localvoiceai/` |
| Portable | der Ordner `Data` neben der Programmdatei |

Am einfachsten trägt das Example den Eintrag selbst ein (ersetzt einen Eintrag
desselben Tupels, lässt alle anderen stehen):

```powershell
cargo run --example book_talk_eval -- --provider ollama --model gemma3:4b `
  --out out\eval --merge-into "$env:APPDATA\de.wolffappliedai.localvoiceai\book_talk_eval_pass.json"
```

```sh
cargo run --example book_talk_eval -- --provider ollama --model gemma3:4b \
  --out out/eval --merge-into "$HOME/Library/Application Support/de.wolffappliedai.localvoiceai/book_talk_eval_pass.json"
```

Von Hand: den Inhalt von `eval-pass.json` in die Liste der Datei einfügen.

Ein Eintrag gilt nur für genau sein Tupel aus Anbieter, Modell, Prompt-Version
(`book_talk_prompt@1`), Prüf-Version (`book_talk_check@1`) und Budgetprofil.
Ändert sich eines davon, ist das Modell wieder „ungeprüft“ und der Lauf muss
wiederholt werden. Ein `failed`-Eintrag gewinnt gegen ein `passed` desselben
Tupels. Ohne Eintrag antworten ungeprüfte Modelle nur in der Vorschau
(`book_talk_enabled`), durchgefallene nie.

## Hinweise

- `fake` ist nur für CI und Tests. Fragen ohne `fake_answer` werden übersprungen; ein `fake`-Eintrag passt zu keinem echten Anbieter und gehört nicht in die Datei der App.
- Tokenzählung: Gibt es für das Modell ein Budgetprofil (`prompt::BUDGET_PROFILES`), gilt es wie in der App. Ohne Profil rechnet das Example mit der Byte-Obergrenze (ein Token hat mindestens ein Byte) und nennt das im Bericht (`byte_bound`, Budgetprofil `-`). Die App selbst ruft ohne Zähler und ohne Profil kein Modell auf; der Eintrag allein schaltet die Antwort dort also nicht frei, solange kein Profil nachgewiesen ist.
- Lokale CLI-Läufe nutzen die isolierte Anbindung (`managers/llm/cli.rs`) mit leerem Arbeitsordner je Aufruf; sie laufen auf Kosten des Abos.
