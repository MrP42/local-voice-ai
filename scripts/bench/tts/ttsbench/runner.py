"""Befehl ``run``: Konfigurationen seriell messen, Ressourcen sampeln, Lauf-Manifest schreiben."""
from __future__ import annotations

import datetime as dt
import json
import os
import shutil
import sys
import time
from pathlib import Path

import benchlib

from . import HARNESS_VERSION, config as cfgmod, corpus as corpus_mod, hostinfo, rowschema
from .sampler import ResourceSampler
from .sources import AppSource, FakeSource

REPO_ROOT = Path(__file__).resolve().parents[4]


def tts_root() -> Path:
    return benchlib.bench_dir() / "tts"


def default_exe() -> Path:
    env = os.environ.get("LVA_BENCH_CLI")
    if env:
        return Path(env)
    base = REPO_ROOT / "apps" / "local-voice" / "src-tauri" / "target"
    for sub in ("release", "debug"):
        p = base / sub / "local-voice-ai.exe"
        if p.exists():
            return p
    return base / "release" / "local-voice-ai.exe"


def new_run_id(source: str) -> str:
    return dt.datetime.now().strftime("%Y%m%d-%H%M%S") + f"-{source}"


def resolve_run(run: str) -> Path:
    """Lauf-ID oder Pfad -> Lauf-Ordner."""
    p = Path(run)
    if (p / "run.json").exists():
        return p
    cand = tts_root() / run
    if (cand / "run.json").exists():
        return cand
    raise SystemExit(f"Lauf '{run}' nicht gefunden (gesucht: {p}, {cand})")


# --------------------------------------------------------------------------- Sperre (nie ASR parallel zur Messung)


def pid_alive(pid: int) -> bool:
    try:
        import psutil
        return psutil.pid_exists(pid)
    except ImportError:
        pass
    if sys.platform == "win32":
        import ctypes
        h = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)
        if not h:
            return False
        ctypes.windll.kernel32.CloseHandle(h)
        return True
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def lock_path() -> Path:
    return tts_root() / ".run.lock"


def active_lock() -> dict | None:
    p = lock_path()
    if not p.exists():
        return None
    try:
        d = json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return d if d.get("pid") and pid_alive(int(d["pid"])) and int(d["pid"]) != os.getpid() else None


def acquire_lock(run_id: str) -> None:
    other = active_lock()
    if other:
        raise SystemExit(f"Abbruch: Messlauf '{other.get('run_id')}' (PID {other.get('pid')}) läuft bereits - "
                         "Messungen und ASR laufen nie parallel.")
    lock_path().parent.mkdir(parents=True, exist_ok=True)
    lock_path().write_text(json.dumps({"pid": os.getpid(), "run_id": run_id, "started": hostinfo.now_iso()}),
                           encoding="utf-8")


def release_lock() -> None:
    try:
        d = json.loads(lock_path().read_text(encoding="utf-8"))
        if d.get("pid") == os.getpid():
            lock_path().unlink()
    except (OSError, json.JSONDecodeError):
        pass


# --------------------------------------------------------------------------- Lauf


def _entry_base(cfg: cfgmod.BenchConfig, eff: dict) -> dict:
    return {**cfg.to_dict(), "effective": eff}


def run_configs(args) -> int:
    cfgs = cfgmod.select(cfgmod.load(args.config), args.configs)
    if args.source == "fake":
        source = FakeSource(seed=args.seed, error_rate=args.fake_error_rate)
    else:
        source = AppSource(Path(args.exe) if args.exe else default_exe())
    run_id = args.run_id or new_run_id(args.source)
    run_dir = tts_root() / run_id
    if (run_dir / "run.json").exists() and not args.resume:
        raise SystemExit(f"Lauf-Ordner existiert bereits: {run_dir} (--resume zum Fortsetzen oder andere --run-id)")
    runnable = [c for c in cfgs if c.status == "run"]

    if args.dry_run:
        print(f"Lauf-ID {run_id}  Quelle {args.source}  Ordner {run_dir}")
        for c in cfgs:
            if c.status == "run":
                w, k, s = _effective(c, args)
                n = len(corpus_mod.subset(corpus_mod.load(c.corpus), s))
                print(f"  messen  {c.id:28s} {c.provider}/{c.voice or '-'} {c.device:5s} "
                      f"{n} Sätze x ({w} warm + {k} kalt) = {n * (w + k)} Synthesen")
            else:
                print(f"  {c.status:18s} {c.id}: {c.reason}")
        return 0

    ok, why = source.available()
    if runnable and not ok:
        raise SystemExit(why)
    if args.source == "app" and runnable:
        benchlib.ram_gate(args.min_free_ram_mb)  # Start-Gate: zu wenig RAM -> nichts starten
    acquire_lock(run_id)
    try:
        return _run_locked(args, cfgs, source, run_id, run_dir)
    finally:
        release_lock()


def _effective(cfg: cfgmod.BenchConfig, args) -> tuple[int, int, str]:
    warm = cfg.warm_runs if args.warm is None else args.warm
    cold = cfg.cold_runs if args.cold is None else args.cold
    subset = cfg.subset if args.subset is None else args.subset
    return warm, cold, subset


def _run_locked(args, cfgs, source, run_id: str, run_dir: Path) -> int:
    run_dir.mkdir(parents=True, exist_ok=True)
    host = hostinfo.collect(REPO_ROOT)
    (run_dir / "host.json").write_text(json.dumps(host, indent=2, ensure_ascii=False), encoding="utf-8")

    corpora: dict[str, dict] = {}
    for c in cfgs:
        if c.status == "run" and c.corpus not in corpora:
            src = corpus_mod.resolve_path(c.corpus)
            dst = run_dir / "corpus" / f"{c.corpus}.jsonl"
            dst.parent.mkdir(exist_ok=True)
            shutil.copyfile(src, dst)  # Momentaufnahme: der Lauf bleibt auch bei spaeterer Korpusaenderung auswertbar
            corpora[c.corpus] = {**corpus_mod.info(c.corpus), "snapshot": f"corpus/{c.corpus}.jsonl"}

    manifest = {
        "schema": "tts-bench-run/1", "run_id": run_id, "created_at": hostinfo.now_iso(), "finished_at": None,
        "harness_version": HARNESS_VERSION, "source": args.source,
        "exe": str(source.exe) if hasattr(source, "exe") else None,
        "config_file": str(Path(args.config).resolve()) if args.config else None,
        "overrides": {"warm": args.warm, "cold": args.cold, "subset": args.subset, "seed": args.seed},
        "corpora": corpora, "host": host, "configs": [],
    }
    entries: dict[str, dict] = {}
    if args.resume and (run_dir / "run.json").exists():
        old = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
        entries = {e["id"]: e for e in old.get("configs", []) if e.get("state") in ("measured", "failed")}

    def save() -> None:
        manifest["configs"] = [entries[c.id] for c in cfgs if c.id in entries]
        (run_dir / "run.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")

    for cfg in cfgs:
        warm, cold, subset = _effective(cfg, args)
        eff = {"warm": warm, "cold": cold, "subset": subset, "corpus": cfg.corpus}
        if cfg.status != "run":
            entries[cfg.id] = {**_entry_base(cfg, eff), "state": cfg.status, "reason": cfg.reason}
            print(f"[{cfg.id}] {cfg.status}: {cfg.reason}")
            save()
            continue
        if cfg.id in entries and args.resume:
            print(f"[{cfg.id}] übersprungen (--resume, bereits {entries[cfg.id]['state']})")
            continue
        entries[cfg.id] = _measure(cfg, eff, source, run_dir, corpora[cfg.corpus], args)
        save()

    manifest["finished_at"] = hostinfo.now_iso()
    save()
    failed = [e["id"] for e in entries.values() if e["state"] == "failed"]
    print(f"Lauf {run_id} fertig: {run_dir}")
    if failed:
        print("Fehlgeschlagen: " + ", ".join(failed))
    return 1 if (failed and args.strict) else 0


def _measure(cfg: cfgmod.BenchConfig, eff: dict, source, run_dir: Path, corpus_info: dict, args) -> dict:
    entry = {**_entry_base(cfg, eff), "state": "failed", "reason": None}
    cdir = run_dir / cfg.id
    cdir.mkdir(parents=True, exist_ok=True)
    items = corpus_mod.subset(corpus_mod.load(str(run_dir / corpus_info["snapshot"])), eff["subset"])
    expected = len(items) * (eff["warm"] + eff["cold"])
    entry["rows_expected"] = expected

    gpu_check = None
    if cfg.uses_gpu and source.name == "app":
        gpu_check = hostinfo.gpu_other_processes()
        entry["gpu_exclusive"] = gpu_check
        if args.require_exclusive and gpu_check.get("exclusive") is not True:
            entry["state"] = "failed"
            entry["reason"] = "GPU nicht exklusiv (--require-exclusive): " + str(gpu_check.get("reason") or "unbekannt")
            print(f"[{cfg.id}] {entry['reason']}")
            return entry
        if gpu_check.get("exclusive") is False:
            print(f"[{cfg.id}] WARNUNG: GPU nicht exklusiv ({gpu_check.get('reason')}) - im Ergebnis vermerkt")

    sampler = ResourceSampler(interval_ms=args.sample_ms, gpu=(source.name == "app"))
    print(f"[{cfg.id}] messe {len(items)} Sätze x ({eff['warm']} warm + {eff['cold']} kalt) ...")
    sampler.start()
    time.sleep(0.5 if source.name == "app" else 0)  # Basiswert (VRAM) vor dem Start sammeln
    try:
        res = source.run(cfg, run_dir / corpus_info["snapshot"], eff["subset"], eff["warm"], eff["cold"], cdir, sampler)
    finally:
        resources = sampler.stop()
    entry["resources"] = resources if source.name == "app" else {
        **resources, "ram_reason": "fake-Quelle: kein Prozess", "cpu_reason": "fake-Quelle: kein Prozess"}
    entry["duration_s"] = round(res.duration_s, 2)
    entry["returncode"] = res.returncode
    entry["command"] = res.command
    if res.stderr_tail.strip():
        entry["stderr_tail"] = res.stderr_tail[-600:]

    rows, problems = rowschema.read_rows(cdir / "rows.jsonl")
    entry["rows_found"] = len(rows)
    entry["rows_ok"] = sum(1 for r in rows if r["ok"])
    entry["schema_problems"] = problems[:20]
    env = None
    if (cdir / "env.json").exists():
        try:
            env = json.loads((cdir / "env.json").read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            problems.append("env.json ist kein gültiges JSON")
    entry["env"] = env
    actual = (env or {}).get("device")
    entry["device_actual"] = actual
    entry["device_mismatch"] = bool(actual and cfg.device not in ("any", actual))

    if res.error and not rows:
        entry["reason"] = res.error + (f": {res.stderr_tail.strip()[-300:]}" if res.stderr_tail.strip() else "")
    elif not rows:
        entry["reason"] = "keine gültigen Zeilen in rows.jsonl" + (f" ({problems[0]})" if problems else "")
    elif entry["rows_ok"] == 0:
        first_err = next((r.get("error") for r in rows if not r["ok"]), None)
        entry["reason"] = f"alle {len(rows)} Synthesen fehlgeschlagen: {first_err}"
    else:
        entry["state"] = "measured"
        if res.error:
            entry["reason"] = f"Lauf unvollständig beendet: {res.error}"
        elif len(rows) < expected:
            entry["reason"] = f"nur {len(rows)} von {expected} erwarteten Zeilen"
    print(f"[{cfg.id}] {entry['state']}: {entry['rows_ok']}/{expected} ok" + (f" - {entry['reason']}" if entry["reason"] else ""))
    return entry
