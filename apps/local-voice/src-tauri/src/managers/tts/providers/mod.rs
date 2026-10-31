//! Anbieterschicht des Vorlesens (Goal tts-provider, Paket P1).
//!
//! Hier liegt die zweite NAHT neben dem [`super::engine::TtsEngine`]-Trait:
//! ein dyn-fähiger Vertrag ([`TtsProvider`]), über den die App beliebig viele
//! Sprachausgabe-Anbieter verwaltet — Fish und Piper als dünne Hüllen,
//! weitere (Paket P2: phonon, openai-speech, ollama-speech) als Adapter. Der
//! `TtsEngine`-Trait und der Fish-/Piper-Code bleiben unverändert; die
//! Hüllen lesen Zustand und Stimmen aus den vorhandenen Funktionen und
//! delegieren die Synthese (siehe [`builtin`]).
//!
//! ## Bausteine
//!
//! * [`contract`] — Typen (Deskriptor, Fähigkeiten, Lizenz, Gesundheit,
//!   Stimmen, Optionen, Fehlerklassen), [`CancelToken`], der Trait
//!   [`TtsProvider`] und die zwei Helfer, über die jeder Adapter läuft:
//!   [`guarded`] (Abbruch + Frist) und [`send_chunk`] (begrenzter
//!   Stream-Kanal mit Rückstau-Zähler).
//! * [`wav`] — PCM in WAV packen, WAV-Header lesen (auch Streaming-Header).
//! * [`registry`] — [`ProviderRegistry`]: Verzeichnis, Gesundheit mit
//!   Zeitgrenze je Anbieter, `request_id → CancelToken`, [`with_retry`].
//! * [`mock`] — deterministischer Testanbieter (Sinus/Stille, 24 kHz mono).
//! * [`builtin`] — [`FishProvider`] und [`PiperProvider`].
//! * [`phonon`] — [`PhononProvider`]: Phonon über das CLI `ptts` (Prozess je
//!   Satz, Job-Objekt, Offline-Umgebung), Paket P2.
//! * [`openai_speech`] — [`OpenAiSpeechProvider`]: jeder Server mit
//!   `POST /audio/speech` (echtes Chunk-Streaming, Loopback-Pflicht), Paket P2.
//! * [`ollama_speech`] — [`OllamaSpeechProvider`]: Ollama-Sprachausgabe auf
//!   dem OpenAI-Adapter, mit Version-/Modell-/Plattformprüfung, Paket P2.
//!   Alle drei sind `Experimental` + `opt_in`; sie werden von P3 registriert,
//!   nicht hier.
//! * [`policy`] — Einstellungen `tts_providers` (Opt-in, Lizenz-Zustimmungen,
//!   Fernzugriffs-Freigaben), Konfigurationsvorrang (Override > Umgebung >
//!   Einstellungen > Adaptervorgabe) und die Guardrails [`check_use`] — rein,
//!   ohne `AppHandle`. Paket P3a.
//! * [`setup`] — baut die Registry aus den aufgelösten Konfigurationen
//!   (alle fünf Anbieter immer registriert) und hält sie aktuell
//!   ([`ProviderHost`]). Paket P3a.
//! * [`speakers`] — die Sprecher eines Anbieters mit Stimmwechsel (`voice_switching`):
//!   aus seiner Stimmenliste die Namen für Marker `<Name>` im Vorlesetext, die
//!   gehaltene Liste ([`speakers::RosterCache`]) und der Schlüssel, der die
//!   Stimme eines Satzes an ihre Welt bindet. Rein. Paket "mehrere Sprecher".
//! * [`pipeline`] — [`ProviderEngine`]: ein Registry-Anbieter als Engine der
//!   Vorlese-Pipeline (`tts_engine = "provider:<id>"`, Variante
//!   `EngineImpl::Provider`). Paket P3a.
//! * [`preview`] — Probestimme mit Stream-Wiedergabe, Abbruch und dem Ereignis
//!   `tts-provider-preview`. Paket P3a.
//! * [`bench`] — die Benchmark-Schleife (`--tts-bench`, Schnittstelle zum
//!   Python-Harness `scripts/bench/tts`): Korpus, Messung über
//!   `stream_synthesize`, `rows.jsonl`/`env.json`/WAVs. Rein, mit
//!   injizierter Registry. Paket P3b.
//! * [`host`] — der `TtsManager` als Gastgeber (Fish-Synthese, Momentaufnahme
//!   für die Befehle in `commands/tts_providers.rs`). Paket P3a.
//! * `contract_tests` (nur Tests) — `contract_suite`, der Vertragstest-Satz,
//!   den jeder Adapter in seinen Tests aufruft; `test_http` (Fake-HTTP-Server)
//!   und `phonon_fake` (Fake-`ptts`, mit `rustc` gebaut) sind ihre Testhilfen.
//!
//! ## Entscheidung: async ohne `async-trait`, aber dyn-fähig
//!
//! Anders als `TtsEngine` (RPITIT, nicht dyn-kompatibel) liefern die
//! async-Methoden hier `futures_util::future::BoxFuture`; die Registry hält
//! `Arc<dyn TtsProvider>`. Preis: je Aufruf eine Allokation für das
//! Future — bei Satz-Synthese (Sekunden) unerheblich. Der Echtzeit-Audiopfad
//! (Player) liegt hinter dieser Schicht: sie läuft nie im Audio-Callback, sie
//! legt nur fertige Chunks in einen BEGRENZTEN Kanal.
//!
//! ## Verträge, auf die sich Aufrufer verlassen
//!
//! * `synthesize` liefert immer WAV; `opts.format` steuert nur die Chunks
//!   von `stream_synthesize`. `opts.timeout` ist die Gesamtfrist.
//! * Streams: `seq` ab 0 lückenlos, genau ein `is_last` als letzter Chunk,
//!   `request_id` aus dem Token. Wiederholung gibt es nur vor dem ersten
//!   gelieferten Chunk ([`stream_with_retry`]) — sonst hörte man Teile doppelt.
//! * Abbruch/Frist lassen das Provider-Future fallen; Subprozess-Adapter
//!   setzen `kill_on_drop`, damit kein Restprozess bleibt.
//! * Kein Modul dieser Schicht ruft `settings::get_settings(&AppHandle)`
//!   (die Test-Exe startet sonst nicht); Werte reicht der Aufrufer herein.
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Gleichzeitige Aufträge, doppelte `request_id` | Registry-Tabelle hinter `Mutex` (nie über `await` gehalten), doppelte Kennung ist `BadRequest`, [`RequestGuard`] räumt beim Drop auf; Tests `gleichzeitige_auftraege…`, `auftraege_sind_eindeutig…` |
//! | Abbruch mitten im Vorgang (Latenz, zwischen Chunks, im Rückstau, in der Retry-Pause) | [`guarded`]/[`send_chunk`]/[`with_retry`] hören auf den Token; Tests `cancel_von_aussen…`, `mock_bricht_zwischen_den_chunks_ab`, `…rueckstau…`, `retry_stoppt…` |
//! | Abgebrochenes Future (Task-Abbruch) | Guard-Drop entfernt den Auftrag; Mock zählt `in_flight` herunter; Test `ein_abgebrochenes_future…` |
//! | Voller Speicher / langsamer Abnehmer | Stream-Kanal begrenzt (Rückstau statt Wachstum), Überlauf über `sink_stall_count` sichtbar, kein Chunk geht still verloren; Mock deckelt Signal auf 20 s; WAV-Schreiber prüft die 4-GiB-Grenze (`checked_riff_len`) |
//! | Empfänger des Streams weg | `send_chunk` → `Cancelled`, kein Hängen, keine Panik; Vertragsprüfung `check_stream` |
//! | Fehlendes Programm/Gerät | Hüllen melden `available=false` mit `reason`, `synthesize` endet vor jedem Prozessstart mit `Unavailable`; Tests in `builtin` |
//! | Absturz/Hänger eines Kindprozesses oder Anbieters | Frist ergibt `Timeout`; Registry fängt Panik/Hänger in `health_all` (`catch_unwind` + Zeitgrenze); Piper-Prozess endet spätestens nach Frist + 2 s (siehe Grenzen) |
//! | Wiederholung erzeugt doppeltes Audio | `with_retry` nur bei `retryable`, nur ohne gelieferten Chunk, hart begrenzt (1–5 Versuche), Pause abbrechbar |
//! | Kaputtes/abgeschnittenes WAV | `parse_wav` ohne Panik (Präfix- und Müll-Tests), Streaming-Header (`0xFFFFFFFF`) verkraftet |
//! | Einstellungsänderung während eines Auftrags (P3a) | `ProviderHost::sync` ersetzt Anbieter an Ort und Stelle, die Auftragstabelle bleibt; laufende Aufträge halten ihren Anbieter und bleiben abbrechbar; Test `ein_neuaufbau_waehrend_eines_auftrags…` |
//! | Abbruch der Probestimme (P3a) | Synthese-Marke UND Wiedergabe-Flag; kein Audio mehr nach `cancel`, genau ein Abschlussereignis; Tests in `preview` |
//! | Kein Ausgabegerät / Player-Fehler (P3a) | `error` (`unavailable`) mit dem Grund des Players, die Synthese wird beendet; Test `ohne_ausgabegeraet…` |
//! | Zu langer Probetext/zu viel Audio (P3a) | 1000 Zeichen, 8 Mio. Abtastwerte, 120 s Frist; Test `zu_langes_audio_wird_begrenzt` |
//! | Abbruch mitten im Satz beim Vorlesen (P3a) | `ProviderEngine::synthesize` hört auf das Abbruch-Flag, lässt das Anbieter-Future fallen; Test `abbruch_mitten_in_der_synthese…` |
//! | Widerruf einer Freigabe während eines Auftrags (P7n) | Engine, Probestimme und Probe des Ollama-Anbieters prüfen `check_use` gegen den gemeinsamen, aktuellen Stand [`LiveApprovals`] — je Satz bzw. Teil, nicht gegen einen beim Start eingefrorenen; `ProviderEngine::synthesize` bricht den laufenden Satz binnen 200 ms ab, `PreviewService::enforce` beendet die Probestimme; die Befehle der Oberfläche setzen den Stand nach dem Speichern und rufen `refresh_from_settings`; Tests `ein_entzogenes_opt_in_stoppt…`, `ein_zurueckgenommener_fernzugriff…`, `ein_widerruf_beendet…` |
//! | Instanz und Prüfgegenstand aus verschiedenen Ständen (P7n) | Modell und Adresse liefert die Instanz selbst ([`TtsProvider::use_subject`]); Engine, Probestimme und Momentaufnahme holen beides aus EINEM Zugriff auf die Registry; Tests `der_gegenstand_der_pruefung_stammt_aus_der_instanz…`, `die_instanz_meldet_denselben_gegenstand…` |
//! | Guardrail-Umgehung (P3a) | `check_use` an JEDER Nutzung (Vorlesen vor dem Cache, Probestimme, Stimmenliste, Wahl); Konfiguration kann `allow_remote` nicht setzen; Tests in `policy`, `pipeline`, `preview`, `commands::tts_providers` |
//! | Alte/kaputte Einstellungsdatei (P3a) | `tts_providers` mit `#[serde(default)]`; ein kaputtes Feld verwirft `salvage_settings`; Tests in `settings` |
//!
//! ## Grenzen (bewusst, im Report genannt)
//!
//! * Piper läuft über `run_piper_blocking` auf einem Blocking-Thread; ein
//!   Abbruch beendet das Warten sofort, den Prozess aber erst nach seiner
//!   eigenen Frist (Auftragsfrist + 2 s). Ein Satz dauert Sekunden; ein
//!   echtes Kill bräuchte eine Änderung an `piper.rs` (nicht Teil von P1).
//! * Die Synthese von Fish wird von außen hereingereicht (Begründung in
//!   [`builtin`]); ohne Closure ist `FishProvider::synthesize` `InvalidConfig`.
//!
//! dead_code/unused_imports: bis Paket P3 die Schicht anschließt, nutzt sie
//! außer den Tests niemand; die `pub use`-Zeilen unten sind die Oberfläche für
//! P2/P3 — der Vertrag ist trotzdem vollständig und getestet.
#![allow(dead_code, unused_imports)]

pub mod audio8;
pub mod bench;
pub mod builtin;
pub mod contract;
pub mod host;
pub mod mock;
pub mod ollama_speech;
pub mod openai_speech;
pub mod phonon;
pub mod pipeline;
pub mod policy;
pub mod preview;
pub mod registry;
pub mod samples;
pub mod setup;
pub mod speakers;
pub mod wav;

#[cfg(test)]
pub(crate) mod contract_tests;
#[cfg(test)]
pub(crate) mod phonon_fake;
#[cfg(test)]
pub(crate) mod test_http;

pub use audio8::{Audio8Config, Audio8Provider};
pub use builtin::{FishProvider, PiperProvider, SynthFn};
pub use contract::{
    guarded, send_chunk, sink_stall_count, AudioChunk, AudioClip, AudioEncoding, AudioFormat,
    CancelToken, Device, LicenseInfo, ProviderCaps, ProviderDescriptor, ProviderHealth,
    ProviderTier, StreamingMode, SynthesisOptions, TtsError, TtsErrorKind, TtsProvider,
    VoiceDescriptor,
};
pub use mock::{MockFailure, MockProvider};
pub use ollama_speech::{OllamaSpeechConfig, OllamaSpeechProvider};
pub use openai_speech::{is_loopback_url, LicenseConfig, OpenAiSpeechConfig, OpenAiSpeechProvider};
pub use phonon::{PhononConfig, PhononProvider};
pub use pipeline::ProviderEngine;
pub use policy::{
    check_use, license_fingerprint, resolve_provider_configs, LicenseAck, LiveApprovals,
    ProviderOverride, ProviderOverrides, ResolvedConfigs, TtsProvidersSettings, UseGate,
    UseSubject,
};
pub use preview::{PreviewPhase, PreviewService, TtsProviderPreview};
pub use registry::{
    stream_with_retry, with_retry, DeliveryTracker, ProviderRegistry, RequestGuard, RetryPolicy,
};
pub use setup::{build_registry, sync_registry, sync_registry_gated, ProviderHost, ProviderPaths};
pub use speakers::ProviderSpeaker;
