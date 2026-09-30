"""P3e: reference RTTM for the TTS radio play from the read-aloud script JSON.

The .mp3.json written by the app's read-aloud export carries one entry per
sentence: {text, voice, start_ms, end_ms}. The spans are contiguous (each TTS
clip includes its own leading/trailing silence and [break] pauses), so the
speaker label per span is exact, but a span is not all speech.

Two reference files are written:
  <uri>.spans.rttm  the raw spans (speaker exact, silence counted as speech)
  <uri>.rttm        the spans cut to speech: 10 ms frames above a level
                    threshold (dBFS), gaps < --bridge-ms bridged, islands
                    < --min-ms dropped, always clipped to the span

The trimmed file is the one scored (the other reference RTTMs of the
diarization corpus are speech-only as well); the spans file is kept to show
how much the DER depends on the trimming.

Usage:
  python p3e_reference.py --script X.mp3.json --wav X_16k.wav --uri hoerspiel
         --out-dir DIR [--threshold-db -50] [--bridge-ms 300] [--min-ms 50]

stdlib only.
"""
import argparse
import array
import json
import math
import os
import sys
import wave

FRAME_MS = 10


def frame_db(path):
    with wave.open(path, "rb") as w:
        if w.getnchannels() != 1 or w.getsampwidth() != 2:
            sys.exit("need mono 16-bit PCM WAV")
        rate = w.getframerate()
        pcm = array.array("h", w.readframes(w.getnframes()))
    step = rate * FRAME_MS // 1000
    out = []
    for i in range(0, len(pcm), step):
        chunk = pcm[i:i + step]
        if not chunk:
            break
        e = sum(x * x for x in chunk) / len(chunk)
        out.append(10.0 * math.log10(e / (32768.0 ** 2) + 1e-12))
    return out, len(pcm) * 1000 // rate


def speech_runs(db, a, b, thr, bridge, min_len):
    """Speech runs (frame indices, end exclusive) inside frames [a, b)."""
    runs = []
    cur = None
    for f in range(a, min(b, len(db))):
        if db[f] > thr:
            if cur is None:
                cur = [f, f + 1]
            elif f - cur[1] < bridge:
                cur[1] = f + 1
            else:
                runs.append(cur)
                cur = [f, f + 1]
    if cur is not None:
        runs.append(cur)
    return [r for r in runs if r[1] - r[0] >= min_len]


def rttm_line(uri, start_s, dur_s, spk):
    return "SPEAKER {} 1 {:.3f} {:.3f} <NA> <NA> {} <NA> <NA>\n".format(uri, start_s, dur_s, spk)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--script", required=True)
    ap.add_argument("--wav", required=True)
    ap.add_argument("--uri", required=True)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--threshold-db", type=float, default=-50.0)
    ap.add_argument("--bridge-ms", type=int, default=300)
    ap.add_argument("--min-ms", type=int, default=50)
    ap.add_argument("--suffix", default="", help="name suffix for sensitivity variants")
    ap.add_argument("--from-s", type=float, default=None,
                    help="excerpt: --wav is already cut to [from, to); spans are clipped and shifted")
    ap.add_argument("--to-s", type=float, default=None)
    args = ap.parse_args()

    with open(args.script, encoding="utf-8") as f:
        script = json.load(f)
    segs = script["segments"]
    if args.from_s is not None:
        lo, hi = int(args.from_s * 1000), int((args.to_s or 1e9) * 1000)
        segs = [dict(s, start_ms=max(s["start_ms"], lo) - lo, end_ms=min(s["end_ms"], hi) - lo)
                for s in segs if s["end_ms"] > lo and s["start_ms"] < hi]
    db, audio_ms = frame_db(args.wav)
    bridge = max(1, args.bridge_ms // FRAME_MS)
    min_len = max(1, args.min_ms // FRAME_MS)

    spans, trimmed = [], []
    per_voice = {}
    for s in segs:
        v = s["voice"]
        a_ms, b_ms = int(s["start_ms"]), int(s["end_ms"])
        spans.append(rttm_line(args.uri, a_ms / 1000.0, (b_ms - a_ms) / 1000.0, v))
        st = per_voice.setdefault(v, {"sentences": 0, "span_s": 0.0, "speech_s": 0.0, "empty": 0})
        st["sentences"] += 1
        st["span_s"] += (b_ms - a_ms) / 1000.0
        runs = speech_runs(db, a_ms // FRAME_MS, (b_ms + FRAME_MS - 1) // FRAME_MS,
                           args.threshold_db, bridge, min_len)
        if not runs:
            st["empty"] += 1
        for r0, r1 in runs:
            a_s = max(r0 * FRAME_MS, a_ms) / 1000.0
            b_s = min(r1 * FRAME_MS, b_ms) / 1000.0
            if b_s > a_s:
                trimmed.append(rttm_line(args.uri, a_s, b_s - a_s, v))
                st["speech_s"] += b_s - a_s

    os.makedirs(args.out_dir, exist_ok=True)
    base = os.path.join(args.out_dir, args.uri + args.suffix)
    with open(base + ".spans.rttm", "w", encoding="utf-8", newline="\n") as f:
        f.writelines(spans)
    with open(base + ".rttm", "w", encoding="utf-8", newline="\n") as f:
        f.writelines(trimmed)
    summary = {
        "script": os.path.basename(args.script),
        "audio_ms": audio_ms,
        "script_end_ms": segs[-1]["end_ms"],
        "sentences": len(segs),
        "voices": len(per_voice),
        "threshold_db": args.threshold_db,
        "bridge_ms": args.bridge_ms,
        "min_ms": args.min_ms,
        "span_s": round(sum(v["span_s"] for v in per_voice.values()), 2),
        "speech_s": round(sum(v["speech_s"] for v in per_voice.values()), 2),
        "per_voice": {k: {kk: round(vv, 2) if isinstance(vv, float) else vv for kk, vv in v.items()}
                      for k, v in sorted(per_voice.items(), key=lambda kv: -kv[1]["span_s"])},
        "turns_trimmed": len(trimmed),
    }
    print(json.dumps(summary, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
