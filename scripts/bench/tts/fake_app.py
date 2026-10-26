"""Attrappe des App-Binarys fuer die Tests von ``AppSource`` (Aufrufvertrag wie ``--tts-bench``).

Modus ueber ``FAKE_APP_MODE``: ok (Standard, schreibt rows.jsonl/env.json/WAVs), fail (kein Ergebnis, Exit 3),
hang (schlaeft, fuer den Zeitueberschreitungs-Test). Nur fuer Tests, keine Messquelle.
"""
import argparse
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ttsbench.config import BenchConfig  # noqa: E402
from ttsbench.sources import FakeSource  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tts-bench", action="store_true", required=True)
    ap.add_argument("--tts-provider", required=True)
    ap.add_argument("--tts-voice")
    ap.add_argument("--tts-lang", required=True)
    ap.add_argument("--bench-corpus", required=True)
    ap.add_argument("--bench-subset")
    ap.add_argument("--bench-warm", type=int, required=True)
    ap.add_argument("--bench-cold", type=int, required=True)
    ap.add_argument("--bench-out", required=True)
    ap.add_argument("--json", action="store_true", required=True)
    a = ap.parse_args()
    mode = os.environ.get("FAKE_APP_MODE", "ok")
    if mode == "fail":
        print("Anbieter nicht installiert (Attrappe)", file=sys.stderr)
        return 3
    if mode == "hang":
        time.sleep(120)
        return 0
    cfg = BenchConfig(id=a.tts_provider, provider=a.tts_provider, voice=a.tts_voice, language=a.tts_lang, device="cpu")
    FakeSource(seed=1).run(cfg, Path(a.bench_corpus), a.bench_subset or "full", a.bench_warm, a.bench_cold,
                           Path(a.bench_out), None)
    print('{"ok": true}')
    return 0


if __name__ == "__main__":
    sys.exit(main())
