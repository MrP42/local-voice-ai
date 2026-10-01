# -*- coding: utf-8 -*-
"""Zahlentreue: Anteil der Zahl-Token der Folie (z. B. 12,4 / 43,0 / 1,250 / 01.01.2027),
die exakt im OCR-Text vorkommen. Satz: Videobilder (B_webcam)."""
import collections, json, os, re
M = os.path.dirname(os.path.abspath(__file__))
TRUTH = json.load(open(os.path.join(M, "slides", "truth.json"), encoding="utf-8"))
NUM = re.compile(r"[+\-−]?\d[\d.,:/]*\d|\d")

def nums(s):
    return [n.replace("−", "-").lstrip("+-") for n in NUM.findall(s)]

def score(texts):
    hit = tot = 0
    for sid, txt in texts.items():
        if sid.startswith("s11_build") and sid != "s11_build3_de":
            continue
        t = collections.Counter(nums(" ".join(TRUTH[sid]))); o = collections.Counter(nums(txt))
        hit += sum((t & o).values()); tot += sum(t.values())
    return hit, tot

ocr = json.load(open(os.path.join(M, "out", "ocr_texts.json"), encoding="utf-8"))
rows = {}
for k, v in ocr.items():
    s, e, sid = k.split("|")
    if s == "video": rows.setdefault(e, {})[sid] = v
for name in ["gemma4-e4b", "gemma4-12b"]:
    p = os.path.join(M, "out", f"vision_{name}.json")
    if os.path.exists(p):
        rows[f"LLM-OCR {name}"] = {r["sid"]: r["ocr_text"] for r in json.load(open(p, encoding="utf-8"))["rows"]}
out = {e: "%d/%d" % score(t) for e, t in rows.items()}
json.dump(out, open(os.path.join(M, "out", "numbers.json"), "w"), indent=1)
for e, s in out.items(): print(f"{e:<32} {s}")
