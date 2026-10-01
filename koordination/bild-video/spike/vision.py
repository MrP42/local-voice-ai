# -*- coding: utf-8 -*-
"""Bildanalyse mit der gebuendelten llama.cpp-Laufzeit (llama-server + mmproj).
Aufruf: python vision.py <name> <gguf> <mmproj> [image_max_tokens]
Startet einen EIGENEN llama-server (Port 18089, ohne Fenster), misst VRAM,
Latenz je Bild, Typ-Erkennung, Kernfakten und OCR-Qualitaet; beendet den
Server per PID. RAM-Waechter: unter 6 GB freiem RAM wird sofort beendet."""
import base64, json, os, subprocess, sys, threading, time, urllib.request
M = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(M, "pylib"))
import psutil
from ocr import metrics, TRUTH

RT = r"C:\Users\wolff\AppData\Local\de.wolffappliedai.localvoiceai\llm\runtime\llm-runtime-windows-x64-cuda"
PORT = 18089
NOWIN = 0x08000000
TYPES = ["Titelfolie", "Aufzählung", "Fließtext", "Tabelle", "Diagramm", "Schema", "Foto", "Code"]
OK = {"s01": {"Titelfolie"}, "s02": {"Aufzählung"}, "s03": {"Diagramm"}, "s04": {"Tabelle"},
      "s05": {"Schema", "Diagramm"}, "s06": {"Aufzählung"}, "s07": {"Foto"}, "s08": {"Fließtext"},
      "s09": {"Code"}, "s10": {"Aufzählung"}, "s11": {"Aufzählung"}, "s12": {"Diagramm"},
      "s13": {"Aufzählung"}, "s14": {"Diagramm", "Schema"}}
FACTS = {"s01": ["Quartalsbericht"], "s02": ["Agenda"], "s03": ["Süd", "6,8|6.8"], "s04": ["13,1|13.1"],
         "s05": ["Ingest", "Store"], "s06": ["37"], "s07": ["Prüfstand"], "s08": ["vorläufig"],
         "s09": ["Retry|retry|Wiederhol"], "s10": ["5.6|5,6"], "s11": ["Zuliefer"], "s12": ["Acme", "38"],
         "s13": ["StVO|§"], "s14": ["Pilot"]}

SCHEMA = {"type": "object", "properties": {
    "art": {"type": "string", "enum": TYPES},
    "beschreibung": {"type": "string"},
    "kernaussage": {"type": "string"}}, "required": ["art", "beschreibung", "kernaussage"]}
P_DESC = ("Das Bild ist eine Folie aus einer Präsentation. Antworte als JSON auf Deutsch: "
          "art = Art des Hauptinhalts, beschreibung = was zu sehen ist (höchstens 2 Sätze), "
          "kernaussage = wichtigste Aussage mit konkreten Zahlen, falls vorhanden. Erfinde nichts.")
P_OCR = ("Gib den gesamten sichtbaren Text dieser Folie wortgetreu wieder, Zeile für Zeile, "
         "in der Originalsprache, ohne Kommentar und ohne Formatierung.")

def vram_used():
    out = subprocess.run(["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits"],
                         capture_output=True, text=True, creationflags=NOWIN).stdout
    return int(out.strip().splitlines()[0])

def post(payload, timeout=300):
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/chat/completions",
                                 data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"})
    t = time.perf_counter()
    with urllib.request.urlopen(req, timeout=timeout) as r:
        j = json.load(r)
    return j, time.perf_counter() - t

def ask(img, prompt, schema=None, max_tokens=600):
    b64 = base64.b64encode(open(img, "rb").read()).decode()
    p = {"messages": [{"role": "user", "content": [
        {"type": "image_url", "image_url": {"url": f"data:image/png;base64,{b64}"}},
        {"type": "text", "text": prompt}]}],
         "temperature": 0.1, "max_tokens": max_tokens, "chat_template_kwargs": {"enable_thinking": False}}
    if schema:
        p["response_format"] = {"type": "json_schema", "json_schema": {"name": "folie", "schema": schema}}
    j, dt = post(p)
    tm = j.get("timings", {})
    return j["choices"][0]["message"]["content"], dt, tm

def main():
    name, gguf, mmproj = sys.argv[1:4]
    imax = sys.argv[4] if len(sys.argv) > 4 else None
    base_vram = vram_used(); free_ram = psutil.virtual_memory().available / 1e9
    if free_ram < 10:
        sys.exit(f"Start-Gate: nur {free_ram:.1f} GB RAM frei")
    args = [os.path.join(RT, "llama-server.exe"), "-m", gguf, "--mmproj", mmproj, "--host", "127.0.0.1",
            "--port", str(PORT), "-c", "8192", "-ngl", "99", "--parallel", "1", "-t", "8", "--no-webui",
            "--fit", "off"]
    if imax: args += ["--image-max-tokens", imax]
    args += os.environ.get("EXTRA", "").split()
    log = open(os.path.join(M, "out", f"server_{name}.log"), "w")
    t0 = time.perf_counter()
    proc = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, creationflags=NOWIN)
    print("llama-server PID", proc.pid, flush=True)
    stop = threading.Event()
    def guard():
        while not stop.is_set():
            if psutil.virtual_memory().available < 6e9:
                print("RAM-Waechter: beende", proc.pid, flush=True); proc.kill(); return
            time.sleep(0.5)
    threading.Thread(target=guard, daemon=True).start()
    peak_rss = 0
    try:
        while True:
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2).read(); break
            except Exception:
                if proc.poll() is not None: sys.exit("Server beendet")
                time.sleep(0.5)
        t_load = time.perf_counter() - t0
        vram_loaded = vram_used() - base_vram
        ids = sorted({k.split("_build")[0] if "_build" not in k else k for k in TRUTH})
        ids = [k for k in TRUTH if not k.startswith("s11_build") or k == "s11_build3_de"]
        rows = []
        for sid in ids:
            img = os.path.join(M, "frames", sid + ".png")
            txt, dt, tm = ask(img, P_DESC, SCHEMA, 400)
            try: d = json.loads(txt)
            except Exception: d = {"art": "?", "beschreibung": txt, "kernaussage": ""}
            key = sid[:3]
            blob = d.get("beschreibung", "") + " " + d.get("kernaussage", "")
            facts = FACTS[key]; fh = sum(any(a in blob for a in f.split("|")) for f in facts)
            otxt, odt, otm = ask(img, P_OCR, None, 700)
            m = metrics(TRUTH[sid], otxt)
            rss = psutil.Process(proc.pid).memory_info().rss; peak_rss = max(peak_rss, rss)
            rows.append(dict(sid=sid, art=d.get("art"), art_ok=d.get("art") in OK[key], facts=f"{fh}/{len(facts)}",
                             desc_s=round(dt, 2), desc_prompt_ms=round(tm.get("prompt_ms", 0)),
                             desc_gen_tok=tm.get("predicted_n"), ocr_s=round(odt, 2),
                             ocr_recall=round(m["recall"], 3), ocr_cer=round(m["cer"], 3),
                             uml=f'{m["uml_hit"]}/{m["uml"]}', beschreibung=d.get("beschreibung"),
                             kernaussage=d.get("kernaussage"), ocr_text=otxt))
            print(sid, rows[-1]["art"], rows[-1]["art_ok"], rows[-1]["facts"], rows[-1]["desc_s"], rows[-1]["ocr_s"],
                  rows[-1]["ocr_recall"], flush=True)
        vram_peak = vram_used() - base_vram
        n = len(rows)
        summary = dict(model=name, image_max_tokens=imax, load_s=round(t_load, 1), vram_after_load_mb=vram_loaded,
                       vram_after_run_mb=vram_peak, server_rss_gb=round(peak_rss / 1e9, 2),
                       art_ok=f'{sum(r["art_ok"] for r in rows)}/{n}',
                       facts=f'{sum(int(r["facts"].split("/")[0]) for r in rows)}/{sum(int(r["facts"].split("/")[1]) for r in rows)}',
                       desc_s_avg=round(sum(r["desc_s"] for r in rows) / n, 2),
                       ocr_s_avg=round(sum(r["ocr_s"] for r in rows) / n, 2),
                       ocr_recall=round(sum(r["ocr_recall"] for r in rows) / n, 3),
                       ocr_cer=round(sum(r["ocr_cer"] for r in rows) / n, 3),
                       uml=f'{sum(int(r["uml"].split("/")[0]) for r in rows)}/{sum(int(r["uml"].split("/")[1]) for r in rows)}')
        json.dump(dict(summary=summary, rows=rows), open(os.path.join(M, "out", f"vision_{name}.json"), "w",
                  encoding="utf-8"), ensure_ascii=False, indent=1)
        print(summary)
    finally:
        stop.set()
        proc.kill(); proc.wait(timeout=30)
        print("beendet PID", proc.pid, "alive:", psutil.pid_exists(proc.pid), flush=True)

if __name__ == "__main__":
    main()
