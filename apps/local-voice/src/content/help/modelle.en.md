# Models

Three families, all local: transcription models for dictation, language models for translating, summarizing and minutes, reading voices for Piper.

## Transcription models

- **Parakeet** is fast and accurate, even without a GPU. The German-tuned primeLine variant is the first choice for German dictation.
- **Whisper** models are slower but know more languages. Large models need a GPU.
- The active model is shown in the footer and can be switched there. Search filters by name, the language filter by language.
- Accuracy and speed are indicative values from measurements, not guarantees.

## Language models

- Run on a built-in runtime on CPU or GPU. The runtime is downloaded with the first model.
- **Does it fit?** estimates the memory demand against free graphics memory before the download. "Tight" means it runs, but without headroom.
- Qwen 3.5 4B is a good default. Gemma 4 sounds different and is an alternative. Larger models need memory accordingly.
- Providers such as Ollama or a cloud service can be connected under Settings, AI text improvement.

## Reading voices

- Piper voices are small, language-bound voices for the CPU. Download the languages you read. When reading, the app picks the voice by the sentence's language.
- HQ is the best quality with the largest download.
- Your own and cloned voices for Fish Speech are managed under Settings, Read aloud.

## Storage

Deleted models free their space immediately. Where the files live is shown under Settings, General, data directory.
