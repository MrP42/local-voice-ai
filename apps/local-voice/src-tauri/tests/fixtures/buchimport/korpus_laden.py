"""Testkorpus für den Buchimport (lva#149) herunterladen.

Frei verfügbare Bücher, nicht im Repo (Größe, Lizenz): Aufruf
    python korpus_laden.py ZIELORDNER
danach in src-tauri:
    LVA_BUCHKORPUS=ZIELORDNER cargo test --lib pdf_text::tests::korpus -- --ignored --nocapture

Quellen:
- grimm.pdf: archive.org, „Grimms Märchen, Alle 250 Kinder- und Hausmärchen“,
  Ed. Lord Henfield 2022 (CC BY-NC-ND 4.0, nur lokal testen, nicht weitergeben).
- Project Gutenberg (gemeinfrei in den USA): Grimm (2591), Effi Briest (5323),
  Die Verwandlung (22367), Alice (11).
"""
import pathlib
import sys
import urllib.request

QUELLEN = {
    "grimm.pdf": "https://archive.org/download/grimms-marchen-alle-250-kinder-und-hausmarchen-der-bruder-grimm-editor-lord-henfield-2022/"
    "Grimms%20M%C3%A4rchen%2C%20Alle%20250%20Kinder-%20und%20Hausm%C3%A4rchen%20der%20Br%C3%BCder%20Grimm%2C%20Editor%20Lord%20Henfield%2C%202022.pdf",
    "grimm-gutenberg.epub": "https://www.gutenberg.org/ebooks/2591.epub3.images",
    "effi.epub": "https://www.gutenberg.org/ebooks/5323.epub3.images",
    "kafka-verwandlung.epub": "https://www.gutenberg.org/ebooks/22367.epub3.images",
    "alice.epub": "https://www.gutenberg.org/ebooks/11.epub3.images",
}

ziel = pathlib.Path(sys.argv[1])
ziel.mkdir(parents=True, exist_ok=True)
for name, url in QUELLEN.items():
    datei = ziel / name
    if datei.exists():
        continue
    print("lade", name)
    urllib.request.urlretrieve(url, datei)
