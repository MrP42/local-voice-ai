"""Attrappe der App-CLI ``-f <wav> --model <id> --json --out <datei>`` fuer die Tests von ``AppCliEngine``.

Verhaelt sich wie das echte Binary (apps/local-voice/src-tauri/src/lib.rs): verlangt 16 kHz / mono / 16-bit PCM
(sonst Exit 2, kein Resampling), schreibt ``text``, ``audio_secs``, ``bound_backend``. Modus ueber ``FAKE_ASR_MODE``:
ok (Standard), fail (Exit 1). Text ueber ``FAKE_ASR_TEXT``. Die Aufrufe werden in ``FAKE_ASR_LOG`` (JSON-Zeilen) notiert.
Nur fuer Tests, kein Messwerkzeug.
"""
import argparse
import json
import os
import sys

import soundfile as sf


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("-f", dest="wav", required=True)
    ap.add_argument("--model")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--out")
    a = ap.parse_args()
    if os.environ.get("FAKE_ASR_MODE", "ok") == "fail":
        print("error: load_model failed (Attrappe)", file=sys.stderr)
        return 1
    info = sf.info(a.wav)
    if info.samplerate != 16000 or info.channels != 1 or info.subtype != "PCM_16":
        print(f"error: expected 16 kHz mono 16-bit PCM WAV, got {info.samplerate} Hz / {info.channels} ch / "
              f"{info.subtype}", file=sys.stderr)
        return 2
    audio_secs = info.frames / 16000.0
    log = os.environ.get("FAKE_ASR_LOG")
    if log:
        with open(log, "a", encoding="utf-8") as f:
            f.write(json.dumps({"model": a.model, "samplerate": info.samplerate, "audio_secs": audio_secs}) + "\n")
    res = {"text": os.environ.get("FAKE_ASR_TEXT", "hallo welt"), "audio_secs": audio_secs, "model_id": a.model,
           "bound_backend": "CPU (Attrappe)", "load_ms": 1, "best_ms": 1}
    if a.out:
        with open(a.out, "w", encoding="utf-8") as f:
            json.dump(res, f)
    if a.json:
        print(json.dumps(res))
    return 0


if __name__ == "__main__":
    sys.exit(main())
