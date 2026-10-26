"""Befehl ``asr``: WER/CER nachtraeglich fuer die WAVs eines Laufs (faster-whisper oder ``fake``).

Laeuft NACH dem Syntheselauf und nie parallel zu Messungen (gemeinsame Sperre mit ``run``).
WER/CER sind ein Verständlichkeits-Proxy, kein Natürlichkeitsmaß.
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import math
import tempfile
from pathlib import Path

import benchlib  # Pfad wird in ttsbench/__init__ gesetzt
from . import corpus as corpus_mod, hostinfo, rowschema, runner, textmetrics
from . import normalize as nz

PROXY_NOTE = ("WER/CER sind ein Verständlichkeits-Proxy (ASR liest die Synthese zurück), "
              "kein Natürlichkeitsmaß; ASR-Fehler fließen in die Werte ein.")
AUTO_MODELS = ("large-v3-turbo", "large-v3", "medium", "small")
# Whisper large-v3-turbo Q8 aus dem lokalen App-Modellordner (wie scripts/bench/sentence_bench.py, "whisper-turbo-q8")
APP_CLI_MODEL = "handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf"
APP_CLI_RATE = 16_000  # die App-CLI verlangt 16 kHz mono 16-bit PCM und resampelt nicht selbst (lib.rs, --transcribe-file)


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


def to_16k_mono_pcm16(src: Path, dst: Path) -> bool:
    """Schreibt ``src`` als 16 kHz / mono / 16-bit PCM nach ``dst``; ``False`` = Datei war schon so (nichts geschrieben).

    Die App-CLI lehnt jede andere Form ab (Exit 2) und resampelt nicht selbst; TTS-Ausgaben haben meist 22,05/24/44,1 kHz.
    Mehrkanal wird gemittelt, Resampling per Polyphasenfilter (scipy) mit Tiefpass gegen Aliasing.
    """
    import numpy as np
    import soundfile as sf
    info = sf.info(str(src))
    if info.samplerate == APP_CLI_RATE and info.channels == 1 and info.subtype == "PCM_16":
        return False
    data, rate = sf.read(str(src), dtype="float32", always_2d=True)
    mono = data.mean(axis=1)
    if rate != APP_CLI_RATE:
        try:
            from scipy.signal import resample_poly
        except ImportError as e:
            raise AsrError(f"{src.name}: {rate} Hz muss auf 16 kHz resampelt werden, scipy fehlt (pip install scipy)") from e
        g = math.gcd(int(rate), APP_CLI_RATE)
        mono = resample_poly(mono, APP_CLI_RATE // g, int(rate) // g).astype("float32")
    sf.write(str(dst), np.clip(mono, -1.0, 1.0), APP_CLI_RATE, subtype="PCM_16")
    return True


class AppCliEngine:
    """Lokal installiertes Whisper large-v3-turbo Q8 über das App-Binary (``-f <wav> --model <id> --json --out``).

    Kein Download (das Modell muss in der App installiert sein), je WAV ein Prozess im Job-Objekt (RAM-Deckel,
    BelowNormal, nie parallel). Sprache und Nachbearbeitung (z. B. eigene Wörter) kommen aus den App-Einstellungen,
    die CLI hat keine Sprachoption.
    """
    name = "app-cli"

    def __init__(self, exe: Path | str | None = None, model: str = "auto", timeout_s: float = 600.0,
                 memory_limit_mb: int = 8192, max_processes: int = 4, min_free_ram_mb: int = 4096):
        self.exe = Path(exe) if exe else runner.default_exe()
        if not self.exe.is_file():
            raise AsrError(f"App-Binary nicht gefunden: {self.exe} (--exe oder LVA_BENCH_CLI)")
        self.model_name = APP_CLI_MODEL if model in (None, "", "auto") else model
        self.device = "unbekannt (bound_backend noch nicht gemeldet)"
        self.compute_type = "gguf"
        self.timeout_s = timeout_s
        self.backends: set[str] = set()
        self.n_resampled = 0
        self.n_calls = 0
        benchlib.ram_gate(min_free_ram_mb)  # Start-Gate vor dem ersten Modellstart
        self.runner = benchlib.LimitedRunner(memory_limit_mb=memory_limit_mb, max_processes=max_processes)

    def _app_version(self) -> str | None:
        # nur aus einem Repo-Build (src-tauri/target/<profil>/...) lesbar; der CLI-Vertrag kennt keine Versionsausgabe
        for parent in self.exe.resolve().parents:
            conf = parent / "tauri.conf.json"
            if conf.is_file():
                try:
                    return json.loads(conf.read_text(encoding="utf-8")).get("version")
                except (OSError, ValueError):
                    return None
        return None

    def info(self) -> dict:
        st = self.exe.stat()
        backend = sorted(self.backends)
        return {"engine": self.name, "model": self.model_name, "exe": str(self.exe), "exe_size_bytes": st.st_size,
                "exe_mtime": dt.datetime.fromtimestamp(st.st_mtime, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                "app_version": self._app_version(), "device": self.device, "compute_type": self.compute_type,
                "bound_backend": backend[0] if len(backend) == 1 else (backend or None),
                "language": "App-Einstellung (CLI ohne Sprachoption)",
                "input": "16 kHz mono PCM16 (vom Harness resampelt)",
                "resampled_files": self.n_resampled, "files": self.n_calls}

    def transcribe(self, wav: Path, language: str, ref: str | None = None) -> str:
        with tempfile.TemporaryDirectory(prefix="ttsbench_asr_") as td:
            td_p = Path(td)
            conv = td_p / "in16k.wav"
            src = Path(wav)
            if to_16k_mono_pcm16(src, conv):
                src = conv
                self.n_resampled += 1
            out = td_p / "r.json"
            rc, _, err = self.runner.run([str(self.exe), "-f", str(src), "--model", self.model_name, "--json",
                                          "--out", str(out)], timeout=self.timeout_s)
            if rc != 0 or not out.exists():
                raise AsrError(f"App-CLI fehlgeschlagen für {Path(wav).name}: rc={rc} {err.strip()[-300:]}")
            d = json.loads(out.read_text(encoding="utf-8"))
        self.n_calls += 1
        backend = d.get("bound_backend")
        if backend:
            self.backends.add(str(backend))
            self.device = str(backend)
        return str(d.get("text", "")).strip()


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
    exe = getattr(args, "exe", None)
    engine = args.engine
    if engine == "auto":  # lokal installierte App-Engine bevorzugen, sonst faster-whisper
        engine = "app-cli" if (Path(exe) if exe else runner.default_exe()).is_file() else "faster-whisper"
        print(f"HINWEIS: --engine auto -> {engine}")
    if engine == "app-cli":
        return AppCliEngine(exe, args.model, min_free_ram_mb=getattr(args, "min_free_ram_mb", 4096))
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
        result["asr"] = engine.info()  # nach dem Lauf: enthält z. B. das gemeldete Backend (bound_backend)
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
