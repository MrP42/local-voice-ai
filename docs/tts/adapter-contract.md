# TTS-Adaptervertrag

Stand 26.10.2026 · Goal `tts-provider` (Paket D1) · Quelle der Wahrheit ist der Code unter
`apps/local-voice/src-tauri/src/managers/tts/providers/` (kurz **P/**). Fundstellen `Datei:Zeile` beziehen sich auf
diesen Stand; verschiebt sich eine Zeile, gilt der Code. Recherche: [provider-research.md](provider-research.md),
Plan: [implementation-plan.md](implementation-plan.md).

## 1. Überblick

Jeder Sprachausgabe-Anbieter ist ein `Arc<dyn TtsProvider>` in der `ProviderRegistry` (P/mod.rs:11-51). Fish und Piper
sind dünne Hüllen um den vorhandenen Code (P/builtin.rs), `phonon`, `openai-speech`, `ollama-speech` sind Adapter
(P/phonon.rs, openai_speech.rs, ollama_speech.rs), `mock` ist der Testanbieter (P/mock.rs). Die Schicht läuft nie im
Audio-Callback; sie legt nur fertige Chunks in einen begrenzten Kanal (P/mod.rs:58-60). Kein Modul der Schicht ruft
`settings::get_settings(&AppHandle)`; Werte reicht der Aufrufer herein (P/mod.rs:71-72).

## 2. Typen (P/contract.rs)

IPC-Typen tragen serde + `specta::Type`, Felder und Enum-Werte `snake_case` (contract.rs:3-6).
`AudioChunk`/`AudioClip` tragen rohe Bytes und gehen nie über IPC (contract.rs:7-8).

| Typ | Fundstelle | Inhalt |
|---|---|---|
| `ProviderTier` | :24 | `Supported`, `Experimental`, `BenchmarkOnly`, `Unavailable` |
| `StreamingMode` | :40 | `None`, `Sentence`, `Chunk` |
| `Device` | :49 | `Cpu`, `Cuda`, `Metal`, `Remote` |
| `AudioEncoding` | :60 | `Pcm16Le` (`pcm16_le`), `F32Le` (`f32_le`), `Wav` |
| `AudioFormat` | :85 | `sample_rate: u32`, `channels: u16`, `encoding` |
| `AudioChunk` | :96 | `request_id`, `seq: u32`, `format`, `data: Vec<u8>`, `is_last`, `elapsed_ms: u64` |
| `AudioClip` | :108 | `format`, `data` (immer vollständiges WAV), `duration_ms` |
| `LicenseInfo` | :118 | `code`, `weights`, `non_commercial`, `verified`, `source_url`, `verified_on: Option<String>`, `note` |
| `ProviderCaps` | :133 | `languages`, `streaming`, `formats`, `sample_rates`, `cloning`, `devices`, `platforms`, `style_tags` |
| `ProviderDescriptor` | :149 | `id` (`[a-z0-9][a-z0-9_-]*`), `label`, `tier`, `opt_in`, `caps`, `license` |
| `ProviderHealth` | :163 | `available`, `reason`, `hint`, `device`, `version`, `checked_ms` |
| `VoiceDescriptor` | :199 | `id`, `label`, `language` (`und` = beliebig), `gender`, `license` (nur bei Abweichung) |
| `SynthesisOptions` | :216 | `language`, `voice`, `format`, `speed`, `seed`, `timeout` (Vorgabe `de`, `Wav`, 60 s, :226-236) |
| `TtsErrorKind` | :242 | `Unavailable`, `InvalidConfig`, `LicenseBlocked`, `PrivacyBlocked`, `Timeout`, `Cancelled`, `BadRequest`, `Backend`, `Network` |
| `TtsError` | :276 | `kind`, `retryable`, `message` |
| `CancelToken` | :358 | klonbar, trägt `request_id`; `cancel()`, `is_cancelled()`, `cancelled().await` |

## 3. Der Trait (contract.rs:526-577, wörtlich)

```rust
pub trait TtsProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn health_check(&self) -> BoxFuture<'_, ProviderHealth>;
    fn list_voices<'a>(&'a self, language: Option<&'a str>)
        -> BoxFuture<'a, Result<Vec<VoiceDescriptor>, TtsError>>;
    fn synthesize<'a>(&'a self, text: &'a str, opts: &'a SynthesisOptions, cancel: &'a CancelToken)
        -> BoxFuture<'a, Result<AudioClip, TtsError>>;
    fn stream_synthesize<'a>(&'a self, text: &'a str, opts: &'a SynthesisOptions,
        cancel: &'a CancelToken, sink: mpsc::Sender<AudioChunk>)
        -> BoxFuture<'a, Result<(), TtsError>>;          // Standardimplementierung vorhanden
    fn cache_tag(&self, voice: Option<&str>) -> String;
}
```

Dyn-fähig über `futures_util::future::BoxFuture` statt `async-trait` (keine neue Abhängigkeit); Implementierungen
schreiben `Box::pin(async move { … })` (contract.rs:515-517, mod.rs:55-60).

## 4. Semantik je Methode

* **`descriptor`**: statisch und stabil (zwei Aufrufe liefern Gleiches, contract_tests.rs:159). Alle Fähigkeiten,
  Lizenzfelder und Plattformen sind gefüllt (contract_tests.rs:161-197).
* **`health_check`**: antwortet in 5 s; „nicht verfügbar“ heißt immer `reason` (lesbar) und möglichst `hint`
  (contract.rs:160-162, contract_tests.rs:216). Die Registry setzt `checked_ms` und fängt Panik und Hänger je Anbieter
  ab (registry.rs:194-216, `health_all` :297).
* **`list_voices(language)`**: Filter über `language_matches` (Hauptkürzel, `de_DE` → `de`; `und` passt immer,
  contract.rs:433-449). Stimmen vollständig und eindeutig (contract_tests.rs:235).
* **`synthesize`** liefert **immer ein vollständiges WAV**; `opts.format` steuert nur die Chunks des Streams
  (contract.rs:211-214, mod.rs:64-65). `duration_ms` kommt aus dem Header. Leerer Text ist `BadRequest`; nicht
  verfügbar ist `Unavailable` **ohne** Prozessstart oder Netzzugriff (contract.rs:520-523).
* **`stream_synthesize`**: Standard ruft `synthesize` und sendet **ein** Chunk mit `is_last = true`
  (contract.rs:550-575) — korrekt für `Sentence`/`None`. Nur wer echte Teil-Audios liefert, überschreibt die Methode
  und deklariert `StreamingMode::Chunk`. Die Standardimplementierung rechnet nie um: ein Rohformat gibt es nur, wenn
  der Clip es schon ist, sonst `BadRequest` (wav.rs:195-220).
* **`cache_tag`**: siehe Abschnitt 9.

### Stream-Chunks

* `seq` zählt ab 0 lückenlos und monoton; **genau ein** Chunk trägt `is_last` und ist der letzte; `request_id` stammt
  aus dem `CancelToken`; Daten und `format` passen zu `opts.format` (contract.rs:94-95, mod.rs:66-68,
  contract_tests.rs:371-480). `elapsed_ms` ist die Zeit seit Auftragsbeginn (der erste Chunk ergibt die Zeit bis zum
  ersten Ton).
* **Frame-Ausrichtung**: ein Chunk enthält nur ganze Frames (2 Byte je Kanal bei PCM16). Ein Rest wandert in den
  nächsten Chunk, nie ein halbes Sample (openai_speech.rs:13-22, `PcmDecoder` :299). Damit genau der letzte Chunk
  `is_last` trägt, hält `openai-speech` das letzte Frame zurück und sendet es am Ende.
* **Ehrliche `StreamingMode`-Angabe**: `Chunk` nur, wenn vor dem Ende der Synthese echte Teil-Audios fließen. Wer erst
  den ganzen Satz berechnet, ist `Sentence` (contract.rs:36-39). Die Suite prüft, dass ein `Chunk`-Anbieter bei
  genügend langem Text wirklich mehrere Teile liefert (contract_tests.rs:28, 447). Ist: `fish`, `piper`, `phonon` =
  `Sentence` (builtin.rs:122,309, phonon.rs:1047); `openai-speech`, `ollama-speech` = `Chunk` (openai_speech.rs:1103;
  ollama übernimmt den Unterbau, ollama_speech.rs:1-12).

## 5. Abbruch und Rückstau

* Jeder Auftrag hat einen `CancelToken` (die Registry führt `request_id → Token`, `begin_request` registriert,
  doppelte Kennung = `BadRequest`, registry.rs:309-327; der `RequestGuard` räumt beim Drop auf).
* **`guarded(cancel, timeout, what, fut)`** (contract.rs:454) ist der Pflichtweg für `synthesize`: Abbruch ergibt
  `Cancelled`, Fristablauf `Timeout`, in beiden Fällen wird `fut` fallengelassen; ein schon abgebrochener Token startet
  `fut` gar nicht. Subprozess-Adapter setzen `kill_on_drop` bzw. einen eigenen Wächter (phonon.rs:7-28), damit kein
  Restprozess bleibt.
* **`send_chunk(sink, chunk, cancel)`** (contract.rs:491) ist der einzige Weg, einen Chunk zu senden. Der Kanal ist
  begrenzt; ist er voll, wartet der Anbieter (Rückstau, `sink_stall_count()` zählt mit, contract.rs:477-482), aber nur
  bis zum Abbruch. Ist der Empfänger weg, endet der Auftrag mit `Cancelled`. Kein Chunk geht still verloren.
* `opts.timeout` ist die Gesamtfrist (bei Streams bis zum letzten Chunk), kein Leerlauf-Timeout (contract.rs:211-214).
  Die Registry legt ein Sicherheitsnetz von 2 s darüber (`SAFETY_GRACE`, registry.rs:28).

## 6. Fehlerklassen und Wiederholung

`TtsError::new` setzt `retryable` nur für `Network`; `with_retryable` kann `Network`/`Backend` hochstufen, nie
`Cancelled`, `Timeout`, `InvalidConfig`, `LicenseBlocked`, `PrivacyBlocked`, `BadRequest`, `Unavailable`
(contract.rs:284-306). Die Klasse entscheidet über Wiederholung und Anzeige, nicht der Text (contract.rs:238-241).

| Klasse | Typischer Anlass | wiederholbar |
|---|---|---|
| `Unavailable` | Programm/Modell/Dienst fehlt, Plattform nicht unterstützt | nein |
| `InvalidConfig` | ungültige Einstellung, 401/403, 3xx | nein |
| `LicenseBlocked` | gesperrtes (gated) Modell | nein |
| `PrivacyBlocked` | Server nicht auf Loopback ohne `allow_remote` | nein |
| `Timeout` | Frist abgelaufen | nein |
| `Cancelled` | Abbruch, Empfänger weg | nein |
| `BadRequest` | leerer Text, 400/404/422, nicht lieferbares Format | nein |
| `Backend` | Programmfehler, kaputtes Audio; 5xx/429 (openai_speech.rs:36-37) | nur 5xx/429 |
| `Network` | Verbindung, abgerissener Strom, 408 | ja |

**Retry-Regel**: `with_retry` wiederholt nur bei `retryable`, **nur solange noch kein Chunk geliefert wurde**
(`DeliveryTracker`), höchstens `max_attempts` (hart auf 1..=5 begrenzt, Vorgabe 2 mit Pause `150 ms × Versuch`,
registry.rs:25,38-55,75-108) und nie nach Abbruch; die Pause ist abbrechbar. Für Streams gilt dasselbe über
`stream_with_retry` (registry.rs:113): nach dem ersten weitergeleiteten Chunk nie, sonst hörte man Teile doppelt
(mod.rs:66-68).

## 7. Fristen

Auftragsfrist `opts.timeout` (Vorgabe 60 s). Health und Stimmenliste kurz: ptts `--version` 5 s, `--list-voices` 20 s
(phonon.rs:89-91); HTTP-Health und -Stimmen 3 s (openai_speech.rs:83); Ollama Version 2 s, Modelle 3 s, Probe 10 s
(ollama_speech.rs:77-79). Antwortdeckel 256 MiB (openai_speech.rs:81, phonon.rs:95).

## 8. Lizenzfelder und Privacy

* `LicenseInfo.code` (Code) und `weights` (Gewichte/Stimmen) sind getrennt zu füllen; `non_commercial` gilt für die
  Gewichte. **`verified = true` nur mit `source_url` UND `verified_on` (`JJJJ-MM-TT`)**; sonst `false` plus `note`, die
  sagt, was zu prüfen ist (contract.rs:114-117, contract_tests.rs:166-181). Beispiele: Phonon-31m-Gewichte `true`
  (phonon.rs:618-668); Nutzerangaben beim generischen Server sind immer `false` (openai_speech.rs:1025-1050).
* Unbekannte Modelle gelten vorsorglich als `non_commercial = true` (phonon.rs:618-640, ollama_speech.rs:236-250).
* **Privacy**: Adapter, die Text an einen Server senden, prüfen Loopback (`127.0.0.0/8`, `::1`, `localhost`). Andere
  Hosts ergeben `PrivacyBlocked`, außer `allow_remote`. Keine Umleitungen, kein System-Proxy für Loopback; API-Schlüssel
  nie in der Konfiguration, nur der Name der Umgebungsvariable (`api_key_env`), nie in Meldungen
  (openai_speech.rs:23-32, 213).
* Subprozess-Adapter erzeugen keine stillen Downloads (`HF_HUB_OFFLINE=1`, Cache-Vorprüfung, phonon.rs:52-67).

## 9. Cache-Tag-Regel

Neue Anbieter: `"<id>:<modell>:<stimme>"` (contract.rs:572-575). Belege: `openai-speech:kokoro:af_bella`
(openai_speech.rs:1236-1243, Test :2076), `ollama-speech:phonon:Zoey` (Test ollama_speech.rs:1335), Phonon
`phonon:<modell>~<hash>:<stimme>` mit Hash über Modell, Quantisierung und Sprache (phonon.rs:1201-1228). **Fish behält
den leeren Tag** (Legacy-Regel `engine::fish_cache_tag`, builtin.rs:211); Piper `piper/<stimme>`. Die Suite verlangt:
deterministisch; wenn nicht leer, mit der Kennung beginnend und je Stimme verschieden (contract_tests.rs:199-213).

## 10. Neuen Anbieter hinzufügen

1. **Datei** `P/<name>.rs` anlegen, `pub mod <name>;` und `pub use` in P/mod.rs:109-151 ergänzen. Moduldoku nach Muster
   (Entscheidungen, Tabelle „Fehlerfälle und ihre Absicherung“).
2. **Konfigurationsstruktur** mit `#[serde(default)]`, `Serialize/Deserialize/specta::Type`, einer `validate()`, deren
   Meldungen für Nutzer lesbar sind (Vorbild `PhononConfig`, phonon.rs:119-155, 232).
3. **`impl TtsProvider`**: `descriptor` (gültige Kennung, Tier `Experimental`, `opt_in = true` für alles Neue, ehrliche
   `StreamingMode`, Plattformen, Lizenz nach Abschnitt 8), `health_check` mit `reason`/`hint`, `list_voices`,
   `synthesize` über `guarded`, bei echtem Streaming `stream_synthesize` über `send_chunk`, `cache_tag` nach
   Abschnitt 9. Fehler mit der richtigen Klasse (Abschnitt 6).
4. **Tests**: in `#[cfg(test)]` `contract_suite(Arc::new(provider), ContractExpect::available().with_slow(…))`
   aufrufen (contract_tests.rs:115-123). Sie prüft Deskriptor, Health, Stimmen, Synthese (WAV, `BadRequest` bei leerem
   Text), Stream (`seq`, ein `is_last`, Format, Empfänger weg), Abbruch (vorab und mitten im Lauf) und Frist
   (`Timeout`, nicht wiederholbar). Mit `ContractExpect::unavailable()` zusätzlich den Nichtverfügbar-Pfad. `with_slow`
   ist Pflicht für Abbruch/Frist (Fake mit Verzögerung > 1 s).
5. **Fake-Fixtures** (nur Tests): `mock.rs` (Sinus/Stille, 24 kHz, steuerbare Latenz, Chunk-Anzahl, Fehler; Signal
   50 ms je Zeichen, 100 ms bis 20 s); `phonon_fake.rs` (echtes Fake-`ptts`, einmal mit `rustc` gebaut, Verhalten über
   `fake.cfg`: Verzögerung, Marker-Datei für „Prozess wurde beendet“, Fehlercodes, kaputtes WAV); `test_http.rs`
   (HTTP/1.1 auf `127.0.0.1:0`, gestückelte Antworten, hängend/abgerissen, `client_gone` als Beweis des Verbindungsabbaus).
   Neue Fixtures nur, wenn keine passt.
6. **Lizenz** recherchieren, mit Quelle und Datum belegen (Abschnitt 8) und in [licenses.md](licenses.md) eintragen.
7. **Prüfung**: `cargo test --lib managers::tts::providers` (nach Stand der Pakete der Abnahmetest, PAKETE.md P1).

## 11. Einbindung (Einstellungen, Befehle, UI)

Die Schicht ist seit P3a/P3b/P4 angeschlossen. Fundstellen unter **A/** = `apps/local-voice/src-tauri/src/`.

### Registry-Aufbau

* `setup::build_providers` baut **immer alle fünf** Anbieter in fester Reihenfolge: `fish`, `piper`, `phonon`,
  `openai-speech`, `ollama-speech` (P/setup.rs:44-64). Auch unkonfiguriert und ohne Freigabe: so lassen sie sich listen und
  prüfen. Die Guardrails gelten an der **Nutzung**, nicht an der Registrierung (P/setup.rs:1-9). `mock` wird nie
  registriert (nur Tests).
* Die Registry lebt lange. `ProviderHost::sync` ersetzt Anbieter an Ort und Stelle und trägt nicht mehr vorhandene aus; die
  Auftragstabelle bleibt stehen, ein laufender Auftrag hält seinen alten Anbieter und bleibt abbrechbar
  (P/setup.rs:10-17, `ProviderHost` :106, `sync` :132). Der `TtsManager` ist der Gastgeber (P/host.rs); die
  Fish-Synthese wird als `SynthFn` hereingereicht (P/host.rs:37).
* Kennung des generischen Servers: `openai-speech` trägt die Kennung seiner Konfiguration; eine ungültige oder eingebaute
  Kennung (`RESERVED_IDS`: `fish`, `piper`, `phonon`, `ollama-speech`, `mock`) wird auf `openai-speech` zurückgesetzt
  (P/policy.rs:61-63, 354-357).

### Einstellungsfeld `tts_providers`

`AppSettings.tts_providers: TtsProvidersSettings` (A/settings.rs:813; Struktur P/policy.rs:81-98). Das Feld ist optional
(`#[serde(default)]`): alte Dateien bleiben gültig, alles ist aus bzw. leer (Tests A/settings.rs:3090-3115).

| Feld | Inhalt | Wer setzt es |
|---|---|---|
| `phonon`, `openai_speech`, `ollama_speech` | Konfiguration je Adapter (`PhononConfig`, …) | `tts_provider_set_config` |
| `enabled` | Opt-in je Kennung | nur `tts_provider_set_enabled` |
| `license_acks` | Zustimmung (`provider_id`, `fingerprint`, `acked_at`) | nur `tts_provider_acknowledge_license` |
| `allow_remote` | Kennungen, die eine Adresse außerhalb von Loopback nutzen dürfen | nur `tts_provider_set_allow_remote` |
| `voices` | letzte Stimmenauswahl je Kennung | `tts_provider_select` (Oberfläche und Agentenbrücke) |

Ein allgemeiner Setter für das Feld existiert bewusst nicht (A/commands/tts_providers.rs:1-25).

### Konfigurationsvorrang

Deterministisch, rein (`resolve_provider_configs`, P/policy.rs:310; Regeln P/policy.rs:9-29):

```
Override (Argument)  >  Umgebung LVA_TTS_<ANBIETER>_<FELD>  >  Einstellungen (tts_providers)  >  Vorgabe des Adapters
```

* Überschreibbar sind **nur** `exe_path`, `model`, `base_url`, `voice` (`OVERRIDABLE_FIELDS`, P/policy.rs:66).
  `<ANBIETER>` ist die Kennung in Großbuchstaben, `-` wird `_`: `LVA_TTS_PHONON_EXE_PATH`,
  `LVA_TTS_OPENAI_SPEECH_BASE_URL`, `LVA_TTS_OLLAMA_SPEECH_MODEL`, `LVA_TTS_OPENAI_SPEECH_VOICE` (`env_var_name`,
  P/policy.rs:187). Felder, die ein Anbieter nicht kennt (`LVA_TTS_PHONON_BASE_URL`), werden ignoriert; leere Werte
  (nach `trim`) gelten als nicht gesetzt.
* Stimme: Override > Umgebung > `voices[<id>]` > `voice` der Anbieter-Konfiguration > keine (P/policy.rs:23-24, 431-445).
* Freigaben (`enabled`, `license_acks`, `allow_remote`) lassen sich auf **keinem** dieser Wege setzen. Die Adapter-Schalter
  `allow_remote` (openai-speech, ollama-speech) und `allow_network` (phonon) werden aus `allow_remote` der Einstellungen
  **abgeleitet** (P/policy.rs:25-29, 350, 387, 420). `apply_config_json` setzt den Schalter beim Speichern auf `false`
  und prüft nur das Format: eine entfernte Adresse lässt sich speichern, benutzen darf man sie erst mit Freigabe
  (P/policy.rs:635-702).
* Herkunft je Feld liefert `ResolvedConfigs::source` (`default`, `settings`, `env`, `override`; P/policy.rs:145-153, 214).
  Die CLI füllt die Overrides (siehe [cli-and-automation.md](cli-and-automation.md)).

### Guardrails (wörtlich aus `check_use`, P/policy.rs:513-585)

`check_use` ist die **eine** Stelle, die entscheidet, ob ein Anbieter benutzt werden darf. In dieser Reihenfolge:

1. `opt_in`: die Kennung muss in `enabled` stehen, sonst `LicenseBlocked` („nicht freigeschaltet“; die Klasse steht für
   „der Nutzer muss erst zustimmen“).
2. `opt_in && (non_commercial || !verified)` (`license_needs_ack`, P/policy.rs:484): ein `license_ack` mit dem **aktuellen**
   Fingerabdruck ist nötig, sonst `LicenseBlocked`. Eine Zustimmung mit altem Fingerabdruck gilt nicht mehr
   („Modell oder Lizenzangaben haben sich geändert … erneut bestätigen“).
3. Adresse außerhalb von Loopback: die Kennung muss in `allow_remote` stehen, sonst `PrivacyBlocked`. Gilt unabhängig von
   `opt_in`. Eine unparsbare Adresse ist `InvalidConfig` (P/policy.rs:489-501).

Fish und Piper haben `opt_in = false` und keine Adresse und bleiben frei (nie gesperrt, auch wenn ihre Lizenz ungeprüft ist).

**Fingerabdruck** (`license_fingerprint`, P/policy.rs:461): SHA-256 über längenpräfixierte Teile (Länge als `u64` little-endian,
dann Bytes) von Kennung, Gewichts-Lizenz (`license.weights`), `nc`/`c` (nichtkommerziell) und Modell; die ersten 32
Hexzeichen. Er wird gespeichert; ein Test hält einen Referenzwert fest, der Algorithmus darf sich nicht ändern. Ändert sich
Modell oder Lizenzangabe, wird erneut bestätigt. `acknowledge_in` nimmt nur den **aktuellen** Fingerabdruck an
(A/commands/tts_providers.rs:145-170). `UseState` (P/policy.rs:587) reicht ihn mit dem Sperrgrund an die Oberfläche.

**Wo geprüft wird** (Aufrufer, nicht Adapter): Vorlesen (`ProviderEngine::guard`, vor dem Cache-Lookup, P/pipeline.rs:177),
Probestimme (P/preview.rs:186), Stimmenliste und Auswahl (P/host.rs:102), CLI (`Plan::authorize`, A/tts_cli.rs:353) und die
Brückenwerkzeuge (über dieselbe Momentaufnahme).

### Engine-Wert `provider:<id>`

Die Wahl steht im bestehenden Feld `tts_engine` als `provider:<id>` (`PROVIDER_PREFIX`, A/managers/tts/engine.rs:39;
`provider_id_from_setting` :59, `engine_setting_for` :82). `provider:fish` und `provider:piper` sind Schreibweisen der
eingebauten Engines und laufen über deren unveränderte Pfade; ältere App-Fassungen kennen den Wert nicht und fallen auf
Fish zurück (P/pipeline.rs:1-30). Im Kern ist es die Variante `EngineImpl::Provider(ProviderEngine)`
(A/managers/tts/mod.rs:156): Satz-Pipeline, Satz-Cache, Abbruch über den Generationszähler und Player bleiben unverändert;
`synthesize` bricht auch mitten im Satz ab. Ein Anbieter spricht in **einer** gewählten Stimme (kein Sprecherwechsel per
Marker, P/pipeline.rs:68-70).

### Tauri-Befehle (A/commands/tts_providers.rs, registriert A/lib.rs:2436-2446)

| Befehl | Zeile | Wirkung |
|---|---|---|
| `tts_providers_list` | :239 | alle Anbieter mit Beschreibung, Gesundheit (parallel, Frist 4 s je Anbieter), Freigabestand, Fingerabdruck, Auswahl |
| `tts_provider_health` | :248 | Gesundheit eines Anbieters |
| `tts_provider_voices` | :258 | Stimmenliste (Frist 10 s); gilt als Nutzung, also mit `check_use` |
| `tts_provider_get_config` | :283 | gespeicherte Konfiguration als JSON-Text (Fish/Piper: `bad_request`) |
| `tts_provider_set_config` | :296 | Konfiguration prüfen (`validate()`) und speichern; Freigabe-Schalter werden neutralisiert |
| `tts_provider_set_enabled` | :305 | Opt-in setzen/zurücknehmen, **nur Oberfläche** |
| `tts_provider_acknowledge_license` | :318 | Lizenz zum aktuellen Fingerabdruck bestätigen, **nur Oberfläche** |
| `tts_provider_set_allow_remote` | :342 | Fernzugriff erlauben/zurücknehmen, **nur Oberfläche** |
| `tts_provider_select` | :358 | Anbieter und Stimme fürs Vorlesen wählen (prüft `check_use`, wirkt auch mitten im Vorlesen) |
| `tts_provider_preview` | :388 | Probestimme starten, liefert `request_id`; läuft das Vorlesen, `bad_request` |
| `tts_provider_cancel` | :400 | Probestimme abbrechen (Synthese und Wiedergabe) |

Schreibzugriffe laufen nacheinander unter einer Sperre (`SETTINGS_LOCK`, `mutate_settings`), damit sich zwei Befehle nicht
überschreiben.

**Nur die Oberfläche darf** `tts_provider_set_enabled`, `tts_provider_acknowledge_license` und
`tts_provider_set_allow_remote` anbieten. Sie sind ein Klick des Nutzers und dürfen nie in die Agentenbrücke, in eine
CLI-Automatik oder einen anderen unbeaufsichtigten Weg aufgenommen werden, sonst höbe ein Agent die Lizenz- und
Datenschutzsperre selbst auf (P/policy.rs:39-44, A/commands/tts_providers.rs:9-19). Die Brücke nutzt nur `select_provider`
und lesende Kernfunktionen; die CLI kennt keinen dieser Befehle. Ein Test der Brücke hält fest, dass kein Werkzeug
Freigaben ändern kann (`no_tool_can_change_enabled_acks_or_remote`, A/agent_bridge/tools/tts_providers/tests.rs:640).

### Ereignis `tts-provider-preview`

`TtsProviderPreview { request_id, phase, ttfa_ms, error_kind, message }` (P/preview.rs:96). Ablauf: `started` → `first_audio`
(mit `ttfa_ms`) → genau **ein** Abschluss `done` | `cancelled` | `error` (`error_kind`, `message`). `started` kann vor der
Antwort des Befehls ankommen; die Oberfläche ordnet über `request_id` zu und puffert ein erstes unbekanntes Ereignis.
Grenzen: Text höchstens 1000 Zeichen, Audio höchstens 8 000 000 Abtastwerte, Gesamtfrist 120 s (P/preview.rs:75-79).
Ein neuer Start löst eine laufende Probestimme ab; startet das Vorlesen, wird sie abgebrochen.

### Oberfläche

Gruppe „Sprachausgabe-Anbieter“ im Reiter Ausgabe (`apps/local-voice/src/components/settings/tts/providers/`):
`TtsProvidersGroup.tsx` lädt `ttsProvidersList` und zeigt, was die Befehle liefern (neue Anbieter erscheinen ohne
UI-Änderung). Freigabe-Ablauf (`nextApprovalStep`, providerModel.ts:18): Schalter „Aktivieren“, dann Lizenzdialog (wenn
`license_needs_ack && !license_acked`), dann Fernzugriffsdialog (wenn `remote && !allow_remote`). Wählbar sind nur die
Stufen `supported` und `experimental` (`isSelectableTier`, providerModel.ts:31). Details, Konfigurationsfelder, Stimmenwahl
und Probestimme mit Abbrechen: `ProviderDetails.tsx`, `useProviderPreview.ts`.

## 12. Offen

* Lizenz-Zustimmung hat **keinen Widerruf-Befehl**: ein Opt-out (`tts_provider_set_enabled(false)`) nimmt die Freischaltung
  zurück, die gespeicherte Zustimmung (`license_acks`) bleibt bis zur Änderung des Fingerabdrucks stehen.
* Fish-Synthese wird von außen als `SynthFn` hereingereicht; ohne Closure `InvalidConfig` (mod.rs:101-102).
* Piper-Abbruch: siehe [installation-troubleshooting.md](installation-troubleshooting.md), „Bekannte Grenzen“ (B2).
* Owner-Entscheidungen und bekannte Grenzen: [CHANGELOG.md](CHANGELOG.md).
