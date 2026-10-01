# Lokaler Stack für Besprechungen auf Granola-Niveau — Recherche

Stand: 29.09.2026. Ziel: komplett lokal, Open Source, ohne Abo, Hauptsprache Deutsch (plus Englisch, gemischt),
Tauri 2 / Rust / React, Windows 11 zuerst, macOS später, Zielrechner RTX 4090 + i9-13900K + 64 GB, aber auch
schwächere Rechner ohne NVIDIA.

Kennzeichnung der Aussagen:
- **[B]** belegt in der genannten Quelle (Zahl steht dort so).
- **[S]** Schätzung oder Herleitung von mir (Rechnung oder Erfahrungswert), nicht gemessen.
- **[?]** widersprüchlich, nur aus Sekundärquelle oder nicht verifiziert — vor dem Einbau prüfen.

Quellen stehen als Kürzel `Qn` im Text, die URLs am Ende (Abschnitt 9).

---

## 0. Kurzfassung und Ist-Zustand

### 0.1 Empfehlung je Bereich

| Bereich | Empfehlung | Kennzahl | Lizenz | Alternative |
|---|---|---|---|---|
| STT live | **Parakeet-TDT-0.6B-v3** (GGUF Q8, 740 MB) auf VAD-Segmenten | 5,24 % WER de (FLEURS); 13–15x Echtzeit auf reiner Laptop-CPU, 24x auf iGPU | CC-BY-4.0 | Voxtral Realtime (echtes Streaming, nur starke GPU) |
| STT final (Nachlauf) | **Whisper large-v3 (Q8)** bzw. **Qwen3-ASR-1.7B (Q8)** als Nachlauf über die ganze Aufnahme | 4,13 % bzw. 4,25 % WER de (FLEURS, gleicher Prüfstand) | Apache-2.0 / MIT | Cohere Transcribe (5,06 %, aber kein Code-Switching, keine Zeitstempel) |
| Diarisierung | **NVIDIA Nemotron-3-Diarization** (100M, ONNX int8 ca. 120 MB) offline je Kanal; Fallback pyannote community-1 | 9,25 % DER AMI (Headset), 11,14 % AMI (Fernfeld); bis 8 Sprecher | OpenMDW-1.1 | pyannote community-1 (17,0 % AMI, CC-BY-4.0, HF-Gating) |
| Erfassung | Windows: `wasapi`-Crate (vorhanden), Prozess-Loopback optional; macOS: Core-Audio-Tap über `cpal` >= 0.17 | Process-Loopback ab Windows 10 Build 20348; Tap ab macOS 14.6 (cpal) | MIT/Apache | ScreenCaptureKit (`screencapturekit` 10.x) |
| Echo | Mikro = "Ich", Systemton = "Andere"; bei Lautsprechern zusätzlich AEC mit `sonora` (reines Rust) | 4,2 µs je 10-ms-Frame @16 kHz (M4 Max) | BSD-3-Clause | Windows-11-eigene AEC (Build >= 22621) |
| VAD | **Silero VAD v6.x** (Upgrade von v4 im Repo) | 32 ms/Frame, ca. 100k Parameter; P 98,35 % / R 96,62 % | MIT | TEN VAD (Agora-Klausel) |
| LLM Protokoll | **Qwen3.8-27B** (Q5_K_M, 20,9 GB) auf 4090; **Qwen3.5-9B** bzw. **Gemma 4 12B** (Q4) für schwächere Rechner | 27B: ca. 2.660 tok/s Prefill, ca. 40 tok/s Decode auf 4090 -> 25k-Token-Transkript in ca. 10 s Prefill | Apache-2.0 | Qwen3.6-35B-A3B (MoE), Gemma 4 26B-A4B |
| Embeddings | **BGE-M3** (dense, GGUF Q8) Standard; **Qwen3-Embedding-4B** für starke Rechner | MIRACL-de nDCG@10: 57,59 bzw. 62,98 | MIT / Apache-2.0 | EmbeddingGemma-300M (334 MB, Gemma-Bedingungen) |
| Speicher/Suche | **SQLite (rusqlite, vorhanden) + sqlite-vec + FTS5**, Fusion per RRF | brute force reicht bis ca. 100k Chunks | MIT/Apache | LanceDB, wenn > 1 Mio. Chunks |
| Kalender | Phase 1 ICS-URL, Phase 2 Microsoft Graph (BYO-Client-ID), macOS EventKit | kostenlos | — | Google Calendar API (Verifizierungs-Bürokratie) |

### 0.2 Ist-Zustand im Repo (gelesen, nicht verändert)

Wichtig für die Bewertung, weil vieles schon da ist [B: eigener Code-Einblick]:

- `managers/meetings/` existiert (M8): `recorder.rs` mit Dual-Capture (Mikro über `cpal`, Systemton über `wasapi`-Loopback
  des Default-Render-Endpoints, Windows-only), `chunker.rs` (20-s-Blöcke, Schnitt an der ruhigsten Stelle),
  `retranscribe.rs`, `minutes.rs` (LLM mit striktem JSON-Schema, Map-Reduce ab 16.000 Zeichen), `store.rs`, `export.rs`.
- Mikro-Meeting-Aufnahme läuft **ohne VAD** (die WAV muss lückenlos sein), Loopback pflegt Stille-Padding.
  **Keine AEC**, **keine Diarisierung**, kein Per-App-Loopback, kein macOS-Loopback (Stub).
- STT läuft über `transcribe-cpp` 0.1.3 (GGUF/ggml, Vulkan/Metal/CUDA-Features) und `transcribe-rs` 0.3.8 (ONNX).
  Der Katalog enthält bereits: Parakeet-TDT-0.6B-v3, Cohere-Transcribe-03-2026, Qwen3-ASR-0.6B/1.7B,
  Voxtral-Mini-4B-Realtime-2602, Nemotron-3.5-ASR-Streaming, whisper-large-v3/-turbo, Granite-Speech, Canary u. a.
- Der gebündelte `llama-server` ist **llama.cpp b10938** (Windows: Vulkan, CPU, **CUDA 13.3**; macOS Metal). Das ist
  neuer als b10896, das Qwen3.8 verlangt (Q11). LLM-Katalog: Qwen3 0.6/4/8B, Qwen3.5 4/9B, Gemma 3 4B, Gemma 4 E4B/12B.
  **Es fehlt die 24-GB-Klasse** (Qwen3.8-27B, Qwen3.6-35B-A3B, Gemma 4 26B-A4B/31B).
- Silero-VAD liegt als `silero_vad_v4.onnx` im Repo (Crate `vad-rs`); `cpal` steht auf 0.16.0, aktuell ist 0.18.2.

Folge: Der größte Hebel liegt nicht in neuen STT-Modellen, sondern in **Diarisierung, VAD-gesteuerter Segmentierung,
AEC, Nachlauf-Transkription und dem Notizen-Merge**.

---

## 1. STT für lange Besprechungen (Deutsch)

### 1.1 Kandidaten im Vergleich

WER = Wortfehlerrate Deutsch, **FLEURS-de (gelesene Sprache)** im selben Prüfstand `transcribe.cpp` (GGUF Q8_0), damit
die Zahlen untereinander vergleichbar sind (Q4). Geschwindigkeit = Vielfaches der Echtzeit auf einer **AMD Ryzen 7 PRO 4750U**
(8-Kern-Laptop von 2020, Vulkan-iGPU bzw. reine CPU) — ein guter Stellvertreter für "schwächerer Rechner ohne NVIDIA".

| Modell | Größe (Q8 / Q4_K_M) | WER de (FLEURS) | Tempo Ryzen iGPU / CPU | Lizenz | Zeitstempel | Streaming | Bemerkung |
|---|---|---|---|---|---|---|---|
| Whisper large-v3 | 1,67 GB / 1,0 GB | **4,13 %** | 1,67x / 0,81x | Apache-2.0 (MIT) | Segment | nein | robusteste Sprachmischung, Halluzinationen bei Stille |
| Whisper large-v3-turbo | 886 MB / 536 MB | 4,54 % | 2,5x / n. g. | Apache-2.0 | Segment | nein | 4 Decoder-Schichten; bei sauberem Ton fast wie v3 |
| Qwen3-ASR-1.7B | 2,19 GB / 1,32 GB | **4,25 %** | 4,45x / — | Apache-2.0 | keine (Aligner separat) | im Original ja, in ggml nein | 30 Sprachen, Auto-Sprache; Original-Bericht: 3,92 % FLEURS-de (Q7) |
| Cohere Transcribe 03-2026 (2B) | 2,41 GB / 1,56 GB | 5,06 % | 7,67x / 4,59x | Apache-2.0 | keine | nein | **max. 400 s je Aufruf, keine Sprachmischung, keine Auto-Sprache, "eifrig" bei Nicht-Sprache** (Q5) |
| **Parakeet-TDT-0.6B-v3** | 740 MB / 485 MB | 5,24 % | **24,3x / 13–15x** | CC-BY-4.0 | Token (Wort) | nein (v3) | 25 europäische Sprachen, Auto-Sprache, Interpunktion |
| Parakeet "primeLine" (DE-Feintuning) | 740 MB | 5,98 % | wie v3 | CC-BY-4.0 | Token | nein | schlechter als Basis, bevorzugt "ss" statt "ß" (Q4) |
| Canary-1B-v2 | — | 4,10 % (Leaderboard) | RTFx 634 (GPU) | CC-BY-4.0 [?] | nein | nein | 25 Sprachen; Paper nennt CC BY-SA 4.0 — prüfen |
| Voxtral Realtime 4B | 4,73 GB / 2,83 GB | 6,19 % @480 ms, 4,87 % @960 ms, 4,15 % @2,4 s (Q6) | **0,74x** (zu langsam) | Apache-2.0 | nein | **ja (80 ms–2,4 s)** | 4,4B Parameter; nur mit starker GPU sinnvoll |
| Nemotron-3.5-ASR-Streaming 0.6B | 751 MB / 496 MB | 10,33 % | 17,2x (Vulkan) | OpenMDW-1.1 | Token | **ja** (Chunk 1,12 s) | nur als Live-Notlösung |
| Granite Speech 4.1 2B plus | 1,49 GB (Q4) | 8,06 % | — | Apache-2.0 | Wort, Diarisierung experimentell | nein | nicht besser als Parakeet |
| Voxtral Small 24B | groß | 3,01 % (Leaderboard) | RTFx 42 | Apache-2.0 | nein | nein | zu groß für den Zweck |
| Moonshine (alle Varianten) | klein | — | — | MIT | — | ja (Streaming-Varianten) | **kein Deutsch** (nur en, ar, ja, ko, uk, vi, zh) |
| Kyutai STT | 1B/2,6B | — | — | CC-BY | — | ja | **kein offizielles Deutsch** (nur en, en/fr) |
| SenseVoice / GigaAM | klein | — | — | — | — | — | kein Deutsch (zh/en/ja/ko/yue bzw. Russisch) |

Ergänzend, Open-ASR-Leaderboard Mehrsprachen-Spur (Deutsch, mehrere Datensätze gemittelt, Q1):
Whisper large-v3 4,26 % (RTFx 111), Parakeet v3 4,20 % (RTFx 1720), Canary-1B-v2 4,10 % (634), Cohere Transcribe 3,84 % (491),
Voxtral Small 3,01 % (42). Geschlossene Dienste (ElevenLabs Scribe v2 2,27 %, AssemblyAI 2,34 %) liegen vorn, sind aber
nicht lokal. **Alle offenen Spitzenmodelle liegen bei Deutsch eng beieinander (ca. 4–5 %)**; die Unterschiede
zwischen den Prüfständen (FLEURS 4,1–5,2 % vs. Leaderboard 3,8–4,3 %) sind größer als die Unterschiede zwischen
den Modellen. Weitere Einzelwerte aus der Literatur: Whisper large-v3 FLEURS-de 4,08 % (Q7), 5,94 % (Sekundärquelle, andere
Normalisierung) [?]; Whisper-large-v3-turbo Common-Voice-de 3,85 % gegenüber large-v3 3,48 % (Q12).

### 1.2 Was diese Zahlen NICHT sagen

1. **FLEURS/CommonVoice/MLS sind gelesene Sprache.** Besprechungen sind spontan, überlappend, mit Hall und Kompression.
   Der Abstand ist groß: Parakeet v3 hat auf LibriSpeech-clean 1,93 %, auf AMI (Englisch, Besprechungen) **11,31 %** (Q2).
   Für Deutsch existiert **kein belastbarer öffentlicher Besprechungs-Benchmark** (Suche in Q1/Q2/Q3/Sekundärquellen ergebnislos).
   Erwartung [S]: 8–15 % WER auf echten deutschen Besprechungsaufnahmen, Rangfolge der Modelle offen.
2. **Sprachmischung (Deutsch mit englischen Fachbegriffen, Sätze wechselnd):**
   Cohere: laut Modellkarte "code-switching unsupported" und Sprache muss vorgegeben werden (Q5).
   Parakeet v3: erkennt die Sprache automatisch, Code-Switching nicht ausdrücklich dokumentiert (Q2); in einer (Englisch-Yoruba-)
   Code-Switching-Studie war Parakeet unter den besten Systemen, die absoluten Werte blieben aber schlecht (WER 66 %) — für
   Deutsch-Englisch nicht übertragbar [?].
   Whisper: Mehrsprachig trainiert, in der Praxis der robusteste bei Einzelwörtern in der Fremdsprache [S].
   **Das ist die wichtigste Eigenmessung** (siehe 8.2).
3. **Halluzination bei Stille/Rauschen:** Betrifft Encoder-Decoder-Modelle (Whisper, Cohere: "eager to transcribe, even
   non-speech sounds", Q5). Transducer-Modelle (Parakeet TDT) halluzinieren strukturell seltener [S]. Gegenmittel in 1.5.
4. **Längenlimits je Aufruf:** Cohere 400 s (ggml lehnt längere Eingaben ab, Q4), Qwen3-ASR 1.200 s (Q7),
   Parakeet v3 24 min mit voller Attention bzw. 3 h mit lokaler Attention (Q2), Voxtral Realtime ca. 2,9 h (Q4), Whisper 30-s-Fenster
   mit Verkettung. Bei VAD-Segmenten von <= 30 s ist das für alle irrelevant.

### 1.3 Laufzeit für ein 90-Minuten-Meeting (5.400 s)

Gerechnet aus den Faktoren oben [S, Rechnung]:

| Modell / Rechner | Faktor | Dauer für 90 min |
|---|---|---|
| Parakeet v3, Ryzen-Laptop CPU | 13–15x | ca. 6–7 min |
| Parakeet v3, Ryzen-Laptop Vulkan-iGPU | 24,3x | ca. 3,7 min |
| Parakeet v3, M4 Max CPU / Metal | 35–38x / 182x | ca. 2,4 min / 0,5 min |
| Cohere, Ryzen CPU / iGPU | 4,6x / 7,7x | ca. 20 min / 12 min |
| Qwen3-ASR-1.7B, Ryzen iGPU | 4,45x | ca. 20 min |
| Whisper large-v3-turbo, Ryzen iGPU | 2,5x | ca. 36 min |
| Whisper large-v3, Ryzen iGPU / CPU | 1,67x / 0,81x | ca. 54 min / 111 min |
| Whisper large-v3-turbo int8, **RTX 4090** (faster-whisper) | 35,2x | ca. 2,6 min (Q9) |
| Whisper large-v3 int8, **RTX 4090** (faster-whisper) | 19,7x | ca. 4,6 min (Q9) |
| Parakeet v3 / Qwen3-ASR auf 4090 (CUDA, ggml) | nicht gemessen | Schätzung < 1–2 min [S] |
| i9-13900K reine CPU | nicht gemessen | Parakeet Schätzung 25–40x (2–4 min) [S] |

Konsequenz: Auf **schwachen Rechnern ist Whisper large-v3 als Nachlauf nicht praktikabel** (nur turbo/Parakeet); auf der 4090
ist der Nachlauf mit dem "besten" Modell dagegen billig. Der App-Katalog sollte den Nachlauf **je Hardware-Klasse**
automatisch wählen (das vorhandene `estimate.rs`/Ressourcen-Modul der LLM-Verwaltung ist ein Vorbild).

### 1.4 Architektur für Live-Transkript und Endfassung

- **Live:** Statt fester 20-s-Blöcke **VAD-Segmente (3–12 s)** je Kanal an Parakeet v3. Bei 13–15x Echtzeit auf einer alten
  Laptop-CPU ist ein 10-s-Segment in < 1 s fertig; sichtbare Verzögerung 2–4 s nach Satzende [S]. Das ist "gut genug" für ein
  Notizblock-Gefühl, ohne ein echtes Streaming-Modell zu brauchen.
- **Echtes Streaming (optional, nur starke GPU):** Voxtral Realtime, 4,87 % de bei 960 ms (Q6). Auf dem Ryzen-Laptop
  0,74x -> unbrauchbar; auf Apple M4 Max 6–7x, auf einer 4090 sehr wahrscheinlich problemlos [S]. Nemotron-3.5-Streaming
  (10,33 %) ist nur die Notlösung mit echter Streaming-Latenz.
- **Nach dem Meeting:** Nachlauf über die **gesamte gespeicherte WAV** (Diarisierung zuerst, dann Transkription je Sprecher-Turn)
  mit dem besten für die Hardware zumutbaren Modell. Das Grundgerüst (`retranscribe.rs`) existiert.
- **Zeitstempel vs. Sprecherzuordnung:** Wer je Turn/Segment transkribiert, braucht keine Wortzeitstempel — der Turn ist die
  Einheit. Parakeet (Token-Zeitstempel) ist nur nötig, wenn Wörter innerhalb eines Segments zwischen Sprechern aufgeteilt
  werden sollen. Cohere und Qwen3-ASR (ggml) liefern keine Zeitstempel -> nur mit "diarisieren, dann je Turn transkribieren".

### 1.5 Halluzinationsschutz

1. **Silero-VAD vor jedem Aufruf** und nur Sprachsegmente ans Modell (whisper.cpp hat eine eingebaute Silero-VAD-Option
  `--vad`, GGML-Modell `silero-v6.2.0`, Q10); pro Kanal, damit ein stiller Mikrokanal nichts erzeugt.
2. Bei Whisper: `condition_on_previous_text=false` (bzw. Äquivalent) und `no_speech_threshold`/Log-Prob-Filter [S]; v3 halluziniert
   häufiger als v2 (Q12: 5–10 statt 1–3 Fälle in 45 min bei Filterung [B, Diskussionsstand]).
3. WASAPI-Falle: Das Flag `AUDCLNT_BUFFERFLAGS_SILENT` garantiert keine genullten Puffer — ohne `fill(0)` wird Müll transkribiert (Q22).
   Der vorhandene Code paddet Stille; die Nullung explizit prüfen.
4. Nachfilter [S]: Segmente mit hoher Wiederholungsrate, sehr kurzer Sprechzeit bei langem Text oder Standardphrasen
   ("Untertitel der Amara.org-Community" u. ä.) verwerfen.

### 1.6 Rust-Anbindung

| Weg | Reifegrad | Bemerkung |
|---|---|---|
| **`transcribe-cpp` 0.1.3** (vorhanden; Rust-Bindings des Projekts `handy-computer/transcribe.cpp`, MIT) | im Repo produktiv | 70+ Modelle als GGUF (Whisper, Parakeet, Cohere, Qwen3-ASR, Voxtral, Nemotron, Sortformer-Diarizer …), Backends CUDA/Vulkan/Metal/HIP + tinyBLAS-CPU; Quantisierungen F16/Q8_0/Q6_K/Q5_K_M/Q4_K_M; jedes Modell gegen die Referenz WER-getestet (Q4) |
| `transcribe-rs` 0.3.x (ONNX: Parakeet, Canary, Cohere, Moonshine, SenseVoice, GigaAM; MIT) | im Repo produktiv | ONNX Runtime; CPU auf Windows (kein DirectML wegen AVX2-Absturz, siehe Cargo.toml) |
| `whisper-rs`, `sherpa-onnx` (offizielle Rust-API), `ort` | verfügbar | **`sherpa-rs` wurde am 06.06.2026 archiviert** (Q13); `ort` steht laut Speakrs weiter auf 2.0.0-rc.x (pre-release) |

**Empfehlung:** Bei `transcribe-cpp` bleiben; alle relevanten Modelle sind dort schon Katalogeinträge.

### 1.7 Empfehlung STT

- **Live:** Parakeet-TDT-0.6B-v3 (Q8). Begründung: Schnellstes Modell mit brauchbarem Deutsch, Token-Zeitstempel, Auto-Sprache,
  klein, in beiden Backends vorhanden, CC-BY-4.0 (Namensnennung nötig).
- **Endfassung:** je Hardware — 4090/starke GPU: Whisper large-v3 (Q8) **oder** Qwen3-ASR-1.7B (Q8); schwache Rechner:
  Parakeet v3 (oder Whisper turbo, wenn GPU vorhanden). Die Auswahl zwischen Whisper und Qwen3-ASR ist eine **Eigenmessung**
  (8.2), weil beide bei FLEURS gleichauf liegen und die Sprachmischung entscheidet.
- **Alternative:** Cohere Transcribe, falls die Aufnahmen rein einsprachig deutsch sind (sehr gutes Leaderboard-Ergebnis 3,84 %),
  mit VAD-Pflicht und Sprachvorgabe.
- **Nicht empfohlen:** Moonshine, Kyutai, SenseVoice, GigaAM (kein Deutsch), Granite (schlechter), Voxtral Small 24B (zu groß).

---

## 2. Sprechertrennung (Diarisierung)

### 2.1 Kandidaten

DER-Werte sind **nicht direkt vergleichbar** (verschiedene Datensätze, Kragen/Collar, Behandlung überlappender Sprache).
pyannote gibt "ohne Kragen, Überlappung zählt mit" an; NVIDIA-Werte sind je Datensatz definiert.

| Modell / Bibliothek | DER (Auswahl) | Sprecherzahl | Größe / Laufzeit | Lizenz | Online? | Anbindung |
|---|---|---|---|---|---|---|
| **NVIDIA Nemotron-3-Diarization** (100M, 2026) | AMI-Headset 9,25 %, AMI-Fernfeld 11,14 %, DIHARD III 12,73 %, CALLHOME 9,10 %; VoiceArena 14,7 % (nächster: 19,3 %) (Q14) | **bis 8** | ca. 120 MB (ONNX int8, Community-Export) [?]; CPU 78 min in 35 s, 57 min in 23 s (Einzelbericht, Q15) | **OpenMDW-1.1** (kommerziell/nicht-kommerziell erlaubt) | ja: Profile 0,32 / 0,64 / 1,04 / 30,4 s | ONNX-Export (onnx-community); NeMo/GGUF; **noch nicht im `transcribe.cpp`-Katalog** (dort nur Sortformer 4spk-v2.1) |
| NVIDIA Streaming Sortformer 4spk-v2.1 | AMI-IHM 14,59 % (F32), 14,23 % (F16), 14,73 % (Q8) (Q4) | **max. 4**, Labels in Ankunftsreihenfolge | 139 MB (Q8); M4-CPU ca. 101x Echtzeit | NVIDIA Open Model License | ja | **in `transcribe.cpp`** (GGUF) |
| pyannote **community-1** (2025) | AMI-IHM 17,0 %, AMI-SDM 19,9 %, AliMeeting 20,3 %, DIHARD3 20,2 %, VoxConverse 11,2 % (Q16); Legacy 3.1: 18,8 / 22,7 / 24,5 / 21,4 / 11,2 | unbegrenzt (Clustering) | Python; Rust-Port **speakrs**: VoxConverse 7,0–7,1 %, RTX 4090 CUDA 59x (121x "fast"), M4 Pro CoreML 529x (Q17) | Gewichte **CC-BY-4.0**, aber **Hugging-Face-Gating** (Konto + Token); Code MIT | nein (offline) | speakrs (Rust, `ort` rc.12, Windows-Support unklar [?], Lizenz nicht geprüft) |
| sherpa-onnx (pyannote-segmentation-3.0 + 3D-Speaker/NeMo/WeSpeaker + Clustering) | Praxis: 16,6 % falsch zugeordnet bei 2 Sprechern/78 min, **53,7 % bei 5 Sprechern/57 min (erkannte nur 2)** (Q15, Einzelbericht) | Clustering-abhängig | int8-Segmentierung 1,5 MB; RTF 0,11–0,12 auf CPU (Q18) | sherpa-onnx Apache-2.0; pyannote-Segmentierung MIT [?] | nein | offizielle Rust-API (sherpa-rs archiviert) |
| DiariZen (WavLM) | AMI 10,2 (mit 0,25 s Kragen) | — | — | Gewichte **CC BY-NC** | nein | **ausgeschlossen (nicht-kommerziell)** |
| Reverb-Diarization v1 | — | — | 9,1 MB | **nicht-kommerziell** | nein | **ausgeschlossen** |
| MOSS-Transcribe-Diarize, Granite `--diarize`, Multitalker-Parakeet (nur Englisch) | Multitalker: cpWER 19,35 % AMI | bis 4 | 0,6–3 GB | verschieden | teils | im Katalog; für Deutsch unklar/nur Englisch |

### 2.2 Bewertung

- **Nemotron-3-Diarization** ist derzeit die beste offene Wahl: 8 Sprecher, einheitlich niedrige DER (auch Fernfeld),
  Streaming **und** Offline, permissive Lizenz **ohne Gating** (Modell lässt sich ohne HF-Token herunterladen/spiegeln),
  klein genug für CPU. Trainiert auf 21+ Sprachen (u. a. David-AI-Sammlungen), Deutsch nicht ausgewiesen; Diarisierung ist
  überwiegend akustisch und sprachneutral [S].
- **Schwächen:** Modell ist Monate alt, ONNX-Export nur von der Community, **kein dokumentierter ONNX-Runtime-Code und keine
  Zustandsverwaltung für Streaming** (Q14), Zählgenauigkeit ab 5 Sprechern nur ca. 60–78 % (Q14). Die Rust-Anbindung ist die
  offene Arbeit: Mel-Features (10 ms) + Chunk-Zustand selbst bauen oder auf die künftige `transcribe.cpp`-Aufnahme warten.
- **Sortformer 4spk-v2.1** ist die Fallback-Stufe, die **heute schon** über `transcribe-cpp` lauffähig ist (max. 4 Sprecher,
  Labels können bei knappen Entscheidungen im Stream kippen, Q4).
- **pyannote community-1** ist bei Anzahl/Zuordnung sehr gut und bietet "exclusive diarization" für sauberes Zusammenführen mit
  ASR-Zeitstempeln, aber: HF-Gating als Distributions-Hürde für ein Open-Source-Projekt (Spiegelung unter CC-BY-4.0 rechtlich
  prüfen), Python-Stack, Rust nur über junge Ports.
- **Klassisches sherpa-onnx-Clustering** hat auf echten Besprechungen mit mehr als 2 Sprechern enttäuscht (Einzelbericht,
  aber deckungsgleich mit der Erfahrung, dass Embedding-Clustering bei vielen Sprechern die Sprecherzahl unterschätzt) [?].

### 2.3 Wie man es einbaut

1. **Kanaltrennung zuerst:** Mikrokanal = "Ich" (ohne Diarisierung), Systemton-Kanal wird diarisiert -> "Andere 1..n". Das
  verkleinert das Problem (typisch 1–4 Fremdsprecher) und spart Rechenzeit. Für **Hybrid-Besprechungen** (mehrere Personen
  vor einem Mikro) den Mikrokanal ebenfalls diarisieren, schaltbar.
2. **Offline-Profil (30,4 s Kontext)** für die Endfassung, optional Niedriglatenz-Profil (1,04 s) für Live-Etiketten später.
3. **Zusammenführung:** Für jedes ASR-Segment den Sprecher mit größter Zeitüberlappung wählen; enthält ein Segment einen
  Sprecherwechsel, an der Diarisierungsgrenze teilen und Teilstücke erneut transkribieren (bei Parakeet billig).
4. **Namen:** Teilnehmerliste aus dem Kalendertermin vorschlagen ("Andere 1 = ?"), Nutzer bestätigt einmal. **Stimm-Fingerabdrücke
  über Besprechungen hinweg** (wie OpenWhispr, Abschnitt 6) sind möglich (Embeddings 3D-Speaker/WeSpeaker/TitaNet), aber
  **biometrische Daten (DSGVO Art. 9)** — nur lokal, nur mit ausdrücklicher Einwilligung, löschbar [S].
5. Gesprächsanteile für den vorhandenen Protokollkopf (`stats.rs`) fallen dabei ab.

### 2.4 Empfehlung Diarisierung

- **Standard:** Nemotron-3-Diarization (ONNX int8) offline je Kanal, Integration als eigener Spike (Aufwand grob 2–4 Tage [S]).
- **Sofort verfügbar:** Sortformer 4spk-v2.1 über `transcribe-cpp` als erste Stufe/Rückfall (max. 4 Sprecher).
- **Alternative:** pyannote community-1 über `speakrs`, falls Nemotron in eigenen Tests bei Deutsch enttäuscht.
- **Ausgeschlossen:** DiariZen, Reverb (nicht-kommerzielle Lizenzen).

---

## 3. Audio-Erfassung, Echo, VAD

### 3.1 Windows

| Frage | Befund |
|---|---|
| Systemton (alles) | WASAPI-Loopback des Render-Endpoints — **im Repo bereits umgesetzt** über das `wasapi`-Crate. `cpal` öffnet einen Input-Stream auf einem Ausgabegerät laut Praxisberichten (seit 0.13.1) ebenfalls im Loopback-Modus (Q19, nicht in der README belegt). |
| Nur eine App | **Process Loopback** (`ActivateAudioInterfaceAsync` mit `AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK`), "include/exclude target process tree"; Mindestversion **Windows 10 Build 20348** (Q20). Erhält die App keinen Ton, kommt Stille. Das `wasapi`-Crate (0.24.0, 12.08.2026, MIT) bietet `new_application_loopback_client(pid, include_tree)` und ein Beispiel `record_application` (Q21). |
| Endpoint-Wechsel | Loopback ist an den Endpoint gebunden; beim Wechsel (Headset an-/abstecken) muss neu gestartet werden (Gerätebenachrichtigungen des Crates). Der vorhandene Code meldet Endpoint-Verlust über ein Fehlerflag. |
| Stillen Endpoint | Windows liefert bei Stille **keine** Puffer statt Stille-Puffer; ereignisgesteuerte Schleifen hängen -> Stille selbst einfügen (Q22). Das Repo tut das schon ("padded silence frames"). |

Process Loopback lohnt sich nur, wenn man **nur die Besprechungs-App** abgreifen und Musik/Benachrichtigungen ausschließen will.
Die PID-Ermittlung (Teams/Zoom/Browser, Kindprozesse -> "include tree") ist der Aufwand. Für Version 1 reicht der Endpoint-Loopback
plus Hinweis "Benachrichtigungen aus".

### 3.2 macOS

| Weg | Berechtigung | Befund |
|---|---|---|
| **Core Audio Process Tap** (`AudioHardwareCreateProcessTap`, `CATapDescription`) | nur "Systemaudio-Aufnahme" (Q23), kein Bildschirmaufnahme-Recht; Info.plist `NSAudioCaptureUsageDescription` | seit macOS 14.2/14.4; Rust: **`cpal` >= 0.17 (20.12.2025) unterstützt Loopback auf macOS > 14.6**, 0.18.0 (06.06.2026) behebt UB, stille Ausfälle und UID-Kollisionen des Aggregat-Geräts (Q24). Pro-App-Taps erfordern eigenes Objective-C/Swift (z. B. Swift-Sidecar). |
| ScreenCaptureKit | Bildschirmaufnahme-Recht | Crate `screencapturekit` 10.0.3; Ton "nicht isoliert" (Hintergrundgeräusche), Berechtigung schwerer zu erklären (Q25) |

Fallen aus der Praxis (Q22): verwaister Tap nach hartem Prozessende liefert beim nächsten Start **still nur Nullen**
(Aufräumen per Signalhandler), Berechtigung wird erst bei `AudioDeviceStart` durchgesetzt (Tap-Erzeugung gelingt ohne Recht).

**Empfehlung macOS:** `cpal` auf 0.18.x anheben und dessen Loopback nutzen; `cpal` 0.18 hat Breaking Changes (`play()` explizit,
einheitlicher `cpal::Error`, `StreamConfig` per Wert, Standard 48 kHz, Q24) — Anhebung als eigene, kleine Aufgabe einplanen.

### 3.3 Echo, "Ich" vs. "Andere"

- **Billige Sprecherzuordnung:** Mikro = "Ich", Systemton = "Andere". Granola macht es genauso; das Repo hat das Schema schon
  (`label_for_channel`).
- **Problem Lautsprecher:** Ohne Kopfhörer landet der Systemton im Mikro -> doppelte Transkripte und falsch als "Ich" markierte
  Fremdsprecher. Wichtiger als Modellfragen; **das Repo hat keine AEC** [B: eigener Code-Einblick].
- Optionen:

| Option | Aufwand | Qualität | Anmerkung |
|---|---|---|---|
| Kopfhörer erkennen/Hinweis | klein | umgeht das Problem | Endpoint-Typ "Kopfhörer" auswerten [S] |
| **`sonora`** (reine-Rust-Portierung von WebRTC M145: AEC3, Rauschunterdrückung, AGC2; 0.2.0 vom 29.07.2026; BSD-3-Clause) | mittel | sehr gut; volle Testparität mit der C++-Referenz (2.400+ Tests); 4,2 µs je 10-ms-Frame @16 kHz (M4 Max) (Q26) | Windows-x86_64 (SSE2/AVX2) und macOS-ARM64 unterstützt; Projekt jung (ca. 85 Sterne), aber ohne Meson/C++-Build |
| `aec3`-Crate (0.3.2, 12.08.2026) | mittel | ähnlich (Port von AEC3) | Alternative zu `sonora` |
| `webrtc-audio-processing` (tonarino, BSD-3-Clause; Meson/Ninja + C++ nötig, Windows-Bau nicht dokumentiert) | hoch | gut | 330 Sterne; Build-Kette auf Windows unklar (Q27) |
| **Windows 11 eigene AEC**: `IAcousticEchoCancellationControl::SetEchoCancellationRenderEndpoint`, Kommunikationsmodus/"Voice Clarity" | mittel | gut, hersteller-/treiberabhängig | **Windows Build >= 22621** (Q28); das `wasapi`-Crate hat `AcousticEchoCancellationControl` |
| Heuristik: Mikrosegmente verwerfen, deren Text dem Systemton im selben Zeitfenster stark ähnelt | klein | grob | Sicherheitsnetz zusätzlich zur AEC [S] |

AEC3 braucht als Referenz den **Systemton-Kanal in der Zeitachse des Mikros**; das vorhandene `loopback_timeline.rs`
liefert eine zeitachsen-korrekte Loopback-Spur. AEC3 schätzt die Verzögerung selbst.

### 3.4 VAD

| | Silero VAD v5/v6 | TEN VAD |
|---|---|---|
| Größe | ca. 100k Parameter, ONNX; 16 kHz, Frame 512 Samples = 32 ms | 306 KB Bibliothek |
| Genauigkeit | P 98,35 %, R 96,62 % (akademischer Vergleich) | P 96,32 %, R 97,87 % |
| Tempo | schnell | RTF 0,015 (Ryzen); ca. 32 % weniger Rechenzeit |
| Lizenz | **MIT** | Apache-2.0 **mit Zusatzklausel** (kein Einsatz gegen Agora-Interessen) |

(Q29). Silero v6.2.x (Release 6.2.2 mit separatem 16-kHz-`sequence`-Modell) ist aktuell; whisper.cpp bündelt sie als GGML.
**Empfehlung:** Silero v6 (Upgrade vom im Repo liegenden v4; prüfen, ob `vad-rs` die neuere Modellform lädt). Zwei Einsätze:
(a) Segmentgrenzen für die Transkription setzen, (b) Halluzinationsschutz. Das Mikro-Meeting-WAV bleibt lückenlos; VAD wirkt nur
auf die Kopie für ASR.

---

## 4. Lokale LLMs für Protokoll und "Enhanced Notes"

### 4.1 Modelle (Stand 09/2026)

| Modell | Typ / Größe | Kontext | GGUF-Größe (Q4_K_M / Q5_K_M / Q6_K) | Lizenz | Bemerkung |
|---|---|---|---|---|---|
| **Qwen3.8-27B** (Aug 2026) | dicht, 27B | 262k | **17,4 / 20,9 / 23,9 GB** (Q8 29,1 GB passt nicht) | Apache-2.0 | 4090: ca. 2.660 tok/s Prefill, ca. 40 tok/s Decode (Community-Messung, Q4-KV, 260k-Kontext-Konfiguration; Q30); braucht llama.cpp >= b10896 (Q11) |
| Qwen3.6-27B / -35B-A3B (Jul 2026) | dicht / MoE (3B aktiv) | 262k | 27B ca. 18 GB (4-bit); 35B-A3B ca. 22–23 GB | Apache-2.0 | 35B-A3B: ca. 68–122 tok/s Decode auf 4090 (streuend, Q31); alle Experten im VRAM nötig (ca. 21–22 GB) -> knapp neben STT/KV |
| Qwen3.5-9B / -4B (Mär 2026) | dicht | 262k | 9B ca. 5,5 GB (Q4); 4B ca. 3 GB [S] | Apache-2.0 | **bereits im Katalog**; hybride lineare Aufmerksamkeit (ca. 75 % Gated-DeltaNet) -> kleiner KV-Cache (Q32) |
| Gemma 4 (2026): E2B/E4B/12B/26B-A4B/31B | dicht (12B, 31B), MoE (26B-A4B: 25,2B gesamt, 3,8B aktiv), "effektiv" (E2B/E4B) | E-Modelle 128k; 12B/26B/31B **256k** | Q4_0: 2,9 GB (E2B) … 17,5 GB (31B) (Q33) | **Apache-2.0** | 140+ Sprachen vortrainiert; MRCR-v2 8-Needle @128k: 31B 66,4 %, 26B-A4B 44,1 %, 12B 43,4 %, E4B 25,4 % — Langkontext-Abruf fällt bei kleinen Modellen stark ab |
| Mistral Small 4 (Mär 2026) | MoE 119B/6B aktiv | 256k | zu groß (ca. 70 GB) | Apache-2.0 | für Heim-GPU ungeeignet; Ministral 3 (3B/8B/14B, Apache-2.0) ist die passende Größe [?] |
| Llama-Familie | — | — | — | Meta-Lizenz (nicht OSI) | nicht empfohlen (Lizenz, keine Stärke bei Deutsch belegt) |

**Deutsch-Qualität:** Ich habe **keine belastbare deutschsprachige Vergleichsmessung** für Zusammenfassungen gefunden (Suche
nach EuroEval/German-Leaderboards ergebnislos). Die Familien werben mit 140–201 Sprachen. Deshalb: **blinder Eigenvergleich**
(8.2) mit 3–5 echten Besprechungen, Kandidaten Qwen3.8-27B, Gemma 4 31B/26B-A4B, Qwen3.5-9B, Gemma 4 12B, Ministral 3 14B.

### 4.2 VRAM-Planung (RTX 4090, 24 GB)

- STT und LLM laufen **nacheinander** (erst Transkript, dann Protokoll) -> STT-Speicher (0,7–3 GB) ist beim LLM-Lauf frei.
  Das vorhandene LLM-Ressourcenmodul (`managers/llm/resources.rs`, `estimate.rs`) plant das bereits.
- Qwen3.8-27B: **Q5_K_M (20,9 GB)** lässt ca. 3 GB für KV-Cache; wegen der hybriden Architektur ist der KV-Cache klein,
  und 25k Token sind für 24 GB unkritisch (Community: 260k Kontext mit Q4-KV auf 4090, Q30). Q6_K (23,9 GB) nur mit knappem Kontext.
- Schwächere Rechner (8–12 GB oder nur CPU): Qwen3.5-9B Q4 (5,5 GB) oder Gemma 4 12B Q4 (ca. 7 GB [S]); ohne GPU Qwen3.5-4B
  bzw. Gemma 4 E4B. Nur-CPU-Prefill für 25k Token ist langsam (Schätzung 60–150 tok/s -> 3–7 min [S]) — das ist die
  Hauptkosten der Funktion auf schwachen Rechnern.

### 4.3 Kontextlänge und Strategie

- 60–90 min Deutsch = ca. 9.000–12.000 Wörter -> **ca. 15.000–25.000 Token** [S: Deutsch ca. 1,8–2,1 Token/Wort mit
  Qwen/Gemma-Tokenizern]. Inklusive Sprecherlabels und Zeitmarken eher 20–30k.
- **Einzeldurchlauf (Long-Context)** ist für Modelle >= 9B und Kontext <= 32k vernünftig: eine Sicht auf das Gespräch, keine
  Dopplungen, Entscheidungen und Aufgaben über Kapitel hinweg verknüpfbar. Zeitaufwand 4090: ca. 10 s Prefill + Decode 1,5–2k Token
  (ca. 40–50 s bei 40 tok/s).
- **Map-Reduce** für <= 4B-Modelle oder CPU-Rechner: Blöcke (ca. 8k Token) an Themen-/Pausengrenzen mit Überlappung, je Block
  strukturierte Teilnotizen, zweite Stufe fusioniert. Gesamt-Prefill wird **nicht** kleiner (jeder Token wird einmal gelesen),
  nur der Spitzenspeicher; dafür sind Querbezüge schwächer. Das Repo macht das schon ab 16.000 Zeichen — der Schwellenwert
  sollte an **Modellgröße und gemessene Prefill-Geschwindigkeit** gekoppelt werden (bei erster Nutzung einmal messen und
  speichern).
- Empfehlung: **Regel** "wenn Prefill-Zeit des Gesamttranskripts < 60 s und Modell >= 9B -> Einzeldurchlauf, sonst Map-Reduce".

### 4.4 Strukturierte Ausgabe

- `llama-server` wandelt eine **Teilmenge von JSON-Schema** in GBNF um (`response_format: {type: "json_schema", …}` bzw. `json_schema`
  bei `/completion`, Q34); Grenzen bei manchen Regex-Mustern. Das Repo nutzt das bereits (`minutes.rs`).
- Qwen3.x: Denkmodus für Protokolle **abschalten** (`--chat-template-kwargs '{"enable_thinking":false}'`), empfohlene Nicht-Denk-
  Parameter laut Hersteller: temperature 0,7, top_p 0,8, top_k 20, presence_penalty 1,5 (Q35); für Zusammenfassungen niedriger
  ansetzen (0,2–0,4) [S]. **Vermeiden: CUDA 13.2** (führt zu unsinniger Ausgabe, Q35) — das Repo nutzt 13.3.
- **Enhanced Notes (Granola-Muster) [S]:** Eingabe = (1) Eigene Stichpunkte mit Zeitstempel, (2) Transkript mit Sprecher und
  Zeitmarken, (3) Terminmetadaten (Titel, Teilnehmer). Aufgabe: Jeden Stichpunkt anhand des Transkripts an der passenden
  Stelle **ausbauen**, ohne Neues zu erfinden; zusätzlich fehlende Entscheidungen/Aufgaben ergänzen, gekennzeichnet
  ("aus Transkript") gegenüber den eigenen Punkten ("eigene Notiz"). Jede KI-Aussage trägt `[mm:ss]` als Beleg -> prüfbar und
  Grundlage für "Zitat anzeigen". Schema-Felder: `zusammenfassung`, `themen[]`, `entscheidungen[]`, `aufgaben[]{text, verantwortlich, faellig}`,
  `offene_fragen[]`, `notiz_erweiterungen[]{notiz_id, text, belege[]}`.

### 4.5 Empfehlung LLM

- **4090-Klasse:** Qwen3.8-27B Q5_K_M, Denkmodus aus, Einzeldurchlauf, JSON-Schema. Alternative: Gemma 4 26B-A4B oder 31B (Blindtest
  entscheidet), Qwen3.6-35B-A3B wenn Geschwindigkeit wichtiger ist.
- **Mittel/schwach:** Qwen3.5-9B bzw. Gemma 4 12B; darunter Qwen3.5-4B/Gemma 4 E4B mit Map-Reduce. Katalog um die 24-GB-Klasse erweitern.
- **Nicht empfohlen:** Llama (Lizenz), Mistral Small 4 (zu groß).

---

## 5. Suche und Chat über alle Besprechungen

### 5.1 Embedding-Modelle (mehrsprachig, Deutsch)

MIRACL-de nDCG@10 und Eigenschaften (Q36, Sekundärquelle [?] — nur ein Vergleich, mit eigenen Fragen gegenprüfen):

| Modell | Parameter | Dim. | Max. Länge | MIRACL-de | MMTEB | Lizenz | GGUF / llama.cpp |
|---|---|---|---|---|---|---|---|
| Qwen3-Embedding-4B | 4B | 2.560 | 32k | **62,98** | 69,45 | Apache-2.0 | ja |
| Qwen3-Embedding-8B | 7,6B | 4.096 | 32k | 61,92 | 70,58 | Apache-2.0 | ja |
| jina-embeddings-v5-text-small | 677M | — | — | 57,99 | — | **CC BY-NC** (kommerziell separat) | — |
| **BGE-M3** | 568M | 1.024 | 8.192 | 57,59 | 59,56 | **MIT** | ja (dense; Sparse/ColBERT nicht in llama.cpp) |
| EmbeddingGemma-300M | 308M | 768 (MRL bis 128) | 2.048 | 56,60 | — | Gemma-Bedingungen | ja, **Q8 334 MB** |
| Qwen3-Embedding-0.6B | 0,6B | 1.024 (MRL 32–1024) | 32k | 54,21 | 64,33 | Apache-2.0 | ja, **Q8 639 MB** |
| Granite Embedding 311M R2 | 311M | — | 32k | 50,97 | — | Apache-2.0 | — |
| multilingual-e5-large-instruct | 560M | 1.024 | **512** | 43,37 | — | MIT | — (Längenlimit ungeeignet) |

Qwen3-Embedding erwartet auf der **Abfrageseite** eine Anweisung (`Instruct: … Query: …`), Dokumente ohne; ohne Anweisung
1–5 % Verlust (Q37). Server: `llama-server -m … --embedding --pooling last` (Qwen3, Q37); BGE-M3 nutzt CLS-Pooling [S].

**Empfehlung:** **BGE-M3 (dense, Q8-GGUF)** als Standard — beste deutsche Wiederauffindung unter den kleinen Modellen, 8k-Kontext,
MIT. **Qwen3-Embedding-4B** als "Premium" für starke Rechner. EmbeddingGemma nur, wenn die Größe (334 MB) zählt; Gemma-Bedingungen sind
weniger sauber für ein Open-Source-Repo als MIT/Apache. Sparse-Vektoren von BGE-M3 sind mit llama.cpp nicht verfügbar — ersetzt
der Volltextindex (5.2).

### 5.2 Speicher und Suche

| Baustein | Befund |
|---|---|
| **rusqlite 0.37 (bundled)** | im Repo; SQLite mit FTS5 dabei |
| **sqlite-vec** 0.1.9 (31.03.2026; Alphas bis 0.1.10-alpha.4 vom 18.05.2026) | pre-v1 (Brüche möglich), **nur Brute-Force**, keine eingebaute Quantisierung; ca. 1,7 s p50 bei 1 Mio. x 1.024 (Q38) -> linear ca. 170 ms bei 100k [S] |
| **FTS5** | BM25 eingebaut; für Deutsch `unicode61 remove_diacritics 2` oder **`trigram`** (gegen Komposita wie "Kundenbetreuungsgespräch"); der eingebaute Porter-Stemmer ist Englisch [S] |
| Tantivy | eigenständiger Volltext mit Snowball-Stemmern (auch Deutsch) und BM25; nur nötig, wenn FTS5 nicht reicht |
| LanceDB | Rust-nativ, HNSW/IVF-PQ + Tantivy-Volltext + DataFusion; 6 ms p50 bei 1 Mio. (Q38); schwere Abhängigkeit (Arrow/DataFusion) [S] — erst ab > 1 Mio. Chunks |

Größenrechnung [S]: 100.000 Chunks x 1.024 Dim. x f32 = 410 MB; als int8 102 MB; mit MRL auf 512 Dim. (Qwen3) f32 205 MB. Bei
Tausend Besprechungen à 100 Chunks ist Brute-Force ohne ANN-Index ausreichend.

**Empfehlung:** Eine SQLite-Datei (bestehende Meetings-DB oder Nachbar-DB): Tabelle `chunks(meeting_id, start_ms, end_ms, speaker, text)`,
FTS5-Tabelle (trigram) und `vec0`-Tabelle; Abfrage = Top-k je Verfahren + **Reciprocal Rank Fusion**, dann LLM-Antwort mit
Quellenangabe (Besprechung + Zeitmarke). Chunking: 200–400 Token entlang von Sprecherwechseln/Pausen, jeder Chunk trägt Titel,
Datum, Teilnehmer als Kontextzeile ("Contextual Retrieval") [S]. Indexierung nach Protokollerzeugung; danach Embedding-Server
wieder entladen (0,6–2,5 GB).

**Alternative:** LanceDB, falls Vektorsuche zum Engpass wird (unwahrscheinlich).

---

## 6. Open-Source-Alternativen und Lehren

| Projekt | Stack | Lizenz | Stern-/Aktivität | STT | Diarisierung | LLM | Plattform | Stärken / Schwächen |
|---|---|---|---|---|---|---|---|---|
| **Meetily** (Zackriya) | Tauri + Rust + Next.js | MIT | 31,2k Sterne, 661 Commits, aktiv (Q39) | Whisper + Parakeet (ONNX) | **nicht in der Community-Edition** (PRO geplant) | Ollama, Claude, Groq, OpenRouter, eigene | Win/macOS/Linux | + Live-Transkript, Mikro+System gemischt, Import/Re-Transkription; − Open-Core-Aufteilung, Windows-Installer ohne CUDA, AVX2 nötig |
| **Anarlog** (ex Hyprnote -> Char, seit 03.05.2026) | Tauri v2 + React/TS + Rust | **MIT** (früher GPL) | 9,4k Sterne, 9.651 Commits (Q40) | lokal v. a. Apple Speech (macOS); Cloud-Anbieter optional | über Anbieter | Ollama/LM Studio, eigene Schlüssel | macOS reif, **Windows/Linux Beta (08/2026)** | + beste Notizblock-UX, Kalender, Markdown-Export, lokale SQLite; − lokale STT unter Windows schwach, Kommerzielles ausgeklammert |
| **Vibe** (thewh1teagle) | Tauri + Rust + whisper.cpp | MIT | 7,6k Sterne, aktiv (Q41) | Whisper, Parakeet v3, Nemotron 3.5 | ja | Ollama/Claude | Win/macOS/Linux | + Stapelverarbeitung, viele Export-Formate, CLI/HTTP-API; − kein Live-Notizblock |
| **OpenWhispr** | Electron + React 19, better-sqlite3 | MIT | 8,8k Sterne, 2.293 Commits (Q42) | whisper.cpp + Parakeet (sherpa-onnx) | **lokal, mit Stimm-Fingerabdrücken über Besprechungen** | Cloud oder lokal | Win/macOS/Linux | + Kalender (Google/Microsoft/Apple), Auto-Erkennung Zoom/Teams; − Electron, Cloud-Sync/Teams-Funktionen |
| **OpenOats** | Swift, nur macOS | MIT | 2,6k Sterne, 572 Commits (Q43) | Parakeet, Whisper, Qwen3-ASR | beidseitig (2 Kanäle) | Ollama/OpenRouter | macOS 14.2+ | + **Live-Vorschläge aus eigenen Notizen per semantischer Suche**; − nur macOS |
| **Open Granola** | Tauri/Rust + Vue, whisper.cpp + llama.cpp | Apache-2.0 | 6 Sterne, 6 Commits (Proof of Concept) (Q44) | Whisper turbo/Parakeet | fehlt | Qwen3-4B | Win/macOS/Linux | + "kein Netzwerkcode" per CI erzwungen, CoreAudio-Tap/WASAPI/PipeWire; − unreif |
| **Minutes** (silverstein) | Rust | MIT | 1,5k Sterne, 2.263 Commits (Q45) | Whisper | `diarize-streaming`-Sidecar | — | Desktop/CLI | + MCP-Server (34 Werkzeuge), Obsidian/Logseq, Einwilligungsprotokoll |
| **Scriberr** | Go + React + Python, Docker | MIT | 3,1k Sterne, **Entwicklung pausiert** (Q46) | Parakeet/Canary/WhisperX | pyannote/NeMo | Ollama/OpenAI-kompatibel | Server, kein Desktop | + Chat mit Transkript, Ordnerüberwachung; − kein Live/Desktop |
| **Amurex** | Chrome-Erweiterung | **AGPL-3.0** | 2,9k Sterne (Q47) | über Backend | — | über Backend | Meet/Teams im Browser | passt nicht (Browser-Erweiterung, AGPL) |

**Lehren für uns:**
1. **Doppelt erfassen (Mikro + System) ist Standard**; das Repo ist hier weiter als viele (mit Stille-Padding, Absturz-Recovery, Einwilligungs-Tor).
2. **Diarisierung ist der Unterscheider** — Meetily-Community hat sie nicht, Vibe/OpenWhispr schon. Hier lässt sich gezielt überholen.
3. **Stimm-Wiedererkennung über Besprechungen** (OpenWhispr) ist ein Komfortgewinn, aber DSGVO-sensibel (2.3).
4. **Termin als Kontext** (Titel, Teilnehmer, Start automatisch erkennen) machen Anarlog und OpenWhispr; das ist der Sprung von "Recorder" zu "Notizblock".
5. **Live-Vorschläge aus Notizen** (OpenOats) sind ein Alleinstellungsmerkmal, aber ein Folgeschritt (Abschnitt 5 liefert die Basis).
6. **"Kein Netzwerk" per CI** (Open Granola) ist eine billige, glaubwürdige Zusicherung für den Datenschutz-Anspruch.
7. **MCP-/Markdown-/Obsidian-Export** (Minutes) — bei vorhandenem Export kaum Aufwand.
8. **Vermeiden:** Open-Core-Spaltung, Cloud-Standardanbieter, Electron-Gewicht.

---

## 7. Kalender lokal ohne Cloud-Abo

| Weg | Kosten | Aufwand | Verlässlichkeit | Hinweise |
|---|---|---|---|---|
| **ICS-URL** (Outlook.com "veröffentlichen", Google "geheime Adresse im iCal-Format", Nextcloud, iCloud) | 0 | klein (HTTP-Abfrage + ICS-/RRULE-Parser) | **Aktualität 3–24 h+** je nach Anbieter (Q48), Wiederholungen müssen selbst expandiert werden; bei Firmen-M365 ist Veröffentlichen oft **vom Administrator gesperrt** [?] | Null Registrierung, kein OAuth — guter Start |
| **Microsoft Graph** (`/me/calendarView`), delegiertes `Calendars.Read` | 0 | mittel | sehr gut, Push/Abfrage nahezu aktuell | Öffentlicher Client mit **PKCE und Loopback-Redirect `http://localhost`**, "Allow public client flows" = Ja, kein Client-Geheimnis; persönliche Konten: Mandant `consumers`, `requestedAccessTokenVersion=2`; `Calendars.Read` braucht **keine Admin-Zustimmung**, kann aber durch Mandantenrichtlinien eingeschränkt sein (Q49). **Herausgeber-Verifizierung** für mandantenübergreifende Apps ist gratis, verlangt aber Partner-Programm-Konto und verifizierte Domain (Q50). |
| **Google Calendar API**, `calendar.readonly` (sensibler Bereich) | 0 | mittel | sehr gut | Desktop-Client (Loopback, PKCE), Client-Secret liegt im Binary (nicht vertraulich). **Status "Testing": Refresh-Token nur 7 Tage, Nutzerkappe (100), Warnbildschirm.** Produktivstatus braucht Verifizierung: Startseite, Datenschutzerklärung, YouTube-Demovideo, Domain-Nachweis, 3–5 Werktage, kostenlos (Q51) |
| **macOS EventKit** | 0 | klein | sehr gut, nutzt alle in der Kalender-App eingerichteten Konten | Rust: `objc2-event-kit`; Berechtigung "Kalender"; für macOS der Königsweg |
| Windows-Termin-API (`Windows.ApplicationModel.Appointments`) | 0 | mittel | **nicht empfohlen [?]**: "Mail und Kalender" wurde am 31.12.2024 abgeschaltet; ob das neue Outlook den Store noch befüllt, konnte ich nicht belegen | |
| Outlook-COM | 0 | mittel | **nicht empfohlen**: nur klassisches Desktop-Outlook, läuft aus | |

**Empfehlung (gestuft):**
1. **Phase 1:** ICS-URL-Abo (eine URL in den Einstellungen) -> "Nächster Termin" und Teilnehmer als Kontext.
2. **Phase 2 (Windows):** Microsoft Graph mit **Bring-your-own-Client-ID** (Nutzer registriert die App im eigenen Mandanten, kein Verifizierungsaufwand für das Projekt; eigene Nutzung Patricks funktioniert damit sofort).
3. **Phase 2 (macOS):** EventKit.
4. Google nur als "BYO-Client-ID + unverified", solange keine Verifizierung stattfindet; ansonsten über ICS abdecken.
5. Keine Kalenderdaten verlassen den Rechner; Token im Windows-Anmeldeinformationsspeicher/Keychain.

---

## 8. Risiken, Eigenmessungen, Reihenfolge

### 8.1 Die größten technischen Risiken

| # | Risiko | Warum | Gegenmaßnahme |
|---|---|---|---|
| 1 | **Sprechertrennung bei realen deutschen Besprechungen mit > 3–4 Sprechern und Überlappung** | Nemotron-3 ist jung, ONNX nur Community-Export, keine Streaming-Zustandsdokumentation, Zählgenauigkeit ab 5 Sprechern ca. 60–78 %; klassisches Embedding-Clustering versagte im Einzelbericht (53,7 % falsch bei 5 Sprechern) | Kanaltrennung (nur Fremdkanal diarisieren), Sortformer als Rückfall, Nutzer-Korrektur der Sprecherzahl, Eigenmessung (8.2) |
| 2 | **STT-Qualität auf spontanem Deutsch mit Englisch-Einsprengseln** | alle Zahlen stammen aus gelesener Sprache; Cohere kann keine Sprachmischung; AED-Modelle halluzinieren bei Stille | VAD-Pflicht, Nachlauf mit Whisper/Qwen3-ASR, Eigenmessung; Nutzer-Glossar (Eigennamen) als Nachkorrektur über das LLM |
| 3 | **Echo/Kanalvermischung** | Ohne AEC erscheinen Fremdsprecher im Mikrokanal doppelt und als "Ich"; Endpoint-Wechsel, Tap-Lebenszyklus und Berechtigungen auf macOS | `sonora`-AEC oder Kopfhörer-Hinweis, Endpoint-Wechsel-Behandlung, Test mit Lautsprecher-Szenario |
| 4 | Deutsche Protokollqualität der LLMs unbelegt | keine öffentliche Messung gefunden | Blindtest, Belegzeitmarken je Aussage, Schema-Zwang |
| 5 | VRAM-/RAM-Konkurrenz und Laufzeit auf schwachen Rechnern | Nur-CPU-Prefill von 25k Token dauert Minuten; Whisper large-v3 bei 0,8–1,7x Echtzeit | hardwareabhängige Modellwahl, Prefill-Messung, Map-Reduce, klare Wartezeit-Anzeige |
| 6 | Abhängigkeiten: `ort` (2.0-rc), sqlite-vec (pre-v1), `sherpa-rs` archiviert, `cpal`-0.18-Brüche | Wartungsrisiko | Versionen festnageln, kleine Adapterschicht |
| 7 | Rechtliches | Aufzeichnung von Gesprächen ohne Einwilligung (§ 201 StGB), Stimm-Fingerabdrücke = biometrisch | Einwilligungs-Tor (vorhanden), Hinweis-Banner, lokale Speicherung, Löschfristen (`retention.rs` vorhanden); rechtliche Prüfung ausstehend [S] |
| 8 | Modell-Lizenzen bei Weitergabe | CC-BY-4.0 (Parakeet, pyannote) verlangt Namensnennung; NVIDIA Open Model License (Sortformer); Gemma-Bedingungen (EmbeddingGemma); Canary-Lizenz abweichend angegeben | Lizenzliste im Katalog, Namensnennung in "Über" |

### 8.2 Eigenmessung vor dem Festlegen (kleines Testset)

Nur so lassen sich die offenen Punkte schließen; Aufwand [S] ca. 1 Tag inklusive Referenztexte:

1. **Testset:** 3 echte, freigegebene Aufnahmen: (a) reines Deutsch, 2 Sprecher, (b) Deutsch mit vielen englischen Fachbegriffen,
   (c) 4–5 Sprecher mit Überlappung. Je 10 min, Referenztranskript von Hand korrigiert (Rechenzeit sparen: Vorabtranskript mit
   Whisper large-v3, nur korrigieren).
2. **STT:** Parakeet v3, Whisper large-v3 (+turbo), Qwen3-ASR-1.7B, Cohere -> WER, Sprachmisch-Fehler, Halluzinationen bei Stille.
3. **Diarisierung:** Nemotron-3 (ONNX), Sortformer 4spk, pyannote community-1 -> DER/Sprecherzahl-Fehler.
4. **LLM:** Qwen3.8-27B, Gemma 4 31B/26B-A4B, Qwen3.5-9B, Gemma 4 12B -> blinde Bewertung von Fakten-Treue, Deutsch, Belegen.
5. **Embeddings:** 20 selbstformulierte Fragen -> Trefferquote@5 für BGE-M3 vs. Qwen3-0.6B/4B.
Abbruchregel: Liegen die ersten beiden Kandidaten gleichauf, nicht die ganze Reihe fahren (Budgetregel).

### 8.3 Sinnvolle Reihenfolge (grob)

1. Silero v6 + VAD-Segmentierung + Nullungs-Prüfung (Halluzinationsschutz) — klein, sofort wirksam.
2. Sortformer-Diarisierung über `transcribe-cpp` als erster sichtbarer Sprung (Sprecherlabels im Transkript), danach Nemotron-3-Spike.
3. AEC (`sonora`) + Endpoint-Wechsel-Robustheit.
4. Nachlauf-Transkription je Hardware-Klasse; Katalog um 27B/35B-A3B/Gemma 4 26B/31B erweitern.
5. Enhanced-Notes-Prompt/Schema mit Belegzeitmarken; Kalender Phase 1 (ICS).
6. Embedding + FTS5 + sqlite-vec, Chat mit Quellenangabe.
7. macOS-Loopback (`cpal` 0.18), Kalender Phase 2.

---

## 9. Quellen

Q1 Open-ASR-Leaderboard-Paper: https://arxiv.org/html/2510.06961
Q2 Parakeet-TDT-0.6B-v3 Modellkarte: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
Q3 Canary-1B-v2 und Parakeet-TDT-0.6B-v3: https://arxiv.org/html/2509.14128v1
Q4 transcribe.cpp und Modellseiten (Parakeet v3, Whisper large-v3/-turbo, Cohere, Qwen3-ASR-1.7B, Nemotron 3.5, Voxtral Realtime, Sortformer, Granite, Multitalker):
   https://github.com/handy-computer/transcribe.cpp — z. B. https://raw.githubusercontent.com/handy-computer/transcribe.cpp/main/docs/models/parakeet-tdt-0.6b-v3.md,
   .../whisper-large-v3.md, .../whisper-large-v3-turbo.md, .../cohere-transcribe-03-2026.md, .../qwen3-asr-1.7b.md,
   .../nemotron-3.5-asr-streaming-0.6b.md, .../voxtral-realtime.md, .../diar_streaming_sortformer_4spk-v2.1.md,
   .../multitalker-parakeet-streaming-0.6b-v1.md, .../granite-speech-4.1-2b-plus.md, .../parakeet-primeline.md
Q5 Cohere Transcribe: https://huggingface.co/CohereLabs/cohere-transcribe-03-2026 und https://huggingface.co/blog/CohereLabs/cohere-transcribe-03-2026-release
Q6 Voxtral Realtime: https://arxiv.org/html/2602.11298v2
Q7 Qwen3-ASR Technical Report: https://arxiv.org/html/2601.21337v1
Q8 transcribe-rs: https://github.com/cjpais/transcribe-rs
Q9 Whisper-Benchmarks RTX 4090 (faster-whisper): https://www.runpod.io/articles/guides/best-gpu-for-whisper
Q10 whisper.cpp (Silero-VAD eingebaut): https://github.com/ggml-org/whisper.cpp und https://github.com/ggml-org/whisper.cpp/issues/3003
Q11 Qwen3.8-27B GGUF (b10896): https://huggingface.co/bartowski/Qwen3.8-27B-GGUF
Q12 Whisper turbo/large-v3 (Diskussionen): https://github.com/openai/whisper/discussions/2363, https://github.com/openai/whisper/discussions/1762;
    primeline-Vergleich: https://huggingface.co/primeline/whisper-large-v3-turbo-german
Q13 sherpa-rs (archiviert 06.06.2026): https://github.com/thewh1teagle/sherpa-rs; sherpa-onnx: https://github.com/k2-fsa/sherpa-onnx
Q14 Nemotron-3-Diarization: https://huggingface.co/nvidia/Nemotron-3-Diarization/blob/main/README.md, https://huggingface.co/blog/nvidia/nemotron-diarization,
    ONNX: https://huggingface.co/onnx-community/Nemotron-3-Diarization-ONNX
Q15 Praxisvergleich sherpa-onnx vs. Nemotron 3: https://github.com/jankeesvw/omarchy-meeting-recorder/pull/1
Q16 pyannote community-1: https://huggingface.co/pyannote/speaker-diarization-community-1, https://www.pyannote.ai/blog/community-1
Q17 speakrs (Rust, community-1-Pipeline): https://github.com/attevon-llc/speakrs
Q18 sherpa-onnx Diarisierung: https://k2-fsa.github.io/sherpa/onnx/speaker-diarization/models.html
Q19 cpal: https://github.com/RustAudio/cpal
Q20 Windows Process Loopback (Beispiel): https://github.com/microsoft/windows-classic-samples/tree/main/Samples/ApplicationLoopback
Q21 wasapi-Crate: https://docs.rs/wasapi/latest/wasapi/
Q22 Fallen bei System-Audio (Rust/Tauri): https://dev.to/baurzhan_zhetenov_442c4cd/5-weird-bugs-i-hit-capturing-system-audio-cross-platform-rust-tauri-5el8
Q23 Core Audio Taps (Apple): https://developer.apple.com/documentation/CoreAudio/capturing-system-audio-with-core-audio-taps
Q24 cpal-Changelog: https://raw.githubusercontent.com/RustAudio/cpal/master/CHANGELOG.md, Releases: https://github.com/RustAudio/cpal/releases
Q25 System-Audio macOS/Windows im Überblick: https://www.recall.ai/blog/how-to-get-access-to-system-audio; screencapturekit: https://crates.io/crates/screencapturekit
Q26 sonora: https://github.com/dignifiedquire/sonora; aec3: https://crates.io/crates/aec3
Q27 webrtc-audio-processing: https://github.com/tonarino/webrtc-audio-processing
Q28 IAcousticEchoCancellationControl: https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iacousticechocancellationcontrol
Q29 VAD-Vergleich: https://huggingface.co/TEN-framework/ten-vad, https://github.com/snakers4/silero-vad/releases
Q30 Qwen3.8-27B auf RTX 4090: https://github.com/vikesh-c/Qwen-3.8-27B-RTX-4090-3090-llama-cpp, https://github.com/sergiuszm/ninfer-4090
Q31 Qwen3.5/3.6-35B-A3B auf 4090: https://willitrunai.com/blog/qwen-3-5-on-rtx-4090-performance, https://github.com/outsourc-e/qwen36-4090-recipes
Q32 Qwen 3.5–3.8 Übersicht: https://codersera.com/blog/qwen-3-5-complete-guide-2026/, https://unsloth.ai/docs/models/qwen3.5
Q33 Gemma 4: https://ai.google.dev/gemma/docs/core/model_card_4, https://unsloth.ai/docs/models/gemma-4
Q34 llama.cpp Grammatiken/JSON-Schema: https://github.com/ggml-org/llama.cpp/blob/master/grammars/README.md, https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
Q35 Qwen3.6 lokal (Sampling, CUDA-13.2-Warnung): https://unsloth.ai/docs/models/qwen3.6
Q36 Embedding-Vergleich Deutsch (Sekundärquelle): https://wz-it.com/en/blog/best-embedding-models-german/; Qwen3-Embedding: https://arxiv.org/pdf/2506.05176; EmbeddingGemma: https://arxiv.org/pdf/2509.20354
Q37 Qwen3-Embedding-0.6B-GGUF: https://huggingface.co/Qwen/Qwen3-Embedding-0.6B-GGUF; EmbeddingGemma-GGUF: https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF; BGE-M3: https://huggingface.co/BAAI/bge-m3
Q38 Eingebettete Vektorspeicher: https://www.actian.com/blog/developer/comparing-embedded-vector-databases/; sqlite-vec vs. LanceDB (1,7 s vs. 6 ms p50 bei 1 Mio.): https://github.com/dutiona/knowledge-base/issues/65; sqlite-vec: https://docs.rs/crate/sqlite-vec/latest, https://github.com/asg017/sqlite-vec/releases
Q39 Meetily: https://github.com/Zackriya-Solutions/meeting-minutes
Q40 Anarlog: https://github.com/fastrepl/anarlog, https://anarlog.so/blog/char-is-now-anarlog/
Q41 Vibe: https://github.com/thewh1teagle/vibe
Q42 OpenWhispr: https://github.com/OpenWhispr/openwhispr
Q43 OpenOats: https://github.com/yazinsai/OpenOats
Q44 Open Granola: https://github.com/anshuman-pandey/open-granola
Q45 Minutes: https://github.com/silverstein/minutes
Q46 Scriberr: https://github.com/rishikanthc/Scriberr
Q47 Amurex: https://github.com/thepersonalaicompany/amurex
Q48 ICS-Aktualisierung Outlook.com: https://learn.microsoft.com/en-us/answers/questions/4553843/refresh-rate-of-subscribed-ics-calendar-on-outlook
Q49 Graph-Berechtigungen: https://learn.microsoft.com/en-us/graph/permissions-reference, https://graphpermissions.merill.net/permission/Calendars.Read
Q50 Herausgeber-Verifizierung: https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview
Q51 Google Sensitive-Scope-Verifizierung: https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification
Windows Mail/Kalender eingestellt: https://support.microsoft.com/en-us/outlook/windows-mail-calendar-and-people-are-becoming-new-outlook

Hinweis zur Belastbarkeit: Mehrere Werte stammen aus Modellkarten und Herstellerangaben, nicht aus unabhängigen Messungen; einzelne
Praxisberichte (Q15, Q30, Q31) sind Einzelmessungen. Die Rechenbeispiele in 1.3 sind Ableitungen aus den genannten Faktoren.
