# M2 Audio/STT – Spike-Ergebnisse und Entwurf (Paket P2)

Stand 29.09.2026, lv-architect. Gemessen auf dieser Maschine (Windows 11, i9-13900K, RTX 4090, 64 GB; installierte App lief
nebenher mit ~1,2 GB). Spike-Code und Rohdaten: `C:\Users\wolff\lva-spikes\m2\` (außerhalb des Repos).
Pfade: RS = `apps/local-voice/src-tauri/src`, MM = `RS/managers/meetings`.

**Kernaussagen**
1. Live-Latenz ist kein Rechenproblem: Parakeet v3 (ONNX, CPU) braucht für einen 5-s-Block 213 ms. Die ~20 s Wartezeit
   heute kommen allein vom festen 20-s-Chunker (`recorder.rs:35`). VAD-Endpunkte ergeben geschätzt p95 ≈ 2–3 s.
2. AEC mit `sonora` (reines Rust) funktioniert auf Windows und hält AK6 im synthetischen Test: Gegenseite-Wörter im
   Ich-Transkript 81 % → 0 %. Voraussetzung ist eine gemeinsame Zeitachse für Mikrofon und Loopback; die fehlt heute.
3. **Das Windows-Release rechnet STT nur auf der CPU.** `release-windows.yml:178` baut ohne `gpu-vulkan`. Die installierte
   App hat keine `ggml-vulkan.dll`, und `--list-devices` zeigt nur die CPU. Ohne GPU ist Whisper als Enddurchlauf zu
   langsam (CPU RTF 4 → 60 min Audio in 15 min), QG3 wäre damit nur mit Parakeet als Endmodell erreichbar.
4. Whisper halluziniert auf digitaler Stille („Thank you.", „you"), Parakeet nicht. Ein Stille-Gate ist Pflicht.
5. Qwen3-ASR-1.7B scheitert an 60-s-Blöcken (in transcribe-cpp 0.1.3 bei 256 Token gedeckelt), Blöcke ≤ 20 s gehen.

## 1. Verifikation

| Baustein | gefunden? | Lizenz | Version | Befund |
|---|---|---|---|---|
| `sonora` (AEC3/NS/AGC2, Port von WebRTC M145) | ja, crates.io, github.com/dignifiedquire/sonora | BSD-3-Clause | 0.2.0 (29.07.2026), 0.1.0 (02/2026); 388k Downloads | reines Rust, keine C-Abhängigkeit; Windows-Build 12 s, lief ohne Anpassung. `cpal` 0.18 nur optional (Feature `examples`). **Zeitangabe der Recherche nicht bestätigt:** 4,2 µs je Frame (M4 Max, `target-cpu=native`); hier p50 0,33 ms, p99 4,1–4,5 ms je 10-ms-Frame (Standard-Release-Build). Das sind 3,3 % eines Kerns, genug für Echtzeit, beim Enddurchlauf aber ~2 min CPU je Stunde Audio. Liefert Stats (ERL, ERLE, `delay_ms`) |
| `aec3` (RubyBit) | ja | MIT OR BSD-3 | 0.4.0 (16.09.2026) | nicht gemessen; Rückfall, falls `sonora` verwaist |
| `webrtc-audio-processing` (tonarino) | ja | crates.io „non-standard" (BSD-3 laut Repo) | 2.1.0 (13.05.2026) | braucht Meson + C++; Windows-Build nicht dokumentiert → verworfen |
| Windows-eigene AEC (`IAcousticEchoCancellationControl`) | API ab Build 22621, im `wasapi`-Crate vorhanden | – | – | **nicht getestet.** Treiber-/APO-abhängig, verändert die Rohaufnahme, nicht reproduzierbar messbar → nur als spätere Option |
| Silero VAD im Repo | ja: `resources/models/silero_vad_v4.onnx`, `vad-rs` (git cjpais, ort =2.0.0-rc.12), `audio_toolkit/vad/silero.rs` (30-ms-Frames, Schwelle), `SmoothedVad` | MIT | v4 | direkt wiederverwendbar; 132 µs je Frame (Python-ORT, 1 Thread) |
| Silero VAD v6 | ja, snakers4/silero-vad | MIT | v6.2.3 (23.09.2026); `silero_vad_16k_op15.onnx` 1,3 MB | Eingänge `input`, `state[2,B,128]`, `sr`; 512-Sample-Frames + 64 Kontext → **`vad-rs` lädt v6 nicht** (erwartet `h`/`c`). 120 µs/Frame. Upgrade braucht eigenen ~80-Zeilen-Wrapper auf `ort` (schon im Baum) → nicht M2-kritisch |
| Parakeet-TDT-0.6B-v3 | Katalog: ONNX int8 `parakeet-tdt-0.6b-v3` (installiert) + GGUF Q8 (`handy-computer/…-gguf`) | CC-BY-4.0 | – | beide Engines laufen; ONNX liefert Wortzeiten (`TimestampGranularity`), GGUF über transcribe-cpp |
| Whisper large-v3 / turbo | Katalog: v3 Q5_K_M, turbo Q8 (installiert) | Apache-2.0 (HF-Karte) | – | laufen; Segmentzeiten, keine Wortzeiten |
| Qwen3-ASR 0.6B / 1.7B | Katalog, installiert (HF-Cache) | Apache-2.0 | – | 1.7B auf 60 s: `output truncated at 256 tokens` (status 18); ≤ 20-s-Blöcke laufen |
| transcribe-cpp 0.1.3 | ja | MIT | Features `cuda`, `vulkan`, `metal`, `dynamic-backends`, `openmp` | App-Feature `gpu-vulkan` (Cargo.toml:111) wird im Release **nicht** gesetzt; die CI installiert das Vulkan-SDK (`release-windows.yml:87`), benutzt es aber nicht. Mit `cuda` statisch gebaut: `cudart`/`cublas`/`cublasLt` fehlen beim Linken (LNK2019), eigenes `build.rs` löst das; Bau ~25 min bei `CMAKE_CUDA_ARCHITECTURES=89` |
| CUDA-Laufzeit auf dem Rechner | ja | NVIDIA EULA (weiterverteilbar) | `cublasLt64_13.dll` 481 MB | liegt schon unter `%LOCALAPPDATA%\de.wolffappliedai.localvoiceai\llm\runtime\llm-runtime-windows-x64-cuda\`, weil der LLM-Runtime-Download sie mitbringt |
| FLEURS de_de | ja, HF `google/fleurs` | CC-BY-4.0, nicht gegated | Stand 15.05.2026 | test: 862 Zeilen (TSV 560 KB), `audio/test.tar.gz` 569 MB, dev 228 MB. WAV 16 kHz mono **float32** (für `--transcribe-file` nach PCM16 wandeln). Satz-IDs wiederholen sich (mehrere Sprecher je Satz). Die ersten N Dateien lassen sich per Streaming (`tarfile r\|gz`) holen, ohne das ganze Archiv zu laden |
| Zeitachsen Mikro/Loopback | `loopback_timeline.rs` (Device-Position), `mic_capture.rs` (gezählte Samples) | – | – | Nullpunkt `system.wav` = erster Loopback-Puffer, Nullpunkt `mic.wav` = erstes Mikro-Sample. Loopback startet nach dem Mikro in eigenem Thread (≤ 5 s, `recorder.rs:352-383`). Der Versatz wird **nicht gespeichert**, zwei Geräteuhren driften. Verfügbar sind QPC-Zeitstempel: `wasapi::BufferInfo::timestamp` (100 ns) und `cpal` `InputCallbackInfo::timestamp().capture` |

## 2. Messungen

Befehle: App-CLI `B=apps/local-voice/src-tauri/target/release/local-voice-ai.exe` (Release 27.09., CPU-only);
CUDA-Werte aus dem Spike-Programm `lva-spikes\m2\tcbench` (transcribe-cpp 0.1.3 `--features cuda`, gleiche Bibliothek wie in der App).
Latenz = beste von 3–5 Wiederholungen, Laden getrennt (0,6–1,7 s). WER mit derselben Normierung wie `selftest::normalize_word`.
**WER-Werte sind Rauchtests** (124 bzw. 114 Referenzwörter, TTS bzw. 5 FLEURS-Sätze). Ein Modellvergleich ist damit nicht möglich; den liefert P2b.

| Modell | Engine | Gerät | Block | Latenz / RTF | WER (Stichprobe) | Befehl |
|---|---|---|---|---|---|---|
| Parakeet v3 int8 | transcribe-rs ONNX | CPU | 2 s / 3 s / 5 s | 95 / 121 / 213 ms (RTF 21–25) | – | `$B -f audio/block_5s.wav --model parakeet-tdt-0.6b-v3 --repeat 5 --json` |
| Parakeet v3 int8, 2 Prozesse parallel | ONNX | CPU | 5 s | 280–361 ms je Block | – | 2× obiger Befehl, `--repeat 8` |
| Parakeet v3 int8 | ONNX | CPU | 60 s | 4,3 s (RTF 14) | 6,5 % m8 · 3,5 % FLEURS-5 | `$B -f m8_short_de.wav --model parakeet-tdt-0.6b-v3 --reference … --json` |
| Parakeet v3 Q8 | transcribe-cpp | CPU | 5 s / 20 s / 60 s | 368 ms / 1,4 s / 4,1 s (RTF 14–15) | 6,5 % m8 | `bash run_gpu.sh models/parakeet…Q8_0.gguf pk_gguf_cpu cpu` |
| Parakeet v3 Q8 | transcribe-cpp | CUDA 4090 | 3 s / 5 s / 20 s / 60 s | 58 / 200 / 343–583 / 911 ms (RTF 36–66); VRAM +1,5 GB | 6,5 % m8 | `bash run_gpu.sh … pk_gguf_cuda` |
| Whisper large-v3-turbo Q8 | transcribe-cpp | CPU | 60 s | 15,1 s (RTF 4,0) | 3,2 % m8 | `$B -f m8_short_de.wav --model handy-computer/whisper-large-v3-turbo-gguf/…Q8_0.gguf --json` |
| Whisper large-v3-turbo Q8 | transcribe-cpp | CUDA | 2 s / 5 s / 60 s | 62 / 92 / 580 ms (RTF 32–103) | 3,2 % m8 | `tcbench.exe <turbo.gguf> 2 m8_short_de.wav block_5s.wav block_2s.wav` |
| Whisper large-v3 Q5_K_M | transcribe-cpp | CUDA | 5 s / 20 s / 60 s | 464 ms / 1,2–1,5 s / 4,0 s (RTF 11–15); VRAM +1,9 GB | 3,2 % m8 | `bash run_gpu.sh models/whisper-large-v3-Q5_K_M.gguf wl3_cuda` |
| Qwen3-ASR-1.7B Q5_K_M | transcribe-cpp | CPU | 60 s / 3×20 s | **Fehler** (256-Token-Deckel) / 15,2 s (RTF 4,0) | 1,6 % m8 · 2,6 % FLEURS-5 | `$B -f audio/q20_{0,1,2}.wav --model handy-computer/Qwen3-ASR-1.7B-gguf/…Q5_K_M.gguf --json` |
| Qwen3-ASR-1.7B Q5_K_M | transcribe-cpp | CUDA | 5 s / 20 s | 928 ms / 2,3–3,0 s (RTF 5–8); VRAM +3,1 GB | – | `tcbench.exe <qwen1.7b> 3 audio/q20_*.wav audio/block_5s.wav` |
| Qwen3-ASR-0.6B Q8 | transcribe-cpp | CPU | 60 s | 8,9 s (RTF 6,7) | 5,6 % m8 | wie oben |
| Nemotron-3.5-Streaming Q8 | transcribe-cpp | CPU | 60 s | 3,8 s (RTF 16) | 5,6 % m8 | wie oben |
| Stille 30 s (digital 0) | – | CUDA | 30 s | – | turbo: „Thank you." · large-v3: „you" · Parakeet: leer | `python silence_test.py` |
| sonora AEC3 | – | CPU, 1 Thread | 10 ms | p50 0,33 ms · p99 4,1–4,5 ms · mit NS p99 5,8 ms | – | `aecspike.exe mic.wav render.wav out.wav [1]` |
| Silero v4 / v6 | onnxruntime (Python) | CPU, 1 Thread | 30 / 32 ms | 132 / 120 µs | – | im Spike-Log |

**AEC-Szenen** (TTS-Stimmen Hedda = Ich, Stefan = Gegenseite, 60 s, 2 Doppelsprech-Stellen; `lva-spikes\m2\aec\mix*.py`,
`eval.py`, `leak.py`; Transkription Parakeet ONNX). Leak = Anteil der 43 nur in der Gegenseite vorkommenden Wörter, die im Ich-Transkript stehen.

| Szene | Leak ohne AEC | Leak mit AEC | Ich-WER mit AEC | ERLE (nur Gegenseite spricht) | AEC-Verzögerungsschätzung |
|---|---|---|---|---|---|
| Echo −6 dB, 50 ms, RT60 0,3 s, Rauschen −55 dB | 81 % | **0 %** | 4,8 % | 29,6 dB | 48 ms |
| wie oben, 250 ms | 81 % | **0 %** | 0,0 % | 29,4 dB | 248 ms |
| 50 ms + Rauschunterdrückung | 81 % | 0 % | 0,0 % | 41,6 dB | 48 ms |
| hart: Echo 0 dB, Lautsprecher-Verzerrung (tanh), 150 ppm Uhrendrift, Sprung 50→170 ms bei 30 s | 77 % | **0 %** | 11,9 % | 21–29 dB; ~5 s nach dem Sprung nur 6,8 dB | 160 ms |

Ohne AEC liegt die Ich-WER bei 124–129 %: das Echo landet komplett im Ich-Kanal. Die Nahsprache behält ihren Pegel
(−0,6 dB ohne Doppelsprechen, −2,3 dB beim Doppelsprechen). Grenzen der Messung: synthetischer linearer Raum, TTS-Stimmen,
kleine Stichprobe. Ein realer Lautsprecher-Test gehört in die Abnahme (P2c).

## 3. Entwurf

### 3.1 Live-Pipeline
```
cpal-Callback ─► mic.wav (roh, wie heute)          ┐  Capture-Threads schreiben nur WAV + reichen Samples weiter
WASAPI-Loopback ─► system.wav (roh, wie heute)     ┘  (mit Position/QPC)
            │ (channel, samples, qpc)
            ▼
  „meeting-dsp"-Thread (neu, 1 je Besprechung)
    Aligner: Loopback-Frames nach QPC auf die Mikro-Achse legen (Ringpuffer 2 s)
    AEC (sonora, 10 ms) nur für Kanal 0, wenn Systemton aktiv → mic_aec.wav (neu, 3. Schreiber)
    VAD je Kanal (SileroVad v4, 30 ms) → Segmentierer (Vorlauf 300 ms, Nachlauf 600 ms Stille,
      min 400 ms Sprache, max 15 s mit Schnitt an der leisesten 200-ms-Stelle = chunker::cut_point)
    Signalwächter je Kanal (Pegel/Stille/Nullen/Clipping)
            │ WorkItem::Segment(channel, samples, offset_ms, vad_end_ms)
            ▼
  „meeting-transcribe"-Worker (wie heute: EIN Worker, FIFO, eine Engine)
    transcribe_chunk_resilient → Halluzinationsfilter → append_delta → MeetingEvent::Segments
```
- **Warum ein Worker genügt:** Seriell brauchen zwei Kanäle je 5-s-Segment 2 × 213 ms, parallel 2 × 320 ms mit doppeltem
  RAM für die Engine. Ein zweiter Worker spart also nichts. Getrennte Worker je Kanal lohnen erst ab CPU-RTF < 3.
- **Latenzbudget, Ende der Äußerung bis Event** (Soll p95 ≤ 5 s): VAD-Nachlauf 0,6 s + Warteschlange ≤ 1 Segment des
  anderen Kanals (≤ 0,6 s) + Inferenz 15-s-Segment ≤ 1,1 s (CPU RTF 14) + Speichern/Event < 0,1 s ≈ **2,4 s schlimmster
  Normalfall**. Das ist geschätzt; belegt wird es mit dem Harness aus P2b (AK5). Laptop-CPU (RTF 13 laut Recherche): etwa gleich.
- VAD läuft im DSP-Thread, **nie im cpal-Callback** (Echtzeitregel aus `mic_capture.rs:7-11`). Die WAVs bleiben lückenlos
  und roh. VAD und AEC wirken nur auf das, was zur Transkription geht.
- Pause: Beide Kanäle verwerfen wie heute. Der Aligner setzt nach dem Fortsetzen neu auf (QPC), die AEC wird zurückgesetzt.
- Import und Neu-Transkription behalten vorerst den Chunker. Der Enddurchlauf (3.2) nutzt denselben Segmentierer offline.

### 3.2 Enddurchlauf
- **Wann:** direkt nach `stop()`, als eigener Job in einem Thread. Status bleibt `processing`, bis er fertig ist, dann `ready`.
  `stop()` selbst kehrt zurück, sobald der Live-Worker leer ist (Berührpunkt B4 unverändert).
- **Modell je Hardware** (Einstellung `meeting_final_model`: `auto` | Modell-ID | `off`, Gruppe „Besprechungen", kein neuer Reiter):
  `auto` = GPU-Backend vorhanden und ≥ 4 GB VRAM frei → Sieger aus P2b (Kandidaten Whisper-turbo: 60 min in ~35 s;
  Parakeet GPU: ~55 s; large-v3: ~4 min, sprengt QG3). Nur CPU → **kein zweiter Durchlauf**, weil das Live-Transkript
  (Parakeet) dann schon das Endtranskript ist; es werden nur Segmentgrenzen zusammengeführt. Qwen3-ASR nur mit ≤ 20-s-Segmenten.
- **Eingang:** `mic_aec.wav` (fehlt sie, z. B. nach einem Absturz, rechnet die AEC offline auf mic.wav + system.wav mit
  gespeichertem Versatz) und `system.wav`. Segmente kommen aus dem VAD-Segmentierer (max 25 s), die Transkription läuft je Kanal.
- **Ersetzen:** `store.replace_segments(meeting_id, segs, model, granularity)` in EINER Transaktion: segments_json neu,
  `segment_index` ab 0, `segment_epoch + 1` (M1-Spalte), deltas löschen, `transcripts.model` setzen. Das alte
  Live-Transkript wird vorher als `transcript_live.json` in den Besprechungsordner geschrieben (Rückfall/Benchmark,
  **keine DB-Migration nötig**, respektiert B5). Event: vorhandenes `MeetingEvent::Reset`, danach **ein** `Segments` mit
  allen neuen Segmenten, danach neu `MeetingEvent::TranscriptFinal { meeting_id, epoch, model: Option<String> }`
  (kommt auch bei `off`/CPU, sobald der Live-Stand endgültig ist).
- **Wortzeiten:** `StoredSegment` bekommt `#[serde(default)] words: Option<Vec<WordTime{text,start_ms,end_ms}>>`, gefüllt,
  wenn die Engine sie liefert (Parakeet ONNX/GGUF). Grundlage für M3 (Wortzuordnung zu Sprechern). Alte JSONs laden unverändert.
- **Stabile Belege für M1:** Hält M1 KI-Notizen mit `source_segment_ids` der Epoche n, stellt M2 die reine Funktion
  `remap_sources(old: &[StoredSegment], new: &[StoredSegment], ids: &[u32]) -> Vec<u32>` bereit (größte Zeitüberlappung
  im selben Kanal). M1 kann damit Quellen nach dem Enddurchlauf nachziehen, statt „veraltet" zu melden.
- **Ressourcen:** Das Endmodell wird erst geladen, wenn der Live-Worker fertig ist. Nie zwei große STT-Modelle gleichzeitig,
  RAM-Start-Gate (`process_guard`) vor dem Laden, danach Diktatmodell zurück (Muster `recorder.rs:643-647`). Das LLM für
  KI-Notizen startet erst nach `TranscriptFinal` (R4).

### 3.3 AEC-Platzierung: live im DSP-Thread, Ergebnis als `mic_aec.wav`
- Live, weil das Live-Transkript sonst die Gegenseite doppelt als „Ich" zeigt. Das ist der sichtbarste Fehler.
- Kausal und billig (0,33 ms je 10 ms). Die Datei spart dem Enddurchlauf ~2 min CPU je Stunde. Die Roh-`mic.wav` bleibt
  für Neuberechnungen erhalten.
- **Zeitachse:** Beim ersten Puffer je Kanal den QPC-Zeitstempel speichern (`meetings.metadata_json.timeline =
  {mic_qpc0, sys_qpc0, offset_ms}`). Der Aligner legt Loopback-Frames per QPC auf die Mikro-Achse. Liegt das Echo vor der
  Referenz (negativer Versatz), kann AEC3 es nicht entfernen. Driftet der Versatz um mehr als 20 ms, stellt der Aligner
  per Einfügen/Verwerfen einzelner Referenz-Samples nach; ein Sprung von 120 ms kostete im Test ~5 s geringere Dämpfung.
- Einstellung `meeting_echo_cancellation: auto|on|off` (auto = an, wenn Systemton aufgenommen wird). NS/AGC bleiben aus,
  weil ASR Rohsignal bevorzugt; NS verbessert nur die ERLE-Zahl.
- Sicherheitsnetz (billig): Ein Ich-Segment wird verworfen, wenn ≥ 80 % seiner Wörter in einem Gegenseite-Segment
  ±2 s vorkommen **und** sein RMS mindestens 15 dB unter dem Systemsegment liegt.

### 3.4 Stille-Gate gegen Halluzinationen
Dreistufig: (1) Nur VAD-Segmente gehen an STT. Digitale Nullen und Kanäle unter −70 dBFS erzeugen gar keinen Aufruf.
(2) Engine-Parameter: Whisper ohne Vorgängertext-Konditionierung, sofern transcribe-cpp das anbietet (sonst Befund im Paket).
(3) Textfilter `hallucination.rs`: bekannte Floskeln („Thank you.", „you", „Untertitel …", „Vielen Dank fürs Zuschauen"),
Wiederholungsquote > 50 %, > 25 Zeichen je Sekunde Sprache oder Text bei VAD-Sprachanteil < 20 %. Verworfenes nur zählen,
Inhalte nicht ins Log.

### 3.5 Ausfallwächter
Reines Zustandsmodul `signal_watch.rs` je Kanal, gefüttert mit 30-ms-RMS und Nullzähler aus dem DSP-Thread:
`NoData` (3 s kein Puffer), `DigitalZero` (10 s exakt 0 auf dem Mikro), `Silent` (Mikro < −65 dBFS für 30 s, während
der Nutzer laut Einstellung „spricht" – vereinfacht: immer), `Clipping` (> 1 % Vollaussteuerung in 5 s), `Recovered`.
Der Loopback-Kanal meldet nur `NoData` / `LoopbackDied` (der vorhandene Watchdog bleibt), denn Stille der Gegenseite ist normal.
Event `MeetingEvent::Health { meeting_id, channel, state }` → Warnleiste in `RecorderCard.tsx` (de/en), verschwindet bei `Recovered`.

### 3.6 Recovery (auch `processing`)
`recover_orphans` behandelt `recording` **und** `processing`: WAVs reparieren (wie heute), `duration_ms` aus der WAV-Länge
setzen (#15), dann Status `processing` und ein Nachhol-Job im Hintergrund. Er transkribiert je Kanal den Rest ab dem
größten `end_ms` im Transkript und startet danach den Enddurchlauf (falls aktiv), dann `ready`. Schlägt das fehl, wird der
Status trotzdem `ready` mit Lücken-Platzhalter (Muster `import.rs:356`), damit nichts hängen bleibt.

### 3.7 Benchmark-Werkzeug
- Headless `--simulate-meeting --mic <wav> [--system <wav>] [--realtime] [--final-model <id>] --json [--out f]`. Er schiebt
  WAVs durch **denselben** DSP-Thread und Worker (Quelle „Datei" statt cpal/WASAPI), nur in einer `LVA_MEETINGS_DIR`-Sandbox.
  Ausgabe je Segment: `vad_end_ms` (Audiozeit), `emitted_at_ms` (Wanduhr ab Start) → Latenz = emitted − vad_end im
  Echtzeitmodus. Dazu Live- und End-Transkript. WER wird in Rust über `selftest::SelfTestResult::build` berechnet
  (eine WER-Implementierung).
- `scripts/m2-bench.ps1` (nur ASCII) + `scripts/bench/make_corpus.py`:
  (a) **FLEURS-de**: `test.tsv` und die ersten ~260 Dateien per Streaming, je Satz-ID eine Aufnahme → ≥ 220 Sätze, nach
  PCM16 gewandelt, in `%LOCALAPPDATA%\lva-bench\` (nie ins Repo, nie in den Installer). Zwei Messarten: je Satz
  (Modellgüte) und als lange Datei mit 0,5–1,5 s Pausen durch `--simulate-meeting` (Live-WER inkl. Segmentierung,
  End-WER). Unterlagen der Nachweispflicht (Namensnennung CC-BY) in `docs/m2-evidence/`.
  (b) **Synthetischer Mehrsprecher-Korpus**: SAPI-Stimmen Hedda/Katja/Stefan (später Fish-Stimmen), 3 Szenen à 3–10 min
  mit Überlappung, Echo (Mischer aus dem Spike `mix.py`/`mix_hard.py`), Fachbegriffe/Englisch-Einsprengsel. Referenz steht
  fest, weil die Texte vorgegeben sind.
  (c) **Latenz AK5**: Szene (b) auf 10 min im `--realtime`-Modus → p50/p95/max.
- Ergebnis: `docs/m2-evidence/bench.md` (Tabelle Modell × Engine × Gerät: WER Live/Ende, RTF, VRAM) + JSON-Rohdaten.

### 3.8 GPU-/RAM-Koordination (M2-Anteil)
Sequenz statt Koordinator-Neubau: Live-STT auf der CPU (ONNX, ~0,7 GB RAM), Endmodell erst nach dem Live-Worker,
LLM erst nach `TranscriptFinal`. Gemessener VRAM-Bedarf: Parakeet GGUF +1,5 GB, large-v3 Q5 +1,9 GB, Qwen3-ASR-1.7B
+3,1 GB. Vor jedem Laden eines Endmodells: RAM-Gate plus VRAM-Abfrage (DXGI, in `windows`-Features schon vorhanden).
Reicht der Speicher nicht, fällt die Wahl auf `off` mit Hinweis statt Absturz. Ein allgemeiner Koordinator für STT/LLM/TTS
gehört zu M3/M4 (Diarisierung kommt dazu).

## 4. Berührpunkte mit M1 (bitte vom Planer bestätigen)

| # | Punkt | M2 liefert | Folge für M1 |
|---|---|---|---|
| B1 | Events | `Segments`/`Reset`/`State` unverändert; **neu** `TranscriptFinal{meeting_id, epoch, model}`, `Health{…}` | M1 startet Auto-KI-Notizen auf `TranscriptFinal` statt direkt nach `meetings_stop` (Änderung zu M1-B4). Sonst entstehen die Notizen auf dem Live-Stand und sind nach dem Enddurchlauf veraltet |
| B2 | `position_ms()` | bleibt: Mikro-Achse = Frames von `mic.wav` (roh). `mic_aec.wav` hat dieselbe Länge/Achse | keine |
| B3 | `segment_index` | eindeutig je Epoche; der Enddurchlauf erhöht die Epoche über `replace_segments` | `remap_sources()` von M2 nutzen |
| B5 | Migrationen | **M2 fügt keine meetings.db-Migration hinzu** (Live-Kopie als Datei, `words`/`timeline` in bestehenden JSON-Spalten) | keine Kollision mit M1-Index 2 |
| B7 | recorder.rs | M2 baut `channel_callback`/Sinks um (P2a) | M1-Getter `frames_written()` (B2) bleibt erhalten; wer zuerst merged, merged, der andere zieht nach |

## 5. Fehlerfälle
| Fall | Verhalten |
|---|---|
| Loopback startet nicht / stirbt | wie heute weiter nur mit Mikro; AEC aus; `Health` `LoopbackDied`; Enddurchlauf nur Kanal 0 |
| Loopback startet > 2 s nach dem Mikro | Versatz aus QPC; die Referenz beginnt später, AEC erst ab da aktiv |
| Negativer oder driftender Versatz | Aligner korrigiert; bei \|Drift\| > 500 ms AEC-Reset und Log `aec_realign` |
| Headset (kein Echo) | AEC dämpft nichts, kostet 0,33 ms/Frame; Nahsprache bleibt im Pegel |
| VAD-Modell fehlt/lädt nicht | Rückfall auf 20-s-Chunker (heutiger Pfad) + `Health` `VadUnavailable`, Aufnahme läuft |
| Endmodell nicht installiert / OOM / Fehler | Live-Transkript bleibt gültig, `TranscriptFinal{model: None}`, Hinweis „Enddurchlauf übersprungen" |
| Absturz in `recording`/`processing` | 3.6 |
| Qwen3-ASR ≥ 20 s | Segmentierer begrenzt für dieses Modell auf 18 s |
| Endpunkt-Wechsel (Headset ab) | vorhandener `loopback_died`-Pfad; Neustart des Loopbacks nicht Teil von M2 (Risiko) |
| Voller RAM beim Laden des Endmodells | RAM-Gate verweigert → `off` + Hinweis; Live-Pfad lädt nichts Neues |

## 6. Testplan
- Rust, rein (ohne Gerät): `segmenter` (Stille → 0 Segmente; m8_short_de → Grenzen ±300 ms an den TTS-Pausen; 40 s
  Dauerrede → Schnitte ≤ 15 s), `hallucination` (Floskeln, Wiederholung), `signal_watch` (Zustandsfolgen),
  `echo` (Aligner: Versatz ±, Drift, Pause; AEC auf 20-s-Fixture: ERLE ≥ 20 dB nach 5 s), `final_pass` (`replace_segments`
  atomar, Epoche +1, `transcript_live.json` geschrieben, `remap_sources`), Recovery (`processing`-Orphan → `ready`).
- Integration headless: `--simulate-meeting` auf Fixtures (m8_short_de, Echo-Szene, Stille 600 s) mit JSON-Asserts.
- Frontend: Playwright mit Attrappe: `Health`-Event → Warnleiste; `TranscriptFinal` → Transkript neu geladen.
- Benchmarks (nicht in CI, manuell/Abnahme): `m2-bench.ps1` → AK4, AK5, AK6; realer Lautsprechertest durch Patrick (5 min, Protokoll).

## 7. Paketschnitt

Welle 1 parallel: **P2a**, **P2e**, **P2f** (sowie der reine Modulteil von P2c). Welle 2: P2b, Integration P2c. Welle 3: P2d.
Kritischer Pfad: P2a → P2c → P2d. Richtwert gesamt ≈ 900k–1,2 MTok inkl. Reviews (L ≈ 250k, M ≈ 150k, S ≈ 70k).

**P2a – DSP-Thread + VAD-Segmentierer + Stille-Gate (Live-Pfad)** · lv-coder-xhigh · M · Abh.: –
Scope: neu `MM/dsp.rs` (Thread, `WorkItem::Segment`), `MM/segmenter.rs` (rein), `MM/hallucination.rs`; `MM/recorder.rs`
(`channel_callback` reicht Samples an den DSP-Thread statt an den Chunker, `stop()`/`flush_sink` flusht den Segmentierer,
Rückfall-Chunker), `MM/mod.rs`. VAD = vorhandener `SileroVad` v4 je Kanal. Kein Frontend.
Akzeptanz: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib meetings::` → alle grün, neue Tests für
segmenter/hallucination/dsp (≥ 12). Dazu `local-voice-ai.exe --import-meeting` des Stille-Fixtures unverändert grün (Regression).

**P2b – Simulations-/Latenz-Harness + Benchmark (AK4/AK5-Werkzeug)** · lv-coder · M · Abh.: P2a (Harness), Korpusteil sofort
Scope: `RS/cli.rs`, `RS/lib.rs` (`run_headless_meetings`: `--simulate-meeting`), neu `MM/replay.rs` (Dateiquelle in den
DSP-Thread, Wanduhr-Takt), `scripts/m2-bench.ps1`, `scripts/bench/make_corpus.py` (+ `requirements.txt`: numpy, scipy,
soundfile), `docs/m2-evidence/bench.md`. Sandbox Pflicht.
Akzeptanz: `pwsh apps/local-voice/scripts/m2-bench.ps1 -Quick` → JSON mit `latency_p95_ms`, `wer_live`, `wer_final` je
Modell; `-Full` (FLEURS ≥ 220 Sätze + 10-min-Szene realtime) → Tabelle in `bench.md`. AK5 erfüllt: `latency_p95_ms ≤ 5000`.

**P2c – AEC (`sonora`) + Zeitachsen-Anker + `mic_aec.wav`** · lv-coder-xhigh · L · Abh.: reiner Modulteil keine, Integration P2a
Scope: `Cargo.toml` (`sonora = "=0.2.0"`), neu `MM/echo.rs` (Aligner + AEC-Hülle, rein testbar), `RS/audio_toolkit/audio/loopback.rs`
(QPC aus `BufferInfo::timestamp` an den Callback), `MM/mic_capture.rs` (QPC des ersten Frames), `MM/dsp.rs`
(AEC vor VAD Kanal 0, dritter WAV-Schreiber), `MM/store.rs` nur `metadata_json`-Helfer (keine Migration), `RS/settings.rs`
+ `bindings.ts` + `MeetingsSettings`-Gruppe (`meeting_echo_cancellation`), i18n de/en, Third-Party-Notice.
Neue Fixtures `tests/fixtures/m2_echo_{mic,render}.wav` (60 s, per `scripts/make-m2-fixtures.ps1` aus dem Spike-Mischer).
Akzeptanz: `cargo test … --lib meetings::echo` grün; `local-voice-ai.exe --simulate-meeting --mic tests/fixtures/m2_echo_mic.wav
--system tests/fixtures/m2_echo_render.wav --json` → `ich_far_word_leak ≤ 0.10` und Baseline-Lauf mit `--no-aec` ≥ 0.5
(AK6). Mit P2b: Latenz-p95 steigt durch AEC um < 100 ms.

**P2d – Enddurchlauf + Transkript-Ersatz + Recovery `processing`** · lv-coder-xhigh · L · Abh.: P2a, P2c (Offline-AEC-Rückfall), M1-Epoche (Spalte `segment_epoch`)
Scope: neu `MM/final_pass.rs` (Job, Modellwahl `auto`, `remap_sources`), `MM/store.rs` (`replace_segments`, `words` in
`StoredSegment` mit serde-default), `MM/recorder.rs` (`stop()` → Job starten, `recover_orphans` für `processing` + Nachholen
+ Dauer), `RS/managers/transcription.rs` nur, falls für Wortzeiten nötig (`segments_from_result`), `RS/settings.rs`
(`meeting_final_model`), `bindings.ts`, `MeetingModelSetting.tsx` (zweites Auswahlfeld in derselben Karte), i18n.
Akzeptanz: `cargo test … --lib meetings::` grün inkl. Altdaten-Test (segments_json ohne `words` lädt).
`--simulate-meeting --mic m8_short_de.wav --final-model <turbo-id> --json` → `final.revision_epoch == live+1`,
`transcript_live.json` existiert. `--make-orphan` mit Status `processing` + Neustart → `--dump-meeting` zeigt `ready`,
`duration_ms > 0`, Segmente bis zum Ende. QG3-Teil: 60-min-Szene auf der 4090 (nach P2f) Enddurchlauf ≤ 90 s.

**P2e – Ausfallwächter + Warnleiste** · lv-coder · S · Abh.: – (Hook in P2a-`dsp.rs`, sonst `LevelEmitter`)
Scope: neu `MM/signal_watch.rs`, `MeetingEvent::Health` in `MM/recorder.rs`, `bindings.ts`, `RecorderCard.tsx`, i18n de/en,
Playwright-Test `tests/meeting-health.spec.ts`.
Akzeptanz: `cargo test … --lib meetings::signal_watch` grün (≥ 6 Zustandsfolgen); `pnpm test:playwright meeting-health`
→ Leiste „Mikrofon liefert seit 30 s kein Signal" sichtbar und nach `Recovered` weg.

**P2f – GPU-Backend für STT im Windows-Release (Vulkan), gemessen** · lv-coder · M · Abh.: – · **Owner-Entscheidung nötig**
Scope: `.github/workflows/release-windows.yml` (`args: --bundles nsis,updater -- --features gpu-vulkan` o. ä.), Bundling
von `ggml-vulkan.dll` (`tauri.conf.json`-Ressourcen wie `ggml-cpu-*`), `docs/BUILD-WINDOWS.md`. Lokal braucht es das LunarG-SDK
(Installation durch Patrick/Admin). Rückfall-Option, falls Vulkan auf der 4090 deutlich langsamer ist als CUDA: `ggml-cuda.dll`
als Nachlade-Paket mit den cuBLAS-DLLs, die die LLM-Runtime schon herunterlädt (ABI-Kopplung an die ggml-Version von
transcribe-cpp beachten).
Akzeptanz: `local-voice-ai.exe --list-devices` → Vulkan-Gerät „RTX 4090"; `-f m8_short_de.wav --model <turbo> --device-index <vk>
--json` → `rtf ≥ 30`, `bound_backend` = Vulkan; Installergröße vorher/nachher dokumentiert.

## 8. Offen / Risiken
- **R-GPU:** Ohne P2f bleibt der Enddurchlauf auf der CPU, praktisch = Live-Transkript (Parakeet, FLEURS-de laut Literatur 5,2 %,
  AK4 ≤ 6 % knapp). Vulkan-Tempo auf der 4090 ist nicht gemessen (kein SDK installiert). Die CUDA-Zahlen oben sind die Obergrenze.
- `sonora` ist jung (ein Hauptautor). Deshalb die Version festnageln; der Rückfall `aec3` hat dieselbe Schnittstellenform.
- Echter Raum/Lautsprecher und Bluetooth-Latenz (> 200 ms) sind ungetestet, nur synthetisch geprüft.
- Wanduhr-Latenz ohne UI-Rendering; die React-Anzeige kommt dazu (erwartet < 50 ms, nicht gemessen).
- Silero v6 verschoben: v4 reicht für Endpunkte; die Qualitätsdifferenz v4/v6 ist hier nicht gemessen.
- Endpoint-Wechsel während der Besprechung (Headset ab) → Loopback-Neustart fehlt weiterhin (eigenes Paket nach M2).
