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
//! * `contract_tests` (nur Tests) — `contract_suite`, der Vertragstest-Satz,
//!   den jeder Adapter in seinen Tests aufruft.
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

pub mod builtin;
pub mod contract;
pub mod mock;
pub mod registry;
pub mod wav;

#[cfg(test)]
pub(crate) mod contract_tests;

pub use builtin::{FishProvider, PiperProvider, SynthFn};
pub use contract::{
    guarded, send_chunk, sink_stall_count, AudioChunk, AudioClip, AudioEncoding, AudioFormat,
    CancelToken, Device, LicenseInfo, ProviderCaps, ProviderDescriptor, ProviderHealth,
    ProviderTier, StreamingMode, SynthesisOptions, TtsError, TtsErrorKind, TtsProvider,
    VoiceDescriptor,
};
pub use mock::{MockFailure, MockProvider};
pub use registry::{
    stream_with_retry, with_retry, DeliveryTracker, ProviderRegistry, RequestGuard, RetryPolicy,
};
