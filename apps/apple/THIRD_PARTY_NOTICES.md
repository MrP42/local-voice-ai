# iPhone CPU inference dependencies

## Qwen3-4B-Instruct-2507

Local meeting minutes use Qwen3-4B-Instruct-2507, Q4_K_M GGUF by unsloth.
Repository: https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF
Revision: a06e946bb6b655725eafa393f4a9745d460374c9
File: Qwen3-4B-Instruct-2507-Q4_K_M.gguf (2497281120 bytes).
SHA-256: 3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597
License: Apache-2.0, https://www.apache.org/licenses/LICENSE-2.0
Upstream: https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507

## Yogurt — rolling live refinement

Chunking, per-track language locking and hallucination filtering in
`LiveChunker.swift`, `LiveLanguageLock.swift` and `LiveHallucination.swift`
are adapted from FrigadeHQ/yogurt @76e5595 (https://github.com/FrigadeHQ/yogurt).
No system-audio echo filter is included on iPhone.

MIT License

Copyright (c) 2026 Frigade

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Native engines

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
