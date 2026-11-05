# iPhone CPU inference dependencies

These libraries and models are used only by the iPhone target. The Watch target
contains no whisper.cpp, llama.cpp, ONNX, Rust or model weights.

- whisper.cpp v1.7.5: MIT. https://github.com/ggml-org/whisper.cpp/blob/v1.7.5/LICENSE
- llama.cpp b5046 (documented XCFramework release): MIT. https://github.com/ggml-org/llama.cpp/blob/b5046/LICENSE
- Whisper Base and optional Small multilingual weights (OpenAI Whisper conversion): MIT.
  https://huggingface.co/ggerganov/whisper.cpp and https://github.com/openai/whisper/blob/main/LICENSE
- Qwen2.5-0.5B-Instruct-GGUF Q4_K_M: Apache-2.0.
  https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF
- Gemma 4 E2B Instruct GGUF Q4_K_M (Buch-Gespraech, optional download): Apache-2.0.
  https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF at revision 0314792d7f1f7e229411f620751375812bb9faf2,
  SHA-256 740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8, 3106738272 bytes (Hugging Face LFS metadata).

`setup_native_engines.py` pins all dependency SHA-256 values. The Whisper archive hash
was recorded from its official HTTPS release download; the llama hash matches
the publisher's XCFramework documentation. Model hashes were checked against
Hugging Face LFS metadata. Moving upstream refs cannot silently change accepted
bytes. Binaries and weights are excluded from Git. Retain upstream license texts
when packaging or redistributing; no external distribution occurs in this task.
