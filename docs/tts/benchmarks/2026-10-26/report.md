# TTS-Benchmark: Bericht

Erzeugt: 2026-10-26T21:41:58.825Z · Harness `tts-bench/1.0.0` · Normalisierung `norm-v1`

> **Zielwerte sind Ziele, keine Zusagen** (TTFA < 200 ms, RTF < 0,2, WER < 5 %). Spalte „Ziel erreicht“ vergleicht nur den gemessenen Wert mit dem Ziel.  
> WER/CER sind ein Verständlichkeits-Proxy (ASR liest die Synthese zurück), kein Natürlichkeitsmaß; ASR-Fehler fließen in die Werte ein.

## Läufe und Korpus

| Lauf | Quelle | Gestartet | Beendet |
|---|---|---|---|
| 2026-10-26-piper-lat | app | 2026-10-26T18:16:52.495Z | 2026-10-26T18:24:19.941Z |
| 2026-10-26-piper-full | app | 2026-10-26T19:52:00.225Z | 2026-10-26T19:54:22.923Z |
| 2026-10-26-fish-lat | app | 2026-10-26T18:36:28.287Z | 2026-10-26T19:51:29.813Z |
| 2026-10-26-fish-full | app | 2026-10-26T19:54:23.443Z | 2026-10-26T20:03:24.966Z |
| 2026-10-26-phonon-lat | app | 2026-10-26T21:18:21.599Z | 2026-10-26T21:25:54.433Z |
| 2026-10-26-phonon-full2 | app | 2026-10-26T21:26:14.667Z | 2026-10-26T21:28:40.571Z |
| 2026-10-26-status2 | app | 2026-10-26T21:41:58.464Z | 2026-10-26T21:41:58.465Z |

| Korpus | Sätze | Smoke | SHA-256 (LF) |
|---|---|---|---|
| de-v1 | 123 | 12 | `370572db1ae15baa…` |

## Host

| Merkmal | Wert |
|---|---|
| Betriebssystem | Windows 10 (10.0.26200) |
| CPU | 13th Gen Intel(R) Core(TM) i9-13900K (24 Kerne / 32 Threads) |
| RAM | 63,8 GB |
| GPU | NVIDIA GeForce RTX 4090 (24564 MB) |
| GPU-Treiber / CUDA | 621.01 / 13.5 |
| Python | 3.11.9 |
| Git-Commit | `c21d9fabfcfa` (feat/tts-provider, Harness unverändert) |
| Zeitstempel | 2026-10-26T18:16:52.493Z |

## Latenz (warm)

Nur erfolgreiche Synthesen; „Fehler“ = fehlgeschlagene Zeilen der Konfiguration. Perzentile linear interpoliert. RTF = Synthesezeit / Audiodauer (kleiner ist schneller).

### Time to first audio (ms)

| Konfiguration | n | Fehler | Median | P90 | P95 | Mittel | Stdabw. | Min | Max |
|---|---|---|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 483 | 0 | 1069 | 1287 | 2155 | 1129 | 338 | 540 | 2313 |
| fish-s2-pro-cuda | 483 | 0 | 3733 | 5071 | 13616 | 4517 | 3019 | 1359 | 15580 |
| phonon-40m-de-cpu | 483 | 0 | 999 | 1567 | 2080 | 1130 | 364 | 574 | 2695 |

### Gesamtdauer der Synthese (ms)

| Konfiguration | n | Fehler | Median | P90 | P95 | Mittel | Stdabw. | Min | Max |
|---|---|---|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 483 | 0 | 1069 | 1287 | 2155 | 1129 | 338 | 540 | 2313 |
| fish-s2-pro-cuda | 483 | 0 | 3733 | 5071 | 13616 | 4517 | 3019 | 1359 | 15580 |
| phonon-40m-de-cpu | 483 | 0 | 999 | 1567 | 2080 | 1130 | 364 | 574 | 2695 |

### Real-Time-Factor (RTF)

| Konfiguration | n | Fehler | Median | P90 | P95 | Mittel | Stdabw. | Min | Max |
|---|---|---|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 483 | 0 | 0,255 | 0,290 | 0,298 | 0,257 | 0,028 | 0,193 | 0,438 |
| fish-s2-pro-cuda | 483 | 0 | 0,820 | 1,136 | 1,441 | 0,918 | 0,211 | 0,652 | 1,474 |
| phonon-40m-de-cpu | 483 | 0 | 0,240 | 0,312 | 0,332 | 0,251 | 0,040 | 0,163 | 0,398 |

## Kaltstart

Prozessstart bis Anbieter bereit sowie erste Synthese nach dem Start (Zeilen mit `run_kind=cold`).

| Konfiguration | n (ok) | Prozessstart Median (ms) | Prozessstart Max (ms) | TTFA Median (ms) | Gesamt Median (ms) |
|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 36 | – | – | 1081 | 1081 |
| fish-s2-pro-cuda | 36 | 68357 | 111624 | 6237 | 6237 |
| phonon-40m-de-cpu | 36 | – | – | 1246 | 1246 |

## Zielwerte (Ziele, keine Zusagen)

| Konfiguration | TTFA Median (ms) | Ziel < 200 ms erreicht | RTF Median | Ziel < 0,2 erreicht | WER (%) | Ziel < 5 % erreicht |
|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 1069 | nein | 0,255 | nein | 6,64 | nein |
| fish-s2-pro-cuda | 3733 | nein | 0,820 | nein | 4,85 | ja |
| phonon-40m-de-cpu | 999 | nein | 0,240 | nein | 2,26 | ja |

## Qualität (Verständlichkeits-Proxy, kein Natürlichkeitsmaß)

| Konfiguration | ASR (Modell, Version) | Sätze | WER (%) | CER (%) |
|---|---|---|---|---|
| piper-thorsten-high-cpu | app-cli handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf (gguf, CPU, Exe vom 2026-10-26T18:11:49Z) | 123 | 6,64 | 3,10 |
| fish-s2-pro-cuda | app-cli handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf (gguf, CPU, Exe vom 2026-10-26T18:11:49Z) | 123 | 4,85 | 2,33 |
| phonon-40m-de-cpu | app-cli handy-computer/whisper-large-v3-turbo-gguf/whisper-large-v3-turbo-Q8_0.gguf (gguf, CPU, Exe vom 2026-10-26T18:11:49Z) | 123 | 2,26 | 1,15 |

WER je Kategorie (%):

| Konfiguration | alltag | lang | zahlen | datum | akronyme | codeswitch | fachbegriffe | email_url | fragen | ausdruck |
|---|---|---|---|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 0,8 | 2,5 | 4,9 | 7,1 | 3,6 | 14,2 | 11,0 | 20,0 | 0,0 | 3,0 |
| fish-s2-pro-cuda | 0,8 | 1,6 | 9,8 | 0,0 | 4,3 | 3,1 | 11,7 | 18,0 | 0,0 | 1,5 |
| phonon-40m-de-cpu | 0,8 | 1,6 | 0,0 | 0,0 | 0,0 | 2,5 | 7,6 | 7,3 | 0,0 | 1,5 |

## Ressourcen

Eigenes Sampling alle 200 ms. RAM = Working Set des App-Prozessbaums (extern gestartete Server nicht enthalten). VRAM = Gesamtbelegung der GPU, Delta gegen den Wert vor dem Start. „–“ = nicht messbar (Grund unten).

| Konfiguration | RAM Peak (MB) | CPU Mittel (% Maschine) | CPU Peak (% Maschine) | VRAM Peak (MB) | VRAM Delta (MB) | GPU-Last Mittel (%) |
|---|---|---|---|---|---|---|
| piper-thorsten-high-cpu | 372 | 45,6 | 79,5 | 3440 | 7 | 0 |
| fish-s2-pro-cuda | 24121 | 2,9 | 32,6 | 23732 | 20298 | 80 |
| phonon-40m-de-cpu | 333 | 45,9 | 95,6 | 7402 | 0 | 1 |

## Hinweise zu den Messungen

- **fish-s2-pro-cuda**: GPU nicht exklusiv: 51 Compute-Prozess(e) auf der GPU

## Nicht gemessene Konfigurationen

Ohne Zahlen: Status und Grund laut Konfiguration bzw. Lauf.

| Konfiguration | Anbieter | Status | Grund |
|---|---|---|---|
| phonon-31m-en-cpu | phonon | nicht getestet | Nicht gemessen: Das deutsche Modell phonon-40m-de wurde gemessen; der englische Funktionsnachweis ist damit entbehrlich (Modell liegt lokal unter D:/lv-build/phonon/models/phonon-31m-en). |
| ollama-speech-windows | ollama-speech | nicht unterstützt | Speech-Endpunkt laut Ollama-Release-Notes v0.40.11 unter Windows nicht unterstützt; lokaler Client ist 0.34.2, Dienst aus. |
| kokoro | kokoro | nicht unterstützt | Kein deutsches Modell (Sprachen laut Modellkarte ohne Deutsch); über openai-speech anbindbar, aber für den deutschen Korpus nicht messbar. |
| xtts-v2 | xtts-v2 | nicht getestet | Nicht installiert; Gewichte unter Coqui Public Model License (nichtkommerziell). Kein Adapter im Standard. |
| f5-tts | f5-tts | nicht getestet | Nicht installiert; Gewichte CC-BY-NC, Deutsch nur über Community-Finetunes (ebenfalls CC-BY-NC-4.0). Kein Adapter im Standard. |

## Reproduktion

- Korpus und Normalisierung sind versioniert (`norm-v1`); Lauf-Ordner enthält Momentaufnahme des Korpus, `host.json`, `run.json` und je Konfiguration `rows.jsonl`/`env.json`.
- Ablauf: `python scripts/bench/tts/tts_bench.py run` → `asr --run <id>` → `report --runs <id>`; Details in `scripts/bench/tts/README.md`.
