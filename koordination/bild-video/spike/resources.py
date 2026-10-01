# -*- coding: utf-8 -*-
"""(1) VRAM von Gemma 4 E4B OHNE mmproj (Differenz = Kosten des Bild-Projektors).
(2) CPU-only (-ngl 0, mmproj auf CPU): Zeit fuer Beschreibung + OCR einer Folie."""
import json, os, subprocess, sys, time, urllib.request
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import psutil
from vision import ask, RT, PORT, NOWIN, vram_used, P_DESC, P_OCR, SCHEMA
GGUF = r"C:\Users\wolff\AppData\Local\de.wolffappliedai.localvoiceai\llm\models\gemma-4-E4B-it-Q4_K_M.gguf"
MM = os.path.join(M, "models", "gemma4-e4b-mmproj-F16.gguf")

def start(extra, tag):
    if psutil.virtual_memory().available < 10e9: sys.exit("Start-Gate RAM")
    a = [os.path.join(RT, "llama-server.exe"), "-m", GGUF, "--host", "127.0.0.1", "--port", str(PORT), "-c", "8192",
         "--parallel", "1", "-t", "8", "--no-webui", "--fit", "off", "-b", "2048", "-ub", "2048"] + extra
    p = subprocess.Popen(a, stdout=open(os.path.join(M, "out", f"server_{tag}.log"), "w"), stderr=subprocess.STDOUT,
                         creationflags=NOWIN)
    t = time.perf_counter()
    while True:
        try: urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).read(); break
        except Exception:
            if p.poll() is not None: sys.exit("Server beendet " + tag)
            time.sleep(0.5)
    return p, time.perf_counter() - t

res = {}
base = vram_used()
p, tl = start(["-ngl", "99"], "nommproj")
try: res["gpu_ohne_mmproj_vram_mb"] = vram_used() - base; res["gpu_ohne_mmproj_load_s"] = round(tl, 1)
finally: p.kill(); p.wait(30)
time.sleep(3)
base = vram_used()
p, tl = start(["-ngl", "0", "--mmproj", MM, "--no-mmproj-offload"], "cpu")
try:
    res["cpu_load_s"] = round(tl, 1); res["cpu_vram_mb"] = vram_used() - base
    img = os.path.join(M, "frames", "s04_tabelle_de.png")
    _, d1, t1 = ask(img, P_DESC, SCHEMA, 400)
    _, d2, t2 = ask(img, P_OCR, None, 700)
    res["cpu_beschreibung_s"] = round(d1, 1); res["cpu_ocr_s"] = round(d2, 1)
    res["cpu_prompt_ms"] = round(t1.get("prompt_ms", 0)); res["cpu_rss_gb"] = round(psutil.Process(p.pid).memory_info().rss / 1e9, 2)
finally:
    p.kill(); p.wait(30); res["beendet"] = not psutil.pid_exists(p.pid)
json.dump(res, open(os.path.join(M, "out", "resources.json"), "w"), indent=1)
print(res)
