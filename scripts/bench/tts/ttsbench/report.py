"""Befehl ``report``: Aggregation -> results.json / results.csv / report.md (Deutsch, keine Plots)."""
from __future__ import annotations

import csv
import datetime as dt
import json
import shutil
from pathlib import Path

from . import HARNESS_VERSION, hostinfo, rowschema, runner
from . import normalize as nz
from .asr import PROXY_NOTE
from .stats import summarize

# Zielwerte aus dem Goal (keine Zusagen): TTFA < 200 ms, RTF < 0,2, WER < 5 %.
TARGETS = {"ttfa_ms": 200.0, "rtf": 0.2, "wer": 0.05}
NOT_MEASURED_STATES = ("not_tested", "unsupported", "license_restricted", "failed")
STATE_LABEL = {"not_tested": "nicht getestet", "unsupported": "nicht unterstützt",
               "license_restricted": "Lizenz eingeschränkt", "failed": "fehlgeschlagen"}
CATEGORY_ORDER = ("alltag", "lang", "zahlen", "datum", "akronyme", "codeswitch", "fachbegriffe",
                  "email_url", "fragen", "ausdruck")


# --------------------------------------------------------------------------- Aggregation


def _rtf(r: dict) -> float | None:
    if r.get("audio_ms") and r.get("total_ms") is not None and r["audio_ms"] > 0:
        return r["total_ms"] / r["audio_ms"]
    return r.get("rtf")


def _stat(rows: list[dict], key: str) -> dict | None:
    return summarize(r.get(key) for r in rows)


def compute_metrics(rows: list[dict]) -> dict:
    """Kennzahlen aus gueltigen rows. Perzentile nur ueber erfolgreiche Zeilen; Fehler gezaehlt."""
    ok = [r for r in rows if r["ok"]]
    failed = [r for r in rows if not r["ok"]]
    kinds: dict[str, int] = {}
    for r in failed:
        kinds[r.get("error_kind") or "unbekannt"] = kinds.get(r.get("error_kind") or "unbekannt", 0) + 1
    warm = [r for r in ok if r["run_kind"] == "warm"]
    cold = [r for r in ok if r["run_kind"] == "cold"]
    cold_all = [r for r in rows if r["run_kind"] == "cold"]
    for r in warm + cold:
        r["_rtf"] = _rtf(r)
    mismatch = sum(1 for r in ok if r.get("rtf") is not None and r["_rtf"]
                   and abs(r["rtf"] - r["_rtf"]) / r["_rtf"] > 0.02)
    chunked = sum(1 for r in ok if (r.get("chunk_count") or 0) > 1)

    def block(rs: list[dict], with_start: bool = False) -> dict:
        b = {"n": len(rs), "ttfa_ms": _stat(rs, "ttfa_ms"), "total_ms": _stat(rs, "total_ms"),
             "rtf": summarize(r["_rtf"] for r in rs), "audio_ms": _stat(rs, "audio_ms"),
             "max_gap_ms": _stat(rs, "max_gap_ms")}
        if with_start:
            b["process_start_ms"] = summarize(r.get("process_start_ms") for r in cold_all)
        return b

    return {
        "rows_total": len(rows), "rows_ok": len(ok), "rows_failed": len(failed), "errors_by_kind": kinds,
        "warm": block(warm), "cold": block(cold, with_start=True),
        "chunked_rows": chunked, "rtf_mismatch_rows": mismatch,
        "definitions": "RTF = total_ms / audio_ms; TTFA = Anfrage bis erstes Audio; "
                       "Perzentile linear interpoliert; nur erfolgreiche Zeilen",
    }


def _target(value: float | None, target: float, basis: str) -> dict:
    return {"target": target, "value": value, "basis": basis,
            "met": None if value is None else value < target}


def compute_targets(metrics: dict, wer: float | None) -> dict:
    w = metrics["warm"]
    ttfa = (w["ttfa_ms"] or {}).get("median")
    rtf = (w["rtf"] or {}).get("median")
    return {"ttfa_ms": _target(ttfa, TARGETS["ttfa_ms"], "Median, warm"),
            "rtf": _target(rtf, TARGETS["rtf"], "Median, warm"),
            "wer": _target(wer, TARGETS["wer"], "Korpus-WER (Summe Fehler / Summe Referenzwörter)"),
            "note": "Zielwerte sind Ziele aus dem Goal, keine Zusagen."}


def load_run(run_dir: Path) -> dict:
    manifest = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
    asr = None
    if (run_dir / "asr.json").exists():
        asr = json.loads((run_dir / "asr.json").read_text(encoding="utf-8"))
    return {"dir": run_dir, "manifest": manifest, "asr": asr}


def build_results(run_dirs: list[Path]) -> dict:
    runs = [load_run(d) for d in run_dirs]
    order: list[str] = []
    by_id: dict[str, list[tuple[dict, dict]]] = {}
    for run in runs:
        for e in run["manifest"]["configs"]:
            if e["id"] not in by_id:
                order.append(e["id"])
            by_id.setdefault(e["id"], []).append((run, e))

    configs = []
    for cid in order:
        measured = [(r, e) for r, e in by_id[cid] if e["state"] == "measured"]
        base = (measured or by_id[cid])[0][1]
        res = {
            "id": cid, "provider": base["provider"], "voice": base.get("voice"), "language": base["language"],
            "device_expected": base["device"], "status": base["state"] if not measured else "measured",
            "reason": base.get("reason"), "note": base.get("note", ""), "runs": [r["manifest"]["run_id"] for r, _ in by_id[cid]],
            "metrics": None, "quality": None, "quality_reason": None, "resources": None, "targets": None,
            "device_actual": None, "device_mismatch": False, "gpu_exclusive": None, "flags": [],
            "effective": base.get("effective"),
        }
        if not measured:
            configs.append(res)  # keine Zahlen fuer nicht gemessene Konfigurationen
            continue
        rows: list[dict] = []
        for run, e in measured:
            rr, problems = rowschema.read_rows(run["dir"] / cid / "rows.jsonl")
            rows += rr
            if problems:
                res["flags"].append(f"{len(problems)} ungültige Zeile(n) in {run['manifest']['run_id']} ignoriert")
            if e.get("reason"):
                res["flags"].append(f"{run['manifest']['run_id']}: {e['reason']}")
        res["metrics"] = compute_metrics(rows)
        last_e = measured[-1][1]
        res["device_actual"] = last_e.get("device_actual")
        res["device_mismatch"] = bool(last_e.get("device_mismatch"))
        if res["device_mismatch"]:
            res["flags"].append(f"Gerät weicht ab: erwartet {base['device']}, gemeldet {res['device_actual']}")
        res["gpu_exclusive"] = last_e.get("gpu_exclusive")
        if res["gpu_exclusive"] and res["gpu_exclusive"].get("exclusive") is False:
            res["flags"].append("GPU nicht exklusiv: " + str(res["gpu_exclusive"].get("reason")))
        res["env"] = last_e.get("env")
        res["resources"] = last_e.get("resources")
        if res["metrics"]["rtf_mismatch_rows"]:
            res["flags"].append(f"{res['metrics']['rtf_mismatch_rows']} Zeile(n) mit rtf != total_ms/audio_ms (Bericht rechnet neu)")

        wer = None
        for run, _ in reversed(measured):
            a = (run["asr"] or {}).get("configs", {}).get(cid)
            if a:
                s = a["summary"]
                wer = s["wer"]
                res["quality"] = {
                    "asr": run["asr"]["asr"], "norm_version": run["asr"]["norm_version"], "n": s["n"],
                    "wer": s["wer"], "cer": s["cer"], "ref_words": s["ref_words"],
                    "by_category": {c: {"n": v["n"], "wer": v["wer"], "cer": v["cer"]} for c, v in a["by_category"].items()},
                    "note": PROXY_NOTE}
                break
        if res["quality"] is None:
            skipped = next((r["asr"]["skipped"][cid] for r, _ in measured if r["asr"] and cid in r["asr"].get("skipped", {})), None)
            res["quality_reason"] = skipped or "ASR nicht gelaufen (python tts_bench.py asr --run ...)"
        res["targets"] = compute_targets(res["metrics"], wer)
        configs.append(res)

    first = runs[0]["manifest"]
    return {
        "schema": "tts-bench-results/1", "generated_at": hostinfo.now_iso(), "harness_version": HARNESS_VERSION,
        "norm_version": nz.NORM_VERSION,
        "runs": [{"run_id": r["manifest"]["run_id"], "source": r["manifest"]["source"],
                  "created_at": r["manifest"]["created_at"], "finished_at": r["manifest"].get("finished_at"),
                  "overrides": r["manifest"].get("overrides"), "harness_version": r["manifest"]["harness_version"]}
                 for r in runs],
        "corpora": {k: {kk: vv for kk, vv in v.items() if kk != "path"} for k, v in first["corpora"].items()},
        "host": first["host"],
        "targets": {**TARGETS, "note": "Ziele aus dem Goal, keine Zusagen (TTFA < 200 ms, RTF < 0,2, WER < 5 %)"},
        "configs": configs,
    }


# --------------------------------------------------------------------------- CSV

_STAT_KEYS = ("median", "p90", "p95", "mean", "stdev", "min", "max")


def csv_rows(results: dict) -> tuple[list[str], list[dict]]:
    cols = ["id", "provider", "voice", "language", "device_expected", "device_actual", "status", "reason",
            "rows_total", "rows_failed", "warm_n", "cold_n"]
    for m in ("ttfa_ms", "total_ms", "rtf"):
        cols += [f"warm_{m}_{k}" for k in _STAT_KEYS]
    cols += ["cold_ttfa_ms_median", "cold_total_ms_median", "process_start_ms_median", "process_start_ms_max",
             "max_gap_ms_max", "wer", "cer", "asr_model", "ram_peak_mb", "cpu_avg_pct_of_machine",
             "cpu_peak_pct_of_machine", "vram_peak_mb", "vram_delta_peak_mb", "gpu_util_avg_pct",
             "target_ttfa_met", "target_rtf_met", "target_wer_met", "flags"]
    out = []
    for c in results["configs"]:
        row = {k: "" for k in cols}
        row.update({"id": c["id"], "provider": c["provider"], "voice": c["voice"] or "", "language": c["language"],
                    "device_expected": c["device_expected"], "device_actual": c["device_actual"] or "",
                    "status": c["status"], "reason": c["reason"] or "", "flags": " | ".join(c["flags"])})
        m = c["metrics"]
        if m:  # nur gemessene Konfigurationen tragen Zahlen
            row.update({"rows_total": m["rows_total"], "rows_failed": m["rows_failed"],
                        "warm_n": m["warm"]["n"], "cold_n": m["cold"]["n"]})
            for key in ("ttfa_ms", "total_ms", "rtf"):
                s = m["warm"][key]
                for k in _STAT_KEYS:
                    row[f"warm_{key}_{k}"] = _r(s[k]) if s else ""
            row["cold_ttfa_ms_median"] = _r((m["cold"]["ttfa_ms"] or {}).get("median"))
            row["cold_total_ms_median"] = _r((m["cold"]["total_ms"] or {}).get("median"))
            ps = m["cold"].get("process_start_ms")
            row["process_start_ms_median"] = _r((ps or {}).get("median"))
            row["process_start_ms_max"] = _r((ps or {}).get("max"))
            row["max_gap_ms_max"] = _r((m["warm"]["max_gap_ms"] or {}).get("max"))
            q = c["quality"]
            if q:
                row.update({"wer": _r(q["wer"], 4), "cer": _r(q["cer"], 4), "asr_model": q["asr"]["model"]})
            r = c["resources"] or {}
            g = r.get("gpu") or {}
            row.update({"ram_peak_mb": _r(r.get("ram_peak_mb")), "cpu_avg_pct_of_machine": _r(r.get("cpu_avg_pct_of_machine")),
                        "cpu_peak_pct_of_machine": _r(r.get("cpu_peak_pct_of_machine")),
                        "vram_peak_mb": _r(g.get("vram_peak_mb")), "vram_delta_peak_mb": _r(g.get("vram_delta_peak_mb")),
                        "gpu_util_avg_pct": _r(g.get("util_avg_pct"))})
            t = c["targets"]
            for key, col in (("ttfa_ms", "target_ttfa_met"), ("rtf", "target_rtf_met"), ("wer", "target_wer_met")):
                met = t[key]["met"]
                row[col] = "" if met is None else ("ja" if met else "nein")
        out.append(row)
    return cols, out


def _r(v, nd: int = 2):
    return "" if v is None else round(v, nd)


# --------------------------------------------------------------------------- Markdown


def de(v, nd: int = 1) -> str:
    return "–" if v is None else f"{v:.{nd}f}".replace(".", ",")


def _ja(met) -> str:
    return "nicht bewertbar" if met is None else ("ja" if met else "nein")


def _table(header: list[str], rows: list[list[str]]) -> list[str]:
    out = ["| " + " | ".join(header) + " |", "|" + "|".join("---" for _ in header) + "|"]
    out += ["| " + " | ".join(r) + " |" for r in rows]
    return out + [""]


def _stat_row(label: str, s: dict | None, nd: int, n_fail: str) -> list[str]:
    if not s:
        return [label, "0", n_fail] + ["–"] * 7
    return [label, str(s["n"]), n_fail] + [de(s[k], nd) for k in _STAT_KEYS]


def render_markdown(results: dict) -> str:
    L: list[str] = []
    measured = [c for c in results["configs"] if c["status"] == "measured"]
    other = [c for c in results["configs"] if c["status"] != "measured"]
    host = results["host"]
    L += ["# TTS-Benchmark: Bericht", ""]
    L += [f"Erzeugt: {results['generated_at']} · Harness `{results['harness_version']}` · Normalisierung `{results['norm_version']}`", ""]
    L += ["> **Zielwerte sind Ziele, keine Zusagen** (TTFA < 200 ms, RTF < 0,2, WER < 5 %). "
          "Spalte „Ziel erreicht“ vergleicht nur den gemessenen Wert mit dem Ziel.  ", f"> {PROXY_NOTE}", ""]
    if any(r["source"] == "fake" for r in results["runs"]):
        L += ["> **SYNTHETISCHE DATEN (Quelle `fake`):** Selbsttest ohne App, die Zahlen sind keine Messung "
              "und gehören in keinen veröffentlichten Bericht.", ""]
    L += ["## Läufe und Korpus", ""]
    L += _table(["Lauf", "Quelle", "Gestartet", "Beendet"],
                [[r["run_id"], r["source"], r["created_at"], r["finished_at"] or "–"] for r in results["runs"]])
    L += _table(["Korpus", "Sätze", "Smoke", "SHA-256 (LF)"],
                [[k, str(v["count"]), str(v["smoke_count"]), f"`{v['sha256'][:16]}…`"] for k, v in results["corpora"].items()])
    g = host["gpu"]
    gpu_txt = "; ".join(f"{x['name']} ({x['memory_total_mb']:.0f} MB)" for x in g.get("gpus", [])) if g.get("available") else f"nicht messbar: {g.get('reason')}"
    L += ["## Host", ""]
    L += _table(["Merkmal", "Wert"], [
        ["Betriebssystem", host["os"]], ["CPU", f"{host['cpu']} ({host['cpu_physical']} Kerne / {host['cpu_logical']} Threads)"],
        ["RAM", f"{de((host['ram_total_mb'] or 0) / 1024, 1)} GB" if host["ram_total_mb"] else "nicht messbar"],
        ["GPU", gpu_txt], ["GPU-Treiber / CUDA", f"{(g.get('gpus') or [{}])[0].get('driver', '–')} / {g.get('cuda_version') or '–'}"],
        ["Python", host["python"]], ["Git-Commit", f"`{(host['git'].get('commit') or '–')[:12]}` ({host['git'].get('branch') or '–'}, "
                                                    f"Harness {'verändert' if host['git'].get('dirty_harness') else 'unverändert'})"],
        ["Zeitstempel", host["timestamp"]]])

    if measured:
        L += ["## Latenz (warm)", "",
              "Nur erfolgreiche Synthesen; „Fehler“ = fehlgeschlagene Zeilen der Konfiguration. "
              "Perzentile linear interpoliert. RTF = Synthesezeit / Audiodauer (kleiner ist schneller).", ""]
        hdr = ["Konfiguration", "n", "Fehler", "Median", "P90", "P95", "Mittel", "Stdabw.", "Min", "Max"]
        for key, title, nd in (("ttfa_ms", "Time to first audio (ms)", 0), ("total_ms", "Gesamtdauer der Synthese (ms)", 0),
                               ("rtf", "Real-Time-Factor (RTF)", 3)):
            L += [f"### {title}", ""]
            L += _table(hdr, [_stat_row(c["id"], c["metrics"]["warm"][key], nd, str(c["metrics"]["rows_failed"])) for c in measured])
        L += ["## Kaltstart", "",
              "Prozessstart bis Anbieter bereit sowie erste Synthese nach dem Start (Zeilen mit `run_kind=cold`).", ""]
        rows = []
        for c in measured:
            cm = c["metrics"]["cold"]
            ps = cm.get("process_start_ms")
            rows.append([c["id"], str(cm["n"]), de((ps or {}).get("median"), 0), de((ps or {}).get("max"), 0),
                         de((cm["ttfa_ms"] or {}).get("median"), 0), de((cm["total_ms"] or {}).get("median"), 0)])
        L += _table(["Konfiguration", "n (ok)", "Prozessstart Median (ms)", "Prozessstart Max (ms)", "TTFA Median (ms)", "Gesamt Median (ms)"], rows)
        L += ["## Zielwerte (Ziele, keine Zusagen)", ""]
        rows = []
        for c in measured:
            t = c["targets"]
            rows.append([c["id"], de(t["ttfa_ms"]["value"], 0), _ja(t["ttfa_ms"]["met"]), de(t["rtf"]["value"], 3), _ja(t["rtf"]["met"]),
                         de(None if t["wer"]["value"] is None else 100 * t["wer"]["value"], 2), _ja(t["wer"]["met"])])
        L += _table(["Konfiguration", "TTFA Median (ms)", "Ziel < 200 ms erreicht", "RTF Median", "Ziel < 0,2 erreicht", "WER (%)", "Ziel < 5 % erreicht"], rows)
        L += ["## Qualität (Verständlichkeits-Proxy, kein Natürlichkeitsmaß)", ""]
        qrows, cats = [], []
        for c in measured:
            q = c["quality"]
            if q:
                qrows.append([c["id"], f"{q['asr']['engine']} {q['asr']['model']} ({q['asr']['compute_type']}, {q['asr']['device']}, v{q['asr']['version']})",
                              str(q["n"]), de(100 * q["wer"], 2), de(100 * q["cer"], 2)])
                cats += [k for k in q["by_category"] if k not in cats]
            else:
                qrows.append([c["id"], f"nicht ermittelt: {c['quality_reason']}", "–", "–", "–"])
        L += _table(["Konfiguration", "ASR (Modell, Version)", "Sätze", "WER (%)", "CER (%)"], qrows)
        cats = [k for k in CATEGORY_ORDER if k in cats] + [k for k in cats if k not in CATEGORY_ORDER]
        if cats:
            L += ["WER je Kategorie (%):", ""]
            L += _table(["Konfiguration"] + cats, [[c["id"]] + [de(100 * c["quality"]["by_category"][k]["wer"], 1) if k in c["quality"]["by_category"] else "–" for k in cats]
                                                    for c in measured if c["quality"]])
        L += ["## Ressourcen", "",
              "Eigenes Sampling alle 200 ms. RAM = Working Set des App-Prozessbaums (extern gestartete Server nicht enthalten). "
              "VRAM = Gesamtbelegung der GPU, Delta gegen den Wert vor dem Start. „–“ = nicht messbar (Grund unten).", ""]
        rows, reasons = [], []
        for c in measured:
            r = c["resources"] or {}
            g = r.get("gpu") or {}
            rows.append([c["id"], de(r.get("ram_peak_mb"), 0), de(r.get("cpu_avg_pct_of_machine"), 1), de(r.get("cpu_peak_pct_of_machine"), 1),
                         de(g.get("vram_peak_mb"), 0), de(g.get("vram_delta_peak_mb"), 0), de(g.get("util_avg_pct"), 0)])
            for key, val in (("RAM", r.get("ram_reason")), ("CPU", r.get("cpu_reason")), ("GPU", g.get("reason"))):
                if val:
                    reasons.append(f"- {c['id']} · {key}: {val}")
            if not c["resources"]:
                reasons.append(f"- {c['id']}: keine Ressourcendaten im Lauf")
        L += _table(["Konfiguration", "RAM Peak (MB)", "CPU Mittel (% Maschine)", "CPU Peak (% Maschine)", "VRAM Peak (MB)", "VRAM Delta (MB)", "GPU-Last Mittel (%)"], rows)
        if reasons:
            L += ["Nicht messbar:", ""] + reasons + [""]
        flagged = [c for c in measured if c["flags"]]
        if flagged:
            L += ["## Hinweise zu den Messungen", ""]
            for c in flagged:
                L += [f"- **{c['id']}**: " + "; ".join(c["flags"])]
            L += [""]

    if other:
        L += ["## Nicht gemessene Konfigurationen", "", "Ohne Zahlen: Status und Grund laut Konfiguration bzw. Lauf.", ""]
        L += _table(["Konfiguration", "Anbieter", "Status", "Grund"],
                    [[c["id"], c["provider"], STATE_LABEL.get(c["status"], c["status"]), (c["reason"] or "–").replace("|", "/")] for c in other])

    L += ["## Reproduktion", "",
          f"- Korpus und Normalisierung sind versioniert (`{nz.NORM_VERSION}`); Lauf-Ordner enthält Momentaufnahme des Korpus, `host.json`, `run.json` und je Konfiguration `rows.jsonl`/`env.json`.",
          "- Ablauf: `python scripts/bench/tts/tts_bench.py run` → `asr --run <id>` → `report --runs <id>`; Details in `scripts/bench/tts/README.md`.", ""]
    return "\n".join(L)


# --------------------------------------------------------------------------- Ausgabe


def write_report(run_dirs: list[Path], out_dir: Path) -> dict:
    results = build_results(run_dirs)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "results.json").write_text(json.dumps(results, indent=2, ensure_ascii=False), encoding="utf-8")
    cols, rows = csv_rows(results)
    with (out_dir / "results.csv").open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=cols)
        w.writeheader()
        w.writerows(rows)
    (out_dir / "report.md").write_text(render_markdown(results), encoding="utf-8", newline="\n")
    return results


def publish(out_dir: Path, target: Path) -> list[Path]:
    """Kopiert NUR die Zusammenfassung (json/csv/md), nie WAVs oder Rohzeilen."""
    target.mkdir(parents=True, exist_ok=True)
    copied = []
    for name in ("results.json", "results.csv", "report.md"):
        shutil.copyfile(out_dir / name, target / name)
        copied.append(target / name)
    return copied


def default_publish_dir(date: str | None = None) -> Path:
    d = date or dt.date.today().isoformat()
    return runner.REPO_ROOT / "docs" / "tts" / "benchmarks" / d
