#!/usr/bin/env python
"""TTS-Benchmark (Paket P5, Goal tts-provider): Korpus, Messlauf, ASR, Bericht, Hoertest.

    python scripts/bench/tts/tts_bench.py validate-corpus
    python scripts/bench/tts/tts_bench.py run --dry-run                       # Plan zeigen
    python scripts/bench/tts/tts_bench.py run --source fake                   # ohne App (Selbsttest)
    python scripts/bench/tts/tts_bench.py run --configs piper-thorsten-high-cpu --exe <app.exe>
    python scripts/bench/tts/tts_bench.py asr --run <lauf-id>                 # NACH dem Lauf, nie parallel
    python scripts/bench/tts/tts_bench.py report --runs <lauf-id> [--publish]
    python scripts/bench/tts/tts_bench.py listening-kit --run <lauf-id>
    python scripts/bench/tts/tts_bench.py listening-eval --key <key.json> --sheets <bogen.csv>

Rohdaten (WAVs, rows.jsonl) liegen unter %LOCALAPPDATA%\\lva-bench\\tts\\<lauf-id>\\ (benchlib.bench_dir()),
nie im Repo. Mehr: scripts/bench/tts/README.md.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
for _s in (sys.stdout, sys.stderr):
    try:
        _s.reconfigure(encoding="utf-8")
    except (AttributeError, OSError):
        pass

from ttsbench import config as cfgmod, corpus as corpus_mod, listening, report, runner  # noqa: E402
from ttsbench.asr import AsrError, run_asr  # noqa: E402


def cmd_validate_corpus(a) -> int:
    path = corpus_mod.resolve_path(a.corpus)
    items = corpus_mod.load(a.corpus)
    strict = path.stem.startswith("de-v") and not a.basic
    errors, stats = corpus_mod.validate(
        items,
        min_sentences=a.min_sentences if a.min_sentences is not None else (100 if strict else 1),
        min_per_category=a.min_per_category if a.min_per_category is not None else (10 if strict else 1),
        smoke_size=(12 if strict else None),
        required_categories=(corpus_mod.CATEGORIES if strict else None))
    print(f"Korpus {path.name}: {stats['count']} Sätze, Smoke {stats['smoke']}, SHA-256 {corpus_mod.sha256_of(path)[:16]}…")
    print("Kategorien: " + ", ".join(f"{k} {v}" for k, v in sorted(stats["categories"].items())))
    if errors:
        print(f"{len(errors)} Fehler:")
        for e in errors[:50]:
            print("  - " + e)
        return 1
    print("gültig")
    return 0


def cmd_run(a) -> int:
    try:
        return runner.run_configs(a)
    except cfgmod.ConfigError as e:
        raise SystemExit(f"Konfigurationsfehler: {e}")


def cmd_asr(a) -> int:
    try:
        return run_asr(a)
    except AsrError as e:
        raise SystemExit(f"ASR nicht möglich: {e}")


def cmd_report(a) -> int:
    dirs = [runner.resolve_run(r) for r in a.runs]
    out = Path(a.out) if a.out else dirs[0] / "report"
    results = report.write_report(dirs, out)
    n_meas = sum(1 for c in results["configs"] if c["status"] == "measured")
    print(f"Bericht: {out} ({n_meas} gemessen, {len(results['configs']) - n_meas} ohne Zahlen)")
    for name in ("report.md", "results.json", "results.csv"):
        print(f"  {out / name}")
    if a.publish:
        target = Path(a.publish_dir) if a.publish_dir else report.default_publish_dir(a.date)
        for p in report.publish(out, target):
            print(f"  veröffentlicht: {p}")
    return 0


def cmd_listening_kit(a) -> int:
    dirs = [runner.resolve_run(r) for r in a.run]
    kit_id = a.kit_id or f"kit-{a.seed}"
    out = Path(a.out) if a.out else dirs[0] / "listening-kit" / kit_id
    key_dir = Path(a.key_out) if a.key_out else dirs[0] / "listening-key" / kit_id
    res = listening.make_kit(dirs, out, key_dir, a.seed, a.configs, a.subset, a.max_per_config,
                             not a.no_loudness_match, kit_id)
    print(f"Hörtest-Paket: {res['kit_dir']} ({res['n']} Proben, Seed {res['seed']})")
    print(f"Schlüssel (getrennt aufbewahren, nicht an Hörer geben): {res['key_path']}")
    return 0


def cmd_listening_eval(a) -> int:
    key = Path(a.key)
    result = listening.evaluate(key, [Path(s) for s in a.sheets])
    out = Path(a.out) if a.out else key.parent / "auswertung"
    listening.write_eval(result, out)
    print(f"Auswertung: {out / 'listening_results.md'} ({len(result['configs'])} Konfiguration(en) mit Bewertungen)")
    if result["invalid_entries"] or result["unknown_probe_ids"]:
        print(f"Hinweis: {len(result['invalid_entries'])} ungültige Einträge, {len(result['unknown_probe_ids'])} unbekannte IDs ignoriert")
    return 0


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="tts_bench.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)

    v = sub.add_parser("validate-corpus", help="Korpus prüfen (Schema, Kategorien, Normalisierung)")
    v.add_argument("corpus", nargs="?", default="de-v1", help="Name (corpus/<name>.jsonl) oder Pfad")
    v.add_argument("--min-sentences", type=int)
    v.add_argument("--min-per-category", type=int)
    v.add_argument("--basic", action="store_true", help="nur Schema/Normalisierung, keine de-v-Mindestzahlen")
    v.set_defaults(fn=cmd_validate_corpus)

    r = sub.add_parser("run", help="Konfigurationen messen")
    r.add_argument("--config", default=str(cfgmod.DEFAULT_CONFIG), help="TOML mit den Messkonfigurationen")
    r.add_argument("--configs", nargs="+", help="nur diese Konfigurations-IDs")
    r.add_argument("--source", choices=("app", "fake"), default="app")
    r.add_argument("--exe", help="App-Binary (Standard: $LVA_BENCH_CLI oder target/release)")
    r.add_argument("--warm", type=int, help="Warmläufe je Satz (überschreibt die Konfiguration)")
    r.add_argument("--cold", type=int, help="Kaltstarts je Satz (überschreibt die Konfiguration)")
    r.add_argument("--subset", choices=("smoke", "full"), help="Teilmenge (überschreibt die Konfiguration)")
    r.add_argument("--run-id")
    r.add_argument("--resume", action="store_true", help="vorhandenen Lauf fortsetzen, fertige Konfigurationen überspringen")
    r.add_argument("--dry-run", action="store_true", help="Plan ausgeben, nichts starten")
    r.add_argument("--seed", type=int, default=1, help="fake-Quelle: Zufallssaat")
    r.add_argument("--fake-error-rate", type=float, default=0.0, help="fake-Quelle: Anteil fehlgeschlagener Zeilen")
    r.add_argument("--min-free-ram-mb", type=int, default=4096, help="RAM-Start-Gate (App-Quelle)")
    r.add_argument("--sample-ms", type=int, default=200, help="Ressourcen-Sampling-Intervall")
    r.add_argument("--require-exclusive", action="store_true", help="GPU-Konfigurationen nur bei exklusiver GPU messen")
    r.add_argument("--strict", action="store_true", help="Exit-Code 1, wenn eine Messkonfiguration fehlschlägt")
    r.set_defaults(fn=cmd_run)

    s = sub.add_parser("asr", help="WER/CER für die WAVs eines Laufs (nach dem Lauf)")
    s.add_argument("--run", required=True, help="Lauf-ID oder -Ordner")
    s.add_argument("--engine", choices=("faster-whisper", "fake"), default="faster-whisper")
    s.add_argument("--model", default="auto", help="auto = large-v3-turbo, sonst lokal vorhandenes (siehe README)")
    s.add_argument("--device", default="auto", choices=("auto", "cpu", "cuda"))
    s.add_argument("--compute-type", default="auto")
    s.add_argument("--all-runs", action="store_true", help="alle WAVs transkribieren, nicht eine je Satz")
    s.add_argument("--allow-download", action="store_true", help="fehlendes ASR-Modell herunterladen")
    s.set_defaults(fn=cmd_asr)

    b = sub.add_parser("report", help="Aggregation -> results.json/csv, report.md")
    b.add_argument("--runs", nargs="+", required=True, help="Lauf-IDs oder -Ordner (mehrere werden je Konfiguration vereint)")
    b.add_argument("--out", help="Ausgabeordner (Standard: <lauf>/report)")
    b.add_argument("--publish", action="store_true", help="Zusammenfassung nach docs/tts/benchmarks/<datum>/ kopieren")
    b.add_argument("--publish-dir")
    b.add_argument("--date", help="Datum für --publish (Standard: heute)")
    b.set_defaults(fn=cmd_report)

    k = sub.add_parser("listening-kit", help="Blindhörtest-Paket erzeugen")
    k.add_argument("--run", nargs="+", required=True)
    k.add_argument("--configs", nargs="+")
    k.add_argument("--subset", choices=("smoke", "full"), default="smoke")
    k.add_argument("--max-per-config", type=int)
    k.add_argument("--seed", type=int, default=20261026, help="Mischung/IDs; wird im Schlüssel festgehalten")
    k.add_argument("--out", help="Proben-Ordner (Standard: <lauf>/listening-kit/<kit>)")
    k.add_argument("--key-out", help="Schlüssel-Ordner, getrennt von den Proben (Standard: <lauf>/listening-key/<kit>)")
    k.add_argument("--kit-id")
    k.add_argument("--no-loudness-match", action="store_true", help="Lautstärke nicht angleichen")
    k.set_defaults(fn=cmd_listening_kit)

    e = sub.add_parser("listening-eval", help="ausgefüllte Bewertungsbögen mit dem Schlüssel zusammenführen")
    e.add_argument("--key", required=True)
    e.add_argument("--sheets", nargs="+", required=True)
    e.add_argument("--out", help="Ausgabeordner (Standard: neben dem Schlüssel, Unterordner auswertung)")
    e.set_defaults(fn=cmd_listening_eval)
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    return args.fn(args)


if __name__ == "__main__":
    sys.exit(main())
