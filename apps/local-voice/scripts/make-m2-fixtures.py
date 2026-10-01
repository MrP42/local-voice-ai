#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Erzeugt die M2-Echo-Fixtures fuer meetings::echo (AEC + Aligner).

  m2_echo_mic.wav     20 s, 16 kHz mono PCM16: Mikrofonspur = Nahsprecher ("Ich")
                      + Raumecho der Gegenseite + Rauschen.
  m2_echo_render.wav  20 s, gleiche Achse: die Gegenseite so, wie sie aus dem
                      Systemton (Loopback) kommt = AEC-Referenz.

Der Mischer bildet den Spike nach (lva-spikes/m2/aec/mix.py): Echo -6 dB,
50 ms Laufzeit, Raumimpulsantwort mit RT60 0,3 s (viel Hall), Rauschen -55 dB.
Sprache = Windows-SAPI-Stimmen (Stefan = Gegenseite, Hedda = Ich), Texte wie
im Spike.

Zeitplan (Sekunden auf der gemeinsamen Achse; Endzeiten mit den SAPI-Stimmen
dieser Maschine, gemessen) - die Tests in `managers/meetings/echo.rs`
(Konstanten `REGION_*`) haengen daran:

   0.6 -  4.2  Gegenseite spricht, Nahsprecher SCHWEIGT (die ersten 5 s: AEC
               schwingt ein; ERLE ist ab 5 s messbar)
   5.6 -  9.9  nur Gegenseite  -> ERLE-Messfenster (nach 5 s Einschwingzeit)
  11.2 - 14.0  nur Ich         -> Pegelfenster Nahsprache
  15.3 - 18.6  Gegenseite  } Doppelsprechen ca. 15.8 - 18.6
  15.8 - 18.9  Ich         }

Die WAVs sind deterministisch bis auf die TTS-Ausgabe der jeweiligen
Maschine; die eingecheckten Dateien sind massgeblich. Neu erzeugen nur mit
Absicht (-Force) und danach die Tests pruefen.

Aufruf normalerweise ueber make-m2-fixtures.ps1. Abhaengigkeiten: numpy,
scipy, soundfile (pip install numpy scipy soundfile); PowerShell 7 (pwsh) mit
System.Speech - die Stimme "Microsoft Stefan" sieht nur pwsh, Windows
PowerShell 5.1 kennt nur Hedda/Zira.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import fftconvolve

SR = 16_000
DURATION_S = 20
SEED = 7

FAR_VOICE = "Microsoft Stefan"
NEAR_VOICE = "Microsoft Hedda Desktop"

FAR_TEXTS = [
    "Die Ausschreibung für das neue Lager endet am dritten November.",
    "Unser Vertrieb meldet steigende Nachfrage aus Österreich und der Schweiz.",
    "Das Marketingbudget wird um zwölf Prozent gekürzt.",
]
NEAR_TEXTS = [
    "Ich übernehme die Abstimmung mit der Rechtsabteilung.",
    "Die Messebeteiligung in Hannover halte ich für zu teuer.",
]

# (Spur, Textindex, Startzeit in s). Muss zum Kommentar oben und zu den
# REGION_*-Konstanten im Test passen.
PLAN = [
    ("far", 0, 0.5),
    ("far", 1, 5.5),
    ("near", 0, 11.2),
    ("far", 2, 15.2),
    ("near", 1, 15.8),
]
# Nahsprecher darf in den ersten 5 s und bis 11.2 s nicht sprechen.
NEAR_SILENT_UNTIL_S = 11.2

ECHO_GAIN_DB = -6.0
ECHO_DELAY_MS = 50
RT60_S = 0.3
NOISE_DB = -55.0

# ASCII-only PowerShell: rendert jeden Satz als 16-kHz-Mono-PCM16-WAV.
TTS_PS1 = r"""
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech
$dir = $args[0]
$j = Get-Content -Raw -Encoding UTF8 (Join-Path $dir 'sentences.json') | ConvertFrom-Json
$fmt = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
function Render($voice, $list, $prefix) {
  $i = 0
  foreach ($t in $list) {
    $s = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $s.SelectVoice($voice)
    $s.SetOutputToWaveFile((Join-Path $dir ("{0}_{1}.wav" -f $prefix, $i)), $fmt)
    $s.Speak($t)
    $s.Dispose()
    $i++
  }
}
Render $j.far_voice @($j.far) 'far'
Render $j.near_voice @($j.near) 'near'
"""


def synthesize(tmp: Path) -> None:
    (tmp / "sentences.json").write_text(
        json.dumps(
            {
                "far_voice": FAR_VOICE,
                "near_voice": NEAR_VOICE,
                "far": FAR_TEXTS,
                "near": NEAR_TEXTS,
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )
    script = tmp / "tts.ps1"
    script.write_text(TTS_PS1, encoding="ascii")
    last = None
    # Stefan (OneCore) gibt es nur in PowerShell 7; 5.1 sieht nur Hedda/Zira.
    for shell in ("pwsh", "powershell"):
        try:
            subprocess.run(
                [shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script), str(tmp)],
                check=True,
            )
            return
        except (FileNotFoundError, subprocess.CalledProcessError) as e:
            last = e
    raise SystemExit(f"TTS mit System.Speech fehlgeschlagen: {last}")


def load(path: Path) -> np.ndarray:
    x, sr = sf.read(path, dtype="float64")
    assert sr == SR, f"{path.name}: {sr} Hz statt {SR}"
    return x


def mix(tmp: Path) -> tuple[np.ndarray, np.ndarray]:
    far_clips = [load(tmp / f"far_{i}.wav") for i in range(len(FAR_TEXTS))]
    near_clips = [load(tmp / f"near_{i}.wav") for i in range(len(NEAR_TEXTS))]
    n = DURATION_S * SR
    far_tr = np.zeros(n)
    near_tr = np.zeros(n)
    for kind, idx, start in PLAN:
        clip = far_clips[idx] if kind == "far" else near_clips[idx]
        s = int(round(start * SR))
        end = s + len(clip)
        if end > n:
            raise SystemExit(
                f"{kind}_{idx} endet bei {end / SR:.2f} s (> {DURATION_S} s): "
                "Plan/Text anpassen"
            )
        if kind == "near" and start < NEAR_SILENT_UNTIL_S - 1e-9:
            raise SystemExit("Nahsprecher spricht zu frueh")
        (far_tr if kind == "far" else near_tr)[s:end] += clip
        print(f"  {kind}_{idx}: {start:.2f} - {end / SR:.2f} s")
    # Die Tests haengen an diesen Fenstern: Sprecher duerfen sich nur im
    # Doppelsprech-Abschnitt ueberlappen.
    if np.abs(near_tr[: int(NEAR_SILENT_UNTIL_S * SR)]).max() > 0:
        raise SystemExit("Nahsprache vor 11.2 s")
    if np.abs(far_tr[int(11.0 * SR) : int(15.2 * SR)]).max() > 0:
        raise SystemExit("Gegenseite im Nur-Ich-Fenster")
    far_tr *= 0.5 / np.abs(far_tr).max()
    near_tr *= 0.5 / np.abs(near_tr).max()

    rng = np.random.default_rng(SEED)
    # Raumimpulsantwort: Direktpfad + exponentieller Hall-Schwanz, RT60 0,3 s.
    rl = int(0.35 * SR)
    t = np.arange(rl) / SR
    rir = rng.standard_normal(rl) * np.exp(-6.9 * t / RT60_S) * 0.25
    rir[0] = 1.0
    rir /= np.sqrt((rir**2).sum())
    d = int(ECHO_DELAY_MS * SR / 1000)
    echo = fftconvolve(far_tr, rir)[:n]
    echo = np.concatenate([np.zeros(d), echo])[:n] * 10 ** (ECHO_GAIN_DB / 20)
    noise = rng.standard_normal(n) * 10 ** (NOISE_DB / 20)
    mic = near_tr + echo + noise
    return np.clip(mic, -1, 1), far_tr


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out-dir", required=True, type=Path)
    ap.add_argument("--force", action="store_true")
    a = ap.parse_args()
    out = a.out_dir
    out.mkdir(parents=True, exist_ok=True)
    mic_p, ren_p = out / "m2_echo_mic.wav", out / "m2_echo_render.wav"
    if mic_p.exists() and ren_p.exists() and not a.force:
        print("[skip] m2_echo_*.wav vorhanden (-Force zum Neuerzeugen)")
        return
    with tempfile.TemporaryDirectory(prefix="m2-echo-") as td:
        tmp = Path(td)
        synthesize(tmp)
        mic, ren = mix(tmp)
    sf.write(mic_p, mic.astype(np.float32), SR, subtype="PCM_16")
    sf.write(ren_p, ren.astype(np.float32), SR, subtype="PCM_16")
    for p in (mic_p, ren_p):
        print(f"[ok]   {p.name}  {p.stat().st_size / 1024:.0f} KB")


if __name__ == "__main__":
    sys.exit(main())
