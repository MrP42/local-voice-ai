# Schnittstelle App ↔ Harness (`tts-bench-row/1`)

Vertrag zwischen dem Rust-Teil (Paket P3: `--tts-bench`) und dem Python-Harness (Paket P5). Der Harness ist gegen
dieses Schema implementiert und prüft jede Zeile (`ttsbench/rowschema.py`); `--source fake` erzeugt dieselben Dateien
ohne App.

## Aufruf (je Konfiguration seriell, nie parallel)

```
<exe> --tts-bench --tts-provider <id> [--tts-voice <v>] --tts-lang de --bench-corpus <jsonl>
      [--bench-subset smoke] --bench-warm <n> --bench-cold <n> --bench-out <dir> --json
```

- `--bench-corpus`: JSONL-Korpus (Felder `id`, `text`, …, siehe `corpus/README.md`); `--bench-subset smoke` = nur Sätze mit
  `"smoke": true`, ohne Angabe alle.
- `--bench-warm <n>` / `--bench-cold <n>`: Läufe **je Satz** (n × Sätze Synthesen je Art).
  - `cold`: frischer Zustand (Anbieter-Prozess bzw. Modell neu gestartet/geladen); `process_start_ms` = Zeit bis der Anbieter
    bereit ist.
  - `warm`: Anbieter geladen und aufgewärmt (eine Synthese vorab ohne Zeile).
- `--bench-out <dir>`: schreibt `rows.jsonl` (eine Zeile je Synthese, **auch Fehlschläge**), `env.json` und die WAVs.
  `wav_path` ist relativ zu `<dir>` (oder absolut). Mindestens `run_idx 0` jedes Satzes soll als WAV vorliegen
  (ASR, Hörtest); weitere Läufe dürfen `wav_path: null` haben.
- `--json`: eine JSON-Zusammenfassung auf stdout (der Harness liest `rows.jsonl`, nicht stdout).
- Rückgabewert 0, sobald Zeilen geschrieben wurden (auch bei einzelnen Fehlschlägen); ungleich 0, wenn gar nichts gemessen
  werden konnte (Anbieter fehlt, Lizenzsperre, Konfigurationsfehler). Dann gibt `stderr` den Grund.
- Der Harness startet das Binary in einem Job-Objekt (RAM-Deckel, BelowNormal) und beendet es bei Zeitüberschreitung über
  die PID (nie `taskkill /IM`).

## Zeile (`row.schema.json`)

| Feld | Typ | Bedeutung |
|---|---|---|
| `schema` | `"tts-bench-row/1"` | Version |
| `provider`, `voice`, `language` | string, string\|null, string | Anbieter-ID, Stimme (null = Standard), Sprache |
| `utterance_id` | string | ID des Korpussatzes |
| `run_kind` | `cold` \| `warm` | siehe oben |
| `run_idx` | int ≥ 0 | 0-basiert je (Satz, Art) |
| `ok` | bool | Synthese erfolgreich |
| `error_kind`, `error` | string\|null | bei `ok=false` Pflicht; `error_kind` = `TtsErrorKind` (unavailable, invalid_config, license_blocked, privacy_blocked, timeout, cancelled, bad_request, backend, network) |
| `process_start_ms` | number\|null | nur `cold` |
| `ttfa_ms` | number\|null | Anfrage bis erstes Audio. Bei Satz-Streaming (ein Chunk) gleich `total_ms` |
| `total_ms` | number\|null | Anfrage bis letztes Audio (Synthesezeit) |
| `audio_ms` | number\|null | Dauer des erzeugten Audios |
| `rtf` | number\|null | `total_ms / audio_ms` (der Harness rechnet selbst nach und vermerkt Abweichungen > 2 %) |
| `sample_rate`, `channels`, `encoding` | int, int, string | Format der Chunks (`pcm16le`, `f32le`, `wav`) |
| `chunk_count` | int\|null | Anzahl Teil-Audios; 1 = kein echtes Streaming |
| `chunk_gaps_ms` | number[]\|null | Abstände zwischen Chunk-Ankünften (Länge `chunk_count − 1`) |
| `max_gap_ms` | number\|null | Maximum davon (Aussetzer-Risiko) |
| `wav_path` | string\|null | WAV dieser Synthese |
| `started_at` | string | UTC, ISO 8601 |

Bei `ok=true` sind `ttfa_ms, total_ms, audio_ms, rtf, sample_rate, channels, encoding, chunk_count` Pflicht (nicht null),
bei `ok=false` `error_kind` und `error`. Unbekannte Zusatzfelder sind erlaubt und werden ignoriert. Ungültige Zeilen
werden im Bericht gemeldet und nicht gewertet.

Beispiel (warm, Satz-Streaming):

```json
{"schema":"tts-bench-row/1","provider":"piper","voice":"de_DE-thorsten-high","language":"de","utterance_id":"de-v1-alltag-001","run_kind":"warm","run_idx":0,"ok":true,"error_kind":null,"error":null,"process_start_ms":null,"ttfa_ms":92.4,"total_ms":92.4,"audio_ms":3190.0,"rtf":0.029,"sample_rate":22050,"channels":1,"encoding":"pcm16le","chunk_count":1,"chunk_gaps_ms":[],"max_gap_ms":0.0,"wav_path":"wavs/de-v1-alltag-001_warm0.wav","started_at":"2026-10-26T12:00:00.123Z"}
```

## `env.json` (`env.schema.json`)

`{"schema":"tts-bench-env/1","provider":"piper","provider_version":"1.2.0","model":"de_DE-thorsten-high.onnx","device":"cpu"}`
Pflicht sind `schema` und `provider`. `device` (`cpu`, `cuda`, `metal`, `remote`) wird gegen die Erwartung der Konfiguration
geprüft; eine Abweichung steht im Bericht, gemessen wird trotzdem.
