# TTS-Benchmark

Wiederverwendbarer, reproduzierbarer Benchmark für Sprachausgabe-Anbieter (Goal tts-provider, Paket P5). Er misst
Latenz (TTFA, Gesamtdauer, RTF, Kaltstart), Ressourcen (RAM, CPU, VRAM, GPU-Last) und – per ASR-Rücklesung –
Verständlichkeit (WER/CER) auf einem versionierten deutschen Korpus und erzeugt Berichte sowie ein Blindhörtest-Paket.

**Zielwerte** (TTFA < 200 ms, RTF < 0,2, WER < 5 %) sind Ziele aus dem Goal, keine Zusagen; der Bericht sagt nur
„erreicht ja/nein/nicht bewertbar“. **WER/CER sind ein Verständlichkeits-Proxy, kein Natürlichkeitsmaß** – dafür gibt es
den Hörtest.

## Bausteine

| Pfad | Inhalt |
|---|---|
| `tts_bench.py` | CLI mit den Unterbefehlen `validate-corpus`, `run`, `asr`, `report`, `listening-kit`, `listening-eval` |
| `corpus/` | `de-v1.jsonl` (123 Sätze, 10 Kategorien, Smoke 12), `en-func-v1.jsonl`, `README.md` (Änderungsregel), `normalization.md` |
| `configs/default.toml` | Messkonfigurationen (Anbieter, Stimme, Gerät, Status, Läufe) |
| `schema/` | Zeilen-Schema `row.schema.json`, `env.schema.json`, Schnittstellenbeschreibung zur App |
| `ttsbench/` | Harness-Module (Normalisierung, Statistik, Sampling, Quellen, Lauf, ASR, Bericht, Hörtest) |
| `test_tts_bench.py` | pytest ohne Netz/GPU/App |

## Voraussetzungen

Python ≥ 3.11 mit `numpy`, `soundfile`, `pytest`; optional `psutil` (sonst Windows-API) und `faster-whisper` (nur
`asr`). `nvidia-smi` für GPU-Daten (fehlt es, steht „nicht messbar“ mit Grund im Ergebnis). Die App-Quelle braucht das
Binary mit `--tts-bench` (Paket P3); bis dahin: `--source fake`.

## Ablauf

```
# 0. Korpus und Plan prüfen (startet nichts)
python scripts/bench/tts/tts_bench.py validate-corpus
python scripts/bench/tts/tts_bench.py run --dry-run

# 1. Latenzlauf: Smoke-Teilmenge (12 Sätze) x (30 warm + 3 kalt) je Konfiguration, seriell, GPU exklusiv
python scripts/bench/tts/tts_bench.py run --exe <pfad\local-voice-ai.exe> --run-id 2026-10-latenz
#    einzelne Konfigurationen:  --configs piper-thorsten-high-cpu fish-s2-pro-cuda

# 2. Qualitätslauf: alle 123 Sätze einmal (liefert die WAVs für WER/CER und Hörtest)
python scripts/bench/tts/tts_bench.py run --exe <exe> --subset full --warm 1 --cold 0 --run-id 2026-10-qualitaet

# 3. ASR NACH dem Lauf (nie parallel zu Messungen; gemeinsame Sperre)
python scripts/bench/tts/tts_bench.py asr --run 2026-10-qualitaet

# 4. Bericht (mehrere Läufe werden je Konfiguration vereint); --publish kopiert nur json/csv/md nach docs/tts/benchmarks/<datum>/
python scripts/bench/tts/tts_bench.py report --runs 2026-10-latenz 2026-10-qualitaet [--publish --date 2026-10-27]

# 5. Blindhörtest (siehe unten)
python scripts/bench/tts/tts_bench.py listening-kit --run 2026-10-qualitaet --max-per-config 12
```

Selbsttest ohne App und Modell (so laufen auch die Tests):

```
python scripts/bench/tts/tts_bench.py run --source fake --run-id probe
python scripts/bench/tts/tts_bench.py asr --run probe --engine fake
python scripts/bench/tts/tts_bench.py report --runs probe
python -m pytest scripts/bench/tts -q
```

## Befehle

| Befehl | Zweck / wichtige Optionen |
|---|---|
| `validate-corpus [name\|pfad]` | Schema, ≥ 100 Sätze, Kategorien ≥ 10, eindeutige IDs, Smoke 12 mit allen Kategorien, `normalize(text)` = `expected_normalized`, Beispiel-Domains, keine Telefonnummern |
| `run` | `--config`, `--configs`, `--source app\|fake`, `--exe`, `--warm`, `--cold`, `--subset smoke\|full`, `--run-id`, `--resume`, `--dry-run`, `--require-exclusive`, `--min-free-ram-mb`, `--sample-ms`, `--strict` |
| `asr` | `--run`, `--engine faster-whisper\|fake`, `--model auto` (large-v3-turbo, sonst lokal vorhandenes größtes), `--device auto\|cpu\|cuda`, `--compute-type`, `--all-runs`, `--allow-download` |
| `report` | `--runs`, `--out`, `--publish`, `--publish-dir`, `--date` → `results.json`, `results.csv`, `report.md` |
| `listening-kit` | `--run`, `--configs`, `--subset`, `--max-per-config`, `--seed`, `--out`, `--key-out`, `--no-loudness-match` |
| `listening-eval` | `--key`, `--sheets` (ausgefüllte Bögen), `--out` |

## Konfiguration

`configs/default.toml`: je `[[config]]` `id`, `provider`, `voice`, `language`, `device` (erwartet), `status`
(`run` \| `not_tested` \| `unsupported` \| `license_restricted`, außer `run` mit Pflichtfeld `reason`), `warm_runs` (Standard 30,
je Satz), `cold_runs` (Standard 3, je Satz), `corpus`, `subset`, `quality` (false = kein WER/CER), `memory_limit_mb`,
`max_processes`, `timeout_s`. Konfigurationen mit Status ≠ `run` werden nie gestartet und erscheinen im Bericht mit Grund
und **ohne Zahlen**. Mitgeliefert: Piper `de_DE-thorsten-high` (CPU), Fish `s2-pro` (CUDA), Phonon 31m-en (nur
Funktionsnachweis, englisch, ohne WER) und als Statuszeilen Phonon 40m-de (gated, CC BY-NC 4.0), Ollama-Speech
(Windows nicht unterstützt), Kokoro (kein Deutsch), XTTS-v2 und F5-TTS (nicht installiert, nichtkommerzielle Gewichte).

## Messquelle und Ressourcen

Das App-Binary wird je Konfiguration seriell aufgerufen (Vertrag: `schema/README.md`) und schreibt `rows.jsonl` +
`env.json` + WAVs. Der Harness startet es **im Job-Objekt** (`benchlib.LimitedRunner`: RAM-Deckel je Konfiguration, BelowNormal,
Kindprozesse sterben mit) nach dem RAM-Start-Gate (`--min-free-ram-mb`, Standard 4096) und sampelt alle 200 ms:

- RAM (Working Set) und CPU des Prozessbaums (psutil, sonst Windows-API),
- GPU-Last und **Gesamt**-VRAM über einen laufenden `nvidia-smi -lms`, Basiswert vor dem Start und Delta.

Nicht Messbares wird `null` mit Grund (`*_reason`), nie geschätzt. Vor GPU-Konfigurationen prüft der Harness per
`nvidia-smi`, ob andere Prozesse die GPU nutzen, und vermerkt das im Ergebnis (`gpu_exclusive`), ohne etwas zu beenden;
`--require-exclusive` misst dann gar nicht. Der Host (OS, CPU, RAM, GPU, Treiber, CUDA, Python, Git-Commit, Zeitstempel)
steht in `host.json` und im Bericht.

## Ergebnisablage

Rohdaten (WAVs, Zeilen, `run.json`, `host.json`, Korpus-Momentaufnahme, `asr.json`, Berichte) liegen unter
`benchlib.bench_dir()/tts/<lauf-id>/` (Windows: `%LOCALAPPDATA%\lva-bench\tts\`, überschreibbar mit `LVA_BENCH_DIR`),
**nicht im Repo**. Nur `report --publish` kopiert die Zusammenfassung (`results.json`, `results.csv`, `report.md`, keine WAVs) nach
`docs/tts/benchmarks/<datum>/`. Je Konfiguration ein Unterordner: `rows.jsonl`, `env.json`, `wavs/`, `app_stdout.log`,
`app_stderr.log`. Eine Sperrdatei (`tts/.run.lock`) verhindert, dass ASR oder ein zweiter Lauf parallel zu einer Messung läuft.

## Blindhörtest

`listening-kit` nimmt je Satz und Konfiguration eine WAV, legt sie als `proben/<zufällige-id>.wav` ab (Lautstärke auf
−26 dBFS RMS angeglichen, abschaltbar), mischt mit festem Seed und schreibt `bewertungsbogen.csv` (Semikolon, UTF-8:
Natürlichkeit, Aussprache, Prosodie, Artefakte je 1–5, Freitext, Hörerkürzel) und `README.md` mit der Anleitung.
`key.json` (Seed, ID → Konfiguration/Satz/Quell-Hash) liegt **getrennt** in `listening-key/<kit>/` und gehört nicht zu den Hörern.
Gleicher Seed und gleiche Läufe ergeben dasselbe Paket. `listening-eval --key … --sheets bogen_A.csv bogen_B.csv` führt die
ausgefüllten Bögen mit dem Schlüssel zusammen (Mittelwert/Median je Konfiguration und Merkmal). Der Harness erzeugt
nie Bewertungen; ungültige Werte (außerhalb 1–5) und unbekannte IDs werden gemeldet und ignoriert.

## Reproduktion

Ein Bericht ist reproduzierbar, wenn Korpus-SHA-256, Normalisierungsversion (`norm-v1`), Harness-Version, Git-Commit,
App-Binary, Konfigurationsdatei, ASR-Modell/Version/`compute_type` und Host übereinstimmen – alles steht in `results.json`.
Der Lauf-Ordner enthält eine Momentaufnahme des Korpus; `--seed` (fake) und `--seed` (Hörtest) fixieren Zufallsanteile.
Latenzen schwanken mit der Systemlast: GPU exklusiv, nichts anderes Schweres parallel, bei Auffälligkeiten wiederholen.

## Grenzen

- **WER/CER** messen Verständlichkeit über ein ASR (Standard `large-v3-turbo`, sonst das größte lokal vorhandene Modell –
  steht im Ergebnis; Werte verschiedener ASR-Modelle nicht vergleichen). ASR-Fehler fließen ein. Normalisierungsgrenzen:
  `corpus/normalization.md`. Natürlichkeit, Prosodie und Artefakte nur über den Hörtest.
- **RAM** ist der Working Set des Prozessbaums der App. Extern gestartete Server (z. B. der Fish-HTTP-Server) liegen außerhalb;
  dafür zeigt die VRAM-Messung den Systemwert (Gesamt-VRAM, nicht pro Prozess – unter Windows/WDDM nicht zuverlässig
  auslesbar). Prozesse, die zwischen zwei 200-ms-Samples starten und enden, fehlen.
- **TTFA** hängt davon ab, was das Binary als „erstes Audio“ meldet; bei Satz-Streaming ist sie gleich der Gesamtdauer.
- Kleine Stichproben (12 Sätze × 30) belegen Größenordnungen, keine Signifikanz; Perzentile sind linear interpoliert.
- Fake-Quelle und Fake-ASR liefern synthetische Zahlen und gehören nie in veröffentlichte Berichte.
- Keine Plots (keine zusätzliche Abhängigkeit); `results.csv` ist für Tabellenkalkulationen gedacht.
