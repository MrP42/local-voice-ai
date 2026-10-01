# -*- coding: utf-8 -*-
"""Folienerkennung aus Video: Laufzeit und Trefferquote je Methode.
Aufruf: python detect.py  (benutzt video/A_clean.mp4, video/B_webcam.mp4)"""
import json, os, re, subprocess, sys, time
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import numpy as np
from PIL import Image
import imagehash

NOWIN = 0x08000000
TL = json.load(open(os.path.join(M, "video", "timeline.json")))
TRUTH_BOUNDS = [s["start"] for s in TL[1:]]  # 16 Wechsel
UNIQUE_BASE = sorted({s["slide"].split("_build")[0] for s in TL})  # 14 Folien (Builds = 1)

def run(cmd):
    t = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, creationflags=NOWIN)
    return time.perf_counter() - t, p

def score_bounds(det, tol):
    det = sorted(det); used = set(); tp = 0
    for b in TRUTH_BOUNDS:
        c = [i for i, d in enumerate(det) if abs(d - b) <= tol and i not in used]
        if c: used.add(c[0]); tp += 1
    return tp, len(det) - len(used)  # Treffer, Fehlalarme

def ffmpeg_scene(video, thr, pre="", extra_in=(), threads=None):
    vf = (pre + "," if pre else "") + f"select='gt(scene,{thr})',showinfo"
    cmd = ["ffmpeg", "-hide_banner", "-nostats"] + list(extra_in)
    if threads: cmd += ["-threads", str(threads)]
    cmd += ["-i", video, "-an", "-vf", vf, "-f", "null", "-"]
    dt, p = run(cmd)
    times = [float(x) for x in re.findall(r"pts_time:([0-9.]+)", p.stderr.decode("utf-8", "replace"))]
    return dt, times

def sample_hashes(video, fps, w=160, h=90, threads=None):
    """ffmpeg dekodiert, verkleinert und liefert Graustufen-Rohbilder per Pipe."""
    cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error"]
    if threads: cmd += ["-threads", str(threads)]
    cmd += ["-i", video, "-an", "-vf", f"fps={fps},scale={w}:{h}:flags=area,format=gray",
            "-f", "rawvideo", "-"]
    t = time.perf_counter()
    p = subprocess.run(cmd, capture_output=True, creationflags=NOWIN)
    t_dec = time.perf_counter() - t
    buf = np.frombuffer(p.stdout, np.uint8).reshape(-1, h, w)
    t = time.perf_counter()
    hs = [imagehash.phash(Image.fromarray(f), hash_size=8) for f in buf]
    ds = [imagehash.dhash(Image.fromarray(f), hash_size=8) for f in buf]
    t_hash = time.perf_counter() - t
    return t_dec, t_hash, hs, ds, buf

def segment(hashes, fps, thr, stable=2):
    """Wechsel, wenn der Abstand zum aktuellen Folien-Hash > thr ist und der neue
    Zustand `stable` Abtastungen haelt (filtert Ueberblendungen/Zuckler)."""
    bounds = []; cur = hashes[0]; i = 1; n = len(hashes)
    while i < n:
        if hashes[i] - cur > thr:
            j = i
            ok = all(j + k < n and hashes[j + k] - hashes[j] <= thr for k in range(1, stable))
            if ok:
                bounds.append(i / fps); cur = hashes[i]
        i += 1
    return bounds

def identify(frames_idx, buf, refs):
    """Zuordnung erkannter Folien zu Quellfolien (naechster pHash)."""
    out = []
    for i in frames_idx:
        h = imagehash.phash(Image.fromarray(buf[i]))
        best = min(refs.items(), key=lambda kv: kv[1] - h)
        out.append(best[0])
    return out

def main():
    res = []
    refs = {}
    for f in sorted(os.listdir(os.path.join(M, "slides"))):
        if f.endswith(".png"):
            im = Image.open(os.path.join(M, "slides", f)).convert("L").resize((160, 90), Image.BOX)
            refs[f[:-4]] = imagehash.phash(im)
    for vid in ["A_clean.mp4", "B_webcam.mp4"]:
        v = os.path.join(M, "video", vid)
        for thr in [0.05, 0.1, 0.3]:
            dt, ts = ffmpeg_scene(v, thr)
            tp, fp = score_bounds(ts, 1.0)
            res.append(dict(video=vid, method=f"ffmpeg scene>{thr} voll", sec=round(dt, 1), hits=tp, false=fp))
        for thr in [0.05, 0.1]:
            dt, ts = ffmpeg_scene(v, thr, pre="fps=2,scale=480:-2")
            tp, fp = score_bounds(ts, 1.0)
            res.append(dict(video=vid, method=f"ffmpeg fps=2,480p scene>{thr}", sec=round(dt, 1), hits=tp, false=fp))
        dt, ts = ffmpeg_scene(v, 0.05, pre="fps=2,scale=480:-2", threads=4)
        tp, fp = score_bounds(ts, 1.0)
        res.append(dict(video=vid, method="ffmpeg fps=2,480p scene>0.05, 4 Threads", sec=round(dt, 1), hits=tp, false=fp))
        dt, ts = ffmpeg_scene(v, 0.05, extra_in=["-skip_frame", "nokey"])
        tp, fp = score_bounds(ts, 1.0)
        res.append(dict(video=vid, method="ffmpeg nur Keyframes scene>0.05", sec=round(dt, 1), hits=tp, false=fp))
        for fps in [1, 0.5]:
            for threads in [None, 4]:
                t_dec, t_hash, ph, dh, buf = sample_hashes(v, fps, threads=threads)
                for name, hs in [("pHash", ph), ("dHash", dh)]:
                    for thr in [4, 8, 12]:
                        b = segment(hs, fps, thr)
                        tp, fp = score_bounds(b, 1.0 / fps + 0.5)
                        # Repraesentant: letztes Bild vor dem naechsten Wechsel (Build = Endstand)
                        starts = [0] + [int(x * fps) for x in b]
                        ends = starts[1:] + [len(buf)]
                        reps = [max(s, e - 1) for s, e in zip(starts, ends)]
                        ids = identify(reps, buf, refs)
                        uniq = sorted({i.split("_build")[0] for i in ids})
                        # Dubletten-Zusammenfassung: gleiche Folie (Hash <= thr) wiederkehrend
                        repl = [hs[r] for r in reps]; distinct = []
                        for hh in repl:
                            if not any(hh - x <= thr for x in distinct): distinct.append(hh)
                        res.append(dict(video=vid, method=f"{fps} fps {name} thr={thr}" + (" 4 Thr" if threads else ""),
                                        sec=round(t_dec + t_hash, 1), dec=round(t_dec, 1), hash=round(t_hash, 2),
                                        hits=tp, false=fp, segs=len(starts), distinct=len(distinct),
                                        covered=f"{len(set(uniq) & set(UNIQUE_BASE))}/{len(UNIQUE_BASE)}",
                                        final_build=("s11_build3_de" in ids)))
    json.dump(res, open(os.path.join(M, "out", "detect.json"), "w"), indent=1)
    for r in res: print(r)

if __name__ == "__main__":
    main()
