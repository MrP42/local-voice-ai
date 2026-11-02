"""Hörprobe Betonung mit Phonon: Original gegen KI-Vortrag gegen Sprechfassung.

Erzeugt je Abschnitt drei WAV-Dateien und eine index.html zum Vergleichen:

* A  Original: jeder Satz unverändert, wie die App ihn heute an ptts gibt.
* B  Vortrag:  wie A, plus Pausen-Tags (App-Stille) und Tempo-Tags
               (`--condition duration_delta`), so wie das Auto-Tagging sie für
               Phonon setzt (lva: Tempo-Tags).
* C  Sprechfassung: B plus nachgeschärfte Satzzeichen, geteilte Schachtelsätze und
               ausgeschriebene Zahlen (Stufe 2, noch nicht in der App).

Die Fassungen B und C sind hier von Hand vorgegeben (so, wie ein LLM sie liefern
soll), damit die Hörprobe nur Phonon vergleicht, nicht ein bestimmtes Sprachmodell.

Aufruf (Windows, PowerShell):

    py -3.11 scripts/bench/tts/betonung/hoerprobe.py `
        --ptts D:/lv-build/phonon/bin/ptts.exe `
        --model D:/lv-build/phonon/models/phonon-40m-de --out D:/lv-build/tmp/betonung

Nur Standardbibliothek. Keine Netzzugriffe (HF_HUB_OFFLINE=1).
"""

from __future__ import annotations

import argparse
import html
import os
import re
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

PAUSE_MS = {"short pause": 250, "pause": 500, "break": 700, "long pause": 1000}
PACE = {"slower": 0.12, "faster": -0.12}
PACE_MAX = 0.2

# (Kennung, Titel, A Original, B Vortrag, C Sprechfassung)
PASSAGES: list[tuple[str, str, str, str, str]] = [
    (
        "01-spannung",
        "Spannung und Pointe",
        "Er öffnete langsam die Tür. Dahinter war nichts. Gar nichts. Dann hörte er ein Geräusch.",
        "Er öffnete langsam die Tür. [pause] [slower] Dahinter war nichts. [short pause] Gar nichts. [long pause] Dann hörte er ein Geräusch.",
        "Er öffnete langsam die Tür … [pause] [slower] Dahinter war nichts. [short pause] Gar nichts! [long pause] Dann – hörte er ein Geräusch.",
    ),
    (
        "02-dialog",
        "Dialog mit Frage",
        "Kommst du morgen mit, fragte sie. Ich weiß es noch nicht, sagte er. Du weißt es nie.",
        "Kommst du morgen mit, fragte sie. [short pause] [slower] Ich weiß es noch nicht, sagte er. [pause] [faster] Du weißt es nie.",
        "„Kommst du morgen mit?“, fragte sie. [short pause] [slower] „Ich weiß es noch nicht“, sagte er. [pause] [faster] „Du weißt es nie!“",
    ),
    (
        "03-schachtelsatz",
        "Schachtelsatz",
        "Der Antrag, der bereits im März eingegangen war und den die Sachbearbeiterin wegen fehlender Unterlagen zurückgestellt hatte, wurde gestern bewilligt.",
        "[slower] Der Antrag, der bereits im März eingegangen war und den die Sachbearbeiterin wegen fehlender Unterlagen zurückgestellt hatte, wurde gestern bewilligt.",
        "Der Antrag war bereits im März eingegangen. [short pause] Die Sachbearbeiterin hatte ihn wegen fehlender Unterlagen zurückgestellt. [pause] [slower] Gestern wurde er bewilligt.",
    ),
    (
        "04-zahlen",
        "Zahlen und Einheiten",
        "Die Strecke ist 12,5 km lang, der Anstieg beträgt 8 % und die Gebühr 3,50 EUR pro Person.",
        "Die Strecke ist 12,5 km lang, [short pause] der Anstieg beträgt 8 % [short pause] und die Gebühr 3,50 EUR pro Person.",
        "Die Strecke ist zwölfeinhalb Kilometer lang, [short pause] der Anstieg beträgt acht Prozent, [short pause] und die Gebühr drei Euro fünfzig pro Person.",
    ),
    (
        "05-aufzaehlung",
        "Aufzählung und Einschub",
        "Mitzubringen sind Personalausweis, Meldebescheinigung und ein aktuelles Foto. Übrigens gilt das auch für Kinder.",
        "Mitzubringen sind Personalausweis, Meldebescheinigung und ein aktuelles Foto. [pause] [faster] Übrigens gilt das auch für Kinder.",
        "Mitzubringen sind: Personalausweis, Meldebescheinigung, und ein aktuelles Foto. [pause] [faster] Übrigens – das gilt auch für Kinder.",
    ),
    (
        "06-betonung",
        "Kontrastbetonung",
        "Nicht der Bürgermeister hat unterschrieben, sondern seine Stellvertreterin.",
        "[slower] Nicht der Bürgermeister hat unterschrieben, sondern seine Stellvertreterin.",
        "Nicht der Bürgermeister hat unterschrieben. [short pause] [slower] Sondern seine Stellvertreterin.",
    ),
    (
        "07-traurig",
        "Ernster Absatz",
        "Am Abend kam der Anruf. Ihre Großmutter war eingeschlafen. Sie setzte sich auf die Treppe und weinte.",
        "Am Abend kam der Anruf. [pause] [slower] Ihre Großmutter war eingeschlafen. [long pause] [slower] Sie setzte sich auf die Treppe und weinte.",
        "Am Abend kam der Anruf. [pause] [slower] Ihre Großmutter war eingeschlafen … [long pause] [slower] Sie setzte sich auf die Treppe, und weinte.",
    ),
    (
        "08-hektik",
        "Hektische Szene",
        "Schnell, rief er, der Zug fährt gleich. Sie rannten die Treppe hinunter, durch die Halle, auf den Bahnsteig.",
        "[faster] Schnell, rief er, der Zug fährt gleich. [faster] Sie rannten die Treppe hinunter, durch die Halle, auf den Bahnsteig.",
        "[faster] „Schnell!“, rief er. „Der Zug fährt gleich!“ [faster] Sie rannten die Treppe hinunter – durch die Halle – auf den Bahnsteig.",
    ),
]

TAG = re.compile(r"\[([^\]\n]+)\]")
MIN_CHUNK_CHARS = 15  # wie protocol::split_sentences


def parts(text: str) -> list[tuple[str, object]]:
    """Text → Folge aus ("speak", (stück, delta)) und ("silence", ms), wie die App:
    erst an Pausen-Tags zerlegen (protocol::split_pauses), dann in Sprechstücke
    (protocol::split_sentences), je Stück ein ptts-Aufruf mit seinem Tempo."""
    out: list[tuple[str, object]] = []
    pos = 0
    for m in TAG.finditer(text):
        name = m.group(1).strip().lower()
        if name in PAUSE_MS:
            chunk = text[pos : m.start()].strip()
            if chunk:
                out.extend(("speak", s) for s in sentences(chunk))
            out.append(("silence", PAUSE_MS[name]))
            pos = m.end()
    rest = text[pos:].strip()
    if rest:
        out.extend(("speak", s) for s in sentences(rest))
    return out


def visible(s: str) -> int:
    return len(TAG.sub("", s))


def split_sentences(text: str) -> list[str]:
    """Nachbau von protocol::split_sentences (ohne Abkürzungsliste, die Texte hier
    enthalten keine): Ende nach . ! ? … mit folgendem Leerraum, nie in einem Tag,
    Stücke unter 15 sichtbaren Zeichen wandern zum nächsten Satz."""
    spans = [m.span() for m in TAG.finditer(text)]
    result, start = [], 0
    for i, c in enumerate(text):
        if c not in ".!?…" or any(a <= i < b for a, b in spans):
            continue
        if i + 1 < len(text) and not text[i + 1].isspace():
            continue
        chunk = text[start : i + 1].strip()
        if visible(chunk) >= MIN_CHUNK_CHARS:
            result.append(chunk)
            start = i + 1
    tail = text[start:].strip()
    if tail:
        result.append(tail)
    return result


def take_pace(s: str) -> tuple[str, float | None]:
    """Nachbau von protocol::take_pace: gleichgerichtet summieren, widersprüchlich
    gilt das erste, Deckel ±0,2."""
    deltas = [PACE[m.group(1).strip().lower()] for m in TAG.finditer(s) if m.group(1).strip().lower() in PACE]
    clean = re.sub(r"\s+", " ", TAG.sub(lambda m: "" if m.group(1).strip().lower() in PACE else m.group(0), s)).strip()
    if not deltas:
        return clean, None
    mixed = any((d > 0) != (deltas[0] > 0) for d in deltas)
    d = deltas[0] if mixed else sum(deltas)
    d = round(max(-PACE_MAX, min(PACE_MAX, d)), 2)
    return clean, (d or None)


def sentences(chunk: str) -> list[tuple[str, float | None]]:
    result = []
    for s in split_sentences(chunk):
        clean, d = take_pace(s)
        clean = re.sub(r"\s+([.,;:!?])", r"\1", TAG.sub("", clean)).strip()
        if clean:
            result.append((clean, d))
    return result


def synth(args: argparse.Namespace, text: str, delta: float | None, out: Path) -> None:
    cmd = [args.ptts, "--model", args.model, "--lang", "de", "--quant", "q8"]
    if args.voice:
        cmd += ["--voice", args.voice]
    if delta is not None:
        cmd += ["--condition", f"duration_delta={delta}"]
    cmd += [text, "-o", str(out)]
    env = dict(os.environ, HF_HUB_OFFLINE="1")
    subprocess.run(cmd, check=True, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=120)


def render(args: argparse.Namespace, text: str, target: Path, work: Path) -> None:
    frames: list[bytes] = []
    params = None
    for i, (kind, value) in enumerate(parts(text)):
        if kind == "silence":
            frames.append(("silence", value))  # type: ignore[arg-type]
            continue
        sentence, delta = value  # type: ignore[misc]
        tmp = work / f"s{i}.wav"
        synth(args, sentence, delta, tmp)
        with wave.open(str(tmp), "rb") as w:
            params = params or w.getparams()
            frames.append(w.readframes(w.getnframes()))
            # kurze Satzpause wie die Pipeline (Satzgrenze)
            frames.append(("silence", 120))  # type: ignore[arg-type]
    assert params is not None
    with wave.open(str(target), "wb") as out:
        out.setparams(params)
        bytes_per_ms = params.framerate * params.sampwidth * params.nchannels // 1000
        for f in frames:
            if isinstance(f, tuple):
                out.writeframes(b"\x00" * (bytes_per_ms * int(f[1])))
            else:
                out.writeframes(f)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--ptts", required=True)
    ap.add_argument("--model", required=True)
    ap.add_argument("--voice", default=None)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        for key, title, a, b, c in PASSAGES:
            cells = []
            for label, text in (("A", a), ("B", b), ("C", c)):
                name = f"{key}-{label}.wav"
                render(args, text, out / name, work)
                cells.append(
                    f"<td><audio controls src='{name}'></audio><div class='t'>{html.escape(text)}</div></td>"
                )
                print(f"ok {name}", flush=True)
            rows.append(f"<tr><th>{html.escape(title)}</th>{''.join(cells)}</tr>")
    (out / "index.html").write_text(
        "<!doctype html><meta charset='utf-8'><title>Hörprobe Betonung</title>"
        "<style>body{font:14px Segoe UI,sans-serif;margin:16px}td,th{vertical-align:top;padding:8px;border-bottom:1px solid #ddd}"
        ".t{max-width:360px;color:#444;margin-top:4px}</style>"
        "<h1>Hörprobe Betonung mit Phonon</h1><p>A Original · B Pausen und Tempo (KI-Vortrag) · C Sprechfassung</p>"
        f"<table><tr><th></th><th>A</th><th>B</th><th>C</th></tr>{''.join(rows)}</table>",
        encoding="utf-8",
    )
    print(f"fertig: {out / 'index.html'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
