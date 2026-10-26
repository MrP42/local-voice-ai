"""Host-/Laufumgebung festhalten und GPU-Abfragen (nvidia-smi). Nicht Messbares -> ``None`` + Grund."""
from __future__ import annotations

import ctypes
import datetime as dt
import os
import platform
import re
import subprocess
import sys
from ctypes import wintypes
from pathlib import Path

NVIDIA_SMI_TIMEOUT = 15


def now_iso() -> str:
    d = dt.datetime.now(dt.timezone.utc)
    return d.strftime("%Y-%m-%dT%H:%M:%S.") + f"{d.microsecond // 1000:03d}Z"


def run_text(cmd: list[str], timeout: float = NVIDIA_SMI_TIMEOUT, cwd: str | None = None) -> tuple[int, str]:
    """Kleiner Hilfsaufruf (Ausgabe als Text); (-1, Grund) wenn das Programm fehlt oder haengt."""
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, cwd=cwd,
                           stdin=subprocess.DEVNULL, errors="replace",
                           creationflags=0x08000000 if sys.platform == "win32" else 0)
    except FileNotFoundError:
        return -1, f"{cmd[0]} nicht gefunden"
    except subprocess.TimeoutExpired:
        return -1, f"{cmd[0]} Zeitüberschreitung"
    except OSError as e:
        return -1, str(e)
    return p.returncode, (p.stdout or "") + (p.stderr if p.returncode else "")


def nvidia_smi_available() -> bool:
    rc, _ = run_text(["nvidia-smi", "-L"])
    return rc == 0


def gpu_info() -> dict:
    rc, out = run_text(["nvidia-smi", "--query-gpu=index,name,driver_version,memory.total,memory.used,utilization.gpu",
                        "--format=csv,noheader,nounits"])
    if rc != 0:
        return {"available": False, "reason": out.strip() or "nvidia-smi fehlgeschlagen"}
    gpus = []
    for line in out.strip().splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) >= 6:
            gpus.append({"index": int(parts[0]), "name": parts[1], "driver": parts[2],
                         "memory_total_mb": _num(parts[3]), "memory_used_mb": _num(parts[4]),
                         "util_pct": _num(parts[5])})
    cuda = None
    rc2, out2 = run_text(["nvidia-smi"])
    if rc2 == 0:
        m = re.search(r"CUDA (?:UMD )?Version:\s*([\d.]+)", out2)
        cuda = m.group(1) if m else None
    return {"available": True, "gpus": gpus, "cuda_version": cuda,
            "cuda_reason": None if cuda else "CUDA-Version nicht in nvidia-smi-Ausgabe gefunden"}


def _num(s: str) -> float | None:
    try:
        return float(s)
    except ValueError:
        return None  # "[N/A]"


def gpu_other_processes() -> dict:
    """Prueft vor GPU-Messungen, ob andere Prozesse die GPU nutzen (nicht beenden, nur vermerken)."""
    rc, out = run_text(["nvidia-smi", "--query-compute-apps=pid,process_name,used_memory",
                        "--format=csv,noheader,nounits"])
    if rc != 0:
        return {"exclusive": None, "reason": out.strip() or "nvidia-smi fehlgeschlagen",
                "other_processes": [], "util_pct_before": None, "mem_used_mb_before": None}
    procs = []
    for line in out.strip().splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) >= 2 and parts[0].isdigit():
            procs.append({"pid": int(parts[0]), "name": parts[1],
                          "used_mb": _num(parts[2]) if len(parts) > 2 else None})
    info = gpu_info()
    g0 = (info.get("gpus") or [{}])[0]
    util = g0.get("util_pct")
    busy = bool(procs) or (util is not None and util >= 10)
    reason = None
    if procs:
        reason = f"{len(procs)} Compute-Prozess(e) auf der GPU"
    elif util is not None and util >= 10:
        reason = f"GPU-Auslastung vor dem Lauf {util:.0f} %"
    return {"exclusive": not busy, "reason": reason, "other_processes": procs,
            "util_pct_before": util, "mem_used_mb_before": g0.get("memory_used_mb"),
            "note": "Unter Windows (WDDM) listet nvidia-smi nicht alle Prozesse; Auslastung/VRAM vor dem Lauf zusätzlich geprüft."}


# --------------------------------------------------------------------------- CPU / RAM / Git


def cpu_name() -> str:
    if sys.platform == "win32":
        try:
            import winreg
            with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                                r"HARDWARE\DESCRIPTION\System\CentralProcessor\0") as k:
                return str(winreg.QueryValueEx(k, "ProcessorNameString")[0]).strip()
        except OSError:
            pass
    return platform.processor() or platform.machine()


def ram_total_mb() -> int | None:
    try:
        import psutil
        return int(psutil.virtual_memory().total // (1024 * 1024))
    except ImportError:
        pass
    if sys.platform != "win32":
        return None

    class MEMORYSTATUSEX(ctypes.Structure):
        _fields_ = [("dwLength", wintypes.DWORD), ("dwMemoryLoad", wintypes.DWORD),
                    ("ullTotalPhys", ctypes.c_uint64), ("ullAvailPhys", ctypes.c_uint64),
                    ("ullTotalPageFile", ctypes.c_uint64), ("ullAvailPageFile", ctypes.c_uint64),
                    ("ullTotalVirtual", ctypes.c_uint64), ("ullAvailVirtual", ctypes.c_uint64),
                    ("sullAvailExtendedVirtual", ctypes.c_uint64)]
    st = MEMORYSTATUSEX()
    st.dwLength = ctypes.sizeof(MEMORYSTATUSEX)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(st)):
        return None
    return int(st.ullTotalPhys // (1024 * 1024))


def git_info(repo: Path) -> dict:
    rc, out = run_text(["git", "-C", str(repo), "rev-parse", "HEAD"], timeout=10)
    if rc != 0:
        return {"commit": None, "dirty": None, "reason": out.strip()}
    rc2, st = run_text(["git", "-C", str(repo), "status", "--porcelain", "--", "scripts/bench/tts"], timeout=10)
    rc3, br = run_text(["git", "-C", str(repo), "rev-parse", "--abbrev-ref", "HEAD"], timeout=10)
    return {"commit": out.strip(), "branch": br.strip() if rc3 == 0 else None,
            "dirty_harness": bool(st.strip()) if rc2 == 0 else None}


def collect(repo_root: Path | None = None) -> dict:
    try:
        import psutil
        logical = psutil.cpu_count(logical=True)
        physical = psutil.cpu_count(logical=False)
    except ImportError:
        logical, physical = os.cpu_count(), None
    root = repo_root or Path(__file__).resolve().parents[4]
    return {
        "os": f"{platform.system()} {platform.release()} ({platform.version()})",
        "machine": platform.machine(),
        "cpu": cpu_name(),
        "cpu_logical": logical,
        "cpu_physical": physical,
        "ram_total_mb": ram_total_mb(),
        "gpu": gpu_info(),
        "python": sys.version.split()[0],
        "git": git_info(root),
        "timestamp": now_iso(),
    }
