# TTS-Anbieter: Umsetzungsplan

Stand 26.10.2026 · Zweig `feat/tts-provider` (Basis `origin/main` 784d9802) · Goal: `koordination/tts-provider/GOAL.md`
Recherche: [provider-research.md](provider-research.md)

## 1. Ist-Stand (Audit, Kurzfassung)

Pfade relativ zu `apps/local-voice/src-tauri/src` (**R**) und `apps/local-voice/src` (**F**).

| Bereich | Ist | Lücke |
|---|---|---|
| Engine-Naht | `TtsEngine`-Trait + `EngineCaps` + `SynthesisRequest` (R/managers/tts/engine.rs:53-110), Dispatch über `enum EngineImpl {Fish, Piper, Mock}` (mod.rs:147-194) | nicht dyn-fähig, keine Registry, Fish implementiert den Trait nicht |
| Engines | Fish (externer Python-HTTP-Server), Piper (Binary-Subprozess je Satz) | kein Kokoro/XTTS/F5/Phonon |
| Ablauf | Text → Sätze → Synthese Satz N+1 während Satz N spielt (mod.rs:740-913) → `RodioPlayer` (player.rs) | kein Chunk-Streaming, keine Rate/Kanal-Metadaten im Vertrag |
| Abbruch | Generationszähler + `AtomicBool` je Auftrag (`cancel_core`) | — |
| Health | Fish `GET /v1/health`, Piper Pfadprüfung, `availability::runtime_status` fest `{piper, fish}` | nicht generisch |
| Lizenz/Downloads | Piper-Katalog mit Größe, SHA, `license_non_commercial` | Fish ohne Lizenzeintrag |
| UI | Reiter „Ausgabe“ → `ReadAloudTab` → `TtsSettings` (ein Dropdown wählt Engine+Stimme) | Engine-Zweige hart (`=== "piper"`), keine Fähigkeiten im Frontend |
| CLI | `--tts-test --tts-text --tts-voice --tts-out-wav --json` → `bench_fetch` (R/cli.rs, mod.rs:3755) | kein Anbieter-Override, keine Mehrfachläufe/TTFA |
| Agentenbrücke | `tts_page_create`, `tts_render_audio` (R/agent_bridge/catalog.rs, tools.rs) | keine Anbieter-/Stimmen-/Benchmark-Werkzeuge |
| Ollama | LLM OpenAI-kompatibel + `/api/chat`, `/api/tags` | keine Versionserkennung, kein Speech |
| GPU | ASR/LLM mit CUDA→Vulkan→CPU-Selbsttest | TTS: Fish pauschal GPU, kein Rückfall |
| Benchmark | STT-Bench `scripts/bench/` (benchlib: Prozess-Deckel, WER) | kein TTS-Benchmark |

Lokale Lage (26.10.2026): RTX 4090 (Treiber 621.01, CUDA 13.1), Ollama-Client 0.34.2 (Dienst aus), Piper `de_DE-thorsten-high` + 2 EN-Stimmen installiert, Fish `s2-pro` unter `C:\AI\fish-speech`, faster-whisper 1.1.1, Phonon nicht installiert.

## 2. Zielarchitektur

```
UI (Reiter Ausgabe) ─┐       CLI (--tts-providers, --tts-bench)      Agentenbrücke (tts_providers_*)
                     └───────────────┬───────────────────────────────┘
                         commands/tts_providers.rs  (Guardrails: Lizenz-Opt-in, Loopback)
                                     │
                         managers/tts/providers/registry.rs   ProviderRegistry
                                     │ dyn TtsProvider
      ┌──────────────┬──────────────┼───────────────┬──────────────────┬──────────┐
   fish (Hülle)   piper (Hülle)   phonon (ptts.exe)  openai-speech (HTTP) ollama-speech  mock
      │              │                                 ▲ gemeinsamer Client ┘
   bestehende EngineImpl-Pfade unverändert
                                     │
   Vorlese-Pipeline: EngineImpl::Provider(Arc<dyn TtsProvider>) → WAV je Satz → RodioPlayer
```

Grundsätze: Fish/Piper-Code bleibt, die Hüllen delegieren nur. Neue Anbieter erreichen die Vorlese-Pipeline über eine zusätzliche Variante `EngineImpl::Provider`, die `synthesize` aufruft und WAV liefert. Cache-Tag je Anbieter `"<id>:<modell>:<stimme>"`, Fish behält den leeren Tag.

## 3. Anbietervertrag (Semantik, Rust)

```rust
pub enum ProviderTier { Supported, Experimental, BenchmarkOnly, Unavailable }
pub enum StreamingMode { None, Sentence, Chunk }   // ehrlich: nur echte Teil-Audios sind Chunk
pub enum Device { Cpu, Cuda, Metal, Remote }
pub struct LicenseInfo { code: String, weights: String, non_commercial: bool,
                         verified: bool, source_url: String, verified_on: Option<String>, note: String }
pub struct ProviderCaps { languages: Vec<String>, streaming: StreamingMode, formats: Vec<AudioEncoding>,
                          sample_rates: Vec<u32>, cloning: bool, devices: Vec<Device>,
                          platforms: Vec<String>, style_tags: bool }
pub struct ProviderDescriptor { id: String, label: String, tier: ProviderTier, opt_in: bool,
                                caps: ProviderCaps, license: LicenseInfo }
pub struct ProviderHealth { available: bool, reason: Option<String>, hint: Option<String>,
                            device: Option<Device>, version: Option<String>, checked_ms: u64 }
pub struct VoiceDescriptor { id: String, label: String, language: String, gender: Option<String>,
                             license: Option<LicenseInfo> }
pub struct SynthesisOptions { language: String, voice: Option<String>, format: AudioEncoding,
                              speed: Option<f32>, seed: Option<i64>, timeout: Duration }
pub struct AudioFormat { sample_rate: u32, channels: u16, encoding: AudioEncoding } // Pcm16Le | Wav | F32Le
pub struct AudioChunk { request_id: String, seq: u32, format: AudioFormat, data: Vec<u8>,
                        is_last: bool, elapsed_ms: u64 }
pub struct AudioClip { format: AudioFormat, data: Vec<u8> /*WAV*/, duration_ms: u64 }
pub enum TtsErrorKind { Unavailable, InvalidConfig, LicenseBlocked, PrivacyBlocked, Timeout,
                        Cancelled, BadRequest, Backend, Network }
pub struct TtsError { kind: TtsErrorKind, retryable: bool, message: String }

pub trait TtsProvider  // dyn-fähig über futures_util::future::BoxFuture (kein async-trait): Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    async fn health_check(&self) -> ProviderHealth;
    async fn list_voices(&self, language: Option<&str>) -> Result<Vec<VoiceDescriptor>, TtsError>;
    async fn synthesize(&self, text: &str, opts: &SynthesisOptions, cancel: &CancelToken) -> Result<AudioClip, TtsError>;
    /// Standard: ruft synthesize und liefert EIN Chunk (is_last) — Modus bleibt None/Sentence.
    async fn stream_synthesize(&self, text: &str, opts: &SynthesisOptions, cancel: &CancelToken,
                               sink: mpsc::Sender<AudioChunk>) -> Result<(), TtsError>;
    fn cache_tag(&self, voice: Option<&str>) -> String;
}
```

- `cancel(request_id)`: Registry hält `request_id → CancelToken`; Abbruch beendet Subprozess (`kill_on_drop`) bzw. bricht den HTTP-Stream ab. Sink mit begrenzter Kapazität (Backpressure).
- Wiederholung nur bei `retryable` und nur vor dem ersten gelieferten Chunk (sonst doppelte Audioteile).
- Konfigurationsvorrang (deterministisch): CLI-Argument > Umgebungsvariable `LVA_TTS_*` > Einstellungen `tts_providers` > Vorgabe im Adapter. Validierung beim Setzen (URL, Pfad, Bereich).
- Guardrails (in `commands`, nicht im Adapter, damit kein Aufrufer sie umgeht): Anbieter mit `non_commercial || !verified` brauchen `license_ack`; Nicht-Loopback-URL braucht `allow_remote`; beide nur über die UI setzbar, nie über Agentenbrücke/CLI-Agentenmodus.

## 4. Adapter

| id | Stufe | Weg | Streaming | Lizenz (verifiziert 26.10.) |
|---|---|---|---|---|
| `fish` | Supported | Hülle um vorhandenen HTTP-Pfad | Sentence | Code Apache-2.0 / Gewichte s2-pro: Lizenz laut Model Card prüfen |
| `piper` | Supported | Hülle um vorhandenen Subprozess | Sentence | Code GPL-3.0, Stimmen je Katalog (thorsten: CC0 Datensatz) |
| `phonon` | Experimental, opt-in | `ptts.exe --model --lang --voice --quant q8 -o <wav>` je Satz, Version über `ptts --version`, Stimmen über `--list-voices` | Sentence | Code MIT/Apache-2.0; 31m-en Apache-2.0; 40m-* CC BY-NC 4.0 (gated) |
| `openai-speech` | Experimental, opt-in | `POST {base}/audio/speech` mit `response_format=pcm`, Stimmen `GET {base}/audio/voices` | Chunk (gemessen, nicht angenommen) | je Server/Modell konfiguriert |
| `ollama-speech` | Experimental, opt-in | `GET /api/version` ≥ 0.40.11 und Plattform; dann `POST /v1/audio/speech` (Modell konfigurierbar, Vorgabe `phonon`) | Chunk | Modell `phonon:de` CC BY-NC |
| `kokoro`, `xtts-v2`, `f5-tts` | nur Statuszeile | kein Adapter im Standard (Kokoro über `openai-speech` anbindbar, kein Deutsch) | — | siehe Recherche |
| `mock` | nur Tests | deterministische Sinus-/Stille-PCM | Chunk | — |

## 5. Pakete und Reihenfolge

| Paket | Inhalt | Abhängig | Worker |
|---|---|---|---|
| P1 | Vertrag, Registry, Fehlerklassen, Cancel, Mock, Vertragstest-Satz, Hüllen Fish/Piper | — | lv-coder-xhigh |
| P5 | Korpus `de-v1` (≥ 100 Sätze, 10 Kategorien), Normalisierung, Harness `scripts/bench/tts/` (Orchestrierung, Kennzahlen, Aggregation, Berichte, Hörtest-Paket), pytest | — (parallel zu P1) | lv-coder |
| P2 | Adapter phonon / openai-speech / ollama-speech + Fake-EXE/Fake-Server-Tests | P1 | lv-coder-xhigh |
| P3 | Einstellungen `tts_providers`, Guardrails, Tauri-Befehle, `EngineImpl::Provider`, CLI (`--tts-providers`, `--tts-voices`, `--tts-provider`, `--tts-bench`), Agentenbrücke | P2 | lv-coder |
| P4 | UI-Gruppe „Sprachausgabe-Anbieter“ im Reiter Ausgabe (fähigkeitsgesteuert), Probestimme + Abbrechen, Playwright + Screenshot, i18n de/en | P3 | lv-coder |
| P6 | Messlauf (seriell, GPU exklusiv), Ergebnisse, Doku `docs/tts/*`, README | P3, P5 | Planer validiert, lv-coder dokumentiert |
| P7 | Unabhängiges Review (Vertrag, Guardrails, Nebenläufigkeit) | P4, P6 | lv-architect |

Rust-Pakete laufen nacheinander (gemeinsamer `CARGO_TARGET_DIR`, ~7 GB RAM je Testbau).

## 6. Teststrategie

- Unit: Vertragstest-Satz `contract_suite(provider)` (Deskriptor vollständig, Health liefert Grund, Synthese liefert gültiges WAV mit Metadaten, Stream endet mit `is_last`, Abbruch → `Cancelled` ohne Restprozess, Timeout → `Timeout`), Konfigurationsvorrang, Lizenz-/Privacy-Sperren, Versionsvergleich Ollama.
- Integration: echte Anbieter nur, wenn verfügbar (`#[ignore]` + Umgebungsvariable `LVA_TTS_LIVE=1`), sonst Überspringen mit Grund.
- Regression: bestehende `managers::tts`-Tests, Playwright `readaloud-*`, `voice-availability`.
- Benchmark: pytest für Korpus-Schema, Normalisierung, WER/CER, Aggregation (Median/P90/P95), Berichte, Hörtest-Anonymisierung.

## 7. Risiken und Rückweg

- Rückweg: alle neuen Anbieter sind opt-in; ohne Freischaltung verhält sich die App wie bisher. Rückbau = Revert des Zweigs; Einstellungsfeld `tts_providers` ist optional mit `#[serde(default)]`, alte Einstellungsdateien bleiben gültig.
- Deutsche Phonon-Gewichte: gated + CC BY-NC → Owner-Entscheidung (Goal O1).
- Ollama-Speech unter Windows laut Upstream nicht unterstützt → Anbieter meldet „nicht verfügbar“ mit Grund.
- Speicher: Rust-Testbau ~7 GB → `memgate.sh`; GPU-Messungen nur exklusiv.
