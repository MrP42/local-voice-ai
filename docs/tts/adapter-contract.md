# TTS-Adaptervertrag

Stand 26.10.2026 · Goal `tts-provider` (Paket D1) · Quelle der Wahrheit ist der Code unter
`apps/local-voice/src-tauri/src/managers/tts/providers/` (kurz **P/**). Fundstellen `Datei:Zeile` beziehen sich auf
diesen Stand; verschiebt sich eine Zeile, gilt der Code. Recherche: [provider-research.md](provider-research.md),
Plan: [implementation-plan.md](implementation-plan.md).

## 1. Überblick

Jeder Sprachausgabe-Anbieter ist ein `Arc<dyn TtsProvider>` in der `ProviderRegistry` (P/mod.rs:1-42). Fish und Piper
sind dünne Hüllen um den vorhandenen Code (P/builtin.rs), `phonon`, `openai-speech`, `ollama-speech` sind Adapter
(P/phonon.rs, openai_speech.rs, ollama_speech.rs), `mock` ist der Testanbieter (P/mock.rs). Die Schicht läuft nie im
Audio-Callback; sie legt nur fertige Chunks in einen begrenzten Kanal (P/mod.rs:39-42). Kein Modul der Schicht ruft
`settings::get_settings(&AppHandle)`; Werte reicht der Aufrufer herein (P/mod.rs:53-54).

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
schreiben `Box::pin(async move { … })` (contract.rs:515-517, mod.rs:35-42).

## 4. Semantik je Methode

* **`descriptor`**: statisch und stabil (zwei Aufrufe liefern Gleiches, contract_tests.rs:159). Alle Fähigkeiten,
  Lizenzfelder und Plattformen sind gefüllt (contract_tests.rs:161-197).
* **`health_check`**: antwortet in 5 s; „nicht verfügbar“ heißt immer `reason` (lesbar) und möglichst `hint`
  (contract.rs:160-162, contract_tests.rs:216). Die Registry setzt `checked_ms` und fängt Panik und Hänger je Anbieter
  ab (registry.rs:194-216, `health_all` :297).
* **`list_voices(language)`**: Filter über `language_matches` (Hauptkürzel, `de_DE` → `de`; `und` passt immer,
  contract.rs:433-449). Stimmen vollständig und eindeutig (contract_tests.rs:235).
* **`synthesize`** liefert **immer ein vollständiges WAV**; `opts.format` steuert nur die Chunks des Streams
  (contract.rs:211-214, mod.rs:46-47). `duration_ms` kommt aus dem Header. Leerer Text ist `BadRequest`; nicht
  verfügbar ist `Unavailable` **ohne** Prozessstart oder Netzzugriff (contract.rs:520-523).
* **`stream_synthesize`**: Standard ruft `synthesize` und sendet **ein** Chunk mit `is_last = true`
  (contract.rs:550-575) — korrekt für `Sentence`/`None`. Nur wer echte Teil-Audios liefert, überschreibt die Methode
  und deklariert `StreamingMode::Chunk`. Die Standardimplementierung rechnet nie um: ein Rohformat gibt es nur, wenn
  der Clip es schon ist, sonst `BadRequest` (wav.rs:195-220).
* **`cache_tag`**: siehe Abschnitt 9.

### Stream-Chunks

* `seq` zählt ab 0 lückenlos und monoton; **genau ein** Chunk trägt `is_last` und ist der letzte; `request_id` stammt
  aus dem `CancelToken`; Daten und `format` passen zu `opts.format` (contract.rs:94-95, mod.rs:48-50,
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
(mod.rs:48-50).

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
den leeren Tag** (Legacy-Regel `engine::fish_cache_tag`, builtin.rs:210); Piper `piper/<stimme>`. Die Suite verlangt:
deterministisch; wenn nicht leer, mit der Kennung beginnend und je Stimme verschieden (contract_tests.rs:199-213).

## 10. Neuen Anbieter hinzufügen

1. **Datei** `P/<name>.rs` anlegen, `pub mod <name>;` und `pub use` in P/mod.rs:84-113 ergänzen. Moduldoku nach Muster
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

<!-- P3 -->
TODO (Paket P3a/P3b/P4): Registrierung in der Registry, Einstellungsfeld `tts_providers`, Guardrails (`license_ack`,
`allow_remote`), Tauri-Befehle, `EngineImpl::Provider`, CLI und UI. Im Code noch **nicht** vorhanden; geplant laut
implementation-plan.md §3 und §5. Die Registrierung geschieht ausdrücklich nicht im Adapter (mod.rs:29-30).
Dieser Abschnitt wird mit P3 ergänzt.

## 12. Offen

* Guardrail-Verhalten (`opt_in`, `license_ack`) ist nur als Deskriptorfeld und in builtin.rs:22-27 beschrieben, nicht
  umgesetzt.
* Fish-Synthese wird von außen als `SynthFn` hereingereicht; ohne Closure `InvalidConfig` (mod.rs:76-77).
* Piper-Abbruch: siehe [installation-troubleshooting.md](installation-troubleshooting.md), „Bekannte Grenzen“ (B2).
