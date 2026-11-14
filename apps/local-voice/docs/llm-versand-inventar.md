# Inventar der Modellaufrufe (lva#604, Teilpaket 1; Stand nach Teilpaket 3 in Abschnitt I, Compile-Zeit-Zusicherungen in Abschnitt J)

Stand: Basis `4ba8f873` (main), Zweig `feat/601-cloud-sperre`. Alle Pfade relativ zu
`apps/local-voice/src-tauri/`. Zeilennummern gelten für diese Basis; wohin die Wege in Teilpaket 3
gewandert sind und was bewusst draußen blieb, steht in Abschnitt I.
Erhoben per `grep` über `src/` und `vendor/` (Suchbegriffe: `reqwest::Client`, `.post(`, `.send()`,
`chat/completions`, `/api/chat`, `/api/generate`, `/embeddings`, `audio/speech`, `Command::new`,
`llm_client::`, `cli::call*`) und durch Lesen jeder Treffer.

Das Modul `src/llm_versand/` (Vertrag, Typen, Prüfer-Rumpf) ist in Teilpaket 1 nur eingebunden.
Dieses Inventar sagt, **welche Stelle in Teilpaket 3 wohin wandert**.

Legende „Besprechungsinhalt“: **ja** (Text aus Besprechungen geht in den Aufruf), **möglich**
(der Weg kann ihn tragen, die Herkunft ist heute nicht belegt: in #601 `Quellen::unbestimmt()`,
bis eine Herkunft ermittelt ist), **nein** (mit Begründung).
Legende „Ziel in TP3“: **transport** (Aufruf läuft über `llm_versand`, `Ziel` gebunden) oder
**ohne Besprechungsinhalt** (bleibt außerhalb, mit Begründung; #601 sichert das per
Architekturtest ab).

## A. Zentraler Transport (wandert nach `llm_versand/transport/`)

| # | Stelle | Weg | Transportart | Besprechungsinhalt | Ziel in TP3 |
|---|---|---|---|---|---|
| A1 | `llm_client.rs:232`, `:274`, `:303` → `send_inner` `:352` | Chat-Aufruf am Stück | HTTP `POST` `:418`, Client `create_client` `:192` (Umgebungsproxy, folgt Redirects); CLI-Zweig `:365` | ja (über B1–B9) | transport (Einzelversuch; Route nicht neu auflösen: `:369` entfällt) |
| A2 | `llm_client.rs:1120`, `:1141`, `:1161` → `send_stream_full` `:1184` → `stream_inner` `:1225` | Chat-Aufruf als Strom | HTTP `POST` `:1290`; gepinnt lokal `:1275–1280` (`create_pinned_local_client` `:203`: ohne Proxy, ohne Redirect); CLI-Zweig `:1237` | ja (B6, B10–B12) | transport; bei Cloud mit Besprechungsquellen Puffer für Deltas und Denktexte (`:1354`, `:1358`) |
| A3 | `llm_client.rs:1400` (`complete_once`, Aufruf `:1299`, `:1333`) | Rückfall „Strom ohne Strom“ | HTTP `POST` `:1407` | ja (wie A2) | **außerhalb** des Einzeltransports, als neuer Versuch mit eigener Prüfung |
| A4 | `llm_client.rs:597` (`send_cli`) → `call_cli` `:576` | CLI-Aufruf (Claude Code, Codex) | CLI-Prozess über `managers/llm/cli.rs:1258`, `:1270`, `:1353` | ja (wie A1/A2) | transport; CLI-Neuversuch und CLI-Update (`:681`, `:692`) außerhalb, jeder als neuer Versuch; CLI-Denktexte puffern (`:629`) |
| A5 | `managers/llm/cli.rs:1353` (`call_with_options`) → `call_inner` `:1558`, Prozessstart `:1626` | Kern des CLI-Aufrufs | `Command::new` `:1626` | ja | **verschieben** nach `transport` samt Vorschauprüfung (`preview_precheck` `:1535`, `verify_preview` `:1396`, Durchgriff `:1365–1369`); `pub(super)` nur bis `llm_versand` |
| A6 | `llm_client.rs:849` (`send_ollama_native`) → `ollama_native_inner` `:878` | natives Ollama (`/api/chat`) | `reqwest::Client::new()` `:895`, `POST` `:897–899`: **heute nicht lokal erzwungen**, Umgebungsproxy, folgt Redirects | möglich (Aufrufer D6, Vorlese-Übersetzung) | transport mit gebundenem `Ziel`; Cloud wird als Cloud behandelt |
| A7 | `llm_client.rs:1506` (`post_image_prompt`) | Bild + Prompt | `Client::builder()` `:1524`, `POST` `:1529`; **ohne** Compliance-/Budgetprüfung, Umgebungsproxy, folgt Redirects | ja (E1, Folien) | transport (Bildtransport) |
| A8 | `llm_client.rs:751` (`fetch_models`, GET `:771`, CLI-Zweig `:760`) | Modellliste | HTTP `GET` | nein: Verwaltung, kein Inhalt | transport (Netzoperation, so verlangt Codex Runde 5) |
| A9 | `llm_client.rs:934` (`ollama_unload`, POST `:940`) | Modell entladen | `Client::new()` `:940` | nein: Verwaltung, kein Inhalt | transport (wie A8) |

## B. Aufrufer der zentralen Funktionen mit Besprechungsinhalt: **ja**

Diese Stellen gehen in TP3 über die Übergangsfassade `llm_versand::uebergang` (heutige
Signaturen, bisheriges Verhalten, `#[deprecated(note = "entfällt mit #601")]`); #601 stellt sie auf
`versenden(..)` mit belegter Herkunft um. Sie werden **nicht** als `keine()` umetikettiert.

| # | Stelle | Weg | Zentrale Funktion | Bemerkung |
|---|---|---|---|---|
| B1 | `managers/meetings/llm_call.rs:397` | `ask_json` (Einstieg für B2–B8, mit Struktur-Retry, `JSON_ATTEMPTS`) | `send_chat_completion_checked` | Kontextüberlauf-Diagnose `llm_call.rs:339`, `:350` muss erhalten bleiben |
| B2 | `managers/meetings/notes/enhance.rs:1037`, `:1292`, `:1510`, `:2241` | KI-Notizen (vier Aufrufstellen) | über B1 | `enhance.rs:324`, `:347` messen vorher Token am lokalen Server (B13) |
| B3 | `managers/meetings/notes/classify.rs:401` | Einstufung | über B1 | |
| B4 | `managers/meetings/minutes.rs:1062`, `:1198`, `:1394` | Protokoll | über B1 | |
| B5 | `managers/meetings/minutes/project.rs:376`, `:497`, `:632` | Projektprotokoll | über B1 | |
| B6 | `managers/meetings/merge.rs:381` | Zusammenführung zweier Fassungen | über B1 | |
| B7 | `managers/meetings/translate.rs:977` | Übersetzung einer Fassung | über B1 (`retry_chunk` wiederholt) | Wiederholung = neuer Versuch mit eigener Prüfung |
| B8 | `managers/meetings/followup.rs:456` | Folgemail | `send_chat_completion_checked` | |
| B9 | `managers/meetings/chat/controller.rs:35` (Import), `:529` | Chat über Besprechungen | `send_chat_completion_stream` | streamt Deltas und Denktexte an die Oberfläche (`:522`); Puffer nötig |
| B10 | `summarizer.rs:435` (`ask_llm`) | Zusammenfassung; von den Workflows mit Besprechungstext benutzt (`managers/workflows/app_services.rs:137`) | `send_chat_completion` | |
| B11 | `agent/runtime.rs:417` (`post_inner`, Client `:433`, `POST` `:440`) | Agenten-Routing und Extraktion der Workflows (`managers/workflows/agent_route.rs:318`, `agent_actions.rs:192`, `:325`) | **eigener Transport**, kein `llm_client` | Compliance wird geprüft (`:421`); `Target::Local` über `ensure_local`, `Target::Endpoint` (Attrappe, später entfernter Anbieter) trägt eine freie URL; **ohne** Proxy- und Redirect-Vorgabe. Ziel in TP3: transport (lokal über `Ziel::eingebaut`, `Endpoint` aus der URL) |
| B12 | `managers/meetings/search/embed.rs:174` (`POST` `:172`, Client `:150`) | Einbettungen der Besprechungstexte | **eigener Transport** an den eingebauten Embedding-Server (`llm::ensure_embedding`) | immer Loopback; heute weder `check_call` noch `check_budget`. Ziel in TP3: transport als `Ziel::eingebaut` (lokal, keine Besprechungsprüfung) |
| B13 | `managers/llm/mod.rs:423–430` (`tokenize_count`, `POST /tokenize`) | Token messen für die Chunk-Budgets; Aufrufer `notes/enhance.rs:347` (Besprechungstext), `notes/eval.rs:389` (Auswertung), `book_ai/tokens.rs:206` (Buch) | **eigener Transport** an den lokalen Server (`local_server_root` → `ensure_local`) | immer Loopback, Text geht aber hinaus. Ziel in TP3: transport als `Ziel::eingebaut` (lokal; Netzoperation, so verlangt Codex Runde 5) |

Indirekte Aufrufer (Workflows `managers/workflows/app_services.rs:97`, `:116`, `:135`, Tauri-Commands,
MCP) erben die Einstufung der direkten Stelle; sie rufen keinen Transport selbst.

## C. Besprechungsinhalt **möglich** (Herkunft heute nicht belegt)

| # | Stelle | Weg | Zentrale Funktion | Begründung |
|---|---|---|---|---|
| C1 | `projekte/zustand.rs:354` | Projektassistent (`RueckrufModell`) | `send_chat_completion_checked` | Besprechungen lassen sich einem Projekt zuordnen (`projekte/dienst.rs:784`, `projekte/befehle.rs:302`) |
| C2 | `voice_call/turn.rs:246` | Sprachgespräch (Strom) | `send_chat_completion_stream` | App-Kontext kennt den Bereich `meetings` (`app_context/berechtigung.rs:313`, `:449`). Heute schon: Schutz `geschuetzt` (`voice_call/mod.rs:59–65`, `:577`; `turn.rs:74–78`) |
| C3 | `voice_call/mod.rs:706` | Sprachgespräch (Strom, gepinnt) | `send_chat_completion_stream_routed` | wie C2; Empfänger nur das eingebaute Modell (`receiver_is_local` `:399`, `pinned_local`) |
| C4 | `voice_call/projekt.rs:210` | Sprachgespräch mit Projekt | `send_chat_completion_stream_routed` | wie C1 und C2 |

Alle vier laufen in TP3 über die Fassade und in #601 mit `Quellen::unbestimmt()`, solange keine
Herkunft ermittelt ist (Cloud-Ziele sperren dann). C2 und C3 haben bereits einen eigenen
Lokalschutz; der wird in #601 durch den Versandvertrag ersetzt, nicht daneben gelassen.

## D. Ohne Besprechungsinhalt (begründet)

Diese Stellen laufen ebenfalls über die Fassade, brauchen aber kein `Gesperrt`. Begründung je Zeile;
#601 sichert sie per Architekturtest als „aufruferbezogene `keine()`-Ausnahme“ ab.

| # | Stelle | Weg | Transportart | Begründung |
|---|---|---|---|---|
| D1 | `actions.rs:406`, `:465` | Diktat-Nachbearbeitung (`Purpose::PostProcess`) | `send_chat_completion_with_schema` / `send_chat_completion` | Diktattext des Nutzers; kein Besprechungsbezug (Diktat ist während einer Besprechungsaufnahme sogar gesperrt, `actions.rs:675–683`) |
| D2 | `books.rs:564` | Buchzusammenfassung | `send_chat_completion_with_schema` | Buchtext |
| D3 | `book_ai/talk/mod.rs:709` | Buch-Gespräch (Strom, mit CLI-Vorschau `:724–727`) | `send_chat_completion_stream_limited` | Buchtext |
| D4 | `sync/booktalk_relay.rs:1992` | Buch-Gespräch von einem Gerät | `send_chat_completion_stream` (mit `CLI_PREVIEW`) | Buchtext |
| D5 | `book_ai/talk/adapter.rs:117` (`POST`, `send` `:121`, Client `:63`) | Buch-Gespräch, eigener HTTP-Adapter (`HttpModel`) | **eigener Transport**, Umgebungsproxy, folgt Redirects | Buchtext; Ziel in TP3: transport (damit der Zaun vollständig ist) oder ausdrücklich ausgeklammert, Entscheidung in #601 |
| D6 | `book_ai/talk/adapter.rs:189` | Buch-Gespräch über die CLI (`CliModel`) | `cli::call_with_options` direkt | **CLI**, kein natives Ollama (Korrektur Codex Runde 3); Buchtext; Ziel in TP3: transport (CLI-Einstieg wandert) |
| D7 | `book_ai/talk/count.rs:52`, `:67` (Client `:37`) | Token zählen am Server | **eigener Transport** (`/props`, `/apply-template`, `/tokenize`) | Buchtext; nur eingebauter Server |
| D8 | `translator.rs:95`, `:119`, `:153` | Vorlese-Übersetzung (nativ Ollama / HTTP / Entladen) | `send_ollama_native`, `send_chat_completion_checked`, `ollama_unload` | Vorlesetext (Bücher, Verlauf); `translator.rs` hat keinen Besprechungsbezug (0 Treffer) |
| D9 | `tagging.rs:502`, `:524`, `:684` | Vorlese-Auto-Tagging | `send_ollama_native`, `send_chat_completion_with_schema`, `ollama_unload` | Vorlesetext; kein Besprechungsbezug (0 Treffer) |
| D10 | `refinement/ollama.rs:152`, `:192` (Client `:79`, `no_proxy` `:80`) | Diktat-Verfeinerung | **eigener Transport** (`/api/tags`, `/api/generate`) | Diktattext; Standard `127.0.0.1:11434`, ohne Proxy; **Ziel in TP3 über `transport`**, weil eigener Transport |
| D11 | `managers/llm/updates.rs:462` (`probe_claude`) | Probeaufruf der Claude-CLI | `cli::call_with_timeout` `:1270` | Inhalt ist die feste Zeichenfolge `ok?` |
| D12 | `agent/eval.rs:282` (`POST` `:284`, Clients `:589`, `:697`, `:749`) | Auswertungswerkzeug des Agenten | **eigener Transport** | synthetischer Datensatz, nur eigener Lauf gegen einen lokalen Server; nicht in der App erreichbar |

## E. Bild, Verwaltung, Sprachausgabe

| # | Stelle | Weg | Transportart | Besprechungsinhalt | Ziel in TP3 |
|---|---|---|---|---|---|
| E1 | `managers/meetings/slides/vision.rs:185` (Testhelfer `:997`) | Folien-Auswertung per Bild | `llm_client::post_image_prompt` (A7) | **ja** (Folien einer Besprechung); Server immer `ensure_local_vision` (Loopback) | transport (`Ziel::eingebaut`, lokal) |
| E2 | `commands/tts.rs:329` (`llm_ps`, `GET` `:336`), `:395` (Aufwärmen, `POST` `:403`), `:368` (Entladen) | Ollama-Verwaltung | `reqwest::Client::new()`; Adresse über `ollama_native_url` | nein: leere oder verwaltende Anfragen | ohne Besprechungsinhalt (Hinweis: dieselbe schwache Adressprüfung, siehe Befunde) |
| E3 | `voice_call/mod.rs:388`, `:493–496` (`warm_speaker`) | Sprecher aufwärmen | `reqwest::Client::new()` | nein: Anfrage ohne Text | ohne Besprechungsinhalt |
| E4 | `shortcut/mod.rs:1672`, `commands/llm.rs:165` | Modellliste (Aufrufer von A8) | `fetch_models` | nein: Verwaltung | transport (A8) |
| E5 | `managers/llm/server.rs:513` (`/health`), `:834`; `managers/llm/mod.rs:308` (`/slots`); `managers/llm/estimate.rs:212` (HTTP-Range auf den Modell-Hub) | Zustand des lokalen Servers; Größenabfrage am Modell-Hub | `reqwest::Client::builder()` | nein: kein Inhalt | ohne Besprechungsinhalt |
| E6 | `managers/llm/cli.rs:552`, `:1189`, `:2107` | CLI-Statusabfrage, `--help`-Probe | `Command::new` | nein: kein Inhalt | ohne Besprechungsinhalt (kein Modellaufruf) |
| E7 | `managers/tts/providers/openai_speech.rs:812`, `:881`; `managers/tts/gradium.rs:395`; lokale Sprachserver in `managers/tts/mod.rs:631`, `:3990`, `:4264` | Sprachausgabe (OpenAI-kompatibel, Gradium, lokaler Sprachserver) | HTTP | **möglich**: Vorlesetext hat im TTS-Modul keinen Besprechungsbezug (0 Treffer), aber die Antwort eines Sprachgesprächs (C2–C4) kann Besprechungswissen tragen und wird ausgesprochen; die Anbieterschicht hat eigene Freigaben (`allow_remote`, `providers/policy.rs`) | **Entscheidung für #601**: gleichwertig einbeziehen oder ausdrücklich ausklammern |
| E8 | `vendor/wai-providers/src/openai.rs:206` (`chat`), `:228` (`chat_stream`) | eigener Chat-Transport im vendorten Crate | `reqwest` | nicht benutzt: der App-Code nutzt nur `wai_providers::util` (`projekte/dokument.rs:47`, `projekte/recherche.rs:59`) | im Architekturtest festhalten; keine Verdrahtung |

## F. Befunde

1. **Natives Ollama ist nicht lokal erzwungen** (`llm_client.rs:895–899`). `ollama_native_url`
   (`llm_client.rs:828–835`) prüft nur `base.contains("localhost") || base.contains("127.0.0.1")`;
   `https://api.example.com/localhost` besteht die Prüfung. Das `Ziel` leitet dagegen aus dem
   geparsten Host ab (`is_loopback_url`).
2. **Mehrere Wege ohne Proxy- und Redirect-Vorgabe**: A6/A9 (`Client::new()`), A7
   (`post_image_prompt`), B11 (`agent/runtime.rs:433`), D5 (`book_ai/talk/adapter.rs:63`). Die
   erlaubten Verhaltensänderungen (kein Redirect; lokal ohne Proxy) greifen erst, wenn sie über
   `transport` laufen.
3. **A7 prüft weder Compliance noch Budget** (`post_image_prompt` hat keinen Aufruf von
   `check_call`/`check_budget`). Ziel ist heute immer Loopback; mit `transport` kommt die
   Prüfung je Versuch.
4. **Drei verschiedene Lokal-Erkennungen** existieren: `managers/llm/mod.rs:66` (`is_local`: Anbieter-ID
   oder Platzhalteradresse, **sieht nicht auf die tatsächliche Adresse**; wird für das `Ziel`
   bewusst nicht benutzt), `voice_call/mod.rs:399` (`receiver_is_local`, ruft `is_local`) und
   `managers/compliance/mod.rs:81` (`compliance::is_local`, in #505 verwendet). #601 führt sie auf
   `Ziel` zusammen.
5. **CLI-Pfade**: `book_ai/talk/adapter.rs:189` ist **CLI**, kein natives Ollama.
   `managers/llm/updates.rs:462` ist ein Probeaufruf.
6. **`Url`-Fallen**: `cli://localhost` wird von der `url`-Kiste als Hostname `localhost` gelesen;
   das `Ziel` erlaubt `Lokal` darum nur für `http`/`https`.

## G. Aufruferliste der Übergangsfassade (Stand Teilpaket 3)

Maßgeblich ist die Konstante `llm_versand::uebergang::AUFRUFER` (Datei, Inventarnummer,
Einstufung); der Test `aufruferliste_ist_vollstaendig` hält sie mit dem Quelltext zusammen (jede
Datei außerhalb von `llm_versand`, die `llm_versand::uebergang` benutzt, steht dort, und jede
Zeile dort benutzt sie).

**Mit Besprechungsinhalt (ja)**: B1 (`managers/meetings/llm_call.rs`, damit B2–B7), B8
(`followup.rs`), B9 (`chat/controller.rs`), B10 (`summarizer.rs`), B11 (`agent/runtime.rs`), B12
(`managers/meetings/search/embed.rs`), B13 (`managers/llm/mod.rs`, `tokenize_count`), E1
(`slides/vision.rs`).
**Möglich**: C1–C4 (`projekte/zustand.rs`, `voice_call/{turn,mod,projekt}.rs`), E7 (Entscheidung offen).
**Ohne Besprechungsinhalt**: D1–D4, D6, D8, D9, D11, E2, E4 über die Fassade; D5, D7, D10, D12,
E3, E5, E6, E8 bleiben außerhalb (siehe I.2).
Die Fassade deckt B, C und D; A8/A9/E4 laufen über `transport`.

## H. Überschneidungen mit offenen PRs (geprüft am 14.11.2026)

Quelle: `gh api repos/MrP42/local-voice-ai/pulls/<n>/files` (die Dateiliste entspricht dem Dreipunktvergleich
gegen den jeweiligen Merge-Base). TP1 berührt nur `src/lib.rs` (1 Zeile), `src/llm_versand/**` (neu) und
dieses Dokument. TP2 berührt `src/llm_versand/pruefer.rs`, `pruefer/` und `pruefer_tests.rs` (alle neu bzw. eigen)
und in `mcp/mod.rs` nur `read_ki_sperre`: Die Funktion ist in `read_ki_sperre_lesbar` (unlesbar bleibt als
`None` erkennbar) und einen Aufrufer mit der alten Rückfallwirkung aufgeteilt, das Verhalten ist gleich;
`ki_freigabe.rs` bleibt unverändert. TP3 berührt voraussichtlich `llm_client.rs`, `managers/llm/cli.rs`,
`managers/llm/mod.rs`, `managers/llm/updates.rs`, `managers/meetings/llm_call.rs`,
`managers/meetings/chat/controller.rs`, `managers/meetings/followup.rs`, `summarizer.rs`,
`translator.rs`, `tagging.rs`, `voice_call/*`, `projekte/zustand.rs`, `book_ai/talk/*`,
`sync/booktalk_relay.rs`, `actions.rs`, `books.rs`, `managers/meetings/slides/vision.rs`,
`agent/runtime.rs`, `refinement/ollama.rs`, `commands/llm.rs`, `commands/tts.rs`,
`shortcut/mod.rs`.

| PR | Stand | Berührte Dateien (Auszug) | Überschneidung mit TP1 | mit TP2/TP3 | Wirkung auf #601 |
|---|---|---|---|---|---|
| #430 (Notizbuch als zweite Notizquelle) | offen, `dirty`, Basis `19c6771c` (älter) | `managers/meetings/chat/controller.rs` (+152/−9), `followup.rs` (+16/−4), `notes/enhance.rs` (+131/−11), `workflows/app_actions.rs` (+9/−1), `mcp/tools.rs` (+14), `store.rs` (+3), `lib.rs` (+4 bei `initialize_core_logic`, weit von den `mod`-Zeilen), neu `notizbuch/schutz.rs` | keine (nur `lib.rs`, andere Stelle) | **Konfliktrisiko in TP3**: `chat/controller.rs`, `followup.rs`, `notes/enhance.rs` (dort stellt TP3 die Importe der Fassade um) | Notizbuch-Lokalschutz (`notizbuch/schutz.rs`, `fuer_modell(notes, lokal)`) nutzt `managers::llm::is_local`; #601 ersetzt das durch `Ziel`/`Quellen` (Herkunft `nb:`-Blöcke). **Bestehender Notizbuch-Lokalschutz bleibt Pflicht**, bis die Herkunft über `Quellen` trägt |
| #504 (Hinweis an Teilnehmende) | Entwurf, `dirty`, Basis `024204d0` | `managers/compliance/mod.rs` (+1), `compliance/notice.rs` (neu), `meetings/recorder.rs`, `meetings/export.rs`, `settings.rs` (+24), `workflows/hub.rs`, `lib.rs` (+12 in der Befehlsliste) | keine | gering: `compliance/mod.rs` (eine Zeile), `settings.rs` | keine inhaltliche (kein Modellaufruf) |
| #505 (Vorbereitung vor dem Termin) | Entwurf, Basis `024204d0` | `commands/meeting_prep.rs` (neu), `managers/people/prep.rs` (neu), `people/brief.rs`, `calendar/service.rs`, `settings.rs` (+30), `lib.rs` (+3 in der Befehlsliste) | keine | gering | **zusätzlicher Besprechungsversand**: `commands/meeting_prep.rs:112` ruft `send_chat_completion_checked` mit Besprechungsnotizen (`compliance::is_local` `:73`). Muss mit #601 in die Aufruferliste (Klasse B) und auf `versenden(..)` umgestellt werden, sobald gemergt |
| #600 (Aussprache-Regeln) | offen, sauber, Basis `4ba8f873` (gleich) | `voice_call/mod.rs` (+333), `managers/tts/{mod,protocol,pronunciation,abbreviations,aussprache}.rs`, Frontend | keine | **mittel in TP3**: `voice_call/mod.rs` (dort liegt `:706`); die geänderten Hunks betreffen die Sprachausgabe (`voice_call_synthesize`), nicht den Chat-Aufruf | `managers/tts/mod.rs` berührt E7 (Sprachausgabe); keine Änderung am Modellversand |

Folgerung: **Teilpaket 1 ist mit allen vier PRs konfliktfrei.** Für TP3 vorher erneut prüfen
(`chat/controller.rs`, `followup.rs`, `notes/enhance.rs` mit #430; `voice_call/mod.rs` mit #600) und
die Merge-Reihenfolge mit dem Planer klären. #505 muss nach seinem Merge in dieses Inventar.

## I. Stand nach Teilpaket 3

### I.1 Wohin die Wege gewandert sind

Alles unter `src/llm_versand/`; `transport/` ist privat (`pub(super)`, keine Reexporte).

| # | Neuer Ort | Anmerkung |
|---|---|---|
| A1 | `transport/http.rs` `chat_einmal`; Ablauf in `versand.rs` `chat` | ein Versuch an das gebundene `Ziel`, Route wird nicht neu aufgelöst |
| A2 | `transport/http.rs` `strom`; Puffer in `versand.rs` (`Versuchsausgabe`) | bei Cloud mit Quellen ungleich `keine()` alle Deltas und Denktexte erst nach R3 |
| A3 | `transport/http.rs` `ohne_strom`; neuer Versuch in `versand.rs` `strom` | eigene Prüfung, eigene Compliance/Budget, nach `Gesperrt` kein Rückfall |
| A4 | `transport/cli_chat.rs` `einmal`; Neuversuch und Update in `versand.rs` `cli_lauf` | höchstens ein Update je Aufruf, jeder Neuversuch neu geprüft |
| A5 | `transport/cli.rs` (samt Vorschauprüfung und Tests) | aus `managers/llm/cli.rs` verschoben; dort bleiben Argumentaufbau, Parser, Klassifikation |
| A6 | `transport/ollama.rs` `nativ` | Zielart aus dem Host, kein `contains("localhost")` mehr; kein Redirect; lokal ohne Proxy |
| A7 | `transport/bild.rs` `senden` | weiterhin ohne Compliance/Budget (nur eingebauter Bildserver, kein Verbrauch) |
| A8 | `transport/modelle.rs` `liste` | |
| A9 | `transport/ollama.rs` `entladen` | |
| B11 | `transport/json.rs` `post` über `uebergang::json_post` | Compliance prüft der Aufrufer wie bisher; Überlauf als Marke statt Fehlertext |
| B12 | `transport/json.rs` `post` | Loopback; weiterhin weder Compliance noch Budget (kein Modellaufruf) |
| B13 | `transport/json.rs` `post` | Loopback |

`llm_client.rs` enthält nur noch Typen, SSE-Dekoder, Aufruf-Kontexte der CLI (`CLI_*`,
`DENK_SENKE`) und reine Helfer, **keinen** Netzzugriff. `versenden` ist der einzige Einstieg mit
Vertrag; die Übergangsfassade (`uebergang`, G) gibt den Aufrufern bis #601 ihre alten
Signaturen und ist **kein** Schutz.

### I.2 Wege mit eigenem Transport, die nicht umgezogen sind

Alle ohne Besprechungsinhalt; #601 sichert sie per Architekturtest.

| # | Stelle | Grund, warum sie bleibt |
|---|---|---|
| D5 | `book_ai/talk/adapter.rs` (`HttpModel`) | Buchtext, kein Besprechungsinhalt. Der Weg folgt weiterhin Redirects und dem Umgebungsproxy. **Offen**: in #601 entscheiden, ob er für einen vollständigen Zaun umzieht |
| D7 | `book_ai/talk/count.rs` | Buchtext; nur eingebauter Server (`/props`, `/apply-template`, `/tokenize`) |
| D10 | `refinement/ollama.rs` | Diktattext; eigener Client mit `no_proxy`; kein Besprechungsbezug. **Offen**: kein Redirect-Verbot dort |
| D12 | `agent/eval.rs` | Auswertungswerkzeug mit synthetischen Daten, nur gegen einen lokalen Server; in der App nicht erreichbar |
| E2 | `commands/tts.rs` (`llm_ps`, Aufwärmen) | Verwaltung von Ollama, leere Anfragen; dieselbe schwache Adressprüfung wie früher A6 (`contains`) |
| E3 | `voice_call/mod.rs` (`warm_speaker`) | Anfrage ohne Text |
| E5 | `managers/llm/server.rs`, `mod.rs` (`/slots`), `estimate.rs` | Zustand des lokalen Servers, Größenabfrage am Modell-Hub; kein Inhalt |
| E6 | `managers/llm/cli.rs` (Statusabfrage, `--help`) | kein Modellaufruf |
| E7 | `managers/tts/**` | Sprachausgabe; Entscheidung für #601 (gleichwertig einbeziehen oder ausklammern) |
| E8 | `vendor/wai-providers/src/openai.rs` | wird vom App-Code nicht benutzt |

### I.3 Erlaubte Verhaltensänderungen (konkret)

1. **Kein Redirect** für jedes Ziel im Rohtransport (früher folgten Cloud-Wege und natives Ollama,
   Bild, `ollama_unload`, Agent, Einbettungen und Tokenzählung Weiterleitungen).
2. **Lokale Route ohne Proxy**, auch ohne den aus der Umgebung (Loopback-Ziele, auch im Agenten, bei
   Einbettungen und Tokenzählung); Cloud-Ziele wie bisher mit dem Umgebungsproxy.
3. **Compliance und Budget bei jedem Versuch**, auch bei lokalen Zielen, den Rückfällen (Strom ohne
   Strom, CLI nach dem Update) und beim nativen Ollama.
4. **Kein Anbieter- und kein CLI-Text** in Fehlern und im Log. Die Aufrufer bekommen weiter ihre
   Textform (`VersandFehler::alte_textform`: `API request failed with status N`, `HTTP request
   failed`, `cli_*:`-Präfixe, Kontextüberlauf-Marken, `Zeitlimit`, `server_crashed:`), aber ohne
   den Text des Anbieters. Bild-Fehler tragen keinen Antworttext mehr; Verbindungsfehler des
   Agenten und der Einbettungen tragen nur noch eine feste Meldung bzw. `embedding_http: <Code>`.
5. **Agent**: ein Kontextüberlauf wird an denselben Marken erkannt wie im Chat (etwas mehr als die
   zwei früheren: etwa `context length`, `too many tokens`).
6. **Natives Ollama und `ollama_unload`**: Zielart aus dem geparsten Host statt
   `contains("localhost")`.
7. **`Ziel`-Wächter**: eine Anfrage, deren Anbieter-ID nicht die des Ziels ist, ist `Gesperrt`
   (`ziel_gewechselt`).

### I.4 Bewusst unverändert

- `call_cli` fällt wie bisher durch, wenn ein CLI-Anbieter ohne bekannte CLI konfiguriert ist
  (latente Eigenheit, nicht Teil dieses Pakets).
- Der Bildweg bleibt ohne Compliance und Budget (Befund F.3): er geht nur an den eingebauten
  Bildserver auf Loopback und verbraucht nichts.
- Roh-Fehlertexte der CLI in `cli_roh` (Buch-Gespräch, Probelauf) bleiben, wie sie waren; dort geht
  kein Besprechungsinhalt hinein.

## J. Compile-Zeit-Zusicherungen (Teilpaket 4)

Was der Versandvertrag **verbietet**, ist mit dem Fehlercode des Compilers belegt, nicht nur damit,
dass etwas nicht übersetzt. Mechanismus und Begründung stehen in der Moduldoku von `llm_versand`
(Abschnitt „Compile-Zeit-Zusicherungen“) und im Kopf von `src/llm_versand/zusicherungen.rs`.

**Warum keine `compile_fail`-Doctests:** `mod llm_versand;` ist in `lib.rs` privat, Doctests sind
externe Crates und sähen immer nur „Modul privat“; eine positive Kontrolle ist dort unmöglich. Auf
stabilem Rust (1.97) prüft `rustdoc` den Fehlercode hinter `compile_fail,E…` nicht (gemessen: ein
Snippet mit `E0599` besteht unter `compile_fail,E0603`). `trybuild` wäre eine neue Abhängigkeit und
ebenfalls ein externes Crate. Gewählt ist deshalb: Proben **im** Crate (`src/llm_versand/sonden/`,
nur mit `--cfg lva_sonde`), übersetzt vom Test `zusicherungen::compile_zeit_zusicherungen` per
`cargo rustc --lib --profile check`, ausgewertet über die JSON-Diagnosen. Jede Probe hat einen
Verstoß und daneben denselben Code ohne ihn (positive Kontrolle); jede Probe meldet im Kontrolllauf
eine Marke, damit sie nicht aus Versehen nie übersetzt wurde.

| Probe | Code | Zusicherung |
|---|---|---|
| `aussen_e0603_transport` | E0603 | Rohtransport `llm_versand::transport` von außen (aus `crate::managers::llm`) unerreichbar |
| `aussen_e0603_versand` | E0603 | `llm_versand::versand` privat, nur `versenden` reexportiert |
| `aussen_e0603_freigabe` | E0603 | `Versandfreigabe` von außen nicht benennbar |
| `aussen_e0603_ausstellen` | E0603 | `freigabe_ausstellen` von außen unerreichbar |
| `aussen_e0599_quellen_default` | E0599 | `Quellen::default()` gibt es nicht |
| `innen_e0603_cli` | E0603 | `transport::cli` auch in `llm_versand` privat (nur `cli_roh`) |
| `innen_e0603_pruefer_app` | E0603 | `pruefer::app` privat (nur der Reexport) |
| `innen_e0624_ausstellen` | E0624 | `Versandfreigabe::ausstellen` privat im Prüfer |
| `innen_e0451_literal` | E0451 | Felder der `Versandfreigabe` privat, kein Struktur-Literal |
| `innen_e0599_klon` | E0599 | `Versandfreigabe` hat kein `clone()` |
| `innen_e0599_default` | E0599 | `Versandfreigabe` hat kein `Default` |
| `innen_e0382_zweimal` | E0382 | eingelöste Freigabe ist verbraucht (zweites Einlösen) |
| `innen_e0382_kopie` | E0382 | `Versandfreigabe` ist nicht `Copy` |

Läufe: `kontrolle` (alles ohne Verstoß muss übersetzen), `aufloesung` (E0603), `typ` (E0599, E0624),
`feld` (E0451), `ausleihe` (E0382). Getrennt, weil `rustc` je Phase abbricht (Besitzfehler kommen erst
nach fehlerfreier Typprüfung). Ein Fehler in einer fremden Datei oder mit anderem Code lässt den Lauf
scheitern. Zusätzlich ohne Prozess bei jedem Testbuild: `Versandfreigabe` ohne `Clone`, `Copy`,
`Default`, `Serialize`, `Deserialize` (Tests in `pruefer`), `Quellen` ohne `Default`.

**Betrieb:** Der Probentest dauert beim ersten Lauf gut zwei Minuten, danach je Lauf etwa 15 bis 20
Sekunden. Er überschreibt die Prüfartefakte (`cargo check`) des Crates in `target`; der nächste
`cargo check`/`clippy` übersetzt das Crate einmal neu. `LVA_SONDEN_UEBERSPRINGEN=1` lässt ihn aus (wird
gemeldet). Der Schalter `lva_sonde` hängt in `src/llm_versand/mod.rs` und `src/managers/llm/mod.rs`
(je eine `cfg`-Zeile, dort auch `#![allow(unexpected_cfgs)]`); ein normaler Build übersetzt nichts davon.
