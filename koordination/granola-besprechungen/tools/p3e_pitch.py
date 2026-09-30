"""P3e: rough fundamental frequency (F0) per reference voice - an indicator of how
similar the voices are, not a speaker embedding.

For every voice, up to --frames 40 ms frames are taken evenly from its reference turns
(mono 16 kHz 16-bit WAV); per frame the autocorrelation peak in 70..450 Hz is taken if
the normalised peak is >= 0.5 (voiced). Reported: median and quartiles of F0.

Usage: python p3e_pitch.py --wav X.wav --ref X.rttm [--frames 150] --out pitch.json
stdlib only.
"""
import argparse
import array
import json
import wave


def autocorr_f0(x, rate, fmin=70.0, fmax=450.0):
    n = len(x)
    mean = sum(x) / n
    x = [v - mean for v in x]
    e0 = sum(v * v for v in x)
    if e0 <= 0:
        return None
    best, best_lag = 0.0, None
    for lag in range(int(rate / fmax), int(rate / fmin) + 1):
        s = 0.0
        for i in range(n - lag):
            s += x[i] * x[i + lag]
        r = s / e0
        if r > best:
            best, best_lag = r, lag
    return rate / best_lag if best_lag and best >= 0.5 else None


def quart(v):
    v = sorted(v)
    q = lambda p: v[min(len(v) - 1, int(p * (len(v) - 1) + 0.5))]
    return q(0.25), q(0.5), q(0.75)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--wav", required=True)
    ap.add_argument("--ref", required=True)
    ap.add_argument("--frames", type=int, default=150)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    with wave.open(a.wav, "rb") as w:
        rate = w.getframerate()
        pcm = array.array("h", w.readframes(w.getnframes()))
    turns = {}
    with open(a.ref, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            turns.setdefault(p[7], []).append((float(p[3]), float(p[3]) + float(p[4])))
    flen = int(0.04 * rate)
    out = {}
    for voice, ts in sorted(turns.items()):
        starts = []
        for s0, s1 in ts:
            t = s0 + 0.05
            while t + 0.04 < s1 - 0.05:
                starts.append(t)
                t += 0.04
        step = max(1, len(starts) // a.frames)
        f0s = []
        for t in starts[::step][: a.frames]:
            i = int(t * rate)
            f0 = autocorr_f0(pcm[i:i + flen], rate)
            if f0:
                f0s.append(f0)
        q1, med, q3 = quart(f0s) if f0s else (None, None, None)
        out[voice] = {"frames": min(a.frames, len(starts[::step])), "voiced": len(f0s),
                      "f0_q1_hz": round(q1) if q1 else None, "f0_median_hz": round(med) if med else None,
                      "f0_q3_hz": round(q3) if q3 else None}
        print(voice, out[voice])
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, indent=1)


if __name__ == "__main__":
    main()
