# TTS-Anbieter: Primärquellen-Recherche

**Stand:** 26.10.2026 (alle Quellen an diesem Tag abgerufen)
**Zweck:** Grundlage für die TTS-Provider-Integration in Local Voice AI (Tauri/Rust, Windows 11, RTX 4090).

**Methode:** Nur Primärquellen: offizielle GitHub-Repos (README/Docs roh über
`raw.githubusercontent.com`, Releases und Lizenz über die GitHub-API), Hugging-Face-Model-Cards
und `api/models`, die offizielle Ollama-Doku (`docs.ollama.com/*.md`), die OpenAI-OpenAPI-Spezifikation
(`openai/openai-openapi`, Branch `manual_spec`) und Herstellerblogs. Zitate sind wörtlich aus dem
Rohtext übernommen. Status **verifiziert** = steht so in der Primärquelle; **nicht verifiziert** =
nicht gefunden, nur indirekt oder widersprüchlich belegt. Nichts installiert oder getestet;
Laufzeitverhalten (Latenz, VRAM, Klang) ist durchgehend **nicht verifiziert**.

---

## A. Phonon (Gradium)

### Namensgleiche Projekte

| Projekt | Was es ist | Quelle | Relevant? |
|---|---|---|---|
| **gradium-ai/xn-ptts** (Phonon TTS) | „Phonon TTS inference using xn“ (Repo-Beschreibung) | https://github.com/gradium-ai/xn-ptts | **ja, das TTS-Projekt** |
| fermionresearch/phonon | „Phonon: open speech recognition models (Phonon-2, Phonon-1)“, also **Spracherkennung**, kein TTS | https://github.com/fermionresearch/phonon | nein (Verwechslungsgefahr!) |
| KDE/phonon | „Multi-platform sound framework for application developers“ | https://github.com/KDE/phonon | nein |
| phonon-framework/phonon | „responsive front-end framework“ | https://github.com/phonon-framework/phonon | nein |

### Befunde

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| Upstream ist Gradium (Paris/SF, gegründet von Kyutai-Mitgründern): „Gradium, the voice AI company founded by the cofounders of Kyutai research lab, today released Phonon“ | https://gradium.ai/blog/gradium-releases-phonon | 26.10.2026 | verifiziert |
| Open-Source-Release: „BERLIN, NVIDIA GTC , October 21, 2026“. Vorher ein Ankündigungspost vom 3. April 2026 und das Open-Sourcing am 20.10.2026 (verwandte Posts auf derselben Seite) | dto. | 26.10.2026 | verifiziert |
| GitHub-Org `gradium-ai`, Repo `xn-ptts`. HF-Org `gradium` | https://github.com/gradium-ai/xn-ptts | 26.10.2026 | verifiziert |
| Aktuelles Release **v0.4.2** vom 25.10.2026, davor v0.4.1 vom 24.10.2026. Assets u. a. `ptts-0.4.2-x86_64-pc-windows-msvc.zip` | https://api.github.com/repos/gradium-ai/xn-ptts/releases | 26.10.2026 | verifiziert |
| Runtime: eigener Rust-Inferenzstack („xn“), ohne PyTorch. Gewichte als GGUF: „model.q8.gguf, 74.6 MB (8-bit quantized)“ (40m-de), dazu Safetensors | README; https://huggingface.co/gradium/phonon-40m-de | 26.10.2026 | verifiziert |
| Bindings: „Phonon ships with integrations for Rust, Python, Swift and the browser through WebAssembly“, dazu eine C-FFI (`ptts.h`, „for Linux, Windows, macOS, iOS and Android“) | Blog; README | 26.10.2026 | verifiziert |
| Windows: CLI-Archiv „Windows x64 \| `x86_64-pc-windows-msvc` \| Windows 10 or later; x86-64-v3 CPU“ | https://github.com/gradium-ai/xn-ptts/blob/main/docs/cli.md | 26.10.2026 | verifiziert |
| **CUDA nicht in den Fertig-Binaries:** „The other downloads run on the CPU only. For CUDA or Vulkan, build from source with that backend feature.“ Außerdem: „Quantized weights such as `--quant q8` always run on the CPU.“ | docs/cli.md | 26.10.2026 | verifiziert |
| Laut Blog gibt es einen CUDA-Backend: „includes a NVIDIA CUDA backend that allows Phonon to run on professional and gaming GPUs“. Ob der unter Windows baut, ist nicht belegt | Blog | 26.10.2026 | Windows+CUDA **nicht verifiziert** |
| Echtes Streaming: „Phonon streams frame by frame instead of waiting for the whole sentence“, `tts.stream(text)` in Python, Rust und WASM | README | 26.10.2026 | verifiziert |
| **Deutsch vorhanden:** `gradium/phonon-40m-de` (nur Deutsch) und `gradium/phonon-40m-multilingual` (en/fr/de/es/pt). „turns text into natural 24 kHz speech in German“ | README-Tabelle; HF-Card 40m-de | 26.10.2026 | verifiziert |
| Das öffentliche `phonon-31m-en` kann nur Englisch: „The model speaks only the languages above.“ | https://huggingface.co/gradium/phonon-31m-en | 26.10.2026 | verifiziert |
| **Lizenz Code:** „The code is **MIT OR Apache-2.0**, at your option“ (der Blog nennt nur Apache 2.0) | README → License | 26.10.2026 | verifiziert |
| **Lizenz Gewichte:** Phonon 31m: Apache 2.0. Phonon 40m (alle Sprachen, multilingual): **CC BY-NC 4.0**, „which does not allow commercial use“ | README-Tabelle; HF-Card 40m-de | 26.10.2026 | verifiziert |
| **Deutsche Gewichte sind gated:** „The Phonon 40m models are gated. To use one, accept its terms on its Hugging Face page; approval is automatic. Then set `HF_TOKEN`“ | README | 26.10.2026 | verifiziert |
| Stimmen: Katalog `gradium/phonon-voices`; „Phonon comes with a catalog of ~100 voices.“ Klonen und Voice Design nur über Gradium Studio/API (Cloud) | README; HF-Card 40m-de | 26.10.2026 | verifiziert; Zahl deutscher Stimmen **nicht verifiziert** |
| Tempo: Steuerung über die Kondition `duration_delta`. Im OpenAI-Server ist `speed` „Only `1.0`: the model has no rate control yet“ | README; ptts-openai-server/README.md | 26.10.2026 | verifiziert (widersprüchlich, siehe Offene Punkte) |
| **OpenAI-kompatibler Server** `ptts-openai-server`: „compatible with OpenAI's text-to-speech API: `POST /v1/audio/speech`“, Port 8880, dazu `GET /v1/audio/voices`, `GET /v1/models` und `GET /health`, ohne Authentifizierung | https://github.com/gradium-ai/xn-ptts/blob/main/ptts-openai-server/README.md | 26.10.2026 | verifiziert |
| Server-Image nur Linux: „It is built for `linux/amd64` and `linux/arm64`“. Alternativ `cargo run --release -p ptts-openai-server`. Ob es nativ unter Windows baut, ist nicht belegt | dto. | 26.10.2026 | Windows-nativ **nicht verifiziert** |
| WebSocket-Server `ptts-ws-server`: „streaming text and audio over one connection“ | README | 26.10.2026 | verifiziert |

**Schema `ptts-openai-server` (wörtlich aus der Feldtabelle):**
- `input`: „Required, up to 4096 characters. Long text is split at sentence ends.“
- `voice`: „A voice of this server, in any case … or `default`“. Andere Namen wie `alloy` führen zu einem 400er, außer sie sind als Alias gesetzt.
- `response_format`: „`mp3` (the default), `opus`, `wav` or `pcm` (headerless 16-bit mono at 24 kHz). `aac` and `flac` are a 400.“
- `speed`: „Only `1.0`“. Mit `--ignore-speed` wird der Wert akzeptiert und ignoriert.
- `model`, `instructions`, `stream_format`: „Accepted and ignored. The reply is always the audio bytes, streamed as they are generated.“
- Das Modell wird serverseitig über `PTTS_MODEL` gewählt (z. B. `hf://gradium/phonon-40m-de`), die Sprache über `PTTS_LANG`. `--lang` ist Pflicht.

---

## B. Ollama Speech/TTS

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| **Ja, Ollama kann seit Oktober 2026 Sprache erzeugen.** Release-Notes v0.40.11 (24.10.2026): „Ollama can now generate speech on macOS. `ollama run` plays text or a chat model's reply aloud as the audio is generated, and apps can stream audio through the OpenAI-compatible `/v1/audio/speech` endpoint.“ | https://github.com/ollama/ollama/releases/tag/v0.40.11 | 26.10.2026 | verifiziert |
| Die Mindestversion widerspricht sich je nach Quelle: Die Speech-Doku sagt „Start Ollama v0.40.9 or later“, die OpenAI-Kompatibilitätsdoku sagt „Note: Added in Ollama v0.40.8“, die Release-Notes kündigen es erst in v0.40.11 an (v0.40.8/9 erwähnen Speech nicht) | https://docs.ollama.com/capabilities/speech · https://docs.ollama.com/api/openai-compatibility | 26.10.2026 | widersprüchlich → **v0.40.11 als sichere Untergrenze** |
| Endpunkt `POST http://localhost:11434/v1/audio/speech` (OpenAI-kompatibel). Ein natives `/api/...`-Speech-Endpoint ist nicht dokumentiert. `/api/show` liefert die Capability `speech` und `voices` | Speech-Doku | 26.10.2026 | verifiziert |
| Unterstützte Felder: `model`, `input` („up to 4,096 characters“), `voice`, `response_format` (nur `wav` (Standard) und `pcm`; „`mp3`, `opus`, `aac`, `flac`“ nicht), `speed` („`1` only“), `stream_format` („`audio` only“). `instructions` nicht unterstützt | OpenAI-Kompatibilitätsdoku | 26.10.2026 | verifiziert |
| Antwort: „Audio is 16-bit, mono, 24 kHz.“ Streaming: „The response streams while audio is generated, so you can start playing it right away“. Der WAV-Header enthält Maximalwerte als Länge | Speech-Doku | 26.10.2026 | verifiziert |
| Strikt: „Options the model doesn't support, such as other formats, speeds, or `instructions`, return a `400` error instead of being ignored.“ | Speech-Doku | 26.10.2026 | verifiziert |
| Modell: **Phonon**. „Phonon uses Gradium's 31M English checkpoint“ (Stimmen u. a. `Zoey`, `Easton`, `Freya`) | Speech-Doku | 26.10.2026 | verifiziert |
| **Deutsch in Ollama:** Die Bibliothek listet „`phonon:40m-de` ( `phonon:de` ) German, 40M 141 MB“ und `phonon:multilingual`. Lizenz: „40M … checkpoints are released under CC BY-NC 4.0“ | https://ollama.com/library/phonon | 26.10.2026 | verifiziert (Tag existiert); Funktion **nicht getestet** |
| **Windows:** Die Release-Notes sagen ausdrücklich „on macOS“. Die Speech-Doku nennt aber für die Wiedergabe „PowerShell on Windows“. Ob der Server-Endpunkt unter Windows Audio erzeugt, ist nicht eindeutig belegt | Release v0.40.11; Speech-Doku | 26.10.2026 | **nicht verifiziert / widersprüchlich** |
| Aktuellste Ollama-Version zum Abrufzeitpunkt: v0.40.11 (24.10.2026) | https://api.github.com/repos/ollama/ollama/releases | 26.10.2026 | verifiziert |

---

## C. Etablierte Alternativen: Kurzsteckbriefe

### Piper (OHF-Voice/piper1-gpl)

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| Aktiver Nachfolger `OHF-Voice/piper1-gpl`, Release v1.8.0 (04.09.2026). Code-Lizenz **GPL-3.0** (PyPI: „GPL-3.0-or-later“) | https://github.com/OHF-Voice/piper1-gpl · https://pypi.org/project/piper-tts/ | 26.10.2026 | verifiziert |
| Windows: Wheel `win_amd64.whl` für piper-tts 1.8.0 | PyPI-JSON | 26.10.2026 | verifiziert |
| CUDA: „To use CUDA for GPU acceleration: … `use_cuda=True` … This requires the `onnxruntime-gpu` package“ (ONNX-Runtime) | docs/API_PYTHON.md | 26.10.2026 | verifiziert |
| Streaming: „For streaming, use `PiperVoice.synthesize`: `for chunk in voice.synthesize("...")`“, in der CLI über `--output-raw` | docs/API_PYTHON.md · docs/CLI.md | 26.10.2026 | verifiziert |
| **Deutsch offiziell:** `de_DE-eva_k-x_low`, `-karlsson-low`, `-kerstin-low`, `-mls-medium`, `-pavoque-low`, `-ramona-low`, `-thorsten-{low,medium,high}`, `-thorsten_emotional-medium` | https://huggingface.co/rhasspy/piper-voices (voices.json) | 26.10.2026 | verifiziert |
| Stimmlizenz je Model Card, z. B. thorsten medium/high: Dataset „License: CC0“, kerstin low: „License: CC0“. Die übrigen deutschen Stimmen wurden nicht geprüft | MODEL_CARD je Stimme | 26.10.2026 | thorsten/kerstin verifiziert; Rest **nicht verifiziert** |
| HTTP-Server **nicht** OpenAI-kompatibel: `POST /synthesize` mit `text`, `voice`, `speaker`, `speaker_id`, `length_scale` („speaking speed“), Port 5000, `GET /voices`, `GET /info` | docs/API_HTTP.md | 26.10.2026 | verifiziert |

### Kokoro v1.0 (82M)

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| „an open-weight TTS model with 82 million parameters“. Gewichte und Code Apache-2.0 („With Apache-licensed weights“). v1.0 vom 27.01.2025. `kokoro-v1_0.pth` ≈ 327 MB | https://huggingface.co/hexgrad/Kokoro-82M · HF-API | 26.10.2026 | verifiziert |
| **Kein Deutsch:** VOICES.md listet nur American/British English, Japanese, Mandarin, Spanish, French, Hindi, Italian und Brazilian Portuguese | https://huggingface.co/hexgrad/Kokoro-82M/blob/main/VOICES.md | 26.10.2026 | verifiziert; deutsche Community-Stimmen **nicht verifiziert** |
| Server **Kokoro-FastAPI** (remsky), v0.9.0 vom 10.09.2026, Apache-2.0: „Dockerized FastAPI wrapper for Kokoro-82M“, „OpenAI-compatible API“, Port 8880 | https://github.com/remsky/Kokoro-FastAPI | 26.10.2026 | verifiziert |
| CUDA im Docker-Image nur Linux („CPU and NVIDIA GPU (CUDA): linux/amd64 + linux/arm64“), nativ unter Windows per `.\start-gpu.ps1` | Kokoro-FastAPI README | 26.10.2026 | verifiziert (Windows-Lauf **nicht getestet**) |
| Schema: `"model": "kokoro"`, `"input"`, `"voice": "af_bella"`, `"response_format": "mp3",  # Supported: mp3, wav, opus, flac, aac, pcm`, `"speed": 1.0`. Streaming über `response_format="pcm"` und `stream=True`, außerdem ein Feld `"stream": False` | Kokoro-FastAPI README | 26.10.2026 | verifiziert |

### XTTS-v2 (Coqui)

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| **Deutsch offiziell:** „XTTS-v2 supports 17 languages: … German (de) …“, Stimmklonen aus einem „6-second audio clip“ | https://huggingface.co/coqui/XTTS-v2 | 26.10.2026 | verifiziert |
| **Gewichte nichtkommerziell:** LICENSE.txt (CPML): „This license allows only non-commercial use of a machine learning model and its outputs.“ Die Lizenzseite `coqui.ai/cpml` liefert 404 (Coqui existiert nicht mehr) | https://huggingface.co/coqui/XTTS-v2/blob/main/LICENSE.txt | 26.10.2026 | verifiziert |
| Gewichte: `model.pth` ≈ 1.868 MB plus `dvae.pth` ≈ 211 MB, unverändert seit 11.12.2023 | HF-API | 26.10.2026 | verifiziert |
| Code: Original `coqui-ai/TTS` (MPL-2.0, letztes Release v0.22.0 von 2023). Gepflegter Fork `idiap/coqui-ai-TTS` v0.28.0 (19.10.2026), MPL-2.0 | GitHub-API | 26.10.2026 | verifiziert |
| Windows: „0.24.2: Prebuilt wheels are now also published for macOS and Windows“. Streaming: „XTTS can stream with <200ms latency.“ Server `TTS/server/server.py` (Port 5002, nicht OpenAI-kompatibel) | idiap/coqui-ai-TTS README | 26.10.2026 | verifiziert |

### F5-TTS

| Befund | Quelle | Abgerufen | Status |
|---|---|---|---|
| „Our code is released under MIT License. The pre-trained models are licensed under the CC-BY-NC license due to the training data Emilia“ | https://github.com/SWivid/F5-TTS | 26.10.2026 | verifiziert |
| Release 1.1.22 (23.07.2026). Gewichte F5TTS_v1_Base ≈ 1.348 MB (Safetensors). Basismodell nur Englisch/Chinesisch (`F5-TTS_Emilia-ZH-EN`) | GitHub-API; HF-API; README | 26.10.2026 | verifiziert |
| **Deutsch nur über Community-Finetunes, alle CC-BY-NC-4.0:** `aihpi/F5-TTS-German` („Datasets: Common Voice (Mozilla) and Emilia_DE“), `marduk-ra/F5-TTS-German`, `hvoss-techfak/F5-TTS-German` | HF-API cardData; aihpi-Card | 26.10.2026 | verifiziert |
| CUDA über PyTorch („Install pytorch with your CUDA version“). Windows wird im README nicht ausdrücklich genannt | README | 26.10.2026 | Windows **nicht verifiziert** |
| Streaming: „Basic TTS with Chunk Inference“ und eine Client-Server-Benchmark-Zeile (RTF 0.0394). Ein OpenAI-kompatibler Server ist nicht dokumentiert, nur Gradio-App und CLI | README | 26.10.2026 | Chunk-Streaming **nicht verifiziert** |

---

## D. Gemeinsame OpenAI-Schnittstelle `/v1/audio/speech`

**Referenz (OpenAI-Spezifikation, `CreateSpeechRequest`):** Pflichtfelder: „required: - model - input - voice“.
`input`: „The maximum length is 4096 characters.“ `response_format`: „Supported formats are `mp3`, `opus`,
`aac`, `flac`, `wav`, and `pcm`“, „default: mp3“. `speed`: „Select a value from `0.25` to `4.0`. `1.0` is
the default.“ `instructions`: „Does not work with `tts-1` or `tts-1-hd`.“
Quelle: https://raw.githubusercontent.com/openai/openai-openapi/manual_spec/openapi.yaml (abgerufen
26.10.2026). Die Doku-Seite auf platform.openai.com lieferte 403. `stream_format` steht in diesem Spec-Stand
**nicht** → in der OpenAI-Primärquelle **nicht verifiziert**.

| Server | Lizenz / Stand | Abweichungen vom OpenAI-Schema | Status |
|---|---|---|---|
| **Ollama** (Phonon) | Gewichte 31m Apache / 40m CC BY-NC · v0.40.11 | nur `wav` (Standard!) und `pcm`, `speed` nur 1, `stream_format` nur `audio`, unbekannte Optionen ergeben einen 400er | verifiziert |
| **ptts-openai-server** (Phonon) | MIT/Apache · v0.4.2 | Standard `mp3`, `aac`/`flac` ergeben einen 400er, `speed` nur 1.0, `model`/`stream_format` werden ignoriert, Modell kommt aus `PTTS_MODEL`, immer gestreamt | verifiziert |
| **Kokoro-FastAPI** | Apache-2.0 · v0.9.0 | alle sechs Formate, `speed` frei, zusätzlich `stream`, Stimmen-Mischung `af_sky+af_bella` | verifiziert |
| **openedai-speech** (Piper=`tts-1`, XTTS=`tts-1-hd`) | **AGPL-3.0, Repo archiviert**, letztes Release 0.18.2 vom 16.08.2024 | „response_format: `mp3`, `opus`, `aac`, `flac`, `wav` and `pcm`“, „speed 0.25-4.0“ | verifiziert, aber **nicht mehr gepflegt** |
| Piper-HTTP / Coqui-Server / F5 | – | **nicht** OpenAI-kompatibel (eigene Endpunkte) | verifiziert |

Kleinster gemeinsamer Nenner aller drei aktiven Server: `POST {base}/v1/audio/speech` mit
`{"model", "input" (≤ 4096 Zeichen), "voice", "response_format": "wav"|"pcm"}`, ohne `speed` bzw. mit
`speed` 1.0. Antwort sind rohe Audio-Bytes (gestreamt), PCM ist 16-bit mono 24 kHz (Ollama, Phonon).
Kokoro-PCM ist laut Beispiel `rate=24000`. Stimmen abfragen: `GET /v1/audio/voices` (Phonon-Server,
Kokoro-FastAPI). Bei Ollama stattdessen `/api/show` → `voices`.

---

## Konsequenzen für die Integration

1. **Ein Client für alle:** Einen generischen OpenAI-`/v1/audio/speech`-Provider in Rust bauen (Basis-URL, Modell
   und Stimme konfigurierbar). Er deckt Ollama, ptts-openai-server und Kokoro-FastAPI ab. Immer
   `response_format: "pcm"` anfordern (16-bit mono 24 kHz) und `speed` weglassen. So entfallen
   WAV-Header-Probleme (Maximal-Längen bei Ollama und Phonon), und alle drei Server liefern Streaming.
2. **Deutsch hängt an der Lizenz:** Die einzigen deutschen Phonon-Gewichte (`40m-de`, `40m-multilingual`)
   sind **CC BY-NC 4.0** und gated (HF-Token). Für eine kommerziell vertriebene App sind sie nicht nutzbar,
   ebenso wenig XTTS-v2 (CPML) und alle F5-German-Finetunes (CC-BY-NC). **Auch Piper ist nicht pauschal
   kommerziell sauber** (Nachtrag 26.10.2026): Der thorsten-Datensatz steht unter CC0, das Modell
   `de_DE-thorsten-high` ist laut MODEL_CARD aber „Finetuned from U.S. English lessac voice (high quality)“,
   und die lessac-Daten (Blizzard 2013) stehen unter einer eigenen Forschungslizenz
   (https://www.cstr.ed.ac.uk/projects/blizzard/2013/lessac_blizzard2013/license.html). Kommerziell ist
   je Stimme die Abstammung zu prüfen. Die GPL des Piper-Codes betrifft die Einbettung als Bibliothek;
   ein separater Prozess ist zu prüfen.
3. **Phonon unter Windows:** Das fertige `ptts.exe` läuft nur auf der CPU (x86-64-v3/AVX2). Die 4090
   nützt nur bei einem Eigenbau mit Feature `cuda`, und q8 läuft ohnehin auf der CPU. Bei 31–40M Parametern
   ist CPU-Betrieb vermutlich ausreichend (Latenz **nicht verifiziert**). Der OpenAI-Server ist offiziell
   nur ein Linux-Image, nativ unter Windows ungetestet.
4. **Ollama als Bequemlichkeitsweg:** Wer Ollama schon für LLMs betreibt, bekommt TTS ohne zweiten Dienst.
   Windows ist laut Release-Notes aber nicht zugesagt („on macOS“). Vor dem Einbau unter Windows ≥ v0.40.11
   selbst testen. Strenges 400-Verhalten berücksichtigen (keine `mp3`, kein `speed`).
5. **Kokoro** ist eine starke, kommerziell freie Option für Englisch und andere Sprachen, **aber ohne Deutsch**.
   **openedai-speech** nicht mehr einplanen (archiviert, AGPL).

## Offene Punkte

- Ollama-Speech unter **Windows**: Funktioniert `/v1/audio/speech` mit `phonon:de` unter Windows? Ab welcher
  Version genau (v0.40.8, v0.40.9 oder v0.40.11, die Quellen widersprechen sich)? Nur durch einen eigenen
  Test zu klären.
- Phonon: Baut `ptts`/`ptts-openai-server` mit `--features cuda` unter Windows (MSVC)? Wie groß sind Latenz
  und RTF auf der CPU des Zielrechners?
- Phonon-Tempo: README nennt `duration_delta` als Tempo-Kondition, der Server sagt „no rate control yet“.
  Ist `--condition duration_delta=…` ein brauchbarer Ersatz für `speed`?
- Zahl und Qualität der **deutschen** Phonon-Stimmen im Katalog `gradium/phonon-voices`: nicht geprüft.
- Gibt es eine kommerzielle Lizenz für Phonon 40m bei Gradium? Nicht gefunden.
- Lizenzen der übrigen deutschen Piper-Stimmen (eva_k, karlsson, mls, pavoque, ramona): einzeln prüfen.
- GPL-3.0 von piper1-gpl: Ist die Anbindung als separater Prozess für die App-Lizenz unkritisch? Das muss
  juristisch geklärt werden.
- `stream_format` im aktuellen OpenAI-Schema: im verfügbaren Spec-Stand nicht enthalten, nicht verifiziert.
