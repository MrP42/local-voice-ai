# -*- coding: utf-8 -*-
"""Realtest CC-Video: dHash thr=4 @1 fps + Dubletten, Kontaktabzug, PDF-Seitenzahl."""
import json, os, subprocess, sys, time
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import numpy as np
from PIL import Image, ImageDraw
from detect import sample_hashes, segment, ffmpeg_scene

V = os.path.join(M, "video", "C_cc_talk.webm")
t_dec, t_hash, ph, dh, buf = sample_hashes(V, 1)
res = {"decode_s": round(t_dec, 1), "hash_s": round(t_hash, 2), "samples": len(buf)}
for thr in [4, 8]:
    b = segment(dh, 1, thr)
    starts = [0] + [int(x) for x in b]; ends = starts[1:] + [len(buf)]
    reps = [max(s, e - 1) for s, e in zip(starts, ends)]
    distinct = []
    for r in reps:
        if not any(dh[r] - dh[x] <= thr for x in distinct): distinct.append(r)
    res[f"thr{thr}"] = dict(segments=len(starts), distinct=len(distinct), starts=starts)
dt, ts = ffmpeg_scene(V, 0.05, pre="fps=2,scale=480:-2")
res["ffmpeg_scene_0.05"] = dict(sec=round(dt, 1), changes=len(ts), times=[round(t) for t in ts])
try:
    from pypdf import PdfReader
    res["pdf_pages"] = len(PdfReader(os.path.join(M, "video", "C_cc_talk_slides.pdf")).pages)
except Exception as e:
    res["pdf_pages"] = f"? ({e})"
# Kontaktabzug der Repraesentanten (thr=4, nach Dubletten)
st = res["thr4"]["starts"]; ends = st[1:] + [len(buf)]
reps = [max(s, e - 1) for s, e in zip(st, ends)]
od = os.path.join(M, "real_frames"); os.makedirs(od, exist_ok=True)
thumbs = []
for k, r in enumerate(reps):
    p = os.path.join(od, f"r{k:02d}_{r:04d}s.png")
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", str(r + 0.5), "-i", V,
                    "-frames:v", "1", p], creationflags=0x08000000, check=True)
    thumbs.append((r, Image.open(p).resize((270, 152))))
cols = 6; rows = (len(thumbs) + cols - 1) // cols
sheet = Image.new("RGB", (cols * 280, rows * 175), "white"); d = ImageDraw.Draw(sheet)
for i, (r, im) in enumerate(thumbs):
    x, y = (i % cols) * 280 + 5, (i // cols) * 175 + 5
    sheet.paste(im, (x, y)); d.text((x, y + 154), f"#{i} {r//60}:{r%60:02d}", fill="black")
sheet.save(os.path.join(M, "out", "real_contact.png"))
json.dump(res, open(os.path.join(M, "out", "real.json"), "w"), indent=1)
print(json.dumps({k: (v if k not in ("thr4", "thr8") else {kk: vv for kk, vv in v.items() if kk != "starts"}) for k, v in res.items()}))
