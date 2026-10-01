# -*- coding: utf-8 -*-
"""Testvideo-Generator fuer die Folienerkennung (M7, D8).

Rendert 16 Bilder (14 Folien, eine davon mit drei Aufbaustufen; 1920x1080, Deutsch und Englisch: Titel, Agenda,
Balken-/Kreisdiagramm, Tabelle, Flussdiagramm, Foto, Fliesstext, Code, dunkle
Folie, Aufbaustufen, Sonderzeichen, Gantt) mit bekanntem Text (Ground Truth)
und eine Zeitleiste fuer ein 10-Minuten-Video mit Aufbaustufen und
Ruecksprung auf Folie 3.

Ausgabe unter --out (Vorgabe: src-tauri/tests/fixtures/slides-video, per
.gitignore ausgeschlossen, nichts davon wird eingecheckt):
  slides/*.png          die Folien
  slides/truth.json     erwarteter Text je Folie
  video/timeline.json   Beginn/Ende je Abschnitt
  video/concat.txt      ffmpeg-concat-Liste
  video/slides.mp4      nur mit --mp4 (siehe unten)

Aufruf:
  python apps/local-voice/scripts/make-slides-video.py [--out DIR] [--mp4]

Abhaengigkeiten:
  - Python 3 und Pillow (pip install pillow)
  - Windows-Schriften Segoe UI, Calibri, Consolas (C:\\Windows\\Fonts); ein anderer
    Ordner per Umgebungsvariable LVA_FONT_DIR
  - nur fuer --mp4: ffmpeg im PATH (libx264, aac). Das Video hat eine stille
    Tonspur (--no-audio laesst sie weg) und 2 Bilder/s; es ist so ein
    importierbares 10-Minuten-Video, kein echter Vortrag.
Das Skript schreibt nur unter --out und startet nichts anderes.
"""
import argparse, json, math, os, random, shutil, subprocess, sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
_ap = argparse.ArgumentParser(description="Testvideo fuer die Folienerkennung erzeugen")
_ap.add_argument("--out", default=os.path.join(HERE, "..", "src-tauri", "tests", "fixtures", "slides-video"),
                 help="Ausgabeordner (Vorgabe: src-tauri/tests/fixtures/slides-video)")
_ap.add_argument("--mp4", action="store_true", help="zusaetzlich video/slides.mp4 per ffmpeg erzeugen")
_ap.add_argument("--no-audio", action="store_true", help="mit --mp4: ohne stille Tonspur")
ARGS = _ap.parse_args()

M = os.path.abspath(ARGS.out)
SL = os.path.join(M, "slides"); VD = os.path.join(M, "video")
os.makedirs(SL, exist_ok=True); os.makedirs(VD, exist_ok=True)
W, H = 1920, 1080
FD = os.environ.get("LVA_FONT_DIR", "C:/Windows/Fonts").rstrip("/\\") + "/"
def F(name, size):
    try:
        return ImageFont.truetype(FD + name, size)
    except OSError:
        sys.exit(f"Schrift {FD}{name} fehlt (Segoe UI, Calibri, Consolas noetig; "
                 "anderer Ordner per LVA_FONT_DIR).")
SEG, SEGB, CAL, CONS = "segoeui.ttf", "segoeuib.ttf", "calibri.ttf", "consola.ttf"

slides = {}  # id -> list of text lines (ground truth)

def base(title, dark=False, accent=(0, 90, 160)):
    bg = (18, 32, 58) if dark else (255, 255, 255)
    im = Image.new("RGB", (W, H), bg); d = ImageDraw.Draw(im)
    fg = (255, 255, 255) if dark else (20, 20, 20)
    d.rectangle([0, 0, W, 14], fill=accent)
    d.text((110, 70), title, font=F(SEGB, 64), fill=fg)
    d.line([110, 165, W - 110, 165], fill=accent, width=4)
    return im, d, fg

def save(sid, im, lines):
    im.save(os.path.join(SL, f"{sid}.png")); slides[sid] = lines

def bullets(d, items, y=230, size=44, fg=(20, 20, 20), x=150, gap=1.9):
    f = F(SEG, size)
    for it in items:
        d.ellipse([x - 40, y + size * 0.45, x - 22, y + size * 0.45 + 18], fill=(0, 90, 160))
        d.text((x, y), it, font=f, fill=fg); y += int(size * gap)
    return y

# 01 Titelfolie DE
im = Image.new("RGB", (W, H), (0, 70, 130)); d = ImageDraw.Draw(im)
t1 = "Quartalsbericht Q3 2026 – Überblick"; t2 = "Präsentiert von Jürgen Größmann"
t3 = "Müller & Söhne GmbH · Düsseldorf"
d.text((160, 380), t1, font=F(SEGB, 84), fill="white")
d.text((160, 520), t2, font=F(SEG, 48), fill=(220, 230, 240))
d.text((160, 600), t3, font=F(SEG, 40), fill=(200, 215, 230))
save("s01_titel_de", im, [t1, t2, t3])

# 02 Agenda DE
items = ["Ergebnisse im Überblick", "Kundenzufriedenheit und Rückmeldungen",
         "Maßnahmen für das nächste Quartal", "Größere Risiken und Abhängigkeiten",
         "Offene Fragen und Nächste Schritte"]
im, d, fg = base("Agenda"); bullets(d, items); save("s02_agenda_de", im, ["Agenda"] + items)

# 03 Balkendiagramm DE
title = "Umsatz nach Region (Mio. €)"
im, d, fg = base(title)
regs = [("Nord", 4.2), ("Süd", 6.8), ("Ost", 3.1), ("West", 5.5), ("Übersee", 2.4)]
x0, y0, bw = 260, 900, 220
d.line([200, y0, 1750, y0], fill=(80, 80, 80), width=3)
lines = [title]
for i, (r, v) in enumerate(regs):
    x = x0 + i * 290; h = int(v * 95)
    d.rectangle([x, y0 - h, x + bw, y0], fill=(0, 110, 190))
    val = f"{v:.1f}".replace(".", ",")
    d.text((x + 60, y0 - h - 60), val, font=F(SEGB, 40), fill=fg)
    d.text((x + 40, y0 + 15), r, font=F(SEG, 40), fill=fg)
    lines += [val, r]
save("s03_balken_de", im, lines)

# 04 Tabelle DE
title = "Kennzahlen im Vergleich"
im, d, fg = base(title)
rows = [["Kennzahl", "Q2 2026", "Q3 2026", "Veränderung"],
        ["Umsatz (Mio. €)", "12,4", "13,1", "+5,6 %"],
        ["Bruttomarge", "41,2 %", "43,0 %", "+1,8 Pkt."],
        ["Kundenzufriedenheit", "7,9", "8,3", "+0,4"],
        ["Rücksendungen", "3,1 %", "2,7 %", "−0,4 Pkt."],
        ["Mitarbeitende", "214", "227", "+13"]]
cw = [620, 300, 300, 360]; y = 230
for ri, row in enumerate(rows):
    x = 150
    if ri == 0: d.rectangle([150, y - 8, 150 + sum(cw), y + 62], fill=(0, 90, 160))
    elif ri % 2 == 0: d.rectangle([150, y - 8, 150 + sum(cw), y + 62], fill=(232, 240, 248))
    for ci, cell in enumerate(row):
        d.text((x + 16, y), cell, font=F(SEGB if ri == 0 else SEG, 40),
               fill="white" if ri == 0 else fg)
        x += cw[ci]
    y += 100
lines = [title] + [c for r in rows for c in r]
save("s04_tabelle_de", im, lines)

# 05 Flussdiagramm EN
title = "Data Pipeline Architecture"
im, d, fg = base(title)
boxes = ["Ingest", "Validate", "Transform", "Store"]
subs = ["Kafka topics", "Schema checks", "Spark jobs", "Parquet on S3"]
lines = [title]
for i, (b, s) in enumerate(zip(boxes, subs)):
    x = 150 + i * 430
    d.rounded_rectangle([x, 420, x + 330, 620], radius=24, outline=(0, 90, 160), width=6, fill=(235, 244, 252))
    d.text((x + 40, 450), b, font=F(SEGB, 52), fill=fg)
    d.text((x + 40, 540), s, font=F(SEG, 34), fill=(60, 60, 60))
    lines += [b, s]
    if i < 3:
        d.line([x + 335, 520, x + 425, 520], fill=(0, 90, 160), width=8)
        d.polygon([(x + 425, 520), (x + 400, 505), (x + 400, 535)], fill=(0, 90, 160))
note = "Fig. 2: End-to-end latency budget is 450 ms per event."
d.text((150, 760), note, font=F(SEG, 36), fill=(60, 60, 60)); lines.append(note)
save("s05_pipeline_en", im, lines)

# 06 Bullets EN mit Zahlen
items = ["Latency reduced by 37% after caching", "Throughput: 1,250 req/s at p95 < 80 ms",
         "Error rate down from 0.42% to 0.07%", "Rollout to EU-West completed on Sep 14",
         "Next: multi-region failover (Q4)"]
im, d, fg = base("Key Findings"); bullets(d, items); save("s06_findings_en", im, ["Key Findings"] + items)

# 07 Foto-Folie DE (Verlauf + Rauschen + Formen)
im = Image.new("RGB", (W, H), (0, 0, 0)); px = im.load()
random.seed(7)
ph = Image.new("RGB", (1400, 760))
pd = ImageDraw.Draw(ph)
for yy in range(760):
    pd.line([0, yy, 1400, yy], fill=(60 + yy // 8, 90 + yy // 12, 120 + yy // 10))
for _ in range(40):
    cx, cy, r = random.randint(0, 1400), random.randint(200, 760), random.randint(20, 140)
    pd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=(random.randint(80, 200), random.randint(80, 160), random.randint(40, 120)))
pd.rectangle([500, 300, 900, 700], fill=(150, 150, 160)); pd.rectangle([560, 360, 840, 520], fill=(40, 60, 80))
ph = ph.filter(ImageFilter.GaussianBlur(3))
im.paste(ph, (260, 90)); d = ImageDraw.Draw(im)
cap = "Abb. 3: Prüfstand in Halle Süd, Aufnahme vom 12. März"
d.text((260, 880), cap, font=F(SEG, 44), fill="white")
save("s07_foto_de", im, [cap])

# 08 Kleiner Text DE (Fliesstext)
title = "Hinweise und Fußnoten"
im, d, fg = base(title)
para = ["Die Angaben beruhen auf vorläufigen Zahlen und können sich nach der Prüfung",
        "durch den Abschlussprüfer noch ändern. Währungseffekte wurden zum Stichtag",
        "30. September bewertet; Sondereinflüsse aus der Schließung des Standorts",
        "Fürth sind gesondert ausgewiesen. Für Rückfragen steht die Abteilung",
        "Unternehmensentwicklung (Frau Bäumer, Durchwahl 4711) zur Verfügung."]
y = 240
for p in para:
    d.text((150, y), p, font=F(CAL, 30), fill=fg); y += 48
save("s08_kleintext_de", im, [title] + para)

# 09 Code EN
title = "Example: Retry with Backoff"
im, d, fg = base(title)
code = ["fn fetch_with_retry(url: &str) -> Result<String, Error> {",
        "    let mut delay = Duration::from_millis(200);",
        "    for attempt in 1..=5 {",
        "        match client.get(url).send() {",
        "            Ok(resp) => return resp.text(),",
        "            Err(e) if attempt < 5 => sleep(delay),",
        "            Err(e) => return Err(e.into()),",
        "        }",
        "        delay *= 2;",
        "    }",
        "}"]
d.rectangle([140, 220, 1780, 220 + 62 * len(code) + 30], fill=(245, 245, 245))
y = 240
for c in code:
    d.text((170, y), c, font=F(CONS, 38), fill=(30, 30, 30)); y += 62
save("s09_code_en", im, [title] + [c.strip() for c in code])

# 10 Dunkle Folie EN
im, d, fg = base("Summary", dark=True, accent=(255, 196, 0))
items = ["Revenue up 5.6% quarter over quarter", "Customer satisfaction at record 8.3",
         "Two open risks need a decision by October 15"]
bullets(d, items, fg=(255, 255, 255))
save("s10_dunkel_en", im, ["Summary"] + items)

# 11 Aufbau in drei Schritten DE (gleiche Folie, Builds)
title = "Maßnahmen für Q4"
allb = ["Lieferkette: zweiten Zulieferer für Gehäuse qualifizieren",
        "Vertrieb: Schulungen für Außendienst in Österreich",
        "Qualität: Prüfquote bei Lötstellen auf 100 % erhöhen"]
for k in range(1, 4):
    im, d, fg = base(title); bullets(d, allb[:k])
    save(f"s11_build{k}_de", im, [title] + allb[:k])

# 12 Kreisdiagramm EN
title = "Market Share 2026"
im, d, fg = base(title)
parts = [("Acme Corp", 38), ("Globex", 27), ("Initech", 21), ("Others", 14)]
cols = [(0, 90, 160), (0, 150, 120), (230, 140, 0), (150, 150, 150)]
a = -90; lines = [title]
for (n, p), c in zip(parts, cols):
    d.pieslice([200, 230, 900, 930], a, a + p * 3.6, fill=c); a += p * 3.6
for i, ((n, p), c) in enumerate(zip(parts, cols)):
    y = 340 + i * 110
    d.rectangle([1050, y + 8, 1100, y + 58], fill=c)
    s = f"{n}: {p}%"; d.text((1130, y), s, font=F(SEG, 48), fill=fg); lines.append(s)
save("s12_kreis_en", im, lines)

# 13 Sonderzeichen DE
title = "Rechtlicher Rahmen nach § 3 StVO"
im, d, fg = base(title)
items = ["„Angepasste Geschwindigkeit“ – nicht nur Höchstwerte",
         "ÄNDERUNGEN ÜBER ÖFFENTLICHE STRASSEN beachten",
         "Bußgeld: 70 € bis 680 € je nach Überschreitung",
         "Gültig ab 01.01.2027 (Entwurf, Stand: Größe/Maße)"]
bullets(d, items, size=42)
save("s13_sonderzeichen_de", im, [title] + items)

# 14 Zeitplan DE (Gantt)
title = "Zeitplan Einführung"
im, d, fg = base(title)
tasks = [("Pilotphase", 0, 3), ("Schulung", 2, 5), ("Rollout Süd", 4, 8), ("Rollout Nord", 6, 10), ("Abnahme", 9, 12)]
months = ["Jan", "Feb", "Mär", "Apr", "Mai", "Jun", "Jul", "Aug", "Sep", "Okt", "Nov", "Dez"]
lines = [title]
for i, m in enumerate(months):
    d.text((560 + i * 105, 220), m, font=F(SEG, 32), fill=fg); lines.append(m)
for j, (t, s, e) in enumerate(tasks):
    y = 300 + j * 120
    d.text((150, y), t, font=F(SEG, 40), fill=fg); lines.append(t)
    d.rounded_rectangle([560 + s * 105, y + 5, 560 + e * 105 - 10, y + 55], radius=12, fill=(0, 110, 190))
save("s14_zeitplan_de", im, lines)

json.dump(slides, open(os.path.join(SL, "truth.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)

# Zeitleiste 600 s: Reihenfolge inkl. Builds und Ruecksprung auf s03
order = [("s01_titel_de", 25), ("s02_agenda_de", 30), ("s03_balken_de", 45), ("s04_tabelle_de", 50),
         ("s05_pipeline_en", 40), ("s06_findings_en", 35), ("s07_foto_de", 20), ("s08_kleintext_de", 45),
         ("s09_code_en", 40), ("s11_build1_de", 15), ("s11_build2_de", 15), ("s11_build3_de", 30),
         ("s03_balken_de", 20), ("s12_kreis_en", 35), ("s13_sonderzeichen_de", 40), ("s14_zeitplan_de", 50),
         ("s10_dunkel_en", 65)]
assert sum(x[1] for x in order) == 600, sum(x[1] for x in order)
t = 0; tl = []
with open(os.path.join(VD, "concat.txt"), "w") as f:
    for sid, dur in order:
        f.write(f"file '../slides/{sid}.png'\nduration {dur}\n"); tl.append({"start": t, "end": t + dur, "slide": sid}); t += dur
    f.write(f"file '../slides/{order[-1][0]}.png'\n")
json.dump(tl, open(os.path.join(VD, "timeline.json"), "w"), indent=1)
print(len(slides), "Folien,", len(tl), "Abschnitte")

if ARGS.mp4:
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        sys.exit("ffmpeg nicht im PATH; --mp4 braucht es.")
    out = os.path.join(VD, "slides.mp4")
    cmd = [ffmpeg, "-y", "-hide_banner", "-loglevel", "error",
           "-f", "concat", "-safe", "0", "-i", os.path.join(VD, "concat.txt")]
    if not ARGS.no_audio:
        cmd += ["-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono"]
    cmd += ["-t", str(t), "-vf", "fps=2,format=yuv420p", "-c:v", "libx264", "-preset", "veryfast", "-crf", "28"]
    if not ARGS.no_audio:
        cmd += ["-c:a", "aac", "-b:a", "32k", "-shortest"]
    cmd += [out]
    subprocess.run(cmd, check=True)
    print("Video:", out, f"({os.path.getsize(out) // 1024} KiB)")
