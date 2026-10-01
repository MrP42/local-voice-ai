# -*- coding: utf-8 -*-
"""D2: erzeugt die Test-Fixtures dieses Ordners (nur zur Nachvollziehbarkeit, die
Ergebnisse sind eingecheckt):

1. `ocr_slide.png` (1280x720): kleine Folie mit "Groesse/Masse", "Bussgeld: 70 EUR" usw.
   (mit Umlauten, ss und Euro), dazu `ocr_slide.words.txt` mit der Wahrheit.
2. `real_case_35.json`: 35 Abschnitts-Repraesentanten im Muster des Realtests C
   (spike-bericht.md: 35 Abschnitte, 24 nach Hash-Dubletten, 14 Textfolien davon
   2 verbliebene Dubletten, dazu Bild-/Kamerafolien ohne Text, Schwarzbilder).
   Die TEXTE sind die echte Windows-OCR-Ausgabe der Spike-Folien
   (`koordination/bild-video/spike/out/ocr_texts.json`); die HASHES und die
   Zeitleiste sind synthetisch (die Aufnahme des Realtests ist CC BY-SA und nicht im Repo).

Aufruf: python make_d2_fixtures.py <pfad-zu-ocr_texts.json>
"""
import json
import os
import random
import re
import sys
import unicodedata

from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = "C:/Windows/Fonts/"


def words(text):
    # Wie `ocr::words`: Buchstaben und Ziffern, Unterstrich trennt, mindestens zwei Zeichen.
    return [w.lower() for w in re.findall(r"[^\W_]+", unicodedata.normalize("NFC", text)) if len(w) >= 2]


# ---------------------------------------------------------------- 1. PNG
LINES = [
    "Größe und Maße",
    "Bußgeld: 70 € pro Verstoß",
    "Höchstgeschwindigkeit 50 km/h innerorts",
    "Änderungen der Straßenverkehrsordnung",
    "Gültig ab 1. Januar 2027",
]


def make_png():
    im = Image.new("RGB", (1280, 720), (255, 255, 255))
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 1280, 10], fill=(0, 90, 160))
    d.text((70, 50), LINES[0], font=ImageFont.truetype(FONTS + "segoeuib.ttf", 60), fill=(20, 20, 20))
    body = ImageFont.truetype(FONTS + "segoeui.ttf", 40)
    y = 190
    for line in LINES[1:]:
        d.text((90, y), line, font=body, fill=(20, 20, 20))
        y += 92
    path = os.path.join(HERE, "ocr_slide.png")
    im.save(path, optimize=True)
    with open(os.path.join(HERE, "ocr_slide.words.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(LINES) + "\n")
    print(path, os.path.getsize(path), "Byte")


# ---------------------------------------------------------------- 2. Realfall
def make_real_case(ocr_texts_json):
    ocr = json.load(open(ocr_texts_json, encoding="utf-8"))
    text = lambda sid: ocr[f"sauber|Windows OCR de-DE|{sid}"]
    rng = random.Random(70)
    # 12 verschiedene Textfolien (echte OCR-Texte)
    ids = ["s01_titel_de", "s02_agenda_de", "s03_balken_de", "s04_tabelle_de", "s05_pipeline_en",
           "s06_findings_en", "s08_kleintext_de", "s09_code_en", "s10_dunkel_en", "s12_kreis_en",
           "s13_sonderzeichen_de", "s14_zeitplan_de"]
    texts = {i: text(i) for i in ids}

    def mutilate(t, keep):
        """Verstuemmelte Ueberblendstufe: nur ein Teil der Woerter bleibt lesbar."""
        toks = t.split()
        kept = [w for w in toks if rng.random() < keep]
        return " ".join(kept)

    def jaccard(a, b):
        sa, sb = set(words(a)), set(words(b))
        return len(sa & sb) / max(1, len(sa | sb))

    # 24 Hash-Gruppen, in der Reihenfolge des Vortrags: (Art, Folien-ID, Text)
    # 12 echte Folien, 5 Varianten derselben Folie mit anderem Hash (Kamerafeld/Ueberblendung,
    # Text vollstaendig oder leicht verstuemmelt), 2 verbliebene Dubletten (stark verstuemmelt),
    # 2 Gruppen ohne Text (Foto, Sprecheraufnahme), dazu 3 weitere Varianten: zusammen 24.
    plan = []
    for i in ids:
        plan.append(("slide", i, texts[i]))
    variants = [("s02_agenda_de", 0.85), ("s05_pipeline_en", 0.8), ("s10_dunkel_en", 0.9),
                ("s12_kreis_en", 0.85), ("s14_zeitplan_de", 0.8)]
    for sid, keep in variants:
        v = mutilate(texts[sid], keep)
        assert jaccard(v, texts[sid]) >= 0.5, (sid, v)
        plan.append(("variant", sid, v))
    for sid in ("s04_tabelle_de", "s09_code_en"):  # bleiben als Dublette stehen
        for _ in range(200):  # so lange verstuemmeln, bis es eine echte Dublette bleibt
            v = mutilate(texts[sid], 0.25)
            if len(words(v)) >= 3 and jaccard(v, texts[sid]) < 0.5:
                break
        assert len(words(v)) >= 3 and jaccard(v, texts[sid]) < 0.5, (sid, v)
        plan.append(("residual", sid, v))
    plan.append(("image", None, "Abb. 3"))      # Foto mit kurzer Unterschrift (< 3 Woerter)
    plan.append(("image", None, ""))            # Sprecheraufnahme
    # 3 weitere Varianten (Aufbaustufen, Vorschau-Ueberblendungen) -> 24 Gruppen
    for sid, keep in [("s01_titel_de", 0.9), ("s03_balken_de", 0.85), ("s13_sonderzeichen_de", 0.9)]:
        v = mutilate(texts[sid], keep)
        assert jaccard(v, texts[sid]) >= 0.5, (sid, v)
        plan.append(("variant", sid, v))
    assert len(plan) == 24, len(plan)

    rng.shuffle(plan)
    # Hashes: je Gruppe ein Zufallswert, mindestens 8 Bit von allen anderen entfernt
    hashes = []
    while len(hashes) < len(plan):
        h = rng.getrandbits(64)
        if all(bin(h ^ o).count("1") > 8 for o in hashes):
            hashes.append(h)
    # 9 Rueckspruenge zu fruehen Gruppen: gleicher Hash (Abstand <= 2), gleicher Text
    segs = []
    for (kind, sid, t), h in zip(plan, hashes):
        segs.append({"kind": kind, "slide": sid, "hash": h, "text": t})
    returns = []
    for k, src in enumerate(rng.sample(range(len(segs)), 9)):
        s = dict(segs[src])
        s["hash"] ^= (1 << rng.randrange(64)) if k % 2 else 0
        s["kind"] = "return"
        returns.append((src, s))
    # Zeitleiste: Gruppen in Reihenfolge, jeder Rueckspruch nach etwa 3 Gruppen nach seiner Quelle
    order = list(range(len(segs)))
    seq = [("first", i, segs[i]) for i in order]
    for src, s in sorted(returns, key=lambda r: r[0]):
        pos = next(n for n, (k, i, _) in enumerate(seq) if k == "first" and i == src)
        seq.insert(min(len(seq), pos + 3 + rng.randrange(4)), ("return", src, s))
    # 2 Schwarzbilder: am Anfang und in der Mitte
    black = {"kind": "black", "slide": None, "hash": 0, "text": ""}
    seq.insert(0, ("black", -1, black))
    seq.insert(len(seq) // 2, ("black", -1, black))
    assert len(seq) == 35, len(seq)
    out, t = [], 0
    for _, _, s in seq:
        dur = rng.randrange(6, 40) * 1000
        out.append({
            "start_ms": t, "end_ms": t + dur, "rep_ms": t + dur - 1000,
            "hash": f"{s['hash']:016x}", "black": s["kind"] == "black",
            "kind": s["kind"], "slide": s["slide"], "text": s["text"],
        })
        t += dur
    path = os.path.join(HERE, "real_case_35.json")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print(path, os.path.getsize(path), "Byte,", len(out), "Abschnitte")


if __name__ == "__main__":
    make_png()
    if len(sys.argv) > 1:
        make_real_case(sys.argv[1])
