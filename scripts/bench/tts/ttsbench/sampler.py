"""Ressourcen-Sampling waehrend eines Messlaufs (eigenes Sampling, Standard 200 ms).

* RAM (Working Set) und CPU des Kindprozesses samt Nachfahren: ``psutil`` falls vorhanden, sonst
  Windows-API per ctypes (Toolhelp + PSAPI). Prozesse, die zwischen zwei Samples starten und enden,
  werden nicht erfasst; externe Server (z. B. Fish-HTTP-Server, vom Nutzer gestartet) liegen
  ausserhalb des Prozessbaums und fehlen im RAM - dafuer zeigt die VRAM-Messung den Systemwert.
* GPU: ein dauerhaft laufender ``nvidia-smi --query-gpu=... -lms <n>`` (ein Prozess, kein Spawn je
  Sample). VRAM ist der Gesamtwert der GPU (Windows/WDDM liefert keine verlaessliche Pro-Prozess-
  Zahl), daher zusaetzlich Basiswert vor dem Start und Delta.
* Nicht messbar -> ``None`` plus ``*_reason``; nichts wird geschaetzt.
"""
from __future__ import annotations

import ctypes
import subprocess
import sys
import threading
import time
from ctypes import wintypes

IS_WINDOWS = sys.platform == "win32"


# --------------------------------------------------------------------------- Prozessbaum-Sonden


class PsutilProbe:
    name = "psutil"

    def __init__(self, root_pid: int):
        import psutil
        self.psutil = psutil
        self.root = psutil.Process(root_pid)
        self.cache: dict[int, object] = {self.root.pid: self.root}

    def sample(self) -> dict[int, tuple[int, float]]:
        ps = self.psutil
        procs = [self.root]
        try:
            procs += self.root.children(recursive=True)
        except (ps.NoSuchProcess, ps.AccessDenied):
            pass
        out: dict[int, tuple[int, float]] = {}
        for p in procs:
            try:
                t = p.cpu_times()
                out[p.pid] = (p.memory_info().rss, t.user + t.system)
            except (ps.NoSuchProcess, ps.AccessDenied):
                continue
        return out


class _ProcessEntry(ctypes.Structure):
    _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD),
                ("th32ProcessID", wintypes.DWORD), ("th32DefaultHeapID", ctypes.c_size_t),
                ("th32ModuleID", wintypes.DWORD), ("cntThreads", wintypes.DWORD),
                ("th32ParentProcessID", wintypes.DWORD), ("pcPriClassBase", ctypes.c_long),
                ("dwFlags", wintypes.DWORD), ("szExeFile", ctypes.c_wchar * 260)]


class _MemCounters(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD)] + \
               [(n, ctypes.c_size_t) for n in (
                   "PeakWorkingSetSize", "WorkingSetSize", "QuotaPeakPagedPoolUsage",
                   "QuotaPagedPoolUsage", "QuotaPeakNonPagedPoolUsage", "QuotaNonPagedPoolUsage",
                   "PagefileUsage", "PeakPagefileUsage")]


class WinApiProbe:
    """Fallback ohne psutil (nur Windows): Toolhelp-Schnappschuss + GetProcessMemoryInfo/GetProcessTimes."""
    name = "winapi"

    def __init__(self, root_pid: int):
        if not IS_WINDOWS:
            raise RuntimeError("WinApiProbe nur unter Windows")
        k = ctypes.WinDLL("kernel32", use_last_error=True)
        k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
        k.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
        k.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)]
        k.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ProcessEntry)]
        k.OpenProcess.restype = wintypes.HANDLE
        k.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k.CloseHandle.argtypes = [wintypes.HANDLE]
        k.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
        k.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(_MemCounters), wintypes.DWORD]
        self.k = k
        self.root_pid = root_pid

    def _children_map(self) -> dict[int, list[int]]:
        k = self.k
        snap = k.CreateToolhelp32Snapshot(0x2, 0)  # TH32CS_SNAPPROCESS
        m: dict[int, list[int]] = {}
        if not snap or snap == ctypes.c_void_p(-1).value:
            return m
        try:
            e = _ProcessEntry()
            e.dwSize = ctypes.sizeof(e)
            ok = k.Process32FirstW(snap, ctypes.byref(e))
            while ok:
                m.setdefault(e.th32ParentProcessID, []).append(e.th32ProcessID)
                ok = k.Process32NextW(snap, ctypes.byref(e))
        finally:
            k.CloseHandle(snap)
        return m

    def _tree(self) -> list[int]:
        m = self._children_map()
        out, todo = [], [self.root_pid]
        while todo:
            pid = todo.pop()
            if pid in out:
                continue
            out.append(pid)
            todo.extend(m.get(pid, []))
        return out

    def _one(self, pid: int) -> tuple[int, float] | None:
        k = self.k
        h = k.OpenProcess(0x1000 | 0x0010, False, pid)  # QUERY_LIMITED_INFORMATION | VM_READ
        if not h:
            h = k.OpenProcess(0x1000, False, pid)
        if not h:
            return None
        try:
            mc = _MemCounters()
            mc.cb = ctypes.sizeof(mc)
            if not k.K32GetProcessMemoryInfo(h, ctypes.byref(mc), mc.cb):
                return None
            ft = [wintypes.FILETIME() for _ in range(4)]
            if not k.GetProcessTimes(h, *[ctypes.byref(f) for f in ft]):
                return None

            def secs(f: wintypes.FILETIME) -> float:
                return ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7
            return mc.WorkingSetSize, secs(ft[2]) + secs(ft[3])
        finally:
            k.CloseHandle(h)

    def sample(self) -> dict[int, tuple[int, float]]:
        out = {}
        for pid in self._tree():
            r = self._one(pid)
            if r is not None:
                out[pid] = r
        return out


def make_probe(pid: int, force_fallback: bool = False):
    if not force_fallback:
        try:
            return PsutilProbe(pid)
        except ImportError:
            pass
    if IS_WINDOWS:
        return WinApiProbe(pid)
    return None


# --------------------------------------------------------------------------- Sampler


class ResourceSampler:
    def __init__(self, interval_ms: int = 200, gpu: bool = False, force_fallback: bool = False):
        self.interval = interval_ms / 1000.0
        self.interval_ms = interval_ms
        self.want_gpu = gpu
        self.force_fallback = force_fallback
        self._stop = threading.Event()
        self._proc_thread: threading.Thread | None = None
        self._gpu_thread: threading.Thread | None = None
        self._gpu_proc: subprocess.Popen | None = None
        self._ram: list[float] = []
        self._cpu: list[float] = []
        self._cpu_cores: list[float] = []
        self._gpu_samples: list[tuple[float, float, float]] = []  # (t, util, mem_mb)
        self._attach_t: float | None = None
        self._probe_name: str | None = None
        self._proc_reason: str | None = "kein Kindprozess angehängt"
        self._gpu_reason: str | None = None if gpu else "GPU-Messung für diese Konfiguration nicht angefordert"
        self._ncpu = 1
        try:
            import psutil
            self._ncpu = psutil.cpu_count(logical=True) or 1
        except ImportError:
            import os
            self._ncpu = os.cpu_count() or 1

    # -- GPU
    def start(self) -> None:
        if self.want_gpu:
            self._start_gpu()

    def _start_gpu(self) -> None:
        try:
            self._gpu_proc = subprocess.Popen(
                ["nvidia-smi", "--query-gpu=utilization.gpu,memory.used", "--format=csv,noheader,nounits",
                 "-lms", str(self.interval_ms), "-i", "0"],
                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True,
                creationflags=0x08000000 if IS_WINDOWS else 0)
        except (FileNotFoundError, OSError) as e:
            self._gpu_reason = f"nvidia-smi nicht startbar: {e}"
            return
        self._gpu_reason = "keine GPU-Samples empfangen"
        self._gpu_thread = threading.Thread(target=self._gpu_loop, daemon=True)
        self._gpu_thread.start()

    def _gpu_loop(self) -> None:
        assert self._gpu_proc and self._gpu_proc.stdout
        for line in self._gpu_proc.stdout:
            parts = [p.strip() for p in line.split(",")]
            if len(parts) < 2:
                continue
            try:
                util, mem = float(parts[0]), float(parts[1])
            except ValueError:
                self._gpu_reason = f"nvidia-smi liefert keine Zahlen ({line.strip()})"
                continue
            self._gpu_samples.append((time.perf_counter(), util, mem))
            self._gpu_reason = None

    # -- Prozessbaum
    def attach(self, pid: int) -> None:
        self._attach_t = time.perf_counter()
        try:
            probe = make_probe(pid, self.force_fallback)
        except Exception as e:  # noqa: BLE001 - Messung darf den Lauf nie abbrechen
            probe, self._proc_reason = None, f"Sonde nicht erstellbar: {e}"
            return
        if probe is None:
            self._proc_reason = "weder psutil noch Windows-API verfügbar"
            return
        self._probe_name = probe.name
        self._proc_reason = "keine Samples (Prozess zu kurz)"
        self._proc_thread = threading.Thread(target=self._proc_loop, args=(probe,), daemon=True)
        self._proc_thread.start()

    def _proc_loop(self, probe) -> None:
        prev: dict[int, tuple[int, float]] = {}
        prev_t = time.perf_counter()
        first = True
        while True:
            try:
                snap = probe.sample()
            except Exception as e:  # noqa: BLE001
                self._proc_reason = f"Sampling-Fehler: {e}"
                snap = {}
            now = time.perf_counter()
            if snap:
                self._proc_reason = None
                self._ram.append(sum(v[0] for v in snap.values()) / (1024 * 1024))
                if not first:
                    dcpu = 0.0
                    for pid, (_, c) in snap.items():
                        dcpu += max(0.0, c - prev[pid][1]) if pid in prev else 0.0
                    wall = max(now - prev_t, 1e-6)
                    cores = dcpu / wall
                    self._cpu_cores.append(cores)
                    self._cpu.append(100.0 * cores / self._ncpu)
                prev, prev_t, first = snap, now, False
            if self._stop.wait(self.interval):
                break

    def stop(self) -> dict:
        self._stop.set()
        if self._proc_thread:
            self._proc_thread.join(timeout=5)
        if self._gpu_proc:
            try:
                self._gpu_proc.terminate()
            except OSError:
                pass
            if self._gpu_thread:
                self._gpu_thread.join(timeout=3)
            try:
                self._gpu_proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._gpu_proc.kill()
        return self.summary()

    def summary(self) -> dict:
        def avg(v):
            return sum(v) / len(v) if v else None

        ram_ok = bool(self._ram)
        cpu_ok = bool(self._cpu)
        gpu = self._gpu_summary()
        return {
            "interval_ms": self.interval_ms,
            "probe": self._probe_name,
            "samples": len(self._ram),
            "ram_peak_mb": max(self._ram) if ram_ok else None,
            "ram_avg_mb": avg(self._ram),
            "ram_reason": None if ram_ok else self._proc_reason,
            "ram_note": "Working Set des App-Prozessbaums; extern gestartete Server nicht enthalten",
            "cpu_avg_pct_of_machine": avg(self._cpu),
            "cpu_peak_pct_of_machine": max(self._cpu) if cpu_ok else None,
            "cpu_avg_cores": avg(self._cpu_cores),
            "cpu_reason": None if cpu_ok else (self._proc_reason or "weniger als zwei Samples"),
            "logical_cpus": self._ncpu,
            "gpu": gpu,
        }

    def _gpu_summary(self) -> dict:
        s = self._gpu_samples
        if not s:
            return {"available": False, "vram_peak_mb": None, "vram_baseline_mb": None,
                    "vram_delta_peak_mb": None, "util_avg_pct": None, "util_peak_pct": None,
                    "samples": 0, "reason": self._gpu_reason or "keine GPU-Samples"}
        before = [x for x in s if self._attach_t is None or x[0] < self._attach_t]
        after = [x for x in s if self._attach_t is None or x[0] >= self._attach_t] or s
        base_src = before or s[:1]
        baseline = sorted(x[2] for x in base_src)[len(base_src) // 2]
        peak = max(x[2] for x in after)
        utils = [x[1] for x in after]
        return {"available": True, "vram_peak_mb": peak, "vram_baseline_mb": baseline,
                "vram_delta_peak_mb": peak - baseline, "util_avg_pct": sum(utils) / len(utils),
                "util_peak_pct": max(utils), "samples": len(after), "reason": None,
                "note": "VRAM = Gesamtbelegung der GPU (alle Prozesse); Delta gegen Basis vor dem Start"}
