#!/usr/bin/env python
"""Satz-Benchmark: WER je STT-Modell auf FLEURS-de mit der vorhandenen App-CLI.

    python scripts/bench/sentence_bench.py --models parakeet-onnx whisper-turbo-q8 qwen3-0.6b
    python scripts/bench/sentence_bench.py --models parakeet-gguf-cpu --limit 20     # Rauchtest
    python scripts/bench/sentence_bench.py --report                                  # nur Tabelle neu bauen
    python scripts/bench/sentence_bench.py --selftest                                # WER-Selbsttest

Je Satz ein Aufruf der Release-CLI (``-f <wav> --model <id> --reference <text> --json --out``);
Modelle mit Praefix ``tcbench`` laufen ueber das CUDA-Spike-Programm (Spike-Build, NICHT die App).
Normierung wie ``selftest::normalize_word``. WER = Summe Fehler / Summe Referenzwoerter,
RTF (x Echtzeit) = Summe Audio / Summe Transkriptionszeit, Ladezeit getrennt.

Systemschutz: immer nur EIN Prozess, BelowNormal, Job-Objekt mit Speicherdeckel, RAM-Start-Gate.
Ergebnisse (Zeilen + Aggregat) unter ``%LOCALAPPDATA%\\lva-bench\\results\\``; wiederaufnehmbar.
"""
from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import statistics
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchlib  # noqa: E402

CLI_EXE = Path(os.environ.get(
    "LVA_BENCH_CLI",
    r"C:\Users\wolff\local-voice-project\apps\local-voice\src-tauri\target\release\local-voice-ai.exe"))
TCBENCH_EXE = Path(os.environ.get("LVA_BENCH_TCBENCH", r"C:\Users\wolff\lva-spikes\m2\tcbench\target\release\tcbench.exe"))
CUDA_BIN = r"C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.1\bin"
SPIKE_MODELS = Path(r"C:\Users\wolff\lva-spikes\m2\models")
APP_MODELS = Path(os.environ.get("APPDATA", "")) / "de.wolffappliedai.localvoiceai" / "models"
DOC_BEGIN, DOC_END = "<!-- bench:table:begin -->", "<!-- bench:table:end -->"
DEFAULT_DOC = Path(__file__).resolve().parents[2] / "docs" / "m2-evidence" / "bench.md"

# alias -> Beschreibung. kind "cli" = App-CLI (--model <id>), kind "tcbench" = Spike-Programm.
MODELS: dict[str, dict] = {
    "parakeet-onnx": dict(kind="cli", id="parakeet-tdt-0.6b-v3", label="Parakeet TDT 0.6B v3 int8",
                          engine="transcribe-rs (ONNX)", device="CPU"),
    "whisper-turbo-q8": dict(kind="cli", id="handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf",
                             label="Whisper large-v3-turbo Q8", engine="transcribe-cpp", device="CPU"),
    "qwen3-0.6b": dict(kind="cli", id="handy-computer/Qwen3-ASR-0.6B-gguf/Qwen3-ASR-0.6B-Q8_0.gguf",
                       label="Qwen3-ASR 0.6B Q8", engine="transcribe-cpp", device="CPU"),
    "qwen3-1.7b": dict(kind="cli", id="handy-computer/Qwen3-ASR-1.7B-gguf/Qwen3-ASR-1.7B-Q5_K_M.gguf",
                       label="Qwen3-ASR 1.7B Q5_K_M", engine="transcribe-cpp", device="CPU"),
    "parakeet-gguf-cpu": dict(kind="tcbench", path=SPIKE_MODELS / "parakeet-tdt-0.6b-v3-Q8_0.gguf", cuda=False,
                              label="Parakeet TDT 0.6B v3 GGUF Q8", engine="transcribe-cpp (Spike-Build)", device="CPU",
                              note="Spike-Build, nicht App (Modell nicht im App-Katalog installiert)"),
    "parakeet-gguf-cuda": dict(kind="tcbench", path=SPIKE_MODELS / "parakeet-tdt-0.6b-v3-Q8_0.gguf", cuda=True,
                               label="Parakeet TDT 0.6B v3 GGUF Q8", engine="transcribe-cpp (Spike-Build)",
                               device="CUDA RTX 4090", note="Spike-Build, nicht App"),
    "whisper-turbo-cuda": dict(kind="tcbench", path=APP_MODELS / "whisper-large-v3-turbo-Q8_0.gguf", cuda=True,
                               label="Whisper large-v3-turbo Q8", engine="transcribe-cpp (Spike-Build)",
                               device="CUDA RTX 4090", note="Spike-Build, nicht App"),
    "whisper-large-v3-q5-cuda": dict(kind="tcbench", path=SPIKE_MODELS / "whisper-large-v3-Q5_K_M.gguf", cuda=True,
                                     label="Whisper large-v3 Q5_K_M", engine="transcribe-cpp (Spike-Build)",
                                     device="CUDA RTX 4090", note="Spike-Build, nicht App"),
}


def resolve_model(spec: str) -> tuple[str, dict]:
    if spec in MODELS:
        return spec, MODELS[spec]
    if spec.startswith("cli:"):  # beliebige App-Modell-ID
        mid = spec[4:]
        return mid.replace("/", "_"), dict(kind="cli", id=mid, label=mid.split("/")[-1], engine="transcribe-cpp", device="CPU")
    raise SystemExit(f"unbekanntes Modell '{spec}' (Aliase: {', '.join(MODELS)}; oder cli:<Modell-ID>)")


# ---------------------------------------------------------------------------- Lauf
def load_corpus(limit: int | None) -> list[dict]:
    root = benchlib.bench_dir() / "fleurs"
    mp = root / "manifest.json"
    if not mp.exists():
        raise SystemExit(f"{mp} fehlt - zuerst: python scripts/bench/make_corpus.py fleurs")
    sents = json.loads(mp.read_text(encoding="utf-8"))["sentences"]
    for s in sents:
        s["path"] = str(root / s["file"])
    return sents[:limit] if limit else sents


def score_row(ref: str, hyp: str) -> dict:
    e, n = benchlib.word_errors(ref, hyp)
    el, _ = benchlib.word_errors(ref, hyp, lenient=True)
    return {"errors": e, "ref_words": n, "errors_lenient": el}


def run_cli_sentence(runner: benchlib.LimitedRunner, model_id: str, s: dict) -> dict:
    with tempfile.TemporaryDirectory(prefix="lvabench_") as td:
        out = Path(td) / "r.json"
        rc, _, err = runner.run(
            [str(CLI_EXE), "-f", s["path"], "--model", model_id, "--reference", s["text"], "--json", "--out", str(out)],
            timeout=600)
        if rc != 0 or not out.exists():
            return {"error": f"rc={rc} {err.strip()[-300:]}"}
        d = json.loads(out.read_text(encoding="utf-8"))
    row = {"hyp": d.get("text", ""), "audio_s": d.get("audio_secs"), "load_ms": d.get("load_ms"),
           "transcribe_ms": d.get("best_ms"), "backend": d.get("bound_backend")}
    sc = d.get("score") or {}
    if sc:  # Kreuzpruefung gegen die Rust-Wertung
        row["cli_errors"] = sc.get("substitutions", 0) + sc.get("insertions", 0) + sc.get("deletions", 0)
    return row


def run_tcbench_batch(runner: benchlib.LimitedRunner, spec: dict, batch: list[dict]) -> list[dict]:
    env = dict(os.environ)
    env["PATH"] = CUDA_BIN + os.pathsep + env.get("PATH", "")
    if not spec["cuda"]:
        env["CUDA_VISIBLE_DEVICES"] = "-1"
    rc, out, err = runner.run([str(TCBENCH_EXE), str(spec["path"]), "1"] + [s["path"] for s in batch],
                              env=env, timeout=1800)
    load_ms = None
    for tok in err.replace("\n", " ").split():
        if tok.startswith("load_ms="):
            load_ms = int(tok.split("=")[1])
    backend = next((t.split("=")[1] for t in err.replace("\n", " ").split() if t.startswith("backend=")), None)
    lines = [json.loads(l) for l in out.splitlines() if l.startswith("{")]
    if rc != 0 or len(lines) != len(batch):
        return [{"error": f"rc={rc}, {len(lines)}/{len(batch)} Zeilen: {err.strip()[-300:]}"} for _ in batch]
    return [{"hyp": d["text"], "audio_s": d["audio_secs"], "load_ms": load_ms if i == 0 else None,
             "transcribe_ms": d["best_ms"], "backend": backend} for i, d in enumerate(lines)]


def bench_model(alias: str, spec: dict, sents: list[dict], results: Path, fresh: bool) -> Path:
    rows_path = results / f"{alias}.rows.jsonl"
    if fresh and rows_path.exists():
        rows_path.unlink()
    done = {}
    if rows_path.exists():
        for l in rows_path.read_text(encoding="utf-8").splitlines():
            r = json.loads(l)
            done[r["sent_id"]] = r
    todo = [s for s in sents if s["sent_id"] not in done]
    print(f"== {alias}: {len(sents)} Saetze, {len(done)} schon vorhanden, {len(todo)} offen", flush=True)
    runner = benchlib.LimitedRunner(memory_limit_mb=16384 if spec["kind"] == "tcbench" else 8192, max_processes=4)
    if spec["kind"] == "tcbench" and not TCBENCH_EXE.exists():
        raise SystemExit(f"{TCBENCH_EXE} fehlt")
    if spec["kind"] == "cli" and not CLI_EXE.exists():
        raise SystemExit(f"{CLI_EXE} fehlt")
    t0 = time.time()

    def store(s: dict, r: dict) -> None:
        row = {"sent_id": s["sent_id"], "file": s["file"], "ref": s["text"], **r}
        if "hyp" in r:
            row.update(score_row(s["text"], r["hyp"]))
        with rows_path.open("a", encoding="utf-8") as f:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")

    if spec["kind"] == "cli":
        for i, s in enumerate(todo, 1):
            benchlib.ram_gate(3072)
            store(s, run_cli_sentence(runner, spec["id"], s))
            if i % 10 == 0 or i == len(todo):
                print(f"  {i}/{len(todo)} ({time.time() - t0:.0f} s)", flush=True)
    else:
        for i in range(0, len(todo), 40):
            benchlib.ram_gate(3072)
            batch = todo[i:i + 40]
            for s, r in zip(batch, run_tcbench_batch(runner, spec, batch)):
                store(s, r)
            print(f"  {min(i + 40, len(todo))}/{len(todo)} ({time.time() - t0:.0f} s)", flush=True)
    return finalize(alias, spec, rows_path, results)


# ---------------------------------------------------------------------------- Aggregat
def aggregate(rows: list[dict]) -> dict:
    ok = [r for r in rows if "hyp" in r]
    fail = [r for r in rows if "hyp" not in r]
    ref_words = [len(benchlib.words(r["ref"])) for r in rows]
    # Ausfaelle zaehlen ehrlich als komplett verfehlt (alle Referenzwoerter geloescht)
    pairs = [(r["errors"], r["ref_words"]) for r in ok] + [(len(benchlib.words(r["ref"])), len(benchlib.words(r["ref"]))) for r in fail]
    pairs_l = [(r["errors_lenient"], r["ref_words"]) for r in ok] + [
        (len(benchlib.words(r["ref"], True)), len(benchlib.words(r["ref"], True))) for r in fail]
    audio = sum(r["audio_s"] or 0 for r in ok)
    tsec = sum((r["transcribe_ms"] or 0) for r in ok) / 1000
    loads = [r["load_ms"] for r in ok if r.get("load_ms")]
    mism = sum(1 for r in ok if "cli_errors" in r and r["cli_errors"] != r["errors"])
    return {
        "sentences": len(rows), "failed": len(fail), "ref_words": sum(ref_words),
        "errors": sum(e for e, _ in pairs), "wer": benchlib.aggregate_wer(pairs),
        "wer_lenient": benchlib.aggregate_wer(pairs_l),
        "wer_ok_only": benchlib.aggregate_wer([(r["errors"], r["ref_words"]) for r in ok]) if ok else None,
        "audio_s": audio, "transcribe_s": tsec, "rtf": (audio / tsec) if tsec else None,
        "load_ms_median": statistics.median(loads) if loads else None,
        "load_ms_p95": (sorted(loads)[int(0.95 * (len(loads) - 1))] if loads else None),
        "cli_mismatch_sentences": mism,
        "backends": sorted({r.get("backend") for r in ok if r.get("backend")}),
    }


def finalize(alias: str, spec: dict, rows_path: Path, results: Path) -> Path:
    rows = [json.loads(l) for l in rows_path.read_text(encoding="utf-8").splitlines()]
    agg = aggregate(rows)
    doc = {"alias": alias, "spec": {k: str(v) for k, v in spec.items()}, "finished": dt.datetime.now().isoformat(timespec="seconds"),
           "cli_mtime": dt.datetime.fromtimestamp(CLI_EXE.stat().st_mtime).isoformat(timespec="seconds") if CLI_EXE.exists() else None,
           "aggregate": agg, "rows": rows}
    p = results / f"{alias}.json"
    p.write_text(json.dumps(doc, ensure_ascii=False, indent=1), encoding="utf-8")
    print(f"   {alias}: WER {agg['wer'] * 100:.2f} % ({agg['errors']}/{agg['ref_words']}), RTF {agg['rtf'] or 0:.1f}x, "
          f"Laden {agg['load_ms_median']} ms, Ausfaelle {agg['failed']}, -> {p}", flush=True)
    return p


# ---------------------------------------------------------------------------- Bericht
def _fmt(x, d=1):
    return "-" if x is None else f"{x:.{d}f}".replace(".", ",")


def build_table(results: Path) -> str:
    docs = []
    for p in sorted(results.glob("*.json")):
        try:
            docs.append(json.loads(p.read_text(encoding="utf-8")))
        except ValueError:
            continue
    order = {a: i for i, a in enumerate(MODELS)}
    docs.sort(key=lambda d: order.get(d["alias"], 99))
    lines = ["| Modell | Engine | Ger\u00e4t | S\u00e4tze | WER FLEURS-de | RTF | Hinweise |", "|---|---|---|---|---|---|---|"]
    for d in docs:
        a, sp = d["aggregate"], d["spec"]
        notes = []
        if sp.get("note"):
            notes.append(sp["note"])
        notes.append(f"WER weich {_fmt(a['wer_lenient'] * 100, 2)} %")
        if a["load_ms_median"]:
            notes.append(f"Laden {_fmt(a['load_ms_median'] / 1000, 1)} s")
        if a["failed"]:
            notes.append(f"{a['failed']} Ausf\u00e4lle (als komplett falsch gez\u00e4hlt)")
        if a["cli_mismatch_sentences"]:
            notes.append(f"{a['cli_mismatch_sentences']} S\u00e4tze weichen von der Rust-Wertung ab")
        lines.append(
            f"| {sp.get('label', d['alias'])} | {sp.get('engine', '')} | {sp.get('device', '')} | {a['sentences']} "
            f"| **{_fmt(a['wer'] * 100, 2)} %** ({a['errors']}/{a['ref_words']}) | {_fmt(a['rtf'], 1)}\u00d7 | {'; '.join(notes)} |")
    return "\n".join(lines)


def update_doc(doc_path: Path, table: str) -> bool:
    if not doc_path.exists():
        return False
    text = doc_path.read_text(encoding="utf-8")
    if DOC_BEGIN not in text or DOC_END not in text:
        return False
    head, rest = text.split(DOC_BEGIN, 1)
    _, tail = rest.split(DOC_END, 1)
    stamp = dt.date.today().isoformat()
    doc_path.write_text(f"{head}{DOC_BEGIN}\n_Stand {stamp}, automatisch aus den Ergebnis-JSON erzeugt._\n\n{table}\n{DOC_END}{tail}",
                        encoding="utf-8", newline="")
    return True


# ---------------------------------------------------------------------------- Selbsttest
def selftest() -> int:
    ok = True

    def check(name: str, got, want):
        nonlocal ok
        good = got == want
        ok &= good
        print(f"[{'ok' if good else 'FAIL'}] {name}: {got!r}" + ("" if good else f" (erwartet {want!r})"))

    check("gleicher Satz", benchlib.word_errors("Das ist ein Test.", "das ist ein test"), (0, 4))
    check("eine Substitution", benchlib.word_errors("das ist ein test", "das ist kein test"), (1, 4))
    check("Umlaute/ss gefaltet", benchlib.word_errors("Die Gr\u00f6\u00dfe f\u00fcr \u00c4rzte", "die groesse fuer aerzte"), (0, 4))
    check("Zahl vs. Wort zaehlt als Fehler", benchlib.word_errors("am dritten November", "am 3. November"), (1, 3))
    check("Einfuegung + Loeschung", benchlib.word_errors("a b c d", "a x b d e"), (3, 4))
    check("Bindestrich streng", benchlib.word_errors("wolfs- oder hunderudel", "wolfs oder hunderudel"), (1, 3))
    check("Bindestrich weich", benchlib.word_errors("wolfs- oder hunderudel", "wolfs oder hunderudel", lenient=True), (0, 3))
    # Korpus-WER: Summe/Summe, nicht Mittel der Satz-WER: (1+0)/(4+6) = 0,1 statt (0,25+0)/2 = 0,125
    check("Korpus-WER aggregiert", benchlib.aggregate_wer([(1, 4), (0, 6)]), 0.1)
    rows = [
        {"ref": "das ist ein test", "hyp": "das ist kein test", "errors": 1, "ref_words": 4, "errors_lenient": 1,
         "audio_s": 2.0, "transcribe_ms": 500, "load_ms": 1000},
        {"ref": "noch ein satz mit sechs", "hyp": "noch ein satz mit sechs", "errors": 0, "ref_words": 5, "errors_lenient": 0,
         "audio_s": 3.0, "transcribe_ms": 500, "load_ms": 2000},
        {"ref": "ausfall satz", "error": "x"},
    ]
    a = aggregate(rows)
    check("Aggregat Fehler (Ausfall = alle Woerter falsch)", (a["errors"], a["ref_words"]), (3, 11))
    check("Aggregat RTF = Summe Audio / Summe Zeit", a["rtf"], 5.0)
    check("Ladezeit getrennt (Median)", a["load_ms_median"], 1500)
    print("SELFTEST OK" if ok else "SELFTEST FEHLGESCHLAGEN")
    return 0 if ok else 1


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--models", nargs="+", help=f"Aliase ({', '.join(MODELS)}) oder cli:<Modell-ID>")
    ap.add_argument("--limit", type=int, help="nur die ersten N Saetze (Rauchtest)")
    ap.add_argument("--fresh", action="store_true", help="vorhandene Zeilen des Modells verwerfen")
    ap.add_argument("--doc", type=Path, default=DEFAULT_DOC, help="Markdown-Datei mit Tabellen-Markern")
    ap.add_argument("--report", action="store_true", help="nur Tabelle aus vorhandenen Ergebnissen erzeugen")
    ap.add_argument("--selftest", action="store_true", help="WER-Aggregation mit bekanntem Ergebnis pruefen")
    args = ap.parse_args(argv)
    if args.selftest:
        return selftest()
    results = benchlib.bench_dir() / "results"
    results.mkdir(parents=True, exist_ok=True)
    if not args.report:
        if not args.models:
            ap.error("--models oder --report angeben")
        sents = load_corpus(args.limit)
        for spec in args.models:  # strikt nacheinander: nie zwei Benchmark-Prozesse gleichzeitig
            alias, ms = resolve_model(spec)
            if args.limit:  # Rauchtest ueberschreibt nicht das Voll-Ergebnis
                results_dir = results / "smoke"
                results_dir.mkdir(exist_ok=True)
            else:
                results_dir = results
            bench_model(alias, ms, sents, results_dir, args.fresh)
    table = build_table(results / "smoke" if args.limit else results)
    (results / "bench_table.md").write_text(table + "\n", encoding="utf-8")
    print("\n" + table)
    if not args.limit or args.report:
        print(f"\n{'Doku aktualisiert: ' + str(args.doc) if update_doc(args.doc, table) else 'Doku nicht aktualisiert (Datei/Marker fehlen)'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
