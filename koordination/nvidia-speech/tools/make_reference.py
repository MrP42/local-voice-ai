#!/usr/bin/env python3
"""Referenzwerte fuer die Nemotron-3-Diarization-Engine (Paket N1).

Erzeugt die kleinen Goldwerte, gegen die die Rust-Tests in
`apps/local-voice/src-tauri/src/managers/meetings/diarize/nemotron/` pruefen:

  mel_clip.i16, mel_ref.f32   1,2 s Audio (16-bit PCM) und die Mel-Merkmale des
                              Merkmalsextraktors der Referenz (Offline, center=True)
  cache_ref.json              Zustaende des Sprecher-Zwischenspeichers
                              (`Nemotron3DiarizationSpeakerCache`) ueber mehrere Bloecke
                              samt Ueberlauf, dazu `_get_frame_scores` einzeln
  pipeline_ref.json           rohe Turns der ganzen Pipeline fuer zwei 73 s bzw. 93 s lange
                              Pruefaudios, aus den Fixture-WAVs nach einer Vorschrift
                              (`recipe`) zusammengesetzt: hintereinander, ueberlagert, umgekehrt

Die Modellrechnung nutzt DASSELBE int8-ONNX wie die Rust-Engine
(`onnxruntime`), damit Rust und Python eng vergleichbar sind; Merkmalsextraktor,
Processor und Zwischenspeicher sind die Klassen aus `transformers` (Apache-2.0).
Die Blockschleife ist die des Offline-Modus von
`Nemotron3DiarizationForAudioFrameClassification.forward`, nur mit der
ONNX-Sitzung statt des PyTorch-Encoders.

Aufruf (venv nur im Scratchpad, nicht im Repo):

    uv venv venv --python 3.13
    uv pip install --python venv/bin/python onnx onnxruntime numpy soundfile librosa
    uv pip install --python venv/bin/python torch --index-url https://download.pytorch.org/whl/cpu
    uv pip install --python venv/bin/python transformers==5.22.2
    venv/bin/python -I make_reference.py \\
        --model-dir <Ordner mit onnx/model_quantized.onnx(+_data), config.json,
                     preprocessor_config.json, processor_config.json> \\
        --fixtures apps/local-voice/src-tauri/tests/fixtures \\
        --out apps/local-voice/src-tauri/tests/fixtures/nemotron \\
        [--debug-dir <Ordner fuer die vollen Logits>]

Abweichung vom Auftrag "transformers git main": GitHub ist in der Umgebung dieses
Laufs nicht erreichbar; die Veroeffentlichung 5.22.2 (PyPI) enthaelt dieselben
Klassen (Zwischenspeicher, Modell, Konfiguration Byte fuer Byte gleich zu `main`
vom 12.11.2026; Processor und Extraktor weichen nur in Typangaben und darin ab,
dass 5.22.2 die Mel-Filter mit `librosa.filters.mel` statt `mel_filter_bank` baut,
beides Slaney-Filter in float32).
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import json
import math
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
import torch

SAMPLE_RATE = 16_000
LCG_NOTE = "state = (state * 1664525 + 1013904223) mod 2^32; Zug = state >> 8 (24 Bit)"


class Lcg:
    """32-bit-LCG, in Rust nachgebaut (`cache.rs::Lcg`)."""

    def __init__(self, seed: int):
        self.state = seed & 0xFFFFFFFF

    def draw(self) -> int:
        self.state = (self.state * 1664525 + 1013904223) & 0xFFFFFFFF
        return self.state >> 8


# --------------------------------------------------------------------------- Mel


def make_mel(processor, fixtures: Path, out: Path) -> dict:
    mic, _ = sf.read(fixtures / "m2_echo_mic.wav", dtype="int16")
    # 0,2 s digitale Stille (Logarithmus-Untergrenze), dann 1,0 s Aufnahme mit
    # Rauschteppich und Sprachbeginn.
    pcm = np.concatenate([np.zeros(3200, np.int16), mic[:16_000]])
    audio = pcm.astype(np.float32) / 32768.0
    inputs = processor(audio, sampling_rate=SAMPLE_RATE)
    feats = np.asarray(inputs["input_features"][0], dtype=np.float32)
    mask = np.asarray(inputs["attention_mask"][0]).astype(bool)
    valid = int(mask.sum())
    assert valid == len(audio) // 160, (valid, len(audio))
    assert feats.shape[0] == valid + 1 and not feats[valid:].any(), "letzte Zeile muss 0 sein"
    (out / "mel_clip.i16").write_bytes(pcm.astype("<i2").tobytes())
    (out / "mel_ref.f32").write_bytes(feats[:valid].astype("<f4").tobytes())
    return {"samples": len(pcm), "frames": valid, "bins": int(feats.shape[1])}


# ----------------------------------------------------------------------- Cache


def run_scenario(Cache, streaming_cfg, *, name, fifo, period, hidden, chunk, steps, seed, bias, probs_at):
    sub = streaming_cfg.subsampling_factor
    speakers = streaming_cfg.num_speakers
    cache = Cache(streaming_cfg, fifo_length=fifo, speaker_cache_update_period=period)
    rng = Lcg(seed)
    silence = torch.full((hidden,), -1.0)
    records = []
    for i, st in enumerate(steps):
        lookahead, lookback, mask_tail = st["lookahead"], st["lookback"], st["mask_tail"]
        base = i * chunk
        ids = list(range(base - lookback, base + chunk + lookahead))
        new = torch.tensor(
            [[float(v), float(v % 7)] + [0.0] * (hidden - 2) for v in ids], dtype=torch.float32
        )[None]
        cached = cache.get_embeds(new)
        input_embeds = torch.cat([cached, new], dim=1)
        frames = input_embeds.shape[1]
        mask = torch.ones(1, frames, dtype=torch.bool)
        if mask_tail:
            mask[0, -mask_tail:] = False
        rows = []
        for _ in range(frames * sub):
            rows.append([((rng.draw() % 385) - 192 + 16 * b) / 16.0 for b in bias])
        logits = torch.tensor(rows, dtype=torch.float32)[None]
        assert logits.shape == (1, frames * sub, speakers)
        cache.update(
            input_embeds, logits, silence, chunk, mask=mask, num_lookback_frames=lookback
        )
        nc, nf = cache.num_cache_frames, cache.num_fifo_frames
        rec = dict(st)
        rec.update(
            num_cache=nc,
            num_fifo=nf,
            compressed=bool(cache.is_compressed),
            cache_ids=[int(v) for v in cache.embeds[0, :nc, 0].tolist()],
            fifo_ids=[int(v) for v in cache.fifo[0, :nf, 0].tolist()],
        )
        if i in probs_at:
            rec["cache_probs"] = [round(float(v), 7) for v in cache.probs[0, :nc].flatten().tolist()]
        records.append(rec)
    return {
        "name": name,
        "config": {
            "num_speakers": speakers,
            "subsampling": sub,
            "fifo_length": fifo,
            "update_period": period,
            "cache_length": streaming_cfg.speaker_cache_length,
            "silence_frames": streaming_cfg.speaker_cache_silence_frames_per_speaker,
            "score_threshold": streaming_cfg.prediction_score_threshold,
            "latest_boost": streaming_cfg.latest_frames_score_boost,
            "strong_boost_rate": streaming_cfg.strong_boost_rate,
            "weak_boost_rate": streaming_cfg.weak_boost_rate,
            "min_positive_rate": streaming_cfg.min_positive_scores_rate,
        },
        "hidden": hidden,
        "seed": seed,
        "bias": bias,
        "chunk": chunk,
        "steps": records,
    }


def _stable_topk(x, k, dim=-1, largest=True, sorted=True):
    """`torch.topk` mit festgelegter Auswahl bei Gleichstand: der kleinere Index zuerst."""
    assert largest
    idx = torch.sort(x, dim=dim, descending=True, stable=True).indices.narrow(dim, 0, k)
    return x.gather(dim, idx), idx


@contextlib.contextmanager
def deterministic_topk():
    """Der Zwischenspeicher waehlt mit `torch.topk`. Nach dem ersten Verdichten liegt
    derselbe Rahmen mehrfach im Speicher (je Sprecher eine Kopie, exakt gleiche Werte),
    und an der Auswahlgrenze entscheidet bei Gleichstand die (unspezifizierte)
    Auswahl von `torch.topk`, welche Kopie bleibt. Der Inhalt je Sprecher ist dabei
    gleich, nur die Reihenfolge der Kopien im Speicher wechselt, und die Logits des
    naechsten Blocks aendern sich spuerbar (bis 0,8 Logit, siehe `torch_default_topk`
    in `pipeline_ref.json`). Fuer ein reproduzierbares Golden gilt hier dieselbe
    Regel wie in der Rust-Engine: bei Gleichstand der kleinere Index."""
    import transformers.models.nemotron3_diarization.modeling_nemotron3_diarization as m

    class Shim:
        def __getattr__(self, name):
            return getattr(torch, name)

        topk = staticmethod(_stable_topk)

    saved = m.torch
    m.torch = Shim()
    try:
        yield
    finally:
        m.torch = saved


def make_cache(model_cfg, out: Path) -> dict:
    with deterministic_topk():
        return _make_cache(model_cfg, out)


def _make_cache(model_cfg, out: Path) -> dict:
    from transformers.models.nemotron3_diarization.configuration_nemotron3_diarization import (
        Nemotron3DiarizationStreamingConfig,
    )
    from transformers.models.nemotron3_diarization.modeling_nemotron3_diarization import (
        Nemotron3DiarizationSpeakerCache as Cache,
    )

    # Klein: 3 Sprecher, Speicher 12, FIFO 6, Block 4; mit Rueckblick und Auffuellung.
    small_cfg = Nemotron3DiarizationStreamingConfig(
        fifo_length=6,
        speaker_cache_update_period=4,
        speaker_cache_length=12,
        speaker_cache_silence_frames_per_speaker=1,
        num_speakers=3,
        subsampling_factor=2,
    )
    pattern = [(2, 0, 0), (2, 0, 0), (2, 2, 0), (2, 0, 0), (2, 2, 0), (2, 0, 0)]
    small_steps = [
        {"lookahead": la, "lookback": lb, "mask_tail": mt}
        for la, lb, mt in (pattern * 3)[:16]
    ]
    small_steps[-1]["mask_tail"] = 2
    small = run_scenario(
        Cache,
        small_cfg,
        name="small",
        fifo=6,
        period=4,
        hidden=2,
        chunk=4,
        steps=small_steps,
        seed=12345,
        bias=[3, 0, -2],
        probs_at=set(range(len(small_steps))),
    )

    # Voll: Offline-Arbeitspunkt der config.json (264 Rahmen, 8 Sprecher, FIFO 40,
    # Auswurf 300, Block 340 + 40 Vorausschau), fuenf Bloecke, ab dem ersten verdichtet.
    full_cfg = model_cfg.streaming_config
    full_steps = [{"lookahead": 40, "lookback": 0, "mask_tail": 0} for _ in range(4)]
    full_steps.append({"lookahead": 0, "lookback": 0, "mask_tail": 3})
    full = run_scenario(
        Cache,
        full_cfg,
        name="offline",
        fifo=model_cfg.fifo_length,
        period=model_cfg.speaker_cache_update_period,
        hidden=2,
        chunk=model_cfg.chunk_length,
        steps=full_steps,
        seed=987654321,
        bias=[4, 2, 1, 0, -1, -2, -3, -4],
        probs_at={0, 1, len(full_steps) - 1},
    )

    # `_get_frame_scores` einzeln: Rahmen ohne Sprache, genau 0,5, unter der Klemme,
    # gesaettigt, ueberlappend, ein Sprecher mit genug eindeutigen Rahmen.
    probs = [
        [0.9375, 0.0, 0.0625],
        [0.9375, 0.0625, 0.0],
        [0.875, 0.625, 0.0],
        [0.5, 0.5, 0.5],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0],
        [0.1875, 0.8125, 0.1875],
        [0.625, 0.625, 0.625],
        [0.0, 0.6875, 0.0],
        [0.0625, 0.0, 0.5625],
    ]
    scores_cfg = small_cfg
    cache = Cache(scores_cfg, fifo_length=6, speaker_cache_update_period=4)
    scores = cache._get_frame_scores(torch.tensor(probs, dtype=torch.float32)[None])[0]
    scores_json = [
        None if not math.isfinite(float(v)) else round(float(v), 7) for v in scores.flatten().tolist()
    ]
    scores_case = {
        "config": small["config"],
        "probs": [v for row in probs for v in row],
        "scores": scores_json,
    }
    doc = {"lcg": LCG_NOTE, "scenarios": [small, full], "scores": scores_case}
    (out / "cache_ref.json").write_text(json.dumps(doc, separators=(",", ":")), encoding="utf-8")
    return {
        "small_steps": len(small_steps),
        "small_compressed_steps": sum(r["compressed"] for r in small["steps"]),
        "offline_steps": len(full_steps),
    }


# ---------------------------------------------------------------------- Pipeline


def read_wav(path: Path) -> np.ndarray:
    x, sr = sf.read(path, dtype="float32")
    assert sr == SAMPLE_RATE and x.ndim == 1, (path, sr, x.shape)
    return x


def render(fixtures: Path, recipe: list[dict]) -> np.ndarray:
    """Baut das Pruefaudio aus Fixture-WAVs. Die Teile (`parts`) liegen hintereinander;
    in einem Teil werden die Ebenen (`layers`) Abtastwert fuer Abtastwert addiert
    (`offset` in Abtastwerten, `take` kuerzt, `reverse` dreht um, `gain` skaliert,
    alles in float32). Der Rust-Test setzt dieselbe Vorschrift um (`engine.rs`)."""
    parts = []
    for part in recipe:
        layers = []
        for layer in part["layers"]:
            x = read_wav(fixtures / layer["file"])
            if layer.get("reverse"):
                x = x[::-1]
            if "take" in layer:
                x = x[: layer["take"]]
            x = x * np.float32(layer.get("gain", 1.0))
            layers.append((int(layer.get("offset", 0)), x.astype(np.float32)))
        out = np.zeros(max(o + len(x) for o, x in layers), np.float32)
        for o, x in layers:
            out[o : o + len(x)] += x
        parts.append(out)
    return np.concatenate(parts)


def run_pipeline(session, processor, model_cfg, audio: np.ndarray):
    """Offline-Schleife des Modells mit ONNX-Encoder und dem Zwischenspeicher der Referenz."""
    from transformers.models.nemotron3_diarization.modeling_nemotron3_diarization import (
        Nemotron3DiarizationSpeakerCache as Cache,
    )

    sub = model_cfg.audio_config.subsampling_factor
    inputs = processor(audio, sampling_rate=SAMPLE_RATE)
    feats = np.asarray(inputs["input_features"][0], dtype=np.float32)
    mel_mask = np.asarray(inputs["attention_mask"][0]).astype(bool)
    num_frames = feats.shape[0]
    embed_mask = mel_mask[::sub]
    num_embeds = embed_mask.shape[0]
    assert num_embeds == math.ceil(num_frames / sub)

    cache = Cache(
        model_cfg.streaming_config,
        fifo_length=model_cfg.fifo_length,
        speaker_cache_update_period=model_cfg.speaker_cache_update_period,
    )
    cache.lazy_initialization(torch.zeros(1, 1, model_cfg.audio_config.hidden_size))
    chunk_length = model_cfg.chunk_length
    right = model_cfg.chunk_right_context
    assert model_cfg.chunk_left_context == 0
    logits_parts = []
    blocks = 0
    for start in range(0, num_embeds, chunk_length):
        end = min(start + chunk_length, num_embeds)
        num_chunk = end - start
        context_end = min(end + right, num_embeds)
        mel = feats[start * sub : context_end * sub][None]
        cached = cache.get_embeds(None)
        cached_len = cached.shape[1]
        step_mask = np.concatenate(
            [np.ones(cached_len, dtype=bool), embed_mask[start:context_end]]
        )
        out = session.run(
            None,
            {
                "input_features": np.ascontiguousarray(mel, dtype=np.float32),
                "cached_embeds": np.ascontiguousarray(cached.numpy(), dtype=np.float32),
                "attention_mask": step_mask[None].astype(np.int64),
            },
        )
        logits, chunk_embeds, silence = out
        chunk_input = torch.cat([cached, torch.from_numpy(chunk_embeds)], dim=1)
        cache.update(
            chunk_input,
            torch.from_numpy(logits),
            torch.from_numpy(silence),
            num_chunk,
            mask=torch.from_numpy(step_mask)[None],
            num_lookback_frames=0,
        )
        from_row = cached_len * sub
        logits_parts.append(logits[:, from_row : from_row + num_chunk * sub])
        blocks += 1
    logits_all = np.concatenate(logits_parts, axis=1)[:, :num_frames]
    return logits_all, torch.from_numpy(mel_mask)[None], num_frames, blocks


def make_pipeline(model_dir: Path, fixtures: Path, out: Path, debug_dir: Path | None) -> dict:
    import onnxruntime as ort
    from transformers import AutoProcessor
    from transformers.models.nemotron3_diarization.configuration_nemotron3_diarization import (
        Nemotron3DiarizationConfig,
    )

    onnx_path = model_dir / "onnx" / "model_quantized.onnx"
    processor = AutoProcessor.from_pretrained(str(model_dir))
    model_cfg = Nemotron3DiarizationConfig.from_pretrained(str(model_dir))
    so = ort.SessionOptions()
    so.intra_op_num_threads = 4
    session = ort.InferenceSession(str(onnx_path), so, providers=["CPUExecutionProvider"])

    zira, mic, ren = "g5/en_interview_zira.wav", "m2_echo_mic.wav", "m2_echo_render.wav"
    cases = {
        # Drei Aufnahmen hintereinander (73,4 s, drei Bloecke).
        "concat": [{"layers": [{"file": f}]} for f in (zira, mic, ren)],
        # Ueberlappende und umgekehrte Sprache als weitere "Stimmen" (93,4 s, vier
        # Bloecke): Mitschnitt mit eingemischtem Gegenueber, dann rueckwaerts, dann
        # zwei Aufnahmen gleichzeitig.
        "overlap": [
            {"layers": [{"file": zira}]},
            {"layers": [{"file": mic}, {"file": zira, "reverse": True, "gain": 0.8, "offset": 48000, "take": 256000}]},
            {"layers": [{"file": ren, "reverse": True}]},
            {"layers": [{"file": ren}, {"file": mic, "gain": 0.6, "offset": 128000}]},
        ],
    }
    onnx_sha = hashlib.sha256(onnx_path.read_bytes()).hexdigest()
    doc = {
        "config": {
            "chunk_length": model_cfg.chunk_length,
            "chunk_right_context": model_cfg.chunk_right_context,
            "fifo_length": model_cfg.fifo_length,
            "speaker_cache_update_period": model_cfg.speaker_cache_update_period,
        },
        "onnx_sha256": onnx_sha,
        "cases": [],
    }
    report = {}
    for name, recipe in cases.items():
        audio = render(fixtures, recipe)
        with deterministic_topk():
            logits, mel_mask, num_frames, blocks = run_pipeline(session, processor, model_cfg, audio)
        # Zum Vergleich: dieselbe Rechnung mit der Auswahl von `torch.topk` (siehe `deterministic_topk`).
        logits_default, *_ = run_pipeline(session, processor, model_cfg, audio)
        if debug_dir is not None:
            debug_dir.mkdir(parents=True, exist_ok=True)
            np.save(debug_dir / f"ref_logits_{name}.npy", logits[0])
            np.save(debug_dir / f"ref_logits_{name}_torch_default.npy", logits_default[0])
        mask = mel_mask[:, :num_frames]
        segments = processor.extract_speaker_dict(torch.from_numpy(logits), mask)[0]
        turns = [
            {
                "start_ms": int(round(s["Start"] * 1000)),
                "end_ms": int(round(s["End"] * 1000)),
                "speaker": int(s["Speaker"]) + 1,
            }
            for s in segments
        ]
        valid = int(mask.sum())
        active = (torch.from_numpy(logits).sigmoid() > 0.5)[0, :valid]
        per_speaker = [int(v) for v in active.sum(0).tolist()]
        # Zeitangaben sind ganze Rahmen: die Summe je Sprecher muss stimmen.
        for c, total in enumerate(per_speaker):
            assert sum((t["end_ms"] - t["start_ms"]) // 10 for t in turns if t["speaker"] == c + 1) == total
        valid_default = (torch.from_numpy(logits_default).sigmoid() > 0.5)[0, :valid]
        same = (valid_default == active).all(dim=1)
        default_diff = np.abs(logits_default[0, :valid] - logits[0, :valid])
        doc["cases"].append(
            {
                "name": name,
                "torch_default_topk": {
                    "note": "Abweichung der Referenz mit torch.topk-Standardauswahl von dieser (nur zur Information)",
                    "frame_agreement": round(float(same.float().mean()), 5),
                    "max_logit_diff": round(float(default_diff.max()), 4),
                },
                "recipe": recipe,
                "audio": {
                    "samples": int(len(audio)),
                    "seconds": round(len(audio) / SAMPLE_RATE, 3),
                    "frames": valid,
                },
                "blocks": blocks,
                "active_frames_per_speaker": per_speaker,
                "turns": turns,
            }
        )
        report[name] = {
            "seconds": round(len(audio) / SAMPLE_RATE, 3),
            "turns": len(turns),
            "blocks": blocks,
            "frames_per_speaker": per_speaker,
            "torch_default_topk_frame_agreement": round(float(same.float().mean()), 5),
        }
    (out / "pipeline_ref.json").write_text(json.dumps(doc, indent=1), encoding="utf-8")
    return report


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model-dir", type=Path, required=True)
    ap.add_argument("--fixtures", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--debug-dir", type=Path, default=None)
    ap.add_argument("--parts", nargs="+", default=["mel", "cache", "pipeline"],
                    choices=["mel", "cache", "pipeline"])
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    from transformers import AutoProcessor
    from transformers.models.nemotron3_diarization.configuration_nemotron3_diarization import (
        Nemotron3DiarizationConfig,
    )

    torch.set_num_threads(4)
    report = {}
    if "mel" in args.parts:
        processor = AutoProcessor.from_pretrained(str(args.model_dir))
        report["mel"] = make_mel(processor, args.fixtures, args.out)
    if "cache" in args.parts:
        cfg = Nemotron3DiarizationConfig.from_pretrained(str(args.model_dir))
        report["cache"] = make_cache(cfg, args.out)
    if "pipeline" in args.parts:
        report["pipeline"] = make_pipeline(args.model_dir, args.fixtures, args.out, args.debug_dir)
    print(json.dumps(report, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
