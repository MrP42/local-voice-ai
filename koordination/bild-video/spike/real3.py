# -*- coding: utf-8 -*-
"""Realtest Stufe 3: Gemma 4 E4B klassifiziert die 35 Repraesentanten des CC-Videos
(Folie / Bildfolie / Kamera / Leer) und beschreibt sie kurz."""
import json, os, subprocess, sys, threading, time, urllib.request
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import psutil
import vision
from vision import ask, RT, PORT, NOWIN

SCHEMA = {"type": "object", "properties": {
    "art": {"type": "string", "enum": ["textfolie", "bildfolie", "kamera", "leer"]},
    "beschreibung": {"type": "string"}}, "required": ["art", "beschreibung"]}
P = ("Das Bild ist ein Standbild aus dem Video eines Vortrags. Ordne es ein: art = textfolie (Folie mit Text), bildfolie (Folie oder Foto ohne nennenswerten Text), kamera (Raum oder Sprecher gefilmt), leer (schwarz/leer). Dann beschreibe "
     "auf Deutsch in einem Satz, was zu sehen ist. Erfinde nichts.")
P2 = ("Das Bild ist ein Standbild aus dem Video eines Vortrags. Antworte in genau zwei Zeilen. Zeile 1: genau eines der Wörter textfolie, bildfolie, kamera, leer (kamera = Raum oder Sprecher gefilmt; bildfolie = Folie oder Foto ohne nennenswerten Text). Zeile 2: ein Satz auf Deutsch, was zu sehen ist. Erfinde nichts.")
GGUF = r"C:\Users\wolff\AppData\Local\de.wolffappliedai.localvoiceai\llm\models\gemma-4-E4B-it-Q4_K_M.gguf"
MM = os.path.join(M, "models", "gemma4-e4b-mmproj-F16.gguf")

if psutil.virtual_memory().available < 10e9: sys.exit("Start-Gate RAM")
args = [os.path.join(RT, "llama-server.exe"), "-m", GGUF, "--mmproj", MM, "--host", "127.0.0.1", "--port", str(PORT),
        "-c", "8192", "-ngl", "99", "--parallel", "1", "-t", "8", "--no-webui", "--fit", "off", "-b", "2048", "-ub", "2048"]
proc = subprocess.Popen(args, stdout=open(os.path.join(M, "out", "server_real3.log"), "w"), stderr=subprocess.STDOUT,
                        creationflags=NOWIN)
print("PID", proc.pid, flush=True)
try:
    while True:
        try: urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).read(); break
        except Exception:
            if proc.poll() is not None: sys.exit("Server beendet")
            time.sleep(0.5)
    od = os.path.join(M, "real_frames"); rows = []
    FAIL = {r["frame"] for r in json.load(open(os.path.join(M, "out", "real3.json"), encoding="utf-8")) if r.get("art") == "?"}
    for f in sorted(x for x in os.listdir(od) if x in FAIL):
        txt, dt, tm = ask(os.path.join(od, f), P2, None, 120)
        ls = [l for l in txt.strip().splitlines() if l.strip()]
        d = {"art": ls[0].strip().lower() if ls else "?", "beschreibung": " ".join(ls[1:])}
        rows.append(dict(frame=f, s=round(dt, 2), **d)); print(f, d.get("art"), "|", d.get("beschreibung"), flush=True)
    json.dump(rows, open(os.path.join(M, "out", "real3b.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print("Mittel s/Bild:", round(sum(r["s"] for r in rows) / len(rows), 2))
finally:
    proc.kill(); proc.wait(timeout=30); print("beendet", proc.pid, psutil.pid_exists(proc.pid))
