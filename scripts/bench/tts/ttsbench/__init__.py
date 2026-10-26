"""TTS-Benchmark-Harness (Paket P5, Goal tts-provider).

Reihenfolge der Bausteine: Korpus -> Lauf (Quelle: App-Binary oder ``fake``) -> ASR -> Bericht
-> Hoertest-Paket. Nichts hier beruehrt App-Daten oder das Repo ausser ``report --publish``.
"""
import sys
from pathlib import Path

HARNESS_VERSION = "tts-bench/1.0.0"

# benchlib (Prozess-Deckel, bench_dir) liegt im Elternordner scripts/bench
_BENCH_DIR = Path(__file__).resolve().parents[2]
if str(_BENCH_DIR) not in sys.path:
    sys.path.insert(0, str(_BENCH_DIR))
