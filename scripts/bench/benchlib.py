"""Gemeinsame Bausteine der Benchmark-Skripte (Pfade, WER, Prozess-Deckel).

Nichts hier beruehrt das Repo oder produktive App-Daten: alle Ablagen liegen unter
``%LOCALAPPDATA%\\lva-bench\\`` (uebersteuerbar per ``LVA_BENCH_DIR``, z. B. fuer Tests).
"""
from __future__ import annotations

import ctypes
import os
import subprocess
import sys
from ctypes import wintypes
from pathlib import Path

SR = 16000

# --------------------------------------------------------------------------- Pfade


def bench_dir() -> Path:
    env = os.environ.get("LVA_BENCH_DIR")
    if env:
        return Path(env)
    base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
    return Path(base) / "lva-bench"


# --------------------------------------------------------------------------- WER
# Normierung wie selftest::normalize_word (Rust): lower-case, dann diese Zeichen
# entfernen, dann ae/oe/ue/ss. Zahlwoerter werden NICHT auf Ziffern gefaltet.
_PUNCT = set(".,;:!?\"'„“»«()[]")
# "weiche" Variante (nur als Zweitzahl): Bindestriche/Gedankenstriche trennen Woerter,
# typografische Anfuehrungszeichen fallen weg.
_LENIENT_DROP = set("‘’‚‹›")
_LENIENT_SPLIT = set("-‐‑–—/")


def normalize_word(word: str) -> str:
    w = "".join(c for c in word.lower() if c not in _PUNCT)
    return (
        w.replace("ä", "ae")
        .replace("ö", "oe")
        .replace("ü", "ue")
        .replace("ß", "ss")
    )


def words(text: str, lenient: bool = False) -> list[str]:
    if lenient:
        text = "".join(
            " " if c in _LENIENT_SPLIT else ("" if c in _LENIENT_DROP else c) for c in text
        )
    return [w for w in (normalize_word(x) for x in text.split()) if w]


def word_errors(ref: str, hyp: str, lenient: bool = False) -> tuple[int, int]:
    """(Fehler, Referenzwoerter): Levenshtein auf Wortebene (Sub + Ins + Del)."""
    r, h = words(ref, lenient), words(hyp, lenient)
    d = list(range(len(h) + 1))
    for i in range(1, len(r) + 1):
        prev, d[0] = d[0], i
        for j in range(1, len(h) + 1):
            cur = min(d[j] + 1, d[j - 1] + 1, prev + (r[i - 1] != h[j - 1]))
            prev, d[j] = d[j], cur
    return d[len(h)], len(r)


def aggregate_wer(pairs: list[tuple[int, int]]) -> float:
    """Korpus-WER = Summe der Fehler / Summe der Referenzwoerter (kein Mittel der Satz-WER)."""
    errors = sum(e for e, _ in pairs)
    total = sum(n for _, n in pairs)
    return errors / total if total else 0.0


# --------------------------------------------------------------------------- Systemschutz
# Kindprozesse laufen mit niedriger Prioritaet in einem Job-Objekt mit Speicherdeckel und
# KILL_ON_JOB_CLOSE (stirbt das Skript, sterben die Kinder mit). Nur Windows; sonst No-Op.

IS_WINDOWS = sys.platform == "win32"
BELOW_NORMAL_PRIORITY_CLASS = 0x00004000
CREATE_NO_WINDOW = 0x08000000


def ram_available_mb() -> int | None:
    if not IS_WINDOWS:
        return None

    class MEMORYSTATUSEX(ctypes.Structure):
        _fields_ = [
            ("dwLength", wintypes.DWORD),
            ("dwMemoryLoad", wintypes.DWORD),
            ("ullTotalPhys", ctypes.c_uint64),
            ("ullAvailPhys", ctypes.c_uint64),
            ("ullTotalPageFile", ctypes.c_uint64),
            ("ullAvailPageFile", ctypes.c_uint64),
            ("ullTotalVirtual", ctypes.c_uint64),
            ("ullAvailVirtual", ctypes.c_uint64),
            ("sullAvailExtendedVirtual", ctypes.c_uint64),
        ]

    st = MEMORYSTATUSEX()
    st.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st)):
        return None
    return int(st.ullAvailPhys // (1024 * 1024))


def ram_gate(min_free_mb: int = 4096) -> None:
    """Start-Gate: zu wenig freier RAM -> abbrechen statt das System zu belasten."""
    free = ram_available_mb()
    if free is not None and free < min_free_mb:
        raise SystemExit(f"RAM-Gate: nur {free} MB frei (< {min_free_mb} MB) - Abbruch, nichts gestartet")


class _BasicLimit(ctypes.Structure):
    _fields_ = [
        ("PerProcessUserTimeLimit", ctypes.c_int64),
        ("PerJobUserTimeLimit", ctypes.c_int64),
        ("LimitFlags", wintypes.DWORD),
        ("MinimumWorkingSetSize", ctypes.c_size_t),
        ("MaximumWorkingSetSize", ctypes.c_size_t),
        ("ActiveProcessLimit", wintypes.DWORD),
        ("Affinity", ctypes.c_size_t),
        ("PriorityClass", wintypes.DWORD),
        ("SchedulingClass", wintypes.DWORD),
    ]


class _IoCounters(ctypes.Structure):
    _fields_ = [(n, ctypes.c_uint64) for n in (
        "ReadOps", "WriteOps", "OtherOps", "ReadBytes", "WriteBytes", "OtherBytes")]


class _ExtLimit(ctypes.Structure):
    _fields_ = [
        ("Basic", _BasicLimit),
        ("Io", _IoCounters),
        ("ProcessMemoryLimit", ctypes.c_size_t),
        ("JobMemoryLimit", ctypes.c_size_t),
        ("PeakProcessMemoryUsed", ctypes.c_size_t),
        ("PeakJobMemoryUsed", ctypes.c_size_t),
    ]


_LIMIT_ACTIVE_PROCESS = 0x8
_LIMIT_PRIORITY_CLASS = 0x20
_LIMIT_JOB_MEMORY = 0x200
_LIMIT_KILL_ON_CLOSE = 0x2000


class LimitedRunner:
    """Fuehrt Kindprozesse nacheinander aus (nie parallel), BelowNormal, RAM-gedeckelt."""

    def __init__(self, memory_limit_mb: int = 8192, max_processes: int = 4):
        self.job = None
        if IS_WINDOWS:
            k32 = ctypes.windll.kernel32
            k32.CreateJobObjectW.restype = wintypes.HANDLE
            k32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
            k32.SetInformationJobObject.argtypes = [
                wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
            k32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
            job = k32.CreateJobObjectW(None, None)
            info = _ExtLimit()
            info.Basic.LimitFlags = (
                _LIMIT_KILL_ON_CLOSE | _LIMIT_PRIORITY_CLASS | _LIMIT_JOB_MEMORY | _LIMIT_ACTIVE_PROCESS)
            info.Basic.PriorityClass = BELOW_NORMAL_PRIORITY_CLASS
            info.Basic.ActiveProcessLimit = max_processes
            info.JobMemoryLimit = memory_limit_mb * 1024 * 1024
            ok = k32.SetInformationJobObject(job, 9, ctypes.byref(info), ctypes.sizeof(info))
            if job and ok:
                self.job = job
            else:
                print("WARN: Job-Objekt nicht gesetzt (nur BelowNormal-Prioritaet aktiv)", file=sys.stderr)

    def run(self, cmd: list[str], env: dict | None = None, timeout: float = 300.0,
            cwd: str | None = None) -> tuple[int, str, str]:
        flags = (BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW) if IS_WINDOWS else 0
        p = subprocess.Popen(
            cmd, env=env, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, creationflags=flags)
        if self.job:
            ctypes.windll.kernel32.AssignProcessToJobObject(self.job, wintypes.HANDLE(int(p._handle)))
        try:
            out, err = p.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            _kill_tree(p.pid)
            out, err = p.communicate()
            return -9, out.decode("utf-8", "replace"), "TIMEOUT\n" + err.decode("utf-8", "replace")
        return p.returncode, out.decode("utf-8", "replace"), err.decode("utf-8", "replace")


def _kill_tree(pid: int) -> None:
    """Nur ueber die PID (nie taskkill /IM)."""
    if IS_WINDOWS:
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True)
    else:
        try:
            os.kill(pid, 9)
        except OSError:
            pass
