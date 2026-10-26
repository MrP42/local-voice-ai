"""Befehl ``asr``: WER/CER nachtraeglich fuer die WAVs eines Laufs (faster-whisper oder ``fake``).

Laeuft NACH dem Syntheselauf und nie parallel zu Messungen (gemeinsame Sperre mit ``run``).
WER/CER sind ein Verständlichkeits-Proxy, kein Natürlichkeitsmaß.
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

from . import corpus as corpus_mod, hostinfo, rowschema, runner, textmetrics
from . import normalize as nz

PROXY_NOTE = ("WER/CER sind ein Verständlichkeits-Proxy (ASR liest die Synthese zurück), "
              "kein Natürlichkeitsmaß; ASR-Fehler fließen in die Werte ein.")
AUTO_MODELS = ("large-v3-turbo", "large-v3", "medium", "small")


class AsrError(RuntimeError):
    pass


class FasterWhisperEngine:
    name = "faster-whisper"

    def __init__(self, model: str = "auto", device: str = "auto", compute_type: str = "auto",
                 allow_download: bool = False):
        try:
            import faster_whisper
            from faster_whisper.utils import download_model
        except ImportError as e:
            raise AsrError("faster-whisper nicht installiert (pip install faster-whisper)") from e
        self.version = getattr(faster_whisper, "__version__", None)
        if device == "auto":
            try:
                import ctranslate2
                device = "cuda" if ctranslate2.get_cuda_device_count() > 0 else "cpu"
            except Exception:  # noqa: BLE001
                device = "cpu"
        if compute_type == "auto":
            compute_type = "float16" if device == "cuda" else "int8"
        local_only = not allow_download
        names = AUTO_MODELS if model == "auto" else (model,)
        chosen = None
        tried = []
        for n in names:
            try:
                download_model(n, local_files_only=local_only)
                chosen = n
                break
            except Exception as e:  # noqa: BLE001
                tried.append(f"{n}: {type(e).__name__}")
        if chosen is None:
            raise AsrError(f"ASR-Modell nicht lokal vorhanden ({', '.join(tried)}); --allow-download erlaubt den Abruf")
        if model == "auto" and chosen != AUTO_MODELS[0]:
            print(f"HINWEIS: ASR-Modell {AUTO_MODELS[0]} nicht lokal, verwende {chosen} "
                  "(Ergebnis nennt das Modell; für Vergleichbarkeit gleiches Modell je Lauf)")
        self.model_name, self.device, self.compute_type = chosen, device, compute_type
        self.model = faster_whisper.WhisperModel(chosen, device=device, compute_type=compute_type,
                                                 local_files_only=local_only)

    def info(self) -> dict:
        return {"engine": self.name, "version": self.version, "model": self.model_name,
                "device": self.device, "compute_type": self.compute_type, "beam_size": 5, "temperature": 0.0}

    def transcribe(self, wav: Path, language: str, ref: str | None = None) -> str:
        segs, _ = self.model.transcribe(str(wav), language=language, beam_size=5, temperature=0.0,
                                        condition_on_previous_text=False, vad_filter=False)
        return " ".join(s.text.strip() for s in segs).strip()


class FakeAsrEngine:
    """Deterministisch gestoerte Referenz (fuer Tests ohne Modell)."""
    name = "fake"

    def __init__(self, **_):
        self.model_name, self.device, self.compute_type = "fake-asr", "cpu", "none"

    def info(self) -> dict:
        return {"engine": "fake", "version": "1.0", "model": "fake-asr", "device": "cpu", "compute_type": "none"}

    def transcribe(self, wav: Path, language: str, ref: str | None = None) -> str:
        words = (ref or "").split()
        h = int(hashlib.sha256(str(wav.name).encode()).hexdigest()[:8], 16)
        if words and h % 4 == 0:
            words[h % len(words)] = "xyz"
        if len(words) > 3 and h % 7 == 0:
            words.pop()
        return " ".join(words)


def make_engine(args):
    if args.engine == "fake":
        return FakeAsrEngine()
    return FasterWhisperEngine(args.model, args.device, args.compute_type, args.allow_download)


def pick_wavs(rows: list[dict], cdir: Path, all_runs: bool) -> list[tuple[dict, Path]]:
    cand = []
    for r in rows:
        if not r["ok"] or not r.get("wav_path"):
            continue
        p = Path(r["wav_path"])
        p = p if p.is_absolute() else cdir / p
        if p.exists():
            cand.append((r, p))
    if all_runs:
        return cand
    chosen: dict[str, tuple[dict, Path]] = {}
    for r, p in sorted(cand, key=lambda x: (x[0]["run_kind"] != "warm", x[0]["run_idx"])):
        chosen.setdefault(r["utterance_id"], (r, p))
    return list(chosen.values())


def run_asr(args) -> int:
    run_dir = runner.resolve_run(args.run)
    manifest = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
    runner.acquire_lock(f"asr:{manifest['run_id']}")
    try:
        engine = make_engine(args)
        result = {"schema": "tts-bench-asr/1", "created_at": hostinfo.now_iso(), "run_id": manifest["run_id"],
                  "asr": engine.info(), "norm_version": nz.NORM_VERSION, "note": PROXY_NOTE,
                  "configs": {}, "skipped": {}}
        for entry in manifest["configs"]:
            cid = entry["id"]
            if entry["state"] != "measured":
                continue
            if not entry.get("quality", True):
                result["skipped"][cid] = "quality=false: kein WER/CER für diese Konfiguration (Funktionsnachweis)"
                continue
            cdir = run_dir / cid
            rows, _ = rowschema.read_rows(cdir / "rows.jsonl")
            wavs = pick_wavs(rows, cdir, args.all_runs)
            if not wavs:
                result["skipped"][cid] = "keine WAV-Dateien im Lauf"
                continue
            corpus = corpus_mod.by_id(corpus_mod.load(str(run_dir / manifest["corpora"][entry["corpus"]]["snapshot"])))
            print(f"[{cid}] transkribiere {len(wavs)} WAVs mit {engine.info()['model']} ...")
            items = []
            for r, p in wavs:
                ref = corpus[r["utterance_id"]]
                hyp = engine.transcribe(p, entry["language"], ref["expected_normalized"])
                sc = textmetrics.score(ref["expected_normalized"], hyp)
                items.append({"utterance_id": r["utterance_id"], "category": ref["category"],
                              "run_kind": r["run_kind"], "run_idx": r["run_idx"], "wav": str(Path(p).name),
                              "hyp": hyp, "hyp_normalized": nz.normalize(hyp),
                              "ref_normalized": ref["expected_normalized"], **sc})
            result["configs"][cid] = {"language": entry["language"], "items": items, **summarize_items(items)}
        out = run_dir / "asr.json"
        out.write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
        for cid, c in result["configs"].items():
            print(f"[{cid}] WER {100 * c['summary']['wer']:.2f} %  CER {100 * c['summary']['cer']:.2f} %  (n={c['summary']['n']})")
        print(f"geschrieben: {out}")
        return 0
    finally:
        runner.release_lock()


def summarize_items(items: list[dict]) -> dict:
    by_cat: dict[str, list[dict]] = {}
    for it in items:
        by_cat.setdefault(it["category"], []).append(it)
    return {"summary": textmetrics.aggregate(items),
            "by_category": {c: textmetrics.aggregate(v) for c, v in sorted(by_cat.items())}}
