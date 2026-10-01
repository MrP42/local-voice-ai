#!/usr/bin/env python
"""Benchmark-Korpus fuer M2 (Audio/STT) erzeugen und pruefen.

    python scripts/bench/make_corpus.py fleurs   # FLEURS-de (test), >= 220 verschiedene Saetze
    python scripts/bench/make_corpus.py synth    # 3 synthetische Mehrsprecher-Szenen (SAPI)
    python scripts/bench/make_corpus.py --check  # Vollstaendigkeit pruefen, Exit 0/1

Ablage IMMER unter %LOCALAPPDATA%\\lva-bench\\ (nie ins Repo, nie in den Installer);
``LVA_BENCH_DIR`` uebersteuert das (Tests). FLEURS: google/fleurs, CC-BY-4.0.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
import random
import shutil
import sys
import tarfile
import time
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import fftconvolve, resample_poly

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchlib  # noqa: E402
import scenes as sc  # noqa: E402

SR = benchlib.SR
HF = "https://huggingface.co/datasets/google/fleurs/resolve/main/data/de_de"
TSV_URL = f"{HF}/test.tsv"
TAR_URL = f"{HF}/audio/test.tar.gz"
MIN_SENTENCES = 220
DEFAULT_SENTENCES = 240
SCENE_FILES = ("mic.wav", "system.wav", "mic_echo.wav", "mic_echo_hard.wav", "reference.json")


# ============================================================================ FLEURS
def _to_pcm16_16k(data: bytes) -> np.ndarray:
    x, sr = sf.read(io.BytesIO(data), dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != SR:
        x = resample_poly(x, SR, sr).astype(np.float32)
    return np.clip(x, -1.0, 1.0)


def parse_tsv(text: str) -> dict[str, dict]:
    """file_name -> {sent_id, text, gender}. Spalten: id, file, raw, normalisiert, chars, samples, gender."""
    rows: dict[str, dict] = {}
    for line in text.splitlines():
        p = line.rstrip("\r\n").split("\t")
        if len(p) < 3 or not p[0].strip():
            continue
        rows[p[1]] = {"sent_id": p[0], "text": p[2], "gender": p[6] if len(p) > 6 else ""}
    return rows


def cmd_fleurs(n_target: int, force: bool) -> int:
    import requests

    root = benchlib.bench_dir() / "fleurs"
    wav_dir = root / "wav"
    manifest_path = root / "manifest.json"
    if manifest_path.exists() and not force:
        ok, msg = check_fleurs()
        if ok:
            print(f"FLEURS bereits vollstaendig ({msg}) - nichts zu tun (--force erzwingt neu)")
            return 0
    wav_dir.mkdir(parents=True, exist_ok=True)
    benchlib.ram_gate(1024)

    print(f"lade {TSV_URL}")
    r = requests.get(TSV_URL, timeout=60)
    r.raise_for_status()
    (root / "test.tsv").write_bytes(r.content)
    rows = parse_tsv(r.content.decode("utf-8"))
    print(f"test.tsv: {len(rows)} Zeilen, {len({v['sent_id'] for v in rows.values()})} Saetze")

    taken: dict[str, dict] = {}
    seen_files = 0
    t0 = time.time()
    with requests.get(TAR_URL, stream=True, timeout=(20, 120)) as resp:
        resp.raise_for_status()
        # roher Byte-Strom (.tar.gz): tarfile entpackt selbst, kein Voll-Download noetig
        resp.raw.decode_content = False
        with tarfile.open(fileobj=resp.raw, mode="r|gz") as tf:
            for m in tf:
                if not m.isfile():
                    continue
                name = Path(m.name).name
                row = rows.get(name)
                if row is None:
                    continue
                seen_files += 1
                if row["sent_id"] in taken:
                    continue
                pcm = _to_pcm16_16k(tf.extractfile(m).read())
                out_name = f"{row['sent_id']}_{name}"
                sf.write(wav_dir / out_name, pcm, SR, subtype="PCM_16")
                taken[row["sent_id"]] = {
                    "sent_id": row["sent_id"], "file": f"wav/{out_name}", "source_file": name,
                    "text": row["text"], "gender": row["gender"], "duration_s": round(len(pcm) / SR, 3)}
                if len(taken) % 20 == 0:
                    print(f"  {len(taken)} Saetze ({seen_files} Aufnahmen gelesen, {time.time() - t0:.0f} s)")
                if len(taken) >= n_target:
                    break
    if len(taken) < MIN_SENTENCES:
        print(f"FEHLER: nur {len(taken)} Saetze gefunden (< {MIN_SENTENCES})", file=sys.stderr)
        return 1
    manifest = {
        "dataset": "google/fleurs", "config": "de_de", "split": "test", "license": "CC-BY-4.0",
        "source": TAR_URL, "selection": "je Satz-ID die erste Aufnahme in Archivreihenfolge",
        "audio": "PCM16, 16 kHz, mono (aus float32 gewandelt)", "recordings_read": seen_files,
        "sentences": list(taken.values()),
    }
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
    total = sum(s["duration_s"] for s in taken.values())
    print(f"FLEURS fertig: {len(taken)} Saetze, {total / 60:.1f} min Audio, {seen_files} Aufnahmen gelesen")
    return 0


def check_fleurs() -> tuple[bool, str]:
    root = benchlib.bench_dir() / "fleurs"
    mp = root / "manifest.json"
    if not mp.exists():
        return False, f"{mp} fehlt"
    try:
        mf = json.loads(mp.read_text(encoding="utf-8"))
    except ValueError as e:
        return False, f"manifest.json unlesbar: {e}"
    sents = mf.get("sentences", [])
    ids = {s["sent_id"] for s in sents}
    if len(ids) < MIN_SENTENCES or len(ids) != len(sents):
        return False, f"{len(ids)} verschiedene Satz-IDs bei {len(sents)} Eintraegen (< {MIN_SENTENCES} oder Duplikate)"
    for s in sents:
        p = root / s["file"]
        if not p.exists():
            return False, f"{p} fehlt"
        info = sf.info(p)
        if (info.samplerate, info.channels, info.subtype) != (SR, 1, "PCM_16"):
            return False, f"{p.name}: {info.samplerate} Hz/{info.channels} ch/{info.subtype}, erwartet 16000/1/PCM_16"
        if info.duration < 1.0:
            return False, f"{p.name}: nur {info.duration:.2f} s"
        if not s["text"].strip():
            return False, f"{s['sent_id']}: leere Referenz"
    return True, f"{len(ids)} Saetze"


# ============================================================================ Synth
def _tts_key(voice: str, text: str) -> str:
    return hashlib.sha1(f"{voice}|{text}".encode("utf-8")).hexdigest()[:20]


def render_tts(items: list[dict], cache: Path, runner: benchlib.LimitedRunner) -> None:
    """Fehlende Aeusserungen per SAPI (pwsh) rendern; Ergebnis liegt im Cache (sha1 aus Stimme+Text)."""
    cache.mkdir(parents=True, exist_ok=True)
    jobs, seen = [], set()
    for it in items:
        key = _tts_key(it["voice"], it["text"])
        out = cache / f"{key}.wav"
        it["_wav"] = out
        if not out.exists() and key not in seen:
            seen.add(key)
            jobs.append({"voice": it["voice"], "text": it["text"], "out": str(out)})
    if not jobs:
        return
    pwsh = shutil.which("pwsh")
    if not pwsh:
        raise SystemExit("pwsh (PowerShell 7) fehlt: nur dort sind die OneCore-Stimmen Katja/Stefan sichtbar")
    jobs_file = cache / f"jobs_{int(time.time())}.json"
    jobs_file.write_text(json.dumps(jobs, ensure_ascii=False), encoding="utf-8")
    script = Path(__file__).with_name("tts_render.ps1")
    print(f"  TTS: {len(jobs)} Aeusserungen rendern")
    rc, out, err = runner.run(
        [pwsh, "-NoProfile", "-File", str(script), "-Jobs", str(jobs_file)], timeout=1800)
    jobs_file.unlink(missing_ok=True)
    if rc != 0:
        raise SystemExit(f"TTS fehlgeschlagen (rc={rc}): {err.strip()[:500]}")


def _trim(x: np.ndarray, thr_db: float = -50.0) -> np.ndarray:
    thr = 10 ** (thr_db / 20)
    idx = np.flatnonzero(np.abs(x) > thr)
    if idx.size == 0:
        return x
    pad = int(0.02 * SR)
    return x[max(0, idx[0] - pad): idx[-1] + pad]


def _load_utt(path: Path) -> np.ndarray:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != SR:
        x = resample_poly(x, SR, sr).astype(np.float32)
    x = _trim(x)
    rms = float(np.sqrt(np.mean(x ** 2))) or 1e-6
    return (x * (0.08 / rms)).astype(np.float32)  # gleiche Lautheit je Aeusserung


def _peak_norm(x: np.ndarray, peak: float = 0.5) -> np.ndarray:
    m = float(np.abs(x).max())
    return x * (peak / m) if m > 0 else x


def build_scene(scene: sc.Scene, out_dir: Path, tts_cache: Path, runner: benchlib.LimitedRunner) -> dict:
    rng = random.Random(scene.seed)
    utts: list[dict] = []
    batch = max(10, int(scene.target_sec / 8))
    starts: list[float] = []
    durs: list[float] = []
    first = True
    while True:
        new = sc.plan_utterances(scene, rng, batch, first=first)
        first = False
        render_tts(new, tts_cache, runner)
        utts.extend(new)
        audio = [_load_utt(u["_wav"]) for u in utts]
        durs = [len(a) / SR for a in audio]
        srng = random.Random(scene.seed + 1)  # Zeitplan stets aus derselben Quelle -> stabil
        starts = sc.schedule(durs, [u["channel"] for u in utts], srng, scene.p_overlap)
        total = max(s + d for s, d in zip(starts, durs))
        if total >= scene.target_sec:
            break
        batch = max(6, int((scene.target_sec - total) / 8))
    # Abschluss-Aeusserung
    last = sc.closing_utterance(scene, rng)
    utts.append(last)
    render_tts([last], tts_cache, runner)
    audio = [_load_utt(u["_wav"]) for u in utts]
    durs = [len(a) / SR for a in audio]
    srng = random.Random(scene.seed + 1)
    starts = sc.schedule(durs, [u["channel"] for u in utts], srng, scene.p_overlap)
    n = int((max(s + d for s, d in zip(starts, durs)) + 1.0) * SR)

    tracks = {"mic": np.zeros(n, np.float32), "system": np.zeros(n, np.float32)}
    for u, a, s in zip(utts, audio, starts):
        i = int(s * SR)
        tracks[u["channel"]][i:i + len(a)] += a
    near, far = _peak_norm(tracks["mic"]), _peak_norm(tracks["system"])
    nrng = np.random.default_rng(scene.seed)

    def noise(level_db: float) -> np.ndarray:
        return (nrng.standard_normal(n) * 10 ** (level_db / 20)).astype(np.float32)

    def write(name: str, x: np.ndarray) -> None:
        sf.write(out_dir / name, np.clip(x, -1, 1).astype(np.float32), SR, subtype="PCM_16")

    out_dir.mkdir(parents=True, exist_ok=True)
    write("system.wav", far)  # Loopback: digitale Stille, wo die Gegenseite schweigt
    write("mic.wav", near + noise(-60))  # nur Nahsprache (leises Raumrauschen)

    # Echo "moderat" (Spike mix.py): Raum RT60 0,3 s, -6 dB, Verzoegerung je Szene, Rauschen -55 dB
    rir_rng = np.random.default_rng(scene.seed + 7)
    L = int(0.35 * SR)
    t = np.arange(L) / SR
    rir = rir_rng.standard_normal(L) * np.exp(-6.9 * t / 0.3) * 0.25
    rir[0] = 1.0
    rir /= np.sqrt((rir ** 2).sum())
    d = int(scene.echo_delay_ms * SR / 1000)
    echo = fftconvolve(far, rir)[:n]
    echo = np.concatenate([np.zeros(d), echo])[:n] * 10 ** (-6 / 20)
    write("mic_echo.wav", near + echo + noise(-55))

    # Echo "hart" (Spike mix_hard.py): 0 dB, tanh-Verzerrung, 150 ppm Drift, Verzoegerungssprung 50 -> 170 ms
    hrng = np.random.default_rng(scene.seed + 3)
    L = int(0.5 * SR)
    t = np.arange(L) / SR
    rir = hrng.standard_normal(L) * np.exp(-6.9 * t / 0.5) * 0.3
    rir[0] = 1.0
    rir /= np.sqrt((rir ** 2).sum())
    spk = np.tanh(2.5 * far) / 2.5
    spk = np.interp(np.arange(n) * (1 + 150e-6), np.arange(n), spk, right=0.0)
    e = fftconvolve(spk, rir)[:n]
    d1, d2, jump = int(0.05 * SR), int(0.17 * SR), n // 2
    eh = np.zeros(n)
    eh[:jump] = np.concatenate([np.zeros(d1), e])[:jump]
    eh[jump:] = np.concatenate([np.zeros(d2), e])[jump:n]
    write("mic_echo_hard.wav", near + eh + noise(-50))

    mic_words = {w for u in utts if u["channel"] == "mic" for w in benchlib.words(u["text"])}
    far_only = sorted({w for u in utts if u["channel"] == "system" for w in benchlib.words(u["text"])} - mic_words)
    ref = {
        "generator_version": sc.GENERATOR_VERSION, "scene": scene.key, "title": scene.title,
        "seed": scene.seed, "sample_rate": SR, "duration_ms": int(n / SR * 1000),
        "channels": {"mic": "Ich (Hedda)", "system": "Gegenseite (Loopback)"},
        "files": {"mic.wav": "nur Nahsprache", "system.wav": "nur Gegenseite (Referenzsignal fuer AEC)",
                  "mic_echo.wav": f"Nahsprache + Lautsprecher-Echo (-6 dB, {scene.echo_delay_ms} ms, RT60 0,3 s, Rauschen -55 dB)",
                  "mic_echo_hard.wav": "Nahsprache + hartes Echo (0 dB, tanh, 150 ppm Drift, Sprung 50->170 ms bei 50 %)"},
        "utterances": [
            {"id": i, "speaker": u["speaker"], "channel": u["channel"], "text": u["text"],
             "start_ms": int(s * 1000), "end_ms": int((s + dd) * 1000)}
            for i, (u, s, dd) in enumerate(zip(utts, starts, durs))],
        "reference_text": {
            ch: " ".join(u["text"] for u in utts if u["channel"] == ch) for ch in ("mic", "system")},
        "far_only_words": far_only,
    }
    (out_dir / "reference.json").write_text(json.dumps(ref, ensure_ascii=False, indent=1), encoding="utf-8")
    return ref


def cmd_synth(force: bool) -> int:
    root = benchlib.bench_dir() / "synth"
    runner = benchlib.LimitedRunner(memory_limit_mb=4096, max_processes=4)
    benchlib.ram_gate(2048)
    for scene in sc.SCENES:
        out_dir = root / scene.key
        if not force and all((out_dir / f).exists() for f in SCENE_FILES) and check_scene(scene)[0]:
            print(f"{scene.key}: vorhanden - uebersprungen (--force erzwingt neu)")
            continue
        print(f"{scene.key}: {scene.title}")
        ref = build_scene(scene, out_dir, root / "_tts_cache", runner)
        ov = _count_overlaps(ref["utterances"])
        print(f"  {ref['duration_ms'] / 60000:.1f} min, {len(ref['utterances'])} Aeusserungen, {ov} Ueberlappungen")
    return 0


def _count_overlaps(utts: list[dict]) -> int:
    return sum(1 for a, b in zip(utts, utts[1:]) if b["start_ms"] < a["end_ms"] and a["channel"] != b["channel"])


def check_scene(scene: sc.Scene) -> tuple[bool, str]:
    d = benchlib.bench_dir() / "synth" / scene.key
    for f in SCENE_FILES:
        if not (d / f).exists():
            return False, f"{d / f} fehlt"
    try:
        ref = json.loads((d / "reference.json").read_text(encoding="utf-8"))
    except ValueError as e:
        return False, f"{scene.key}: reference.json unlesbar: {e}"
    dur_ms = ref.get("duration_ms", 0)
    if not 180_000 <= dur_ms <= 600_000:
        return False, f"{scene.key}: Dauer {dur_ms / 1000:.0f} s ausserhalb 180..600 s"
    frames = None
    for f in SCENE_FILES[:4]:
        info = sf.info(d / f)
        if (info.samplerate, info.channels, info.subtype) != (SR, 1, "PCM_16"):
            return False, f"{scene.key}/{f}: {info.samplerate} Hz/{info.channels} ch/{info.subtype}"
        if frames is None:
            frames = info.frames
        elif info.frames != frames:
            return False, f"{scene.key}/{f}: {info.frames} Frames, andere Dateien {frames}"
    utts = ref.get("utterances", [])
    if len(utts) < 10:
        return False, f"{scene.key}: nur {len(utts)} Aeusserungen"
    speakers = {u["speaker"] for u in utts}
    if len(speakers) != len(scene.speakers):
        return False, f"{scene.key}: {len(speakers)} Sprecher statt {len(scene.speakers)}"
    prev = -1
    for u in utts:
        if not (0 <= u["start_ms"] < u["end_ms"] <= dur_ms) or u["start_ms"] < prev or not u["text"].strip():
            return False, f"{scene.key}: Aeusserung {u.get('id')} hat ungueltige Zeiten/Text"
        prev = u["start_ms"]
    if _count_overlaps(utts) < 1:
        return False, f"{scene.key}: keine Ueberlappung zwischen Ich und Gegenseite"
    return True, f"{dur_ms / 60000:.1f} min, {len(utts)} Aeusserungen"


def cmd_check() -> int:
    ok_all = True
    ok, msg = check_fleurs()
    print(f"[{'ok' if ok else 'FEHLT'}] fleurs: {msg}")
    ok_all &= ok
    for scene in sc.SCENES:
        ok, msg = check_scene(scene)
        print(f"[{'ok' if ok else 'FEHLT'}] {scene.key}: {msg}")
        ok_all &= ok
    print("Korpus vollstaendig" if ok_all else "Korpus UNVOLLSTAENDIG")
    return 0 if ok_all else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", nargs="?", choices=["fleurs", "synth"])
    ap.add_argument("--check", action="store_true", help="Vollstaendigkeit pruefen (Exit 0/1)")
    ap.add_argument("--sentences", type=int, default=DEFAULT_SENTENCES, help="FLEURS: Zahl der Saetze (>= 220)")
    ap.add_argument("--force", action="store_true", help="vorhandene Ausgabe neu erzeugen")
    args = ap.parse_args(argv)
    if args.check:
        return cmd_check()
    if args.cmd == "fleurs":
        return cmd_fleurs(max(args.sentences, MIN_SENTENCES), args.force)
    if args.cmd == "synth":
        return cmd_synth(args.force)
    ap.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
