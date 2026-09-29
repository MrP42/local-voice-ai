# Namensnennung und Lizenzen der Benchmark-Daten

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

## Modelle

Die gemessenen Modelle (Parakeet-TDT-0.6B-v3: CC-BY-4.0, NVIDIA; Whisper large-v3-turbo: Apache-2.0/MIT,
OpenAI; Qwen3-ASR: Apache-2.0, Alibaba) behalten ihre Lizenzen; die Namensnennung steht in
den jeweiligen Modellkarten und in den Third-Party-Hinweisen der App.
