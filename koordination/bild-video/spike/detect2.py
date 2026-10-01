# -*- coding: utf-8 -*-
"""Nachmessung: (a) Zuordnung mit ausgeblendetem Webcam-Bereich,
(b) adaptive Maske: Kacheln, die sich dauernd aendern (Webcam, Video im Video),
werden ignoriert; Wechsel = Anteil geaenderter Pixel ausserhalb der Maske."""
import json, os, sys, time
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import numpy as np
from PIL import Image
import imagehash
from detect import sample_hashes, score_bounds, TL, UNIQUE_BASE

W, H = 160, 90

def masked_ident(buf_frames, idx, refs_arr, mask):
    out = []
    for i in idx:
        f = buf_frames[i].astype(np.int16)
        best = min(refs_arr.items(), key=lambda kv: np.abs((kv[1] - f))[mask].mean())
        out.append(best[0])
    return out

def adaptive_mask(buf, tile=10, frac=0.3, thr=12):
    d = np.abs(np.diff(buf.astype(np.int16), axis=0)) > thr  # (n-1,H,W)
    th, tw = H // tile, W // tile
    t = d[:, :th * tile, :tw * tile].reshape(d.shape[0], th, tile, tw, tile).any(axis=(2, 4))
    busy = t.mean(axis=0) > frac  # Kachel aendert sich in >30 % der Schritte
    m = np.ones((H, W), bool)
    for y in range(th):
        for x in range(tw):
            if busy[y, x]: m[y * tile:(y + 1) * tile, x * tile:(x + 1) * tile] = False
    return m, int(busy.sum()), th * tw

def pixel_bounds(buf, fps, mask, pix_thr=12, area=0.003, stable=2):
    bounds = []; cur = buf[0].astype(np.int16); n = len(buf)
    for i in range(1, n):
        f = buf[i].astype(np.int16)
        ch = (np.abs(f - cur) > pix_thr)[mask].mean()
        if ch > area:
            ok = all(i + k < n and (np.abs(buf[i + k].astype(np.int16) - f) > pix_thr)[mask].mean() <= area
                     for k in range(1, stable))
            if ok: bounds.append(i / fps); cur = f
    return bounds

def main():
    refs = {}
    for f in sorted(os.listdir(os.path.join(M, "slides"))):
        if f.endswith(".png"):
            refs[f[:-4]] = np.asarray(Image.open(os.path.join(M, "slides", f)).convert("L").resize((W, H), Image.BOX)).astype(np.int16)
    res = []
    for vid in ["A_clean.mp4", "B_webcam.mp4"]:
        for fps in [1, 0.5]:
            t_dec, _, ph, dh, buf = sample_hashes(os.path.join(M, "video", vid), fps)
            t = time.perf_counter(); mask, nb, nt = adaptive_mask(buf); t_mask = time.perf_counter() - t
            for name, fn in [("dHash thr=4", None), ("Pixel+adaptive Maske", "px")]:
                t = time.perf_counter()
                if fn:
                    b = pixel_bounds(buf, fps, mask)
                else:
                    from detect import segment
                    b = segment(dh, fps, 4)
                t_seg = time.perf_counter() - t + (t_mask if fn else 0)
                tp, fp = score_bounds(b, 1.0 / fps + 0.5)
                starts = [0] + [int(x * fps) for x in b]; ends = starts[1:] + [len(buf)]
                reps = [max(s, e - 1) for s, e in zip(starts, ends)]
                ids = masked_ident(buf, reps, refs, mask)
                # Dubletten ueber den ganzen Vortrag: gleiche Folie (maskierte Differenz klein)
                distinct = []
                for r in reps:
                    f = buf[r].astype(np.int16)
                    if not any((np.abs(f - buf[x].astype(np.int16)) > 12)[mask].mean() <= 0.003 for x in distinct):
                        distinct.append(r)
                uniq = {i.split("_build")[0] for i in ids}
                res.append(dict(video=vid, fps=fps, method=name, dec_s=round(t_dec, 1), seg_s=round(t_seg, 2),
                                masked_tiles=f"{nb}/{nt}", hits=f"{tp}/16", false=fp, segs=len(starts),
                                distinct=len(distinct), covered=f"{len(uniq & set(UNIQUE_BASE))}/{len(UNIQUE_BASE)}",
                                build3=("s11_build3_de" in ids),
                                times=[round(x) for x in b]))
    json.dump(res, open(os.path.join(M, "out", "detect2.json"), "w"), indent=1)
    print("Wahrheit:", [s["start"] for s in TL[1:]])
    for r in res: print(r)

if __name__ == "__main__":
    main()
