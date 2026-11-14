# S1.11 Teil B: Abgleich im Sprachdienst von Local Voice — Bestandsaufnahme und Entwurf

Stand: Local Voice `origin/main` @`1ae9e916`, SDK `origin/main` @`8d8dd601` (PR MrP42/wai-sdk#1144).
Issue: MrP42/local-voice-ai#599. Alles nur gelesen; nichts gebaut, nichts ausgeführt außer `git`/`gh`.
Zeilenangaben beziehen sich auf diese Stände. „ungeprüft" heißt: aus dem Code gefolgert, nicht ausgeführt.

## 1. Befund

### 1.1 Wo der Sprachdienst lebt

Pfadpräfix `apps/local-voice/src-tauri/src/` (kurz: `src/`).

| Baustein | Fundstelle | Befund |
|---|---|---|
| Modulwurzel | `src/sprachdienst/mod.rs:34-50` | `app, dienst, engine, kontext, kopplung, verbindung, wav, zugang`, dazu `unix` bzw. `pipe` |
| Start/Stopp | `src/sprachdienst/dienst.rs:94-178`, App-Anbindung `src/sprachdienst/app.rs:178-210`, eingehängt `src/lib.rs:489` | Opt-in, Standard aus |
| Protokoll je Verbindung | `src/sprachdienst/verbindung.rs` (1343 Z.) | eine Aufgabe je Verbindung; Steuerung `steuer` Z. 491-558, `start` Z. 853-939, `audio` Z. 941-971, `ende` Z. 973-991, `takt` Z. 1202-1277, Engine-Aufträge `auftrag` Z. 1308-1342 |
| Operation → Recht | `verbindung.rs:147-160` (`Typ::recht`), Prüfung bei jedem `start` Z. 855-865 | Rechte werden je `start` frisch gelesen; Entzug beendet nur Ströme des Rechts (Z. 806-821) |
| Zugänge und Rechte | `src/sprachdienst/zugang.rs:32-48` (`rechte: Vec<Recht>`), `recht_setzen` Z. 215-234 | Datei `sprachdienst.json` neben den Einstellungen; neue Zugänge ohne Rechte (Z. 203) |
| Vergabe in der Oberfläche | `src/commands/sprachdienst.rs:99-105` (`recht_aus`: nur `hoeren`, `sprechen`), Z. 162-183; `src/components/integrations/SprachdienstPanel.tsx:309` | Kopplung nur per Bestätigung im Hauptfenster |
| `willkommen` | `verbindung.rs:597-607` | `rechte: z.rechte.clone()`: genau die gespeicherten Rechte des Zugangs. Gezielte Vergabe ist damit strukturell möglich |
| Verbindungen je Zugang | `src/sprachdienst/kontext.rs:46-47,69` (`je_gegenstelle: 2`), Zähler Z. 168-169, vor dem Handschlag je Prozess `unix.rs:138`, `pipe.rs:103`, danach je `client`-Kennung `verbindung.rs:586-591` | **Zwei** gleichzeitige Verbindungen je `client` und je Prozess sind heute erlaubt; eine dritte endet mit `beschaeftigt`. Insgesamt 8 (`kontext.rs:65`) |
| Engine-Plätze | `kontext.rs:40-43,66-67`; `auftrag` wartet ohne Frist (`verbindung.rs:1325-1330`) | zwei Plätze global, gemeinsam für Erkennen **und** Sprechen; kein Überlast-Code, nur Warten |

### 1.2 Wie heute erkannt wird

- Schnittstelle `Sprache` (`engine.rs:50-87`): `erkennen(pcm, bereich, abbruch) -> Result<String, EngineFehler>`. App-Seite `app.rs:96-133` ruft `TranscriptionManager::transcribe` (`managers/transcription.rs:2115`) und wartet bis zu 90 s auf das Laden des Modells (`app.rs:32,110-124`).
- **Einzelauftrag** (`erkennen`): Ton sammeln, bei `ende` oder Höchstdauer `erkennung_abschliessen` (`verbindung.rs:994-1012`), Antwort `ergebnis {text}` (Z. 1087-1103).
- **Strom** (`erkennen_strom`): **Teilergebnisse** durch wiederholtes Erkennen einer Kopie des bisherigen Tons alle 1,5 s neuen Tons (`kontext.rs:63-64`, `verbindung.rs:1235-1239,1282-1303`), gesendet als `teil {text}` (Z. 1069-1086).
- Bei Höchstdauer wird das bisherige Audio erkannt (`verbindung.rs:965-969`, `takt` Z. 1231-1232). Für den Abgleich verlangt der Vertrag das Gegenteil (`ueberlaenge`, Anfang nicht auswerten).
- Die gemeinsame Engine ist ein einziger Platz: `transcribe` nimmt sie aus dem Mutex (`transcription.rs:2220-2231`) und scheitert sofort, wenn sie gerade woanders ist (dasselbe `take` im Live-Diktat `transcription.rs:1540`). Zwei Erkennungen zugleich warten also nicht, die zweite scheitert (gelesen, nicht ausgeführt).

### 1.3 Nachbearbeitung und Nutzerwörter (Abnahmepunkt 1)

`transcribe` ist für den Abgleich ungeeignet, nicht nur wegen des Protokolls:

| Stelle | Wirkung auf den Abgleich |
|---|---|
| `transcription.rs:2266-2273` | Nutzerwörter als Startprompt (Whisper-Familie); verschiebt die Erkennung zu Nutzerwörtern |
| `transcription.rs:2275-2280,2346-2356` | `translate_to_english` der Einstellungen: aus „ja" wird „yes" → nie Annahme |
| `transcription.rs:2180-2181` | Sprache aus den Einstellungen (auch `auto`), nicht fest Deutsch |
| `transcription.rs:2441`, `3292-3320` | `post_process_transcription_text`: Schleifen zusammenfassen, unscharfe Nutzerwortkorrektur, Füllwörter |
| `audio_toolkit/text.rs:305-342` | `collapse_stutters`: drei gleiche Wörter → eines. **„ja ja ja" würde zu „ja" = Annahme**, laut SDK ist das `Unbekannt` |
| `audio_toolkit/text.rs:284,359-391` | Füllwörter `äh`, `ähm`, `hm`… entfallen: **„ähm ja" würde zu „ja" = Annahme** |

### 1.4 Wo Inhalt heute austreten kann (Abnahmepunkt 2)

| Stelle | Kanal |
|---|---|
| `transcription.rs:2467-2468` | Debug-Bau: `info!("Transcription result: {}", text)` — vollständiger Text |
| `transcription.rs:2469-2473` | Release: Länge in Zeichen. Für den Abgleich ist schon die Länge Inhalt (2 Zeichen ≈ „ja") |
| `lib.rs:2955-2997` | Protokollziele: Konsole (stdout bzw. stderr headless), Datei `handy.log`, **Webview** (Live-Protokoll der Oberfläche, Z. 2992) |
| `transcription.rs:2394-2431` | Panic der Engine: Nutzlast ins Protokoll, in den Fehlertext und als Ereignis `model-state-changed` an die Oberfläche |
| kein eigener Panic-Hook (`git grep set_hook`) | Standard-Hook schreibt jede Panicmeldung nach stderr, **bevor** `catch_unwind` greift. Ein Schnitt in einen String (`&s[..n]`) panict mit dem Text in der Meldung |
| `transcription.rs:2300-2375` | Fehlertexte der Engines werden mit `{e}` in `anyhow` verpackt; `app.rs:129-132` verwirft sie (gut) |
| `verbindung.rs:433-445` (`strom_fehler`), `450-461` (`strom_abweisen`) | freie `nachricht` und am zentralen `senden` (Z. 361) vorbei; auf Abgleichströmen nicht erlaubt (SDK: leere `nachricht`) |
| `verbindung.rs:859-875` | feste deutsche Texte bei `recht_fehlt`, `nicht_lokal`, `engine_fehlt` — auf Abgleichströmen ebenfalls unzulässig |
| nativer Code (transcribe-cpp/whisper, ONNX) | ob er Text auf stderr schreibt: **ungeprüft**, nur mit echter Engine messbar |

Die Rohaudio-Haltung ist sauber: `pcm` nur im Strom, nach dem Erkennen verworfen (`verbindung.rs:999-1007`).

### 1.5 Vendorte Crate und Skript

- `vendor/wai-voice-dienst/VENDOR.toml`: gepinnt ist **`c51d643e`**, nicht `e624d851` (der Auftragstext ist da veraltet; #589 hat schon gehoben).
- `scripts/vendor-wai-voice-dienst.py:43,51,97-121` (auf `main`): schreibt nur `[dependencies]`, `ERLAUBT` ohne `libc`, `windows-sys`. SDK @`8d8dd601` hat `[features] lokal = ["dep:libc","dep:windows-sys"]`, `nachbau = []` und Zielabhängigkeiten. **#595** (offen, kein Entwurf) löst genau das (Features, Zieltabellen, Paketnamen-Prüfung); `ERLAUBT` um `libc`/`windows-sys` erweitert er bewusst nicht. Beide stehen schon im Lock der App (`libc 0.2.182`, `windows-sys 0.60.2`).
- Diff `c51d643e..8d8dd601` der Crate: +12 488/−79 Zeilen, neu `abgleich.rs`, `leitung{,/unix,/windows}.rs`, `wav.rs`, `nachbau.rs`, `client/{abgleich,anruf,sprechen}.rs`. Für die Dienstseite brechend: neue Varianten `Recht::Abgleich`, `Operation::Abgleich`, `AnDienst::AbgleichEnde`, `AnApp::Abgleich`. Erwartete Übersetzungsfehler an `verbindung.rs:521-557` (`match m`), `879-883` und `886-938` (`match op`); weitere zeigt nur ein Bau (**ungeprüft**). `client::Verbindung<S: Read + Write>` bleibt, Abgleich und `Anrufzugang` verlangen `S: Leitung` (`leitung.rs:47`).
- Abwärtsrisiko: `zugang.rs:120-126` legt eine unlesbare Datei als `.defekt` beiseite. Stünde `"abgleich"` in `rechte` und würde Local Voice auf eine ältere Fassung zurückgesetzt, gingen **alle** Kopplungen verloren.
- Der Lokalitätsschalter `verbindung.rs:866` prüft `engines().lokal`, das aus der **Stimme** kommt (`app.rs:36-49,86`). Ein Abgleich würde mit `nicht_lokal` abgewiesen, sobald die Sprachausgabe entfernt läuft, obwohl die Erkennung lokal ist.

### 1.6 Prüfstand (#591, gemergt)

`apps/local-voice/crates/sprachdienst-pruefstand`: bindet die Originaldateien per `#[path]` ein, führt `sprachdienst/tests.rs` unter Linux/macOS ohne App-Bau aus (`cargo test --locked --manifest-path apps/local-voice/crates/sprachdienst-pruefstand/Cargo.toml`). Nicht eingebunden: `app.rs`, `pipe.rs`. Wächter verlangen: Spiegel `src/sprachdienst/mod.rs` gleich dem Original, Abhängigkeiten zeilengleich mit der App, `Cargo.lock` passend (neu ableiten laut README). Ein neues Dienstmodul braucht also eine Zeile im Spiegel. Vorhanden: Log-Fänger-Test `tests.rs:1694-1746`, Interop mit dem SDK-Client über Unix-Socket `tests.rs:2096ff.`.

### 1.7 Offene PRs und Überschneidungen

| PR | berührt | Überschneidung mit Teil B |
|---|---|---|
| #595 vendor-Features | `scripts/vendor-wai-voice-dienst.py` | **Voraussetzung** für B1 |
| #598 Assistenten/Aussprache | `lib.rs`, `bindings.ts` | B2: Befehlsliste in `lib.rs`, `bindings.ts` neu erzeugt → Konflikt wahrscheinlich, mechanisch |
| #583 G9a-2 MCP-Sperre | `lib.rs`, `settings.rs`, `bindings.ts` | wie oben |
| #565 Assistenten iPhone | `Cargo.toml`, `Cargo.lock`, `lib.rs` | B1 ändert `Cargo.lock` (Features der vendorten Crate) |
| Issue #586 | Sprechen ohne Frist | hält Engine-Plätze (`kontext.rs:66`), die der Abgleich braucht |

Ein offener PR zu „Gesprächsmodus" oder „Nvidia-Ausweichmodell" ist nicht zu finden (Suche per `gh`, offen nur #565/#566/#598 und #543 iOS); `transcription.rs` berührt kein offener PR. `wai-assistent` wird nicht berührt.

## 2. Entwurf

### 2.1 Datenfluss

```
App ── start{abgleich, liste} ──► verbindung: Recht Abgleich? liste == LISTE? erkennung lokal & bereit?
    ◄── guthaben 16 ───────────── Typ::Abgleich { kennung: None, bytes }
    ── Ton ×n ──────────────────► pcm sammeln; > ABGLEICH_MAX_BYTES → abgleich_fehler(Ueberlaenge)
    ── abgleich_ende{kennung} ──► kennung merken, Frist starten, EIN Engine-Auftrag:
                                    spawn_blocking: Sprache::abgleichen(&pcm, bis, abbruch) -> Result<Klasse, AbgleichGrund>
                                      └ app.rs: tm.einordnen(f32, bis, |text| abgleich::klassifizieren(text, Ende::Vollstaendig))
                                         Text lebt nur als &str in diesem Aufruf
                                  Ev::Abgleich { strom, gen, ergebnis: Result<Klasse, AbgleichGrund> }
    ◄── abgleich{klasse, liste, bytes, kennung} oder fehler{code, nachricht: ""}
```

Die Klasse entsteht an genau einer Stelle (SDK-Funktion), aus genau einem Engine-Lauf, der erst nach `abgleich_ende` beginnt. Über die Ereigniswarteschlange läuft nur `Klasse` (Debug `Klasse(..)`) bzw. ein fester Grund.

### 2.2 Schnittstellen (Local Voice)

`src/sprachdienst/engine.rs` — neue Methode mit Vorgabe, damit bestehende Engines und die Attrappe weiter übersetzen:

```rust
/// Ordnet eine ganze Äußerung einer Klasse zu. Kein Text verlässt die Methode.
/// `bis`: Frist des Dienstes; danach kein Warten auf Modell oder Engine.
fn abgleichen(&self, pcm: &[u8], bis: Instant, abbruch: &Arc<AtomicBool>)
    -> Result<Klasse, AbgleichGrund> { Err(AbgleichGrund::EngineFehlt) }
/// Läuft die Erkennung lokal? (getrennt von `engines().lokal`, das die Stimme meint)
fn erkennung_lokal(&self) -> bool { self.engines().lokal }
```

`src/sprachdienst/abgleich.rs` (neu, im Prüfstand testbar): Zustand eines Abgleichstroms, Abbildung Engine-/Dienstfehler → `AbgleichGrund`, Bau der Antwort mit `AnApp::abgleich`/`AnApp::abgleich_fehler`. Keine Funktion nimmt oder liefert `String`.

`src/sprachdienst/verbindung.rs`:
- `Typ::Abgleich { kennung: Option<Kennung>, bytes: u64, frist: Option<Instant> }`, `Typ::recht` → `Recht::Abgleich`; `Ev::Abgleich { strom, gen, ergebnis }`.
- `start`: Recht → Liste (`liste != abgleich::LISTE` → `ListeUnbekannt`) → `erkennung_lokal()` → `bereit()`. Jede Abweisung über `abgleich_fehler`, nie über die Texte von Z. 859-875. Log nur `op=abgleich`.
- `audio`: für `Typ::Abgleich` bei Überschreiten `ABGLEICH_MAX_BYTES` (mehr als 320 000 Bytes) → `Ueberlaenge`, kein `erkennung_abschliessen`; nach `abgleich_ende` weiterer Ton → verworfen wie bei beendeten Strömen.
- neuer Zweig `AnDienst::AbgleichEnde` → `abgleich_abschliessen`; ein `ende` auf einem Abgleichstrom weist schon die `Sitzung` ab.
- `takt`: Guthaben-Nachschub auch für `Typ::Abgleich`; kein `teil_starten`; Leerlauf 15 s → `ZeitUeberschritten`; neue Fristen `Konfig::abgleich_antwort` (Vorschlag 5 s ab `abgleich_ende`, wie die Vorgabe des Clients) und `abgleich_gesamt` (30 s ab `start`) → `ZeitUeberschritten`, Strom schließen, spätes Ergebnis fällt am `gen`-Vergleich (Z. 1088) weg.
- **Ein Ausgangstor:** `senden`, `strom_fehler` und `strom_abweisen` fragen vor dem Einreihen `self.sitzung.ausgang_erlaubt(&m)`; ein abgewiesener Rahmen geht nicht hinaus und beendet die Verbindung (`fatal ungueltig` auf Strom 0, Log ohne Inhalt: `strom=… art=…`). `strom_fehler`/`strom_abweisen` wählen bei `sitzung.ist_abgleich(strom)` `abgleich_fehler` (leere `nachricht`), auch für gemerkte beendete Abgleichströme.
- Engine-Platz für den Abgleich: `auftrag` bekommt eine Variante mit Frist (`timeout_at(bis, acquire_owned())`); ohne Platz bis zur Frist → `Ueberlastet`. Ob der Abgleich einen eigenen Platz neben `engine_global` bekommt (Sprechen hält Plätze ohne Frist, #586), ist Frage F3.

`managers/transcription.rs` — neuer, getrennter Weg (nicht `transcribe` umbauen; Muster wie `transcribe_segments`, Z. 2483-2491):

```rust
pub enum EinordnenFehler { NichtGeladen, Belegt, Fehlgeschlagen, Ausgefallen, Sprache }
/// Vertraulicher Lauf: Sprache fest "de", keine Übersetzung, kein Startprompt, keine Nachbearbeitung,
/// keine Ereignisse, kein Protokoll von Text, Länge oder Panicnutzlast. `f` sieht den Rohtext nur geliehen.
pub fn einordnen<K>(&self, audio: Vec<f32>, bis: Instant, f: impl FnOnce(&str) -> K)
    -> Result<K, EinordnenFehler>
```

Wartet auf die Engine bis `bis` (statt sofort zu scheitern); `Ausgefallen` nach Panic setzt das Modell zurück wie heute, aber das Ereignis trägt einen festen Text. Modell ohne Deutsch → `Sprache` → `EngineFehlt`.

`app.rs`: `AppSprache::abgleichen` = `pcm_zu_f32` + `einordnen(…, |t| abgleich::klassifizieren(t, Ende::Vollstaendig))`, Fehlerabbildung auf `AbgleichGrund`; `erkennung_lokal() = true` (alle Erkennungs-Engines laufen im Prozess; **ungeprüft**, ob ein entfernter STT-Anbieter existiert). Vorwärmen: Anmeldung eines Zugangs mit Recht `abgleich` stößt `initiate_model_load` an (Frage F4).

### 2.3 Panic-Hook (Abnahmepunkt 2)

ADR-0032 (Auflage 2) schließt eine threadlokale Sperre als Mechanismus aus. Vorschlag: beim Start ein **prozessweiter** Hook, der Ort und Thread schreibt, aber nie die Nutzlast, in allen Bauten. Folge: Auch Panics außerhalb des Abgleichs verlieren ihre Meldung im Protokoll (Ort bleibt). Dazu `einordnen` ohne Schnittoperationen auf Text. Alternative: Abgleich in einem Kindprozess mit verworfenem stdio (stärkste Trennung, aber Modell doppelt im Speicher, mehr Latenz). Entscheidung: Frage F1.

### 2.4 Rechte und Anrufzugang (Abnahmepunkt 5)

- `zugang.rs`: neues Feld `#[serde(default, skip_serializing_if = "…")] anruf: bool` statt `"abgleich"` in `rechte`. `hat_recht(Abgleich)` ⇔ `aktiv && anruf`. Ältere Fassungen ignorieren das Feld (keine `deny_unknown_fields` an `Zugang`/`Datei`, Z. 32, 70), beim Zurücksetzen fällt das Recht still weg statt aller Kopplungen. Beim Laden wird ein `Recht::Abgleich` in `rechte` entfernt.
- Invariante im Speicher: `anruf ⇒ ¬hoeren ∧ ¬lernen`. `anruf_setzen(zugang, true)` entfernt `hoeren` und meldet `recht_entzogen(Hoeren)`; `recht_setzen(hoeren, true)` auf einem Anrufzugang → `Fehler::Ungueltig("anrufzugang")`.
- Zusätzlich im Dienst, unabhängig von der Datei: `start` mit `Erkennen`/`ErkennenStrom` für einen Zugang mit `anruf` → `recht_fehlt`.
- `willkommen.rechte` = gespeicherte Rechte + `abgleich` nur bei `anruf`. Für jeden anderen Zugang bleibt das JSON byte-gleich (Goldtest).
- Befehl `sprachdienst_anruf_setzen(zugang, an)` (nur Hauptfenster), `SprachZugang.anruf: bool`, Schalter „Zugang für Anrufe (nur Annahme/Stopp, kein Text)" in `SprachdienstPanel.tsx` neben Z. 309; bei `anruf` ist „Hören" gesperrt. Abschalten → `recht_entzogen(Abgleich)`.
- Zwei Verbindungen: Die Grenze 2 je `client`/Prozess reicht für Sprechen + Abgleich. Ein schneller Neuaufbau, während die alte Verbindung noch zählt, ergibt `beschaeftigt` (Frage F5).

### 2.5 Punkt 9: Kennung

| Grenze | Ort der Lösung | Begründung |
|---|---|---|
| Vergleich nicht konstantzeitig (`#[derive(PartialEq)]` an `msg::Kennung`, Vergleich in `Sitzung::ausgang_erlaubt`) | **SDK-Nachtrag**: `PartialEq` für `Kennung` als konstantzeitiger Vergleich (XOR-Faltung, ohne neue Abhängigkeit) | Local Voice vergleicht Kennungen nicht selbst, es gibt die empfangene zurück. Im Dienst vergleicht nur die `Sitzung` ihren eigenen Wert mit dem, den der Dienst einsetzt; ausnutzbar ist das dort nicht. Für den Client gehört es ins SDK |
| `ausgang_erlaubt` verbraucht die Kennung nicht | **SDK-Nachtrag**: `Sitzung::ausgang_verbuchen(&mut self, &AnApp)`; nach `abgleich` oder `fehler` steht der Strom auf „beantwortet", ein zweiter Abschluss → `ungueltig` | Die Einmaligkeit ist Vertragsregel und gehört in den Zustandsautomaten |
| bis dahin | **Local Voice**: Einmaligkeit strukturell (Strom vor dem Senden aus `stroeme` entfernt, `gen`-Vergleich), Test „zweites Ergebnis desselben Stroms geht nicht hinaus"; nach dem SDK-Nachtrag ruft das Ausgangstor `ausgang_verbuchen` | |

## 3. Testplan

### 3.1 In dieser Umgebung (Linux, ohne Modell, Prüfstand)

Befehl: `cargo test --locked --manifest-path apps/local-voice/crates/sprachdienst-pruefstand/Cargo.toml` (+ Wächter einzeln benannt). Mit erweiterter Attrappe (`impl Sprache for Attrappe`, `tests.rs:92`):

- Klasse nur nach `abgleich_ende`; Engine genau einmal und erst danach gerufen (Zähler in der Attrappe); weder `teil` noch `ergebnis` noch `fertig` auf dem Strom, auch nach dem Ende.
- `bytes` = empfangene Bytes, `kennung` = die aus `abgleich_ende`; `ende` statt `abgleich_ende` → Verbindungsende.
- Überlänge (320 002 Bytes) → `ueberlaenge`, Engine nicht gerufen; genau 320 000 → Klasse.
- Frist: Attrappe hängt → `zeit_ueberschritten`, spätes `Annahme` der Attrappe geht nicht hinaus. Überlast: Plätze belegt → `ueberlastet`. Abbruch, Rechteentzug, Widerruf → nie `abgleich`.
- `liste_unbekannt`; `nicht_lokal` hängt an `erkennung_lokal`, nicht an der Stimme.
- Alle Fehler auf Abgleichströmen mit leerer `nachricht`; `strom_abweisen` auf gemerktem Abgleichstrom ebenso.
- Rechte: Anrufzugang bekommt `willkommen` mit `abgleich`, `erkennen`/`erkennen_strom` → `recht_fehlt`; anderer Zugang: `willkommen` byte-gleich zum Stand vor der Änderung; Datei mit `anruf` von einer Fassung ohne das Feld lesbar (Serde-Test).
- Interop über Unix-Socket mit `client::Anrufzugang` des SDK (eigene kleine `Leitung` für `UnixStream` im Test, `set_read_timeout`), zwei Anrufzugänge parallel (Sprechen + Abgleich), Abbruch eines Abgleichs während die Attrappe rechnet.
- Log-Fänger (`testlog`) mit Kanarienwort in der Attrappe: kein Text, keine Klasse, keine Kennung im Protokoll. Das belegt nur die Dienstschicht, nicht Punkt 2.
- Reine Funktionen in `transcription.rs` (Laufplan mit fester Sprache, keine Nachbearbeitung) übersetzen nur im App-Crate, also **nicht** hier.

### 3.2 Nur auf dem Windows-PC (echte Engine)

- Rust-Tor der App: `cargo check --locked --tests` und `cargo test --locked` für die neuen Tests in `transcription.rs`/`app.rs`.
- **Prüflauf** (neu): kleines Nebencrate `apps/local-voice/crates/abgleich-pruefung` (wie der Prüfstand eigenständig, hängt an der vendorten Crate mit Feature `lokal` und an `hound`). Es startet die gebaute App headless (`--sprachdienst-pruefung`, neuer Schalter neben `--transcribe-file`, `cli.rs:45-100`) in einer Portable-Kopie (Marker `portable`, eigener `Data/`-Ordner, `portable.rs`), legt dort eine Testdatei `sprachdienst.json` mit einem Anrufzugang aus einem frischen Zufallsschlüssel an, verbindet sich über die Named Pipe mit `Anrufzugang` und spielt den Korpus in Echtzeit (20-ms-Rahmen) ein. Voraussetzung: keine laufende Local-Voice-Instanz (Pipe-Name hängt an der SID). Modellordner im Portable-Modus: **ungeprüft**.
- Erfasst werden: `Data/logs/handy.log` bei Stufe Trace, stdout und stderr des Kindprozesses (Release ist ein GUI-Subsystem-Programm, `cli.rs` Kommentar zu `--out`; Umleitung per `Command` mit Pipes: **ungeprüft**), Dateiliste und Größen unter `Data/` und `%TEMP%` vorher/nachher. Kanarien: Korpussätze, die selbst als Erkennungstext auftreten („Kanarienvogel Vierundsiebzig"), die Kennungen (Hex) des Laufs; Klassen werden nicht gesucht, sondern es darf keine Zeile mit `annahme|stopp|unbekannt` aus dem Abgleichpfad geben.
- Eingespeiste Fehler: Feature `abgleich-einspeisung` (nur für den Prüflauf gebaut, Debug **und** Release), das in `einordnen` nach dem echten Engine-Lauf (a) einen Fehler mit Kanarientext, (b) eine Panic mit Kanariennutzlast, (c) eine Verzögerung über die Frist auslöst. Erwartet: fester Code, kein Kanarienwort in irgendeinem Kanal, nie `annahme`.
- Parallel: zweite Verbindung desselben Zugangs spricht (Phonon) während des Abgleichs; Abbruch eines Abgleichs während der Engine-Lauf läuft, sofort danach neuer Abgleich (Messwert: wie oft `ueberlastet`).
- **Nicht still grün:** Diese Läufe sind kein `cargo test`. Der Prüflauf endet mit Exit 3 und „NICHT GELAUFEN", wenn Modell, Stimme oder Pipe fehlen, und schreibt immer einen Bericht (JSON: Commit, Bauprofil, Modell-ID, Gerät, Zählungen). Abnahme nur mit diesem Bericht im PR.

### 3.3 Synthetischer Ton

- Werkzeug: die App selbst, `--tts-test --tts-provider phonon --tts-voice Anton --tts-text "…" --tts-out-wav …` (`cli.rs:107-123`, `tts_cli.rs`; genaue Aufrufform **ungeprüft**), danach auf 16 kHz mono PCM16 umgetastet (`rubato`/`hound` sind Abhängigkeiten der App). Zusätzlich eine zweite deutsche Stimme für die Mailboxansage.
- Lizenz: Stimme „Anton" Apache-2.0, aber die deutschen Phonon-Gewichte `phonon-40m-de` sind **CC BY-NC 4.0** (`catalog.json`, Eintrag `phonon-40m-de`). Erzeugter Ton deshalb nur als lokaler Testton, nie im Repo und nicht in Artefakten. Ins Repo kommen nur die Satzliste, Seeds und das Erzeugerskript. Für dauerhaft im Repo liegende Prüfdateien wäre eine Piper-Stimme mit freier Lizenz nötig (Lizenz je Stimme **ungeprüft**).
- Korpus (je Satz mehrere Varianten in Tempo, Pause, Pegel): Liste 1 einzeln und mit Satzzeichenprosodie; Verneinungen („nein", „nicht okay", „ja, nein"); Fortsetzungen („ja, aber …", „ja, momentan bin ich nicht erreichbar", „stopp war nur ein Beispiel"); Wiederholungen („ja ja ja", „ähm ja" — prüft den Wegfall der Nachbearbeitung); Rauschen (weiß/rosa, Seed, 0/10/20 dB SNR) und reines Rauschen; Echo der eigenen Ansage (Hinweistext verzögert und gedämpft unter die Antwort); Telefonband (300-3400 Hz, 8 kHz G.711 hin und zurück); Mailboxansage allein und mit „ja" darin; Stille.
- Messgrößen je Modell: Fehlannahme (Klasse Annahme bei Soll ≠ Annahme), Fehlstopp, verpasste Annahme/Stopp, Fehlerquote durch Störungen; Latenz a) Dienst (`abgleich_ende` bis Antwort), b) Sprechende (aus dem sauberen TTS-Ton per Energieschwelle bestimmt) bis Klasse bei einer vorgegebenen Endpause des Adapters. Bericht mit Median, p95, Maximum, Stichprobengröße, Hardware; keine Zusage.

### 3.4 Listenversion 2

Gemessen wird, wie oft die Engine für gesprochenes „okay"/„Stopp" die Schreibungen „ok"/„OK"/„Ok" bzw. „stop" liefert und wie viele Fehlannahmen eine Liste 2 auf dem Störkorpus zusätzlich brächte. Dafür muss der Prüflauf im Feature `abgleich-messung` zusätzlich eine Kandidatenklasse nach Liste 2 zählen (nie Text ausgeben). Ergebnis: Vorschlag an das SDK, keine Änderung an `LISTE`.

## 4. Paketzuschnitt

| Paket | Scope | Akzeptanztest (Befehl → Ergebnis) | hängt ab von | Aufwand / Risiko |
|---|---|---|---|---|
| **B0** (SDK) | Nachtrag: konstantzeitiges `Kennung::eq`, `Sitzung::ausgang_verbuchen` + Tests, Leitfaden | `cargo test -p wai-voice-dienst --all-features --locked` → grün, neue Tests rot ohne Änderung | – | S / gering; parallel zu B1-B3 |
| **B1** | Vendoring auf `8d8dd601` (bzw. B0-Stand), `ERLAUBT` + `libc`, `windows-sys` mit Lizenzmeldung (MIT OR Apache-2.0); Dienst übersetzt: `AbgleichEnde` angenommen, `start{abgleich}` → `recht_fehlt` bzw. `engine_fehlt` mit leerer `nachricht`; Ausgangstor an allen drei Sendestellen; Prüfstand-Lock neu | `python3 scripts/vendor-wai-voice-dienst.py "$WAI_SDK" --check` → Exit 0; Prüfstand `cargo test --locked …` → grün inkl. neuer Tests; Windows `cargo check --locked --tests` → Exit 0 | #595 | S-M / mittel: Bruchstellen in `app.rs` sieht nur der Windows-Bau; `Cargo.lock`-Konflikt mit #565 |
| **B2** | Anrufzugang: Feld `anruf`, Invariante, `willkommen`, Dienst verweigert Erkennen, Befehl, Schalter, `bindings.ts` | Prüfstand grün mit Goldtest `willkommen` und Interop `Anrufzugang`; Windows `bun run build` + Rust-Tor | B1 | M / mittel: Konflikte `lib.rs`/`bindings.ts` mit #598, #583 |
| **B3** | Abgleich-Protokoll im Dienst mit Attrappe (`sprachdienst/abgleich.rs`, `verbindung.rs`, `engine.rs`, `kontext.rs`-Fristen), alle Fälle aus 3.1 | Prüfstand `cargo test --locked …` → grün, neue Tests einzeln benannt | B1 (B2 für Interop-Tests) | M-L / mittel: `verbindung.rs` ist Kern aller Ströme |
| **B4** | Vertraulicher Inferenzpfad: `TranscriptionManager::einordnen`, `AppSprache::abgleichen`, Panic-Hook, Vorwärmen | Windows `cargo test --locked` (App-Crate, neue Einheitentests) → grün | B3, F1 | M / hoch: `transcription.rs` (4300 Z.) ist der Diktatkern |
| **B5** | Prüflauf mit echter Engine (Nebencrate, `--sprachdienst-pruefung`, Einspeisung, Korpus-Erzeuger), Leckprüfung Debug+Release | Windows: `cargo run --release --manifest-path apps/local-voice/crates/abgleich-pruefung/Cargo.toml -- …` → Exit 0, Bericht ohne Leck; ohne Modell Exit 3 | B4 | L / hoch: Umgebung (Portable, Modelle, Pipe), lange Läufe ansagen |
| **B6** | Messbericht (Fehlannahme, Fehlstopp, verpasst, Latenz unter Last), Vorschlag Liste 2 | Bericht im PR, Zahlen aus B5-Lauf | B5 | S / gering |

Reihenfolge: #595 → B1 → (B0 parallel) → B2 → B3 → B4 → B5 → B6. B1-B3 sind in der Cloud prüfbar (Prüfstand), B4-B6 nur auf dem PC.

## 5. Offene Fragen an den Prüfer

- **F1** Panic-Hook: prozessweit ohne Nutzlast (alle Panics der App verlieren ihre Meldung) oder Kindprozess für den Abgleich? Empfehlung: prozessweit.
- **F2** Modell für den Abgleich: das gewählte Diktatmodell (kann ohne Deutsch sein, ist während des Diktats belegt) oder ein festes, eigenes Modell mit Deutsch (eigener Speicher, kein Konflikt mit dem Diktat; der Platz der Zusatz-Engine aus #579 ist einfach belegt)? Entscheidung nach B6-Messung möglich, aber B4 braucht eine Vorgabe.
- **F3** Eigener Engine-Platz für den Abgleich, damit Sprechen ohne Frist (#586) und fremdes Erkennen ihn nicht in `ueberlastet` treiben?
- **F4** Vorwärmen des Modells bei Anmeldung eines Anrufzugangs (Speicher und Strom) oder Laden erst beim ersten Abgleich (erster Abgleich dann sicher `ueberlastet`/`zeit_ueberschritten`)?
- **F5** Grenze 2 Verbindungen je `client`: belassen oder für Anrufzugänge 3 (Neuaufbau während Abbau)?
- **F6** Widerspruch zum ADR: ADR-0032 sieht nach der Annahme freie Erkennung der Antwort vor, Abnahmepunkt 5 und `Anrufzugang` verbieten dem Anrufzugang `hoeren`. Braucht der Dialog einen zweiten, getrennt gekoppelten Zugang mit `hoeren`?
- **F7** Darf der Prüflauf mit Feature `abgleich-messung` (nur Testbau, nur synthetischer Korpus) die Liste-2-Kandidatenklasse zählen, oder soll die Messung über `--transcribe-file` mit eigener Rohausgabe laufen?
- **F8** Soll eine Energie- oder VAD-Schranke (Silero liegt in der App) reine Störgeräusche vorab zu `Unbekannt` machen? Nur nach Messung; sie kann nie `Annahme` erzeugen, verschiebt aber verpasste Antworten.
