"""Blindhoertest-Paket: anonymisierte WAVs, getrennter Schluessel, Bewertungsbogen, Auswertung.

Es werden niemals Bewertungen erzeugt: ``listening-eval`` liest nur, was Menschen eingetragen haben.
"""
from __future__ import annotations

import csv
import hashlib
import json
import math
import random
import statistics
from pathlib import Path

import numpy as np
import soundfile as sf

from . import asr as asr_mod, corpus as corpus_mod, hostinfo, rowschema, runner

CRITERIA = ("natuerlichkeit", "aussprache", "prosodie", "artefakte")
CRITERIA_LABEL = {"natuerlichkeit": "Natürlichkeit", "aussprache": "Aussprache",
                  "prosodie": "Prosodie", "artefakte": "Artefakte"}
SHEET_COLUMNS = ("probe_id", "satz") + CRITERIA + ("freitext", "hoerer")
TARGET_RMS_DBFS = -26.0
PEAK_LIMIT = 0.97

README_TEXT = """# Blindhörtest: Anleitung

Sie hören {n} kurze Sprachproben. Die Proben stammen von verschiedenen Sprachausgaben, **welche das ist, wird
Ihnen nicht gesagt** (und steht nicht in diesem Ordner). Die Reihenfolge ist gemischt, die Dateinamen sind
Zufalls-IDs.

## Ablauf

1. Öffnen Sie `bewertungsbogen.csv` (Excel/LibreOffice; Trennzeichen Semikolon, UTF-8).
2. Hören Sie jede Datei in `proben/` mindestens einmal ganz an, mit demselben Kopfhörer und gleicher Lautstärke.
   Die Proben sind auf ähnliche Lautstärke angeglichen.
3. Tragen Sie je Zeile in `natuerlichkeit`, `aussprache`, `prosodie` und `artefakte` eine ganze Zahl von 1 bis 5 ein
   (5 ist jeweils das beste Ergebnis). Leer lassen, wenn Sie eine Probe nicht beurteilen können.
4. Optional: `freitext` für Auffälligkeiten, `hoerer` für Ihr Kürzel (damit mehrere Bögen zusammengeführt werden können).
5. Speichern Sie den Bogen als CSV (nicht als .xlsx) und geben Sie ihn zurück.

## Skala

| Merkmal | 1 | 5 |
|---|---|---|
| Natürlichkeit | klar künstlich/roboterhaft | von einer Menschenstimme kaum zu unterscheiden |
| Aussprache | viele falsch gesprochene Wörter/Zahlen/Namen | alles korrekt gesprochen |
| Prosodie (Betonung, Satzmelodie, Pausen) | monoton oder unpassend | passend zum Inhalt (Frage, Ausruf, Aufzählung) |
| Artefakte (Knacken, Rauschen, Abbrüche, Verzerrung) | stark störend | keine hörbaren Störungen |

## Hinweise für die Durchführung

- Nicht in Gruppen bewerten und nicht über die Proben sprechen, bevor alle Bögen abgegeben sind.
- Der Schlüssel liegt getrennt beim Auswertenden: `python scripts/bench/tts/tts_bench.py listening-eval --key <key.json> --sheets <bogen.csv> ...`.
- Seed dieses Pakets: {seed} (steht ebenfalls nur im Schlüssel-Ordner).
"""


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _match_loudness(src: Path, dst: Path) -> float:
    """Pegelt auf TARGET_RMS_DBFS (RMS) mit Spitzenbegrenzung; gibt die Verstaerkung in dB zurueck."""
    data, sr = sf.read(str(src), dtype="float32", always_2d=True)
    rms = float(np.sqrt(np.mean(np.square(data.astype(np.float64))))) if data.size else 0.0
    if rms <= 1e-9:
        sf.write(str(dst), data, sr, subtype="PCM_16")
        return 0.0
    gain = 10 ** ((TARGET_RMS_DBFS - 20 * math.log10(rms)) / 20)
    peak = float(np.max(np.abs(data)))
    if peak * gain > PEAK_LIMIT:
        gain = PEAK_LIMIT / peak
    sf.write(str(dst), np.clip(data * gain, -1.0, 1.0), sr, subtype="PCM_16")
    return 20 * math.log10(gain)


def _inside(child: Path, parent: Path) -> bool:
    try:
        child.resolve().relative_to(parent.resolve())
        return True
    except ValueError:
        return False


def collect_samples(run_dirs: list[Path], config_ids: list[str] | None, subset: str,
                    max_per_config: int | None) -> list[dict]:
    samples: list[dict] = []
    for run_dir in run_dirs:
        manifest = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
        for entry in manifest["configs"]:
            if entry["state"] != "measured" or (config_ids and entry["id"] not in config_ids):
                continue
            cdir = run_dir / entry["id"]
            rows, _ = rowschema.read_rows(cdir / "rows.jsonl")
            corpus = corpus_mod.by_id(corpus_mod.load(str(run_dir / manifest["corpora"][entry["corpus"]]["snapshot"])))
            allowed = {i["id"] for i in corpus_mod.subset(list(corpus.values()), subset)}
            picked = [(r, p) for r, p in asr_mod.pick_wavs(rows, cdir, False) if r["utterance_id"] in allowed]
            picked.sort(key=lambda x: x[0]["utterance_id"])
            if max_per_config:
                picked = picked[:max_per_config]
            for r, p in picked:
                u = corpus[r["utterance_id"]]
                samples.append({"config_id": entry["id"], "provider": entry["provider"], "voice": entry.get("voice"),
                                "utterance_id": r["utterance_id"], "category": u["category"], "text": u["text"],
                                "source": p, "run_id": manifest["run_id"]})
    samples.sort(key=lambda s: (s["config_id"], s["utterance_id"]))
    return samples


def make_kit(run_dirs: list[Path], out_dir: Path, key_dir: Path, seed: int, config_ids: list[str] | None = None,
             subset: str = "smoke", max_per_config: int | None = None, loudness: bool = True,
             kit_id: str | None = None) -> dict:
    if _inside(key_dir, out_dir) or _inside(out_dir, key_dir):
        raise SystemExit("Schluessel-Ordner und Proben-Ordner duerfen nicht ineinander liegen (Blindtest).")
    samples = collect_samples(run_dirs, config_ids, subset, max_per_config)
    if not samples:
        raise SystemExit("keine WAV-Dateien gefunden (Lauf mit Audio? Konfiguration/Teilmenge pruefen)")
    rng = random.Random(seed)
    order = list(range(len(samples)))
    rng.shuffle(order)
    ids: set[str] = set()
    key_samples: dict[str, dict] = {}
    sheet_rows = []
    (out_dir / "proben").mkdir(parents=True, exist_ok=True)
    key_dir.mkdir(parents=True, exist_ok=True)
    for pos in order:
        s = samples[pos]
        pid = f"p{rng.randrange(16 ** 6):06x}"
        while pid in ids:
            pid = f"p{rng.randrange(16 ** 6):06x}"
        ids.add(pid)
        dst = out_dir / "proben" / f"{pid}.wav"
        if loudness:
            gain = _match_loudness(s["source"], dst)
        else:
            dst.write_bytes(Path(s["source"]).read_bytes())
            gain = 0.0
        key_samples[pid] = {"config_id": s["config_id"], "provider": s["provider"], "voice": s["voice"],
                            "utterance_id": s["utterance_id"], "category": s["category"], "run_id": s["run_id"],
                            "source_sha256": _sha256(Path(s["source"])), "gain_db": round(gain, 2)}
        sheet_rows.append({"probe_id": pid, "satz": s["text"]})
    with (out_dir / "bewertungsbogen.csv").open("w", encoding="utf-8-sig", newline="") as f:
        w = csv.DictWriter(f, fieldnames=SHEET_COLUMNS, delimiter=";")
        w.writeheader()
        for r in sheet_rows:  # in Wiedergabe-Reihenfolge (gemischt)
            w.writerow({**{c: "" for c in SHEET_COLUMNS}, **r})
    (out_dir / "README.md").write_text(README_TEXT.format(n=len(sheet_rows), seed="siehe Schlüssel"), encoding="utf-8", newline="\n")
    kit = kit_id or f"kit-{seed}"
    key = {"schema": "tts-bench-listening-key/1", "kit_id": kit, "seed": seed, "created_at": hostinfo.now_iso(),
           "run_ids": sorted({s["run_id"] for s in samples}), "loudness_matched": loudness,
           "target_rms_dbfs": TARGET_RMS_DBFS if loudness else None, "subset": subset,
           "criteria": list(CRITERIA), "scale": "1 (schlecht) bis 5 (sehr gut)", "samples": key_samples}
    (key_dir / "key.json").write_text(json.dumps(key, indent=2, ensure_ascii=False), encoding="utf-8")
    return {"kit_dir": out_dir, "key_path": key_dir / "key.json", "n": len(sheet_rows), "seed": seed}


# --------------------------------------------------------------------------- Auswertung


def _read_sheet(path: Path) -> list[dict]:
    text = path.read_text(encoding="utf-8-sig")
    first = text.splitlines()[0] if text.strip() else ""
    delim = ";" if first.count(";") >= first.count(",") else ","
    return list(csv.DictReader(text.splitlines(), delimiter=delim))


def _parse_rating(v: str | None) -> tuple[int | None, bool]:
    """(Wert, ungueltig?) - leer ist erlaubt (nicht bewertet), ausser 1..5 ist ungueltig."""
    if v is None or not str(v).strip():
        return None, False
    try:
        x = float(str(v).strip().replace(",", "."))
    except ValueError:
        return None, True
    if x != int(x) or not 1 <= int(x) <= 5:
        return None, True
    return int(x), False


def evaluate(key_path: Path, sheets: list[Path]) -> dict:
    key = json.loads(key_path.read_text(encoding="utf-8"))
    samples = key["samples"]
    ratings: dict[str, dict[str, list[int]]] = {}
    by_cat: dict[tuple[str, str], list[int]] = {}
    invalid: list[str] = []
    unknown: list[str] = []
    raters: set[str] = set()
    n_sheets_rows = 0
    for sheet in sheets:
        for n, row in enumerate(_read_sheet(sheet), 2):
            pid = (row.get("probe_id") or "").strip()
            if not pid:
                continue
            if pid not in samples:
                unknown.append(f"{sheet.name}:{n} {pid}")
                continue
            got = False
            for crit in CRITERIA:
                val, bad = _parse_rating(row.get(crit))
                if bad:
                    invalid.append(f"{sheet.name}:{n} {crit}={row.get(crit)!r}")
                elif val is not None:
                    got = True
                    cid = samples[pid]["config_id"]
                    ratings.setdefault(cid, {c: [] for c in CRITERIA})[crit].append(val)
                    if crit == "natuerlichkeit":
                        by_cat.setdefault((cid, samples[pid]["category"]), []).append(val)
            if got:
                n_sheets_rows += 1
                raters.add((row.get("hoerer") or sheet.stem).strip())
    configs = {}
    for cid, per in ratings.items():
        allv = [v for c in CRITERIA for v in per[c]]
        configs[cid] = {
            "n_ratings": max(len(per[c]) for c in CRITERIA),
            "criteria": {c: ({"n": len(per[c]), "mean": statistics.fmean(per[c]), "median": statistics.median(per[c]),
                              "stdev": statistics.stdev(per[c]) if len(per[c]) > 1 else 0.0} if per[c] else None)
                         for c in CRITERIA},
            "overall_mean": statistics.fmean(allv) if allv else None,
            "natuerlichkeit_by_category": {cat: statistics.fmean(v) for (c2, cat), v in sorted(by_cat.items()) if c2 == cid},
        }
    return {"schema": "tts-bench-listening-results/1", "kit_id": key["kit_id"], "seed": key["seed"],
            "created_at": hostinfo.now_iso(), "sheets": [s.name for s in sheets], "raters": sorted(raters),
            "n_probes": len(samples), "n_rated_rows": n_sheets_rows, "configs": configs,
            "invalid_entries": invalid, "unknown_probe_ids": unknown,
            "note": ("keine Bewertungen vorhanden" if not configs else
                     "Mittelwerte über alle eingetragenen Bewertungen; kleine Stichprobe, keine Signifikanzaussage")}


def write_eval(result: dict, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "listening_results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    cols = ["config_id", "n_ratings"] + [f"{c}_{k}" for c in CRITERIA for k in ("n", "mean", "median")] + ["overall_mean"]
    with (out_dir / "listening_results.csv").open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(f, fieldnames=cols)
        w.writeheader()
        for cid, c in result["configs"].items():
            row = {"config_id": cid, "n_ratings": c["n_ratings"], "overall_mean": _r(c["overall_mean"])}
            for crit in CRITERIA:
                s = c["criteria"][crit]
                row.update({f"{crit}_n": s["n"] if s else 0, f"{crit}_mean": _r(s["mean"]) if s else "",
                            f"{crit}_median": s["median"] if s else ""})
            w.writerow(row)
    L = ["# Blindhörtest: Auswertung", "", f"Paket `{result['kit_id']}` · Seed {result['seed']} · Hörer: "
         f"{', '.join(result['raters']) or '–'} · {result['n_rated_rows']} bewertete Zeilen von {result['n_probes']} Proben", ""]
    if not result["configs"]:
        L += ["**Keine Bewertungen vorhanden.** Es werden keine Werte erzeugt.", ""]
    else:
        L += ["| Konfiguration | Bewertungen | " + " | ".join(CRITERIA_LABEL[c] for c in CRITERIA) + " | Gesamt |",
              "|---|---|" + "---|" * (len(CRITERIA) + 1)]
        for cid, c in result["configs"].items():
            cells = [f"{c['criteria'][k]['mean']:.2f}".replace(".", ",") if c["criteria"][k] else "–" for k in CRITERIA]
            L.append(f"| {cid} | {c['n_ratings']} | " + " | ".join(cells) + f" | {_de(c['overall_mean'])} |")
        L += ["", "Skala 1 (schlecht) bis 5 (sehr gut); Mittelwerte. " + result["note"], ""]
    if result["invalid_entries"]:
        L += ["Ungültige Einträge (ignoriert): " + "; ".join(result["invalid_entries"][:20]), ""]
    if result["unknown_probe_ids"]:
        L += ["Unbekannte Proben-IDs (ignoriert): " + "; ".join(result["unknown_probe_ids"][:20]), ""]
    (out_dir / "listening_results.md").write_text("\n".join(L), encoding="utf-8", newline="\n")


def _r(v):
    return "" if v is None else round(v, 3)


def _de(v):
    return "–" if v is None else f"{v:.2f}".replace(".", ",")
