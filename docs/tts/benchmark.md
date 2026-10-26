# TTS-Benchmark: Methode und Ergebnisse (Messlauf 26.10.2026)

Stand 26.10.2026 · Goal `tts-provider` (Paket P6, Issue #93) · Werkzeug: [`scripts/bench/tts`](../../scripts/bench/tts/README.md)
· Rohbericht: [`benchmarks/2026-10-26/report.md`](benchmarks/2026-10-26/report.md) (`results.json`, `results.csv` daneben).
Alle Zahlen unten stammen aus `results.json` und sind nicht neu gerechnet.

## Methode

- **Korpus `de-v1`:** 123 deutsche Sätze in 10 Kategorien (Alltag, lang, Zahlen, Datum, Akronyme, Codeswitch, Fachbegriffe,
  E-Mail/URL, Fragen, Ausdruck), davon 12 als **Smoke** markiert (alle Kategorien vertreten). SHA-256 (LF) `370572db1ae15baa…`,
  Normalisierung `norm-v1`.
- **Läufe** (je Anbieter seriell, nichts anderes Schweres parallel, App-Binary `--tts-bench`, siehe [CLI-Doku](cli-and-automation.md)):
  Latenz = Smoke × (30 warm + 3 kalt) = 396 Synthesen; Qualität = alle 123 Sätze × (1 warm + 0 kalt). Die Latenztabellen
  fassen die Warmzeilen beider Läufe je Anbieter zusammen (360 + 123 = 483 Zeilen, Kaltzeilen getrennt).
- **TTFA** = Anfrage bis erstes Audio. Beide Anbieter liefern je Satz ein Teilstück; **TTFA ist daher gleich der Gesamtdauer**.
  **RTF** = Synthesezeit / Audiodauer (kleiner ist schneller; < 1 = schneller als Echtzeit).
- **Warm:** Anbieter geladen, eine Aufwärm-Synthese ohne Zeile. **Kalt:** je Satz zuerst zurücksetzen (Piper: ohnehin ein Prozess je
  Satz, Kalt und Warm unterscheiden sich nur im Dateicache; Fish: der selbst gestartete Server wird neu gestartet,
  `process_start_ms` = Zeit bis bereit).
- **WER/CER:** die WAVs werden vom Whisper large-v3-turbo Q8 der App (`app-cli`, CPU, GGUF) zurückgelesen; das ist ein
  Verständlichkeits-Proxy, **kein Natürlichkeitsmaß**. Natürlichkeit, Prosodie und Artefakte nur über den Blindhörtest.
- **Ressourcen:** Sampling alle 200 ms (RAM/CPU des Prozessbaums, Gesamt-VRAM und GPU-Last über `nvidia-smi`).

## Host

| Merkmal | Wert |
|---|---|
| System | Windows, Build 10.0.26200 (der Harness meldet „Windows 10“) |
| CPU / RAM | Intel Core i9-13900K (24 Kerne / 32 Threads) / 63,8 GB |
| GPU | NVIDIA GeForce RTX 4090, 24 GB; Treiber 621.01, CUDA 13.5; **nicht exklusiv**: Desktop-Prozesse belegen ~3,4 bis 8,3 GB bei 0 % Last |
| Software | App 0.21.18 (**Debug-Build**), Python 3.11.9, Commit `c21d9fab` plus zwei nicht eingecheckte Harness-Änderungen (siehe Abweichungen) |

## Ergebnisse (Anbieter, die gemessen wurden)

Latenz, warm, Median / P90 / P95, Fehler = fehlgeschlagene Zeilen (alle 0):

| Konfiguration | TTFA = Gesamt (ms) | RTF | Fehler |
|---|---|---|---|
| Piper `de_DE-thorsten-high`, CPU | 1069 / 1287 / 2155 | 0,255 / 0,290 / 0,298 | 0 von 483 |
| Fish `s2-pro`, CUDA, Standardstimme | 3733 / 5071 / 13616 | 0,820 / 1,136 / 1,441 | 0 von 483 |

Kaltstart (36 Zeilen je Anbieter, alle ok): Piper erste Synthese Median 1081 ms (Prozessstart nicht messbar). Fish Serverstart
Median 68,4 s (Max 111,6 s, der erste Start des Laufs mit kaltem Cache), erste Synthese danach Median 6237 ms.

Verständlichkeit (123 Sätze, ASR `app-cli`, Whisper large-v3-turbo Q8_0 aus der App, CPU, Exe vom 26.10.2026 18:11 UTC):

| Konfiguration | WER | CER | schwächste Kategorien (WER) |
|---|---|---|---|
| Piper | 6,64 % | 3,10 % | E-Mail/URL 20,0 %, Codeswitch 14,2 %, Fachbegriffe 11,0 % |
| Fish | 4,85 % | 2,33 % | E-Mail/URL 18,0 %, Fachbegriffe 11,7 %, Zahlen 9,8 % |

Ressourcen (Peak): Piper RAM 372 MB, CPU im Mittel 45,6 % der Maschine, VRAM-Delta 7 MB. Fish RAM 24,1 GB (enthält offenbar den von der App
selbst gestarteten Server), VRAM-Delta 20,3 GB (Gesamtwert 23,7 GB), GPU-Last im Mittel 80 %.

## Zielwerte (Ziele, keine Zusagen)

| Ziel | Piper | Fish |
|---|---|---|
| TTFA < 200 ms | nein (1069 ms) | nein (3733 ms) |
| RTF < 0,2 | nein (0,255) | nein (0,820) |
| WER < 5 % | nein (6,64 %) | ja (4,85 %; knapp, ASR-abhängig) |

Hinweis: Bei Satz-Streaming ohne Teil-Audio ist TTFA die ganze Synthesedauer; die TTFA-Zeile misst das nicht anders.

## Nicht gemessen (mit Grund)

- **Phonon 31m-en:** in diesem Lauf nicht gemessen, weil das Ausführen der heruntergeladenen `ptts.exe` in dieser Umgebung
  gesperrt ist (Owner-Freigabe P2L steht aus).
- **Phonon 40m-de:** Lizenz eingeschränkt (CC BY-NC 4.0, Owner-Entscheidung O1 offen), nicht geladen.
- **Ollama-Speech:** unter Windows nicht unterstützt (Release Notes v0.40.11; lokal 0.34.2).
- **Kokoro:** kein deutsches Modell. **XTTS-v2, F5-TTS:** nicht installiert, nichtkommerzielle Gewichte, nicht getestet.
- Blindhörtest: Paket erzeugt (40 Proben, Seed 20261026, Schlüssel getrennt), **nicht bewertet**; es gibt keine Natürlichkeitswerte.

## Grenzen

- **Debug-Build** der App. Die Synthese läuft in externen Prozessen (Piper-Subprozess, Fish-Python-Server), die Latenzen
  hängen daher kaum am Rust-Build; Aufruf-, Warte- und Dateiaufwand der App sind aber nicht Release-typisch.
- **Fish braucht 1,4 bis 15,6 s je Satz;** lange Sätze bestimmen das P95. Die Fish-Zahlen gelten für diesen Rechner, die Standardstimme
  ohne Referenz und eine geteilte GPU.
- **VRAM** ist der Gesamtwert der GPU (Delta zum Start), nicht je Prozess. **RAM** ist der Working Set des Prozessbaums der
  App; ein fremd gestarteter Server wäre nicht enthalten (hier startete die App den Fish-Server selbst). Prozesse zwischen zwei
  Samples fehlen.
- **TTFA = Gesamtdauer** (je Satz ein Teilstück); Streaming-Aussetzer werden nicht gemessen.
- **WER/CER sind ein ASR-Proxy:** ASR-Fehler fließen ein (z. B. „DSGVO“ als „DSGV EU“), E-Mail/URL und Zahlen leiden an der
  Schreibweise. Die App-CLI hat keine Sprachoption; Stichprobe von 5 Hypothesen je Anbieter war deutsch. Nur Werte desselben
  ASR-Stands vergleichen. 12 Sätze × 30 Läufe belegen Größenordnungen, keine Signifikanz.
- Einzelmessung eines Tages, keine Wiederholung über Tage; Last des Rechners (Browser, Teams, Installiert-App) lief mit.

## Abweichungen vom Plan

- Erster Fish-Versuch (2 min) scheiterte: Der Harness deckelte App samt Fish-Server mit 16 GB (Job-Objekt); der Server braucht beim
  Laden kurzzeitig ~48 GB Commit und starb mit `0xc0000005`. Korrektur: `memory_limit_mb = 57344` nur für Fish in
  `default.toml` (Test `test_fish_job_cap_leaves_room_for_the_model_load`); der Fehlversuch liegt nur im Rohdatenordner.
- `report` brach bei `app-cli`-ASR mit `KeyError: 'version'` ab: behoben (`_asr_label`, Test
  `test_report_after_app_cli_asr_has_no_version_field`).
- Die Statuszeilen stammen aus einem Messlauf ohne Messung (`configs/status-2026-10-26.toml`), weil der Bericht nur Konfigurationen
  kennt, die in einem Lauf vorkamen; dort steht Phonon 31m-en auf „nicht getestet“ (in `default.toml` bleibt es `run`).
- Hörtest-Paket mit `--subset full` (40 statt 24 Proben).

## Reproduktion

```
export LVA_BENCH_DIR=D:/lv-build/lva-bench; E=D:/lv-build/target-dokumente/debug/local-voice-ai.exe; B=scripts/bench/tts/tts_bench.py
python $B run --source app --exe $E --configs piper-thorsten-high-cpu --subset smoke --warm 30 --cold 3 --run-id 2026-10-26-piper-lat
python $B run --source app --exe $E --configs fish-s2-pro-cuda --subset smoke --warm 30 --cold 3 --run-id 2026-10-26-fish-lat
python $B run --source app --exe $E --configs piper-thorsten-high-cpu --subset full --warm 1 --cold 0 --run-id 2026-10-26-piper-full
python $B run --source app --exe $E --configs fish-s2-pro-cuda --subset full --warm 1 --cold 0 --run-id 2026-10-26-fish-full
python $B asr --run 2026-10-26-piper-full --engine app-cli --exe $E   # danach dasselbe für fish-full; nie parallel zu einer Synthese
python $B run --config scripts/bench/tts/configs/status-2026-10-26.toml --run-id 2026-10-26-status
python $B report --runs 2026-10-26-piper-lat 2026-10-26-piper-full 2026-10-26-fish-lat 2026-10-26-fish-full 2026-10-26-status --publish --date 2026-10-26
python $B listening-kit --run 2026-10-26-piper-full 2026-10-26-fish-full --subset full --max-per-config 20 --seed 20261026 --out <ordner> --key-out <schluessel-ordner>
```

Dauer: Piper-Latenz 7 min 28 s, Fish-Latenz 75 min, Piper/Fish voll 2 min 23 s / 9 min 2 s, ASR je ca. 12 min. Vor jedem Lauf RAM-Gate
(`memgate`: ≥ 20 GB Commit, ≥ 12 GB RAM frei). Vergleich nur bei gleichem Korpus-Hash, ASR-Stand und Host.
