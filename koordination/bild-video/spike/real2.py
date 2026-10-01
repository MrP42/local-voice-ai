# -*- coding: utf-8 -*-
"""Realtest Stufe 2: Text-Abgleich per Windows-OCR auf den Repraesentanten.
- weniger als 3 Woerter -> keine Folie (Kamera, Schwarzbild)
- Wort-Jaccard >= 0.5 zu einer bereits behaltenen Folie -> Dublette (Zeitbereich anhaengen)"""
import json, os, re, sys, time
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
from ocr import win_ocr

od = os.path.join(M, "real_frames")
files = sorted(os.path.join(od, f) for f in os.listdir(od))
t = time.perf_counter(); out = win_ocr(files); t_ocr = time.perf_counter() - t
kept = []  # (datei, woerter, [zeitpunkte])
dropped = []
for f in files:
    txt = out[f][0]; w = set(x.lower() for x in re.findall(r"\w{2,}", txt))
    sec = int(os.path.basename(f).split("_")[1][:-5])
    if len(w) < 3:
        dropped.append((os.path.basename(f), txt.replace("\n", " ")[:40])); continue
    for k in kept:
        j = len(w & k[1]) / max(1, len(w | k[1]))
        if j >= 0.5:
            k[2].append(sec); k[1] |= w; break
    else:
        kept.append([os.path.basename(f), w, [sec]])
res = dict(frames=len(files), ocr_s=round(t_ocr, 2), slides=len(kept), dropped=len(dropped),
           kept=[(k[0], k[2], " ".join(sorted(k[1]))[:60]) for k in kept], dropped_list=dropped)
json.dump(res, open(os.path.join(M, "out", "real2.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
print(json.dumps(res, ensure_ascii=False, indent=0))
