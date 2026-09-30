# Namensnennung und Lizenzen (Benchmark-Daten, Modelle, Bibliotheken)

## FLEURS (Google) - CC-BY-4.0

Die Wortfehlerraten in [`bench.md`](bench.md) stammen aus Aufnahmen und Referenztexten des
Datensatzes **FLEURS**, Konfiguration `de_de`, Teilmenge `test`.

- Quelle: <https://huggingface.co/datasets/google/fleurs>
- Urheber: Google LLC; Conneau, A. et al., *FLEURS: Few-shot Learning Evaluation of Universal
  Representations of Speech*, 2022 (arXiv:2205.12446).
- Lizenz: Creative Commons Namensnennung 4.0 International (CC-BY-4.0),
  <https://creativecommons.org/licenses/by/4.0/>.
- Die Referenztexte in FLEURS beruhen auf FLoRes-101 (Satzquelle: Wikimedia, ebenfalls CC-BY-SA/CC-BY).
- Aenderungen: Audio von float32 nach PCM16 (16 kHz, mono) gewandelt, je Satz-ID eine Aufnahme
  ausgewaehlt (`scripts/bench/make_corpus.py fleurs`). Texte und Audioinhalt sind unveraendert.

Die Audiodateien liegen **nicht** im Repository und **nicht** im Installer, sondern nur lokal unter
`%LOCALAPPDATA%\lva-bench\`. Im Repository stehen ausschliesslich die Skripte und diese Auswertung
(Kennzahlen, keine Audiodaten und keine Satztexte).

## Synthetischer Mehrsprecher-Korpus

Erzeugt aus eigenen Vorlagentexten mit Windows-SAPI-Stimmen (Microsoft Hedda, Katja, Stefan) und
liegt ebenfalls nur lokal unter `%LOCALAPPDATA%\lva-bench\synth\`. Er unterliegt keiner
Fremdlizenz; die Stimmen werden nur zur lokalen Messung genutzt und nicht weitergegeben.

## AMI Meeting Corpus - CC-BY-4.0

Die Sprechertrennung (Diarisierung, DER-Messung `--eval-diarization`) wurde an Ausschnitten des
**AMI Meeting Corpus** geprüft: Entwicklungsstücke EN2002b und TS3003a, Prüfteil ES2004a und IS1009a
(jeweils die ersten 10 Minuten des Headset-Mixes; Zahlen in `koordination/granola-besprechungen/`,
BEFUNDE.md B7 und `entwurf/m3-sprecher.md`).

- Quelle: <https://groups.inf.ed.ac.uk/ami/corpus/>; bezogen über den Datensatz
  <https://huggingface.co/datasets/diarizers-community/ami> (Datensatzkarte: CC-BY-4.0; die Karte von
  `edinburghcstr/ami` nennt ebenfalls CC-BY-4.0).
- Urheber: AMI-Konsortium; Carletta, J. et al., *The AMI Meeting Corpus: A Pre-announcement*, MLMI 2005.
- Lizenz: Creative Commons Namensnennung 4.0 International (CC-BY-4.0),
  <https://creativecommons.org/licenses/by/4.0/>.
- Änderungen: nur Ausschnitte (die ersten 10 Minuten je Besprechung).

Die Audiodateien liegen **nicht** im Repository und **nicht** im Installer, sondern nur lokal unter
`%LOCALAPPDATA%\lva-bench\diar\`.

## Modelle

Die Modelle behalten ihre Lizenzen. Sie werden bei Bedarf über den Modellkatalog geladen und **nicht** mit dem
Installer ausgeliefert; einzige Ausnahme ist die kleine Silero-VAD-Datei. Die Lizenz je Eintrag steht auf der
Modellkarte bzw. im Repository der Quelle (Abruf 29.09.2026). Die App nennt die wichtigsten unter
Einstellungen → Info → Danksagungen.

| Modell | Verwendung | Lizenz | Quelle |
|---|---|---|---|
| Parakeet-TDT-0.6B-v3 (NVIDIA) | Live-Transkript, Diktat | CC-BY-4.0, Namensnennung NVIDIA | <https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3>; GGUF-Umsetzung `handy-computer/parakeet-tdt-0.6b-v3-gguf` (CC-BY-4.0) |
| Qwen3-ASR 0.6B und 1.7B (Alibaba/Qwen) | Enddurchlauf | Apache-2.0 | <https://huggingface.co/Qwen/Qwen3-ASR-1.7B> |
| Whisper large-v3 (OpenAI) | Enddurchlauf | Apache-2.0 laut Modellkarte (Code-Repository `openai/whisper`: MIT) | <https://huggingface.co/openai/whisper-large-v3> |
| Whisper large-v3-turbo (OpenAI) | Diktat, Enddurchlauf | MIT laut Modellkarte (der Katalogeintrag der GGUF-Fassung führt Apache-2.0; beides freizügig) | <https://huggingface.co/openai/whisper-large-v3-turbo> |
| Silero VAD v4 (Silero Team) | Spracherkennung (VAD), `silero_vad_v4.onnx` mitgeliefert | MIT | <https://github.com/snakers4/silero-vad> |
| NVIDIA Streaming Sortformer 4spk v2.1 | Sprechertrennung | **NVIDIA Open Model License** (kein Standard-Open-Source-Text) | <https://huggingface.co/nvidia/diar_streaming_sortformer_4spk-v2.1>; Lizenztext <https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/>; GGUF-Umsetzung `handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf` |
| BGE-M3 (BAAI) | semantische Suche, Chat | MIT | <https://huggingface.co/BAAI/bge-m3>; GGUF `gpustack/bge-m3-GGUF` (MIT) |
| Qwen3 0.6B/4B/8B, Qwen3.5 4B/9B (Alibaba/Qwen) | lokales Sprachmodell (Katalog, wählbar) | Apache-2.0 | Modellkarten `Qwen/Qwen3-*`, `Qwen/Qwen3.5-*` |
| Gemma 4 E4B und 12B (Google) | lokales Sprachmodell (Katalog, wählbar) | Apache-2.0 (Gemma-4-Lizenz) | <https://ai.google.dev/gemma/docs/gemma_4_license> |
| Gemma 3 4B (Google) | lokales Sprachmodell (Katalog, wählbar, vorbestehend) | Gemma Terms of Use (eigene Lizenz mit Nutzungsauflagen, Modellkarte: `gemma`) | <https://huggingface.co/google/gemma-3-4b-it> |

**Sortformer:** NVIDIA Streaming Sortformer 4spk v2.1, Lizenz NVIDIA Open Model License. Das Modell wird
geladen, nicht gebündelt. Der Lizenztext ist am Katalogeintrag (`license_url`) verlinkt und unter Info
erwähnt (Entscheidung E22).

**Vorbestehend, nicht für Besprechungen empfohlen:** Der ASR-Katalog enthält das Modell *Canary 1B* mit der
Lizenz CC-BY-NC-4.0 (nicht kommerziell). Es ist nicht vorgewählt, kein Standardmodell und kein Teil der
Besprechungsfunktion; es kam nicht durch das Goal hinzu. Die Einträge Nemotron ASR Streaming (Lizenz „other“)
sind ebenfalls vorbestehend und nicht vorgewählt.

## Bibliotheken (Rust), neu durch die Besprechungsfunktion

Ermittelt aus dem Unterschied der `Cargo.lock` gegen den Merge-Base mit `main` (`55036b23`); Lizenz je
Crate aus `cargo metadata` (Feld `license`). Neue npm-Pakete gibt es nicht.

| Crate | Version | Lizenz | Zweck |
|---|---|---|---|
| sonora, sonora-aec3, sonora-agc2, sonora-common-audio, sonora-fft, sonora-ns, sonora-simd | 0.2.0 | BSD-3-Clause | Echo-Unterdrückung (AEC3, Port von WebRTC), <https://github.com/dignifiedquire/sonora> |
| calcard | 0.3.14 | Apache-2.0 OR MIT | ICS-Kalender lesen und Serientermine ausrechnen, <https://github.com/stalwartlabs/calcard> |
| mail-parser | 0.11.9 | Apache-2.0 OR MIT | von calcard mitgebracht |
| hashify | 0.2.9 | Apache-2.0 OR MIT | von calcard mitgebracht |
| mail-builder | 1.0.0 | Apache-2.0 OR MIT | `.eml` der Follow-up-Mail, <https://github.com/stalwartlabs/mail-builder> |
| chrono-tz | 0.10.4 | MIT OR Apache-2.0 | von calcard mitgebracht (Zeitzonen der Kalendertermine) |
| ahash | 0.8.12 | MIT OR Apache-2.0 | von calcard mitgebracht |
| phf, phf_shared | 0.12.1 | MIT | von chrono-tz mitgebracht |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | Ableitungsmakros von bytemuck (u. a. genutzt von sonora-agc2) |
| syn | 3.0.6 | MIT OR Apache-2.0 | Bauzeit, über bytemuck_derive |
| transcribe-cpp, transcribe-cpp-sys | 0.2.4 (vorher 0.1.3) | MIT | lokale Spracherkennung und Sprechertrennung (GGUF/ggml), <https://github.com/handy-computer/transcribe.cpp> |

Ohne neues Paket, aber neu als direkte Abhängigkeit: `webview2-com` 0.38.2 (MIT, PDF-Export; kam vorher
nur über `wry`) und das Windows-Feature `Win32_Security_Cryptography` (DPAPI für die Kalender-Geheimnisse).

Keine der neuen Bibliotheken steht unter einer nicht-kommerziellen oder Copyleft-Lizenz. Die Lizenztexte der
Apache-/MIT-Crates liegen in deren Quellpaketen (`LICENSES/`), der Text von sonora steht unten, weil BSD-3-Clause
bei Weitergabe in Binärform den Vermerk verlangt.

**Vorbestehender Befund (nicht durch das Goal, nicht behoben):** `mp3lame-encoder` 0.2.5, der MP3-Export,
trägt laut `cargo metadata` die Lizenz **LGPL-3.0** und bringt LAME als C-Quellen mit. `deny.toml` erklärt, auf
dem Windows-Ziel liege kein LGPL-Crate im Graphen. Diese Aussage stimmt für diesen Crate nicht.
Bewertung und Umgang liegen beim Eigentümer.

### sonora (BSD-3-Clause)

```
Copyright (c) 2011, The WebRTC Project Authors. All rights reserved.
Copyright (c) 2016, Arun Raghavan and contributors. All rights reserved.
Copyright (c) 2026, dignifiedquire. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of Google nor the names of its contributors may
    be used to endorse or promote products derived from this software
    without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Entwicklungswerkzeuge

Diese Pakete laufen nur in der Entwicklung (Playwright-Tests, `devDependencies`). Sie sind **nicht** Teil des
Installers und der App.

| Paket | Verwendung | Lizenz | Quelle |
|---|---|---|---|
| `@axe-core/playwright` 4.13.0 (Deque Systems) | axe-Prüfung der Seite Aufnahmen (`tests/meeting-a11y.spec.ts`) | MPL-2.0 | <https://github.com/dequelabs/axe-core-npm> |
| `axe-core` 4.13.0 (Deque Systems), von `@axe-core/playwright` mitgebracht | Regelwerk der Barrierefreiheitsprüfung | MPL-2.0 | <https://github.com/dequelabs/axe-core> |

Die MPL-2.0 (<https://www.mozilla.org/en-US/MPL/2.0/>) verlangt bei Weitergabe die Nennung und den Quelltext der
Dateien selbst; beides ist hier erfüllt, weil nichts davon weitergegeben wird und die Pakete unverändert aus dem
npm-Register kommen.
