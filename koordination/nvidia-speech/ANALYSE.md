# Nvidia-Speech-Modelle für Local Voice – Analyse und Umsetzungsplan

Stand: 12.11.2026. Quelle aller Modellangaben sind die Modellkarten auf `huggingface.co/nvidia`
(abgerufen am 12.11.2026) und der Code auf `main` (08ef5119). Messwerte aus den Karten sind
Herstellerangaben und nicht auf unserer Hardware nachgemessen, sofern nicht anders vermerkt.

## 1. Kurzfassung

- Der größte Teil der Familie ist schon in der App: Parakeet, Canary, Nemotron 3.5 ASR Streaming,
  Nemotron Speech Streaming EN und Multitalker Parakeet stehen im Katalog, Sortformer 4spk v2.1
  trennt Sprecher.
- **Drei Modelle bringen für Deutsch echten Gewinn**, eines davon sofort umsetzbar:
  1. **Nemotron-3-Diarization** (bis 8 Sprecher statt 4, OpenMDW-1.1): sofort, über das
     vorhandene ONNX Runtime. Das ist Paket N1.
  2. **Nemotron-Labs-Unified-ASR-0.6b** (27.10.2026, OpenMDW-1.1): ein Modell für Diktat live
     und Datei, Deutsch live 5,45 % statt rund 8 bis 10 %. Wartet auf die Engine `transcribe.cpp`.
  3. **Nemotron 3.5 Transcribe 30B-A4B** (29.10.2026, OpenMDW-1.1): Deutsch 2,62 % (FLEURS), mit
     Sprechern, Zeitmarken und Übersetzung. Offiziell erst ab RTX 5090, auf der 4090 nur als
     Messung.
- **Nicht für Deutsch geeignet** (nur Englisch): Parakeet Realtime EOU, Multitalker Parakeet,
  Nemotron Speech Streaming EN, NemotronLabs VoiceChat 11B.

## 2. Die Modelle im Einzelnen

| Modell (exakter Name) | Zweck | Deutsch | Lizenz | Größe | In der App | Urteil |
|---|---|---|---|---|---|---|
| `nvidia/parakeet-tdt-0.6b-v3` | Transkription Datei, 25 europ. Sprachen, Wortzeiten | ja | CC-BY-4.0 | 0,6B, Q8 705 MB | ja (Standard Besprechungen) | bleibt |
| `nvidia/canary-1b-v2` | Transkription und Sprachübersetzung (24 Sprachen ↔ Englisch) | ja | CC-BY-4.0 | 1B, Q8 1,1 GB | ja (Katalog, „nach Englisch übersetzen“) | bleibt; kein Deutsch → andere Sprache als Englisch |
| `nvidia/nemotron-3.5-asr-streaming-0.6b` | Live-Diktat, 80 ms bis 1,12 s | ja (de-DE 8,2 bis 9,9 % FLEURS je Latenz) | OpenMDW-1.1 | 0,6B, Q8 716 MB | ja (Erststart-Modell) | bleibt, bis Unified läuft |
| `nvidia/nemotron-labs-unified-asr-0.6b` | Datei und Live in einem Modell, 25 europ. Sprachen, 160 ms bis 2,08 s | ja (4,32 % Datei; 5,45 % bei 1,12 s; 6,30 % bei 0,56 s; Mittel FLEURS/MCV/MLS) | OpenMDW-1.1 | 0,6B, `.nemo` 2,4 GB | nein | **Kandidat N2**, wartet auf Engine |
| `nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B` (auch `-FP8`, `-NVFP4`) | Transkription 40 Sprachen bis 1 h am Stück, Sprecher, Zeitmarken, Übersetzung, Fragen zum Ton | ja („Leading“; FLEURS 2,62 %, Common Voice 2,99 %) | OpenMDW-1.1 | 30B (4,3B aktiv), NVFP4 rund 20,5 GB | nein | **Kandidat N3**, nur starke GPU |
| `nvidia/Nemotron-3-Diarization` | Sprechertrennung bis 8 Sprecher, live (0,32 s) und Datei | sprachunabhängig | OpenMDW-1.1 | 100M, ONNX int8 114 MB | nein (im Spike gemessen, 30.09.) | **Paket N1, sofort** |
| `nvidia/diar_streaming_sortformer_4spk-v2.1` | Sprechertrennung bis 4 Sprecher | sprachunabhängig | NVIDIA Open Model License | 117M, Q8 133 MB | ja (Standard) | bleibt als Rückfall |
| `nvidia/parakeet_realtime_eou_120m-v1` | Live-Erkennung mit Satzende-Zeichen `<EOU>` für Sprachagenten | **nein, nur Englisch**, ohne Satzzeichen | NVIDIA Open Model License | 120M | nein | nicht einbauen |
| `nvidia/multitalker-parakeet-streaming-0.6b-v1` | überlappende Sprecher live, eine Modellinstanz je Sprecher | **nein, nur Englisch** | NVIDIA Open Model License | 0,6B | Katalog, nicht angebunden | nicht anbinden |
| `nvidia/nemotron-speech-streaming-en-0.6b` | Live-Diktat Englisch | nein | NVIDIA Open Model License | 0,6B | ja (Katalog) | durch Nemotron 3.5 ersetzt |
| `nvidia/NVIDIA-NemotronLabs-VoiceChat-11B` | Vollduplex-Sprachgespräch in einem Modell | **nein, nur Englisch** | OpenMDW-1.1 | 11B | nein | beobachten (Voice-Agent) |
| `nvidia/NVIDIA-NemotronLabs-RE-USE` | Sprachverbesserung (Rauschen, Hall), 9,6M | sprachunabhängig | OpenMDW-1.1 | klein, braucht `mamba-ssm` (PyTorch/CUDA) | nein | zurückgestellt, kein portabler Laufweg |
| `nvidia/Nemotron-Labs-Audex-2B` | Audio-Sprachmodell | – | **nur nichtkommerziell** | 2B | nein | ausgeschlossen |

Lizenzen: CC-BY-4.0, OpenMDW-1.1 und die NVIDIA Open Model License erlauben laut den Karten
kommerzielle Nutzung. Kein Modell wird mit der App ausgeliefert, alle werden auf Wunsch geladen.
Eine Lizenzfrage für Patrick ist dadurch nicht offen.

Befund am Rand: Der Katalog führt `parakeet-unified-en-0.6b` als CC-BY-4.0. Die Karte von Nvidia
und die GGUF-Karte nennen die NVIDIA Open Model License. Wird in N1 korrigiert.

## 3. Abgleich mit dem Ist-Stand

| Bereich | Heute in der App | Nvidia-Kandidat | Gewinn |
|---|---|---|---|
| Diktat live | Nemotron 3.5 ASR Streaming (GGUF, `transcribe-cpp` 0.2.4) | Unified-ASR | Deutsch live rund ein Drittel weniger Fehler bei gleicher Latenz; ein Modell statt zwei |
| Besprechung, Datei | Parakeet v3 (Standard), Nachlauf mit Whisper large-v3 / Qwen3-ASR | Unified-ASR (Datei 4,32 %), Transcribe 30B (2,62 %) | Unified: klein. Transcribe 30B: groß, aber nur mit 24 bis 32 GB VRAM |
| Sprechertrennung | Sortformer 4spk v2.1 (höchstens 4 je Kanal), CAM++ für Zusammenführen | Nemotron-3-Diarization | **bis 8 Sprecher**; Karte/Recherche: DER AMI 9,25 % (Headset) und 11,14 % (Fernfeld), bei uns gemessen Sortformer 15,19 % (AMI) |
| Sprechende im Gespräch (Satzende) | Pegel und Stille (380 ms früh, 700 ms sicher), `turnTaking.ts` | Parakeet Realtime EOU | keiner für Deutsch. Smart Turn (BSD-2) bleibt der Kandidat des Voice-Agent-Threads |
| Überlappende Sprecher | zwei Kanäle (Ich / Gegenseite), sonst nichts | Multitalker Parakeet | keiner für Deutsch |
| Übersetzung | Transkript-Fassung per LLM (`translate.rs`), Diktat nach Englisch über Canary/Whisper | Canary-1b-v2 (schon da), Transcribe 30B, Unified + LLM-Kaskade | keiner sofort; Live-Übersetzung von Besprechungen wäre ein eigenes Vorhaben |
| Stille/Sprache | Silero VAD v4 | – (Silero v6.2 kommt mit `transcribe-cpp` 0.4) | bei Engine-Anhebung mitnehmen |

Bekannte Grenze, die kein Nvidia-Modell löst: sehr ähnliche Stimmen werden verschmolzen
(Hörspiel-Test `abnahme/p3e-hoerspiel.md`: Sortformer 33,84 % DER, Nemotron-3 31,95 %). Dafür
bleibt der Stimmabdruck-Nachschritt der richtige Weg.

## 4. Pakete

### N1 – Sprechertrennung bis 8 Sprecher (sofort)

- Zweite Engine hinter dem vorhandenen Trait `DiarizeEngine`: `nvidia/Nemotron-3-Diarization`
  als ONNX (`onnx-community/Nemotron-3-Diarization-ONNX`, int8) über das schon eingebundene
  `ort` (CPU, läuft auch auf Bürorechnern).
- Das offizielle GGUF von Nvidia (`sortformer.version = v3`) lädt `transcribe-cpp` nicht, auch
  0.4.2 kennt nur Sortformer v2.1. Deshalb ONNX.
- Das ONNX-Modell rechnet nur einen Block. Mel-Vorstufe (128 Bänder), Blockschnitt und der
  Sprecher-Zwischenspeicher (AOSC) werden nach Rust portiert, Vorlage ist die Referenz in
  `transformers` (Apache-2.0). Abnahme: Gleichheit mit der Python-Referenz auf festen Testtönen.
- Katalogeintrag (Zweck `diarization`, gepinnte Revision, SHA-256, Lizenz OpenMDW-1.1).
- `--eval-diarization` wählt die Engine nach Modell, damit der PC die DER auf dem vorhandenen
  Korpus (AMI-Prüfteil, deutsches Fixture) gegen Sortformer misst.
- Tor für die Freischaltung als Standard (N1b): DER auf AMI nicht schlechter als Sortformer,
  deutsches Fixture ≤ 5 %, auf der CPU mindestens zehnfache Echtzeit. Bis dahin bleibt Sortformer
  Standard und Nemotron-3 ist wählbar.

### N2 – Unified-ASR als ein Modell für Diktat und Datei (wartet)

- Blocker: kein GGUF unter `handy-computer`, keine Unterstützung in `transcribe.cpp` 0.4.2. Die
  englische Fassung `parakeet-unified-en-0.6b` wird dort schon unterstützt, die mehrsprachige
  braucht zusätzlich den Sprach-Prompt (wie Nemotron 3.5).
- Vorarbeit ohne Blocker: `transcribe-cpp` von 0.2.4 auf 0.4.x heben (eigener PR, Windows-Tor,
  bringt auch Silero v6.2).
- Sobald das GGUF da ist: Katalog, Messung Deutsch (Fixtures Katja, Besprechungskorpus) gegen
  Nemotron 3.5 und Parakeet v3, CPU-Last messen (das Modell rechnet 5,6 s linken Kontext je
  Block neu), dann Erststart-Modell umstellen.

### N3 – Nemotron 3.5 Transcribe als Nachlauf für starke Rechner (Messung)

- Messauftrag an den PC (RTX 4090): läuft die NVFP4- bzw. eine GGUF-Fassung in 24 GB, wie
  schnell, wie gut auf dem deutschen Besprechungskorpus gegen Whisper large-v3 / Qwen3-ASR?
- Nur bei klarem Vorsprung einbauen, als wählbarer Nachlauf über den vorhandenen
  `llama-server`-ähnlichen Prozessweg, nicht im App-Prozess. Offiziell nennt Nvidia für diese
  Klasse erst die RTX 5090.

### Nicht geplant

EOU, Multitalker, VoiceChat 11B (nur Englisch), RE-USE (kein portabler Laufweg), Audex
(nichtkommerziell).
