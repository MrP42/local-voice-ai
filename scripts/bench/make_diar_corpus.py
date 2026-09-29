#!/usr/bin/env python
"""Korpus fuer die Sprechertrennung (M3, DER / AK7) erzeugen und pruefen.

    python scripts/bench/make_diar_corpus.py              # synth + AMI (Entwicklung + Pruefteil)
    python scripts/bench/make_diar_corpus.py synth        # nur die synthetischen Szenen
    python scripts/bench/make_diar_corpus.py ami          # nur AMI
    python scripts/bench/make_diar_corpus.py --check      # Vollstaendigkeit pruefen, Exit 0/1
    python scripts/bench/make_diar_corpus.py --selftest   # Schnitt-/RTTM-Logik ohne Netz

Ablage IMMER unter %LOCALAPPDATA%\\lva-bench\\diar\\ (``LVA_BENCH_DIR`` uebersteuert),
nie im Repo, nie im Installer. Je Datei ``<name>.wav`` (16 kHz, mono, PCM16) + ``<name>.rttm``:

  <szene>_system     nur die Gegenseite (Kanal 1), Referenz aus reference.json
  <szene>_mix        Mikrofon + Systemton gemischt (wie Import / Praesenz)
  ami_dev_EN2002b    AMI-Entwicklungsstuecke: auf ihnen wurden die Parameter der
  ami_dev_TS3003a    Nachbearbeitung gewaehlt (Spike M3)
  ami_test_ES2004a   AMI-Pruefteil fuer AK7 - NUR fuer die Abnahme, nie zum Einstellen
  ami_test_IS1009a

Synthetische Szenen: aus ``make_corpus.py synth``; die Aeusserungsgrenzen der Referenz werden
an der Energie der Quellspur beschnitten (Stille vorn/hinten weg, innere Pausen >= 500 ms
getrennt), wie im Spike ``prep.py``.

AMI Meeting Corpus (CC-BY-4.0): je die ersten 10 min des Headset-Mix. Audio aus dem
Hugging-Face-Datensatz diarizers-community/ami (Konfiguration ihm, Teil test, fester Stand),
Referenz ``only_words`` aus pyannote/AMI-diarization-setup. Der Edinburgh-Spiegel liefert
weniger als 1 kB/s und wird deshalb nicht benutzt. Heruntergeladen werden nur die
Parquet-Teile, die eine fehlende Besprechung enthalten (je ~300 MB, danach geloescht); welche
das sind, liest das Skript vorher per HTTP-Range aus den Zeitstempel-Spalten (~80 kB je Teil).
Braucht zusaetzlich ``pyarrow``.
"""
from __future__ import annotations

import argparse
import io
import json
import sys
import time
import urllib.request
from pathlib import Path

import numpy as np
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchlib  # noqa: E402

SR = benchlib.SR
FRAME = SR // 100  # 10 ms
ACTIVE_DBFS = -45.0
MIN_GAP_FRAMES = 50  # 500 ms
AMI_MINUTES = 10
AMI_DEV = ("EN2002b", "TS3003a")
AMI_TEST = ("ES2004a", "IS1009a")
HF_REV = "8cdaae2eaf968f3b000b6eb1204ab9b8db006ed0"
HF_BASE = f"https://huggingface.co/datasets/diarizers-community/ami/resolve/{HF_REV}/ihm/"
HF_SHARDS = ("test-00000-of-00003.parquet", "test-00001-of-00003.parquet", "test-00002-of-00003.parquet")
RTTM_URL = "https://raw.githubusercontent.com/pyannote/AMI-diarization-setup/main/only_words/rttms/test/{m}.rttm"
UA = {"User-Agent": "lva-bench/1.0"}


def out_dir() -> Path:
    return benchlib.bench_dir() / "diar"


def ami_name(meeting: str) -> str:
    return ("ami_test_" if meeting in AMI_TEST else "ami_dev_") + meeting


# ============================================================================ RTTM
def write_rttm(path: Path, uri: str, segs: list[tuple[float, float, str]]) -> None:
    lines = [f"SPEAKER {uri} 1 {s:.3f} {e - s:.3f} <NA> <NA> {spk} <NA> <NA>\n"
             for s, e, spk in sorted(segs) if e > s]
    tmp = path.with_suffix(".rttm.tmp")
    tmp.write_text("".join(lines), encoding="ascii")
    tmp.replace(path)


def read_rttm_text(text: str) -> list[tuple[float, float, str]]:
    segs = []
    for line in text.splitlines():
        p = line.split()
        if len(p) >= 8 and p[0] == "SPEAKER":
            s, d = float(p[3]), float(p[4])
            segs.append((s, s + d, p[7]))
    return segs


def cut_rttm(segs: list[tuple[float, float, str]], limit_s: float) -> list[tuple[float, float, str]]:
    """Referenz auf die ersten ``limit_s`` Sekunden begrenzen."""
    return [(s, min(e, limit_s), spk) for s, e, spk in segs if s < limit_s and min(e, limit_s) > s]


def write_wav(path: Path, x: np.ndarray) -> None:
    tmp = path.with_suffix(".wav.tmp")
    sf.write(str(tmp), np.clip(x, -1.0, 1.0), SR, subtype="PCM_16", format="WAV")
    tmp.replace(path)


# ============================================================================ Synthetisch
def active_frames(x: np.ndarray) -> np.ndarray:
    n = len(x) // FRAME
    rms = np.sqrt(np.mean(x[: n * FRAME].reshape(n, FRAME) ** 2, axis=1) + 1e-12)
    return 20 * np.log10(rms) > ACTIVE_DBFS


def trim(act: np.ndarray, start: int, end: int, min_gap: int = MIN_GAP_FRAMES) -> list[list[int]]:
    """Intervalle aktiver Rahmen in [start, end); Luecken >= min_gap trennen."""
    out: list[list[int]] = []
    cur = None
    gap = 0
    for f in range(start, min(end, len(act))):
        if act[f]:
            if cur is None:
                cur = [f, f + 1]
            elif gap >= min_gap:
                out.append(cur)
                cur = [f, f + 1]
            else:
                cur[1] = f + 1
            gap = 0
        else:
            gap += 1
    if cur:
        out.append(cur)
    return out


def synth() -> int:
    root = benchlib.bench_dir() / "synth"
    dest = out_dir()
    dest.mkdir(parents=True, exist_ok=True)
    scenes = sorted(p for p in root.iterdir() if (p / "reference.json").is_file()) if root.is_dir() else []
    if not scenes:
        print(f"keine Szenen unter {root} - zuerst: python scripts/bench/make_corpus.py synth")
        return 1
    for d in scenes:
        ref = json.loads((d / "reference.json").read_text(encoding="utf-8"))
        mic, sr = sf.read(str(d / "mic.wav"), dtype="float32")
        sysw, sr2 = sf.read(str(d / "system.wav"), dtype="float32")
        if sr != SR or sr2 != SR:
            raise SystemExit(f"{d.name}: {sr}/{sr2} Hz statt {SR}")
        n = min(len(mic), len(sysw))
        act = {"mic": active_frames(mic[:n]), "system": active_frames(sysw[:n])}
        segs: dict[str, list] = {"system": [], "mix": []}
        for u in ref["utterances"]:
            for a, b in trim(act[u["channel"]], u["start_ms"] // 10, u["end_ms"] // 10 + 1):
                seg = (a / 100.0, b / 100.0, u["speaker"])
                segs["mix"].append(seg)
                if u["channel"] == "system":
                    segs["system"].append(seg)
        mix = mic[:n] + sysw[:n]
        mix = mix / max(1.0, float(np.max(np.abs(mix))) / 0.95)
        write_wav(dest / f"{d.name}_system.wav", sysw[:n])
        write_wav(dest / f"{d.name}_mix.wav", mix)
        write_rttm(dest / f"{d.name}_system.rttm", f"{d.name}_system", segs["system"])
        write_rttm(dest / f"{d.name}_mix.rttm", f"{d.name}_mix", segs["mix"])
        print(f"{d.name}: {n / SR:.1f} s, system {len(segs['system'])} / mix {len(segs['mix'])} Segmente")
    return 0


# ============================================================================ AMI
class HttpRange(io.RawIOBase):
    """Datei-Objekt ueber HTTP-Range-Anfragen: pyarrow liest damit nur Metadaten und
    kleine Spalten, ohne den ganzen Parquet-Teil zu laden."""

    def __init__(self, url: str):
        super().__init__()
        req = urllib.request.Request(url, method="HEAD", headers=UA)
        with urllib.request.urlopen(req, timeout=60) as r:
            self.size = int(r.headers["Content-Length"])
            self.url = r.geturl()
        self.pos = 0

    def readable(self) -> bool:
        return True

    def seekable(self) -> bool:
        return True

    def tell(self) -> int:
        return self.pos

    def seek(self, off: int, whence: int = 0) -> int:
        self.pos = off if whence == 0 else (self.pos + off if whence == 1 else self.size + off)
        return self.pos

    def read(self, n: int = -1) -> bytes:
        if n is None or n < 0:
            n = self.size - self.pos
        if n == 0 or self.pos >= self.size:
            return b""
        end = min(self.size, self.pos + n) - 1
        req = urllib.request.Request(self.url, headers={**UA, "Range": f"bytes={self.pos}-{end}"})
        data = _retry(lambda: urllib.request.urlopen(req, timeout=120).read())
        self.pos += len(data)
        return data

    def readinto(self, b) -> int:
        d = self.read(len(b))
        b[: len(d)] = d
        return len(d)


def _retry(fn, tries: int = 4):
    for i in range(tries):
        try:
            return fn()
        except OSError as e:  # URLError, ConnectionReset
            if i == tries - 1:
                raise
            print(f"  Netzfehler ({e}), neuer Versuch ...")
            time.sleep(2 * (i + 1))


def fetch_rttm(meeting: str) -> list[tuple[float, float, str]]:
    req = urllib.request.Request(RTTM_URL.format(m=meeting), headers=UA)
    return read_rttm_text(_retry(lambda: urllib.request.urlopen(req, timeout=60).read().decode("ascii")))


def match_row(starts_by_row: list[list[float]], ref_starts: list[float], k: int = 5) -> int | None:
    """Zeile, deren erste k Segmentanfaenge zur Referenz passen (Parquet traegt keine IDs)."""
    want = sorted(ref_starts)[:k]
    for i, st in enumerate(starts_by_row):
        got = sorted(st)[:k]
        if len(got) == len(want) and all(abs(a - b) < 0.011 for a, b in zip(got, want)):
            return i
    return None


def locate(meetings: list[str], refs: dict[str, list]) -> dict[str, tuple[str, int]]:
    import pyarrow.parquet as pq

    found: dict[str, tuple[str, int]] = {}
    for shard in HF_SHARDS:
        if len(found) == len(meetings):
            break
        pf = pq.ParquetFile(HttpRange(HF_BASE + shard))
        starts = pf.read_row_group(0, columns=["timestamps_start"]).column("timestamps_start").to_pylist()
        for m in meetings:
            if m not in found:
                row = match_row(starts, [s for s, _, _ in refs[m]])
                if row is not None:
                    found[m] = (shard, row)
    return found


def download(url: str, dest: Path) -> None:
    """Fortsetzbarer Download (Range) nach dest.part, dann umbenennen."""
    part = dest.with_suffix(dest.suffix + ".part")
    head = urllib.request.Request(url, method="HEAD", headers=UA)
    with urllib.request.urlopen(head, timeout=60) as r:
        total = int(r.headers["Content-Length"])
    have = part.stat().st_size if part.exists() else 0
    t0 = time.time()
    while have < total:
        req = urllib.request.Request(url, headers={**UA, "Range": f"bytes={have}-"})
        try:
            with urllib.request.urlopen(req, timeout=120) as r, part.open("ab") as f:
                while True:
                    chunk = r.read(1 << 20)
                    if not chunk:
                        break
                    f.write(chunk)
                    have += len(chunk)
                    if have % (32 << 20) < (1 << 20):
                        print(f"  {have >> 20}/{total >> 20} MB ({(have >> 20) / max(time.time() - t0, 1e-3):.1f} MB/s)")
        except OSError as e:
            print(f"  Abbruch bei {have >> 20} MB ({e}), setze fort ...")
            time.sleep(3)
    part.replace(dest)


def ami(keep: bool = False) -> int:
    dest = out_dir()
    dest.mkdir(parents=True, exist_ok=True)
    lim = AMI_MINUTES * 60.0
    meetings = list(AMI_DEV + AMI_TEST)
    refs = {m: fetch_rttm(m) for m in meetings}
    for m in meetings:
        write_rttm(dest / f"{ami_name(m)}.rttm", ami_name(m), cut_rttm(refs[m], lim))
    todo = [m for m in meetings if not (dest / f"{ami_name(m)}.wav").exists()]
    if not todo:
        print("AMI: alle Audiodateien vorhanden")
        return 0
    try:
        import pyarrow.parquet as pq
    except ImportError:
        print("AMI braucht pyarrow: pip install pyarrow")
        return 1
    where = locate(todo, refs)
    missing = [m for m in todo if m not in where]
    if missing:
        print(f"AMI: nicht gefunden: {missing}")
        return 1
    tmp_dir = dest / "_download"
    tmp_dir.mkdir(exist_ok=True)
    for shard in sorted({s for s, _ in where.values()}):
        local = tmp_dir / shard
        if not local.exists():
            print(f"lade {shard} ...")
            download(HF_BASE + shard, local)
        table = pq.ParquetFile(str(local)).read_row_group(0, columns=["audio"])
        audio = table.column("audio").to_pylist()
        for m, (s, row) in where.items():
            if s != shard:
                continue
            x, sr = sf.read(io.BytesIO(audio[row]["bytes"]), dtype="float32")
            if x.ndim > 1:
                x = x.mean(axis=1)
            if sr != SR:
                raise SystemExit(f"{m}: {sr} Hz statt {SR}")
            write_wav(dest / f"{ami_name(m)}.wav", x[: int(lim * SR)])
            print(f"{ami_name(m)}: {min(len(x) / SR, lim):.0f} s von {len(x) / SR / 60:.1f} min, "
                  f"{len({spk for _, _, spk in cut_rttm(refs[m], lim)})} Sprecher")
        del table, audio
        if not keep:
            local.unlink()
    if not keep:
        try:
            tmp_dir.rmdir()
        except OSError:
            pass
    return 0


# ============================================================================ Pruefen
def expected_names() -> list[str]:
    scenes = benchlib.bench_dir() / "synth"
    names = []
    if scenes.is_dir():
        for d in sorted(p for p in scenes.iterdir() if (p / "reference.json").is_file()):
            names += [f"{d.name}_system", f"{d.name}_mix"]
    return names + [ami_name(m) for m in AMI_DEV + AMI_TEST]


def check() -> int:
    dest = out_dir()
    bad = []
    for n in expected_names():
        wav, rttm = dest / f"{n}.wav", dest / f"{n}.rttm"
        if not wav.exists() or not rttm.exists():
            bad.append(f"{n}: fehlt")
            continue
        info = sf.info(str(wav))
        if info.samplerate != SR or info.channels != 1:
            bad.append(f"{n}: {info.samplerate} Hz / {info.channels} Kanaele")
        if not read_rttm_text(rttm.read_text(encoding="ascii")):
            bad.append(f"{n}: leere Referenz")
    for b in bad:
        print(b)
    print(f"{dest}: {'vollstaendig' if not bad else f'{len(bad)} Probleme'}")
    return 0 if not bad else 1


def selftest() -> int:
    ok = True

    def expect(name, got, want):
        nonlocal ok
        good = got == want
        ok &= good
        print(f"[{'ok' if good else 'FAIL'}] {name}: {got!r}" + ("" if good else f" (erwartet {want!r})"))

    act = np.array([0, 1, 1, 0, 0, 1, 1, 1] + [0] * 60 + [1, 1], dtype=bool)
    expect("kurze Pause bleibt im Segment", trim(act, 0, 8), [[1, 8]])
    expect("lange Pause trennt", trim(act, 0, len(act)), [[1, 8], [68, 70]])
    expect("Fenster begrenzt", trim(act, 2, 6), [[2, 6]])
    expect("RTTM-Schnitt", cut_rttm([(1.0, 3.0, "A"), (599.0, 605.0, "B"), (601.0, 602.0, "C")], 600.0),
           [(1.0, 3.0, "A"), (599.0, 600.0, "B")])
    expect("Zeile per Anfaenge", match_row([[5.0, 1.0], [0.5, 1.5, 2.5]], [2.5, 0.5, 1.5]), 1)
    expect("Gruppennamen", [ami_name("ES2004a"), ami_name("EN2002b")], ["ami_test_ES2004a", "ami_dev_EN2002b"])
    expect("RTTM lesen", read_rttm_text("SPEAKER u 1 1.500 0.250 <NA> <NA> A <NA> <NA>\nx\n"), [(1.5, 1.75, "A")])
    print("SELFTEST OK" if ok else "SELFTEST FEHLGESCHLAGEN")
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("part", nargs="?", choices=["synth", "ami", "all"], default="all")
    ap.add_argument("--check", action="store_true", help="Vollstaendigkeit pruefen (Exit 0/1)")
    ap.add_argument("--selftest", action="store_true", help="Schnitt-/RTTM-Logik pruefen (ohne Netz)")
    ap.add_argument("--keep-download", action="store_true", help="Parquet-Teile nicht loeschen")
    args = ap.parse_args(argv)
    if args.selftest:
        return selftest()
    if args.check:
        return check()
    code = 0
    if args.part in ("synth", "all"):
        code |= synth()
    if args.part in ("ami", "all"):
        code |= ami(args.keep_download)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
