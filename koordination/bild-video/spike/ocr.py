# -*- coding: utf-8 -*-
"""OCR-Vergleich: Windows.Media.Ocr, Tesseract 5, RapidOCR (PP-OCRv5 latin).
Saetze: slides/*.png (sauber) und frames/*.png (aus B_webcam.mp4, H.264)."""
import asyncio, json, os, re, subprocess, sys, time, unicodedata, collections
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import jiwer

NOWIN = 0x08000000
TRUTH = json.load(open(os.path.join(M, "slides", "truth.json"), encoding="utf-8"))
UML = set("äöüÄÖÜß")
SPECIAL = ["€", "§", "%", "„", "“", "–"]

def toks(s):
    return re.findall(r"\w+", unicodedata.normalize("NFC", s))

def metrics(truth_lines, text):
    t = toks(" ".join(truth_lines)); o = toks(text)
    ct, co = collections.Counter(t), collections.Counter(o)
    hit = sum((ct & co).values())
    tu = [w for w in t if UML & set(w)]; cu = collections.Counter(tu)
    uh = sum((cu & co).values())
    ref = " ".join(" ".join(truth_lines).split()); hyp = " ".join(text.split())
    cer = jiwer.cer(ref, hyp) if hyp else 1.0
    sp_t = sum(" ".join(truth_lines).count(c) for c in SPECIAL)
    sp_h = sum(min(" ".join(truth_lines).count(c), text.count(c)) for c in SPECIAL)
    return dict(words=len(t), recall=hit / max(1, len(t)), precision=hit / max(1, len(o)),
                uml=len(tu), uml_hit=uh, cer=cer, special=sp_t, special_hit=sp_h)

def extract_frames():
    tl = json.load(open(os.path.join(M, "video", "timeline.json")))
    fd = os.path.join(M, "frames"); os.makedirs(fd, exist_ok=True)
    seen = set()
    for s in tl:
        if s["slide"] in seen: continue
        seen.add(s["slide"]); t = (s["start"] + s["end"]) / 2
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", str(t), "-i",
                        os.path.join(M, "video", "B_webcam.mp4"), "-frames:v", "1",
                        os.path.join(fd, s["slide"] + ".png")], creationflags=NOWIN, check=True)

# ---- Windows.Media.Ocr ----
async def _win(paths, lang):
    from winrt.windows.media.ocr import OcrEngine
    from winrt.windows.globalization import Language
    from winrt.windows.graphics.imaging import BitmapDecoder
    from winrt.windows.storage import StorageFile, FileAccessMode
    eng = OcrEngine.try_create_from_language(Language(lang))
    out = {}
    for p in paths:
        t = time.perf_counter()
        f = await StorageFile.get_file_from_path_async(os.path.abspath(p))
        st = await f.open_async(FileAccessMode.READ)
        dec = await BitmapDecoder.create_async(st)
        bmp = await dec.get_software_bitmap_async()
        r = await eng.recognize_async(bmp)
        txt = "\n".join(l.text for l in r.lines)
        out[p] = (txt, time.perf_counter() - t)
    return out, OcrEngine.max_image_dimension

def win_ocr(paths, lang="de-DE"):
    out, mx = asyncio.run(_win(paths, lang))
    return out

# ---- Tesseract ----
def tess(paths, lang="deu+eng"):
    out = {}
    for p in paths:
        t = time.perf_counter()
        r = subprocess.run(["tesseract", p, "stdout", "-l", lang, "--psm", "3"], capture_output=True,
                           creationflags=NOWIN)
        out[p] = (r.stdout.decode("utf-8", "replace"), time.perf_counter() - t)
    return out

# ---- RapidOCR ----
def rapid(paths, threads=4):
    from rapidocr import RapidOCR, LangRec, OCRVersion, ModelType
    t0 = time.perf_counter()
    eng = RapidOCR(params={"Rec.lang_type": LangRec.LATIN, "Rec.ocr_version": OCRVersion.PPOCRV5,
                           "Rec.model_type": ModelType.MOBILE, "Global.log_level": "error",
                           "EngineConfig.onnxruntime.intra_op_num_threads": threads})
    load = time.perf_counter() - t0
    eng(paths[0])  # Aufwaermen
    out = {}
    for p in paths:
        t = time.perf_counter(); r = eng(p)
        txt = "\n".join(r.txts) if r.txts else ""
        out[p] = (txt, time.perf_counter() - t)
    return out, load

def main():
    extract_frames()
    sets = {"sauber": [os.path.join(M, "slides", k + ".png") for k in TRUTH],
            "video": [os.path.join(M, "frames", k + ".png") for k in TRUTH]}
    engines = {}
    allres = {}
    for sname, paths in sets.items():
        res = {}
        res["Windows OCR de-DE"] = win_ocr(paths, "de-DE")
        res["Tesseract 5.4 deu+eng"] = tess(paths)
        r, load = rapid(paths); res["RapidOCR v5 latin (4 Thr)"] = r
        allres[sname] = res
    rows = []; texts = {}
    for sname, res in allres.items():
        for ename, out in res.items():
            agg = collections.Counter(); per = []
            for p, (txt, dt) in out.items():
                sid = os.path.basename(p)[:-4]
                m = metrics(TRUTH[sid], txt); per.append((sid, m, dt))
                texts[f"{sname}|{ename}|{sid}"] = txt
            n = len(per)
            rows.append(dict(set=sname, engine=ename,
                             recall=round(sum(m["recall"] for _, m, _ in per) / n, 3),
                             precision=round(sum(m["precision"] for _, m, _ in per) / n, 3),
                             cer=round(sum(m["cer"] for _, m, _ in per) / n, 3),
                             uml=f'{sum(m["uml_hit"] for _, m, _ in per)}/{sum(m["uml"] for _, m, _ in per)}',
                             special=f'{sum(m["special_hit"] for _, m, _ in per)}/{sum(m["special"] for _, m, _ in per)}',
                             ms_per_img=round(1000 * sum(d for *_, d in per) / n),
                             worst=sorted(((round(m["recall"], 2), s) for s, m, _ in per))[:3]))
    json.dump(rows, open(os.path.join(M, "out", "ocr.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    json.dump(texts, open(os.path.join(M, "out", "ocr_texts.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    for r in rows: print(r)

if __name__ == "__main__":
    main()
