"""Messquellen: das App-Binary (``--tts-bench``, Paket P3) oder ``fake`` (synthetische Zeilen, ohne App)."""
from __future__ import annotations

import ctypes
import hashlib
import json
import math
import random
import subprocess
import sys
import time
from ctypes import wintypes
from dataclasses import dataclass, field
from pathlib import Path

import numpy as np
import soundfile as sf

from . import hostinfo, rowschema
from .config import BenchConfig

import benchlib  # noqa: E402  (Pfad wird in ttsbench/__init__ gesetzt)


@dataclass
class SourceResult:
    returncode: int | None
    error: str | None = None
    duration_s: float = 0.0
    command: list[str] = field(default_factory=list)
    stdout_tail: str = ""
    stderr_tail: str = ""


# --------------------------------------------------------------------------- App-Binary


def app_command(exe: Path, cfg: BenchConfig, corpus_path: Path, subset: str, warm: int, cold: int,
                out_dir: Path) -> list[str]:
    """Schnittstelle zum Rust-Teil (Paket P3): ein Aufruf je Konfiguration, seriell."""
    cmd = [str(exe), "--tts-bench", "--tts-provider", cfg.provider]
    if cfg.voice:
        cmd += ["--tts-voice", cfg.voice]
    cmd += ["--tts-lang", cfg.language, "--bench-corpus", str(corpus_path)]
    if subset and subset != "full":
        cmd += ["--bench-subset", subset]
    cmd += ["--bench-warm", str(warm), "--bench-cold", str(cold), "--bench-out", str(out_dir), "--json"]
    return cmd


def _tail(p: Path, n: int = 2000) -> str:
    try:
        return p.read_text(encoding="utf-8", errors="replace")[-n:]
    except OSError:
        return ""


class AppSource:
    name = "app"

    def __init__(self, exe: Path):
        self.exe = Path(exe)

    def available(self) -> tuple[bool, str | None]:
        if not self.exe.exists():
            return False, f"App-Binary nicht gefunden: {self.exe}"
        return True, None

    def run(self, cfg: BenchConfig, corpus_path: Path, subset: str, warm: int, cold: int,
            out_dir: Path, sampler) -> SourceResult:
        out_dir.mkdir(parents=True, exist_ok=True)
        cmd = app_command(self.exe, cfg, corpus_path, subset, warm, cold, out_dir)
        runner = benchlib.LimitedRunner(memory_limit_mb=cfg.memory_limit_mb, max_processes=cfg.max_processes)
        flags = (benchlib.BELOW_NORMAL_PRIORITY_CLASS | benchlib.CREATE_NO_WINDOW) if benchlib.IS_WINDOWS else 0
        so, se = out_dir / "app_stdout.log", out_dir / "app_stderr.log"
        t0 = time.perf_counter()
        with so.open("wb") as fo, se.open("wb") as fe:
            try:
                p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=fo, stderr=fe, creationflags=flags)
            except OSError as e:
                return SourceResult(None, f"Start fehlgeschlagen: {e}", 0.0, cmd)
            if runner.job:
                ctypes.windll.kernel32.AssignProcessToJobObject(runner.job, wintypes.HANDLE(int(p._handle)))
            sampler.attach(p.pid)
            try:
                rc = p.wait(timeout=cfg.timeout_s)
                err = None if rc == 0 else f"Rückgabewert {rc}"
            except subprocess.TimeoutExpired:
                benchlib._kill_tree(p.pid)  # nur ueber die PID
                p.wait()
                rc, err = -9, f"Zeitüberschreitung nach {cfg.timeout_s} s (Prozessbaum beendet)"
        return SourceResult(rc, err, time.perf_counter() - t0, cmd, _tail(so), _tail(se))


# --------------------------------------------------------------------------- fake

_PROFILES = {
    # ttfa/rtf: (Mittel, Streuung); stream: Chunk-Streaming; sr: Abtastrate
    "piper": dict(ttfa=(None, 0), rtf=(0.07, 0.01), sr=22050, chunks=1, device="cpu", start=(900, 150)),
    "fish": dict(ttfa=(None, 0), rtf=(0.42, 0.05), sr=44100, chunks=1, device="cuda", start=(9000, 800)),
    "phonon": dict(ttfa=(None, 0), rtf=(0.25, 0.03), sr=24000, chunks=1, device="cpu", start=(2500, 300)),
    "stream": dict(ttfa=(140, 25), rtf=(0.15, 0.02), sr=24000, chunks=6, device="cuda", start=(3000, 400)),
}
_DEFAULT_PROFILE = dict(ttfa=(None, 0), rtf=(0.3, 0.05), sr=22050, chunks=1, device="cpu", start=(1000, 100))


def _rng(*parts) -> random.Random:
    h = hashlib.sha256(":".join(map(str, parts)).encode()).hexdigest()
    return random.Random(int(h[:16], 16))


def _write_sine(path: Path, sr: int, ms: float, freq: float) -> None:
    n = int(sr * ms / 1000)
    t = np.arange(n) / sr
    env = np.minimum(1.0, np.minimum(t / 0.02, (n / sr - t) / 0.02)).clip(0, 1)
    data = (0.2 * np.sin(2 * math.pi * freq * t) * env).astype(np.float32)
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), data, sr, subtype="PCM_16")


class FakeSource:
    """Erzeugt deterministische Zeilen + kleine WAVs (nur run_idx 0) nach dem Zeilen-Schema."""
    name = "fake"

    def __init__(self, seed: int = 1, error_rate: float = 0.0):
        self.seed = seed
        self.error_rate = error_rate

    def available(self) -> tuple[bool, str | None]:
        return True, None

    def run(self, cfg: BenchConfig, corpus_path: Path, subset: str, warm: int, cold: int,
            out_dir: Path, sampler) -> SourceResult:
        from . import corpus as corpus_mod
        t0 = time.perf_counter()
        out_dir.mkdir(parents=True, exist_ok=True)
        items = corpus_mod.subset(corpus_mod.load(str(corpus_path)), subset)
        prof = _PROFILES.get(cfg.provider, _DEFAULT_PROFILE)
        rows = []
        clock = time.time()
        for kind, count in (("cold", cold), ("warm", warm)):
            for it in items:
                for idx in range(count):
                    r = _rng(self.seed, cfg.id, it["id"], kind, idx)
                    chars = len(it["text"])
                    audio_ms = round(max(700.0, 62.0 * chars * r.uniform(0.92, 1.08)), 1)
                    rtf = max(0.01, r.gauss(*prof["rtf"])) * (1.8 if kind == "cold" and idx == 0 else 1.0)
                    total_ms = round(audio_ms * rtf, 1)
                    chunks = prof["chunks"]
                    if prof["ttfa"][0] is None:
                        ttfa = total_ms
                    else:
                        ttfa = round(max(20.0, r.gauss(*prof["ttfa"])) * (2.0 if kind == "cold" and idx == 0 else 1.0), 1)
                    gaps = [round(max(1.0, (total_ms - ttfa) / max(chunks - 1, 1) * r.uniform(0.6, 1.4)), 1)
                            for _ in range(chunks - 1)]
                    ok = r.random() >= self.error_rate
                    wav_rel = None
                    row = {
                        "schema": rowschema.ROW_SCHEMA_ID, "provider": cfg.provider, "voice": cfg.voice,
                        "language": cfg.language, "utterance_id": it["id"], "run_kind": kind, "run_idx": idx,
                        "ok": ok, "error_kind": None, "error": None,
                        "process_start_ms": round(max(100.0, r.gauss(*prof["start"])), 1) if kind == "cold" else None,
                        "started_at": _iso(clock),
                    }
                    clock += 0.5
                    if ok:
                        if idx == 0:
                            wav_rel = f"wavs/{it['id']}_{kind}{idx}.wav"
                            _write_sine(out_dir / wav_rel, prof["sr"], audio_ms, 180 + (int(r.random() * 1000) % 200))
                        row.update({
                            "ttfa_ms": ttfa, "total_ms": total_ms, "audio_ms": audio_ms,
                            "rtf": round(total_ms / audio_ms, 4), "sample_rate": prof["sr"], "channels": 1,
                            "encoding": "pcm16le", "chunk_count": chunks, "chunk_gaps_ms": gaps,
                            "max_gap_ms": max(gaps) if gaps else 0.0, "wav_path": wav_rel,
                        })
                    else:
                        row.update({"error_kind": "backend", "error": "synthetischer Fehler (fake)",
                                    "ttfa_ms": None, "total_ms": None, "audio_ms": None, "rtf": None,
                                    "sample_rate": None, "channels": None, "encoding": None,
                                    "chunk_count": None, "chunk_gaps_ms": None, "max_gap_ms": None,
                                    "wav_path": None})
                    rows.append(row)
        with (out_dir / "rows.jsonl").open("w", encoding="utf-8", newline="\n") as f:
            for row in rows:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
        (out_dir / "env.json").write_text(json.dumps({
            "schema": rowschema.ENV_SCHEMA_ID, "provider": cfg.provider, "provider_version": "fake-1.0",
            "model": "fake-sine", "device": prof["device"] if cfg.device == "any" else cfg.device,
            "voice": cfg.voice, "language": cfg.language, "notes": "synthetische Zeilen (--source fake)",
            "fake": True}, indent=2), encoding="utf-8")
        return SourceResult(0, None, time.perf_counter() - t0, ["fake"])


def _iso(ts: float) -> str:
    import datetime as dt
    d = dt.datetime.fromtimestamp(ts, dt.timezone.utc)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + f"{d.microsecond // 1000:03d}Z"
