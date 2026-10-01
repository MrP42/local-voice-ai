# P7b – QG3 Performance: 60-min-Besprechung, Stopp → Enddurchlauf + KI-Notizen

**Gate (GOAL.md):** 60-min-Besprechung auf RTX 4090 → Enddurchlauf + KI-Notizen ≤ 3 min nach Stopp; CPU-only-Pfad funktioniert (Messung dokumentiert).

**Ergebnis:**
- **GPU: erfüllt mit Qwen3.5-9B als Notizmodell: 136,7 s** (Enddurchlauf 94,7 s + KI-Notizen 42,1 s).
- **Mit dem auf diesem Rechner eingestellten Gemma 4 E4B nicht erfüllt: 201 s bzw. 217 s.** Ursache ist ein Kontextüberlauf im ersten Map-Block (Befund 1): 4 abgeschnittene Antworten, danach wird der Block verworfen.
- **CPU-only: funktioniert.** Ohne GPU bleibt nach der „auto“-Regel das Live-Transkript das Endtranskript; Sprecher und KI-Notizen laufen auf der CPU. Hochgerechnet auf 60 min: TranscriptFinal nach ≈ 1,5 min, KI-Notizen nach ≈ 30 min (grobe Schätzung).

Stand 30.09.2026, Release-Build `feat/granola-p7b` (`cargo build --release --features gpu-vulkan`), RTX 4090, i9-13900K, 64 GB.

## Weg

`local-voice-ai.exe --simulate-meeting --scene … --final-model <m> --notes [--notes-model <id>] --json --out …`

Die Aufnahme besteht aus den synthetischen Szenen: Mikrofon mit Lautsprecher-Echo plus Systemton, AEC an. Für 60 min laufen `(scene1, scene2, scene3) × 3 + scene1` = **61,8 min** hintereinander (648 Äußerungen, 3 Stimmen). Die Simulation speist die Aufnahme ein und leert die Live-Pipeline. Das ist der Moment, in dem der Nutzer Stopp drückt. Danach laufen dieselben Aufträge wie nach einem echten Stopp:
- `final_pass::run_job` mit der App-Umgebung: VAD-Neusegmentierung, Sprecher (Sortformer) vor dem End-STT, End-STT, Wortzuordnung, Speichern, `TranscriptFinal`.
- `notes::enhance::enhance_meeting` mit der Entscheidung und Vorlage des Auto-Laufs (neuer CLI-Haken `--notes`, `lib.rs:1082` `simulate_notes`).

Gemessen wird die Wanduhr des Auftrags (`final.report.wall_ms`) plus die Wanduhr der KI-Notizen (`notes.ms`), inklusive Start des llama-servers.
Werkzeug: `koordination/granola-besprechungen/tools/p7b-qg3.ps1 -Variant gpu-qwen|gpu-auto|cpu-auto|cpu-qwen [-NotesModel …] [-CpuPercent …]`. Es arbeitet mit Job-Objekt (RAM-Deckel, BelowNormal, CPU-Deckel, Kill-on-close), RAM-Start-Gate ≥ 10 GB, Zeitlimit und den Sandboxen `LVA_MEETINGS_DIR` und `HF_HOME`. `HF_HOME` enthält Hardlinks der STT-Modelle, es gab keinen Download.

## Befehl → Ergebnis

| Lauf (Rohdaten `abnahme/…`) | Audio | Enddurchlauf | Laden / STT (RTF) | Sprecher | **Stopp → TranscriptFinal** | KI-Notizen | **Stopp → Notizen** | ≤ 180 s |
|---|---|---|---|---|---|---|---|---|
| `p7b-qg3-gpu-qwen-notes-qwen35.json` | 61,8 min | Qwen3-ASR 1.7B Q5_K_M, Vulkan | 1,3 s / 63,1 s (61×) | 11,2 s | **94,7 s** | 42,1 s, Qwen3.5-9B, 3 Schritte, 0 Fehlversuche, 20 Einträge mit Quelle | **136,7 s** | **ja** |
| `p7b-qg3-gpu-qwen.json` (CPU-Deckel 80 %) | 61,8 min | Qwen3-ASR 1.7B, Vulkan | 3,8 s / 75,9 s (51×) | 19,3 s | 118,1 s | 83,3 s, Gemma 4 E4B, **4 abgeschnittene Antworten, Block 1/2 verworfen** | 201,3 s | nein |
| `p7b-qg3-gpu-auto.json` | 61,8 min | `auto` → Whisper large-v3 Q5_K_M, Vulkan | 3,2 s / 97,1 s (40×) | 11,4 s | 130,7 s | 86,3 s, Gemma 4 E4B, 4 abgeschnitten, Block 1/2 verworfen | 217,0 s | nein |
| `p7b-qg3-cpu-auto.json` (ohne `ggml-vulkan.dll`, LLM ohne CUDA-Gerät) | 12,8 min | `auto` → `Keep(CpuOnly)`, Live = Ende | – | 19,0 s (CPU) | **19,0 s** | 391,1 s, Gemma 4 E4B auf CPU, 1 Schritt, 58 Einträge | 410,1 s | – |
| `p7b-qg3-cpu-qwen.json` (wie oben) | 12,8 min | Qwen3-ASR 1.7B **auf der CPU** (bewusste Wahl) | 1,4 s / 212,8 s (3,8×) | 19,0 s | 237,0 s | – | – | – |

Tabelle als CSV: `abnahme/p7b-qg3-summary.csv`. Volle Simulations-JSONs mit allen Segmenten und die Logs liegen unter `%LOCALAPPDATA%\lva-bench\results\p7b\p7b-qg3-*` (nicht im Repo).
Alle Läufe endeten mit Exit 0, ohne übrig gebliebene Prozesse und mit Status `ready`. Job-Spitze (App + llama-server): 7,3 bis 11,1 GB (GPU), 2,9 bis 3,9 GB (CPU).

**Zusammensetzung von 94,7 s** (bester GPU-Lauf): STT 63,1 s + Sprecher 11,2 s + Laden 1,3 s. Die übrigen 19,1 s gehen an die Neusegmentierung beider Spuren (Silero-VAD auf der CPU), die Wortzuordnung und das Speichern von 601 Segmenten. Der Anteil auf der CPU (VAD, Sprecher-Nachbearbeitung) reagiert auf den CPU-Deckel: Mit 80 % statt 100 % dauerte der Enddurchlauf 118 s statt 95 s.

**CPU-only hochgerechnet auf 61,8 min** (linear, Faktor 4,83; nicht gemessen):
- „auto“ (Standard): TranscriptFinal ≈ 19 s × 4,83 ≈ **1,5 min**. KI-Notizen ≈ 391 s × 69,7k/16,0k Zeichen ≈ **28 min** (zwei Map-Blöcke plus Reduce; die Prompt-Verarbeitung wächst eher überlinear, die Zahl ist also eine Untergrenze).
- Qwen3-ASR 1.7B auf der CPU (nur bei ausdrücklicher Wahl): 61,8 min / 3,8 ≈ 16,3 min STT + ≈ 1,5 min Sprecher ≈ **18 min**. Das deckt sich mit bench.md (4,3× → „≈ 14 min Rechenzeit je Stunde“).

## Befunde

1. **Wichtig – KI-Notizen mit Gemma 4 E4B: Der erste Map-Block sprengt den Kontext, die erste Hälfte der Besprechung fehlt in den Notizen.**
   Fundstelle: Blockgröße aus `notes/enhance.rs` `single_pass_budget_chars_for` (Log: „Einzeldurchlauf bis 45513“ Zeichen) bei `n_ctx_slot = 16384`.
   Szenario, gelesen im Serverlog `%LOCALAPPDATA%\de.wolffappliedai.localvoiceai\llm\server.log` direkt nach dem Lauf `gpu-auto` (die Datei wird bei jedem Serverstart neu geschrieben, die Werte stehen deshalb nur hier; nachstellbar mit `p7b-qg3.ps1 -Variant gpu-auto`): Block 1 hat einen Prompt von **15.140 Token**. Es bleiben 1.244 Token für die Antwort, dann kommt `truncated = 1` → „kein gültiges JSON“. Das passiert 2 Versuche × 2 Wiederholungen lang, danach „Block 1/2 nicht ausgewertet – das Ergebnis entsteht aus den übrigen“. Block 2 (8.066 Token Prompt) läuft durch.
   Folgen: rund 38 s verlorene Rechenzeit. Die KI-Notizen einer Stunde decken still nur die zweite Hälfte ab. `ok: true` meldet das nicht.
   Vermutete Ursache: Die Kalibrierung aus P1g (3,35 Zeichen/Token, Überhang ~750) stammt von Qwen3.5. Gemmas Tokenizer liefert auf diesem deutschen Transkript rund 3,0 Zeichen/Token (45.513 / 15.140), und für die Antwort bleibt keine Reserve. Mit Qwen3.5-9B trat der Fehler nicht auf (0 Fehlversuche).
   Kein kleiner Fix in P7b: Budget je Modell oder Antwortreserve berühren die P1g-Kalibrierung. Vorschlag: Nacharbeitspaket „Map-Blockgröße aus Kontext − Antwortreserve je Modell-Tokenizer“, dazu eine Warnung, wenn ein Block verworfen wurde.
   Geschätzt ohne den Fehler (nicht gemessen): Gemma-Notizen ≈ 70 s, also Stopp → Notizen ≈ 165 s mit Qwen3-ASR und ≈ 200 s mit Whisper large-v3.
2. **Klein – „auto“ nimmt Whisper large-v3, obwohl Qwen3-ASR 1.7B auf der GPU schneller war** (RTF 61 bzw. 51 gegen 40). Die WER-Gleichwertigkeit steht in bench.md. Die Reihenfolge `AUTO_GPU_CANDIDATES` (`final_pass.rs:89`) kostet bei 60 min rund 35 s. Das ist ein Produktentscheid, keine Änderung in P7b.

## Grenzen

- Das **Live-Modell** in allen Läufen war Parakeet ONNX int8 (`meeting_model = "parakeet-tdt-0.6b-v3"` ist in den Einstellungen dieses Rechners ausdrücklich gesetzt), nicht der P2g-Standard GGUF. Auf die Zeit nach dem Stopp wirkt das nur im CPU-Fall („Live = Ende“, dort 7,9 % statt 5,5 % FLEURS-WER).
- Die Simulation speist schneller als in Echtzeit ein. Die Zeit nach dem Stopp beginnt erst nach dem Leeren der Pipeline, das ändert also nichts. Nicht enthalten ist der Indexer, der in der App nach `TranscriptFinal` parallel die Vektoren baut (QG5: 3,8 s für 10,7 min).
- „CPU-only“ ist nachgestellt: EXE-Kopie ohne `ggml-vulkan.dll` (transcribe-cpp meldet nur das CPU-Gerät) und `CUDA_VISIBLE_DEVICES=-1` für den llama-server („Backend cuda sieht kein Gerät“; die CPU-Laufzeit von llama.cpp ist auf diesem Rechner nicht installiert). Die Kontextgröße des LLM wird weiter aus dem VRAM geschätzt.
- Messbedingungen: Job-Objekt BelowNormal. Parallel lief ein fremder `pwsh`-Prozess mit hoher CPU-Last, beim ersten Lauf (`gpu-qwen`, 80 %) zusätzlich ein Cargo-Build (P5f). Die Werte sind eher konservativ. Je Variante gibt es einen Lauf, die Streuung ist nicht gemessen.
- KI-Notizen ohne Notizblock (0 Nutzer-Einträge) und mit der Standardvorlage. Mit Notizblock wird der Prompt etwas länger.

## Nachtrag P1i (30.09.2026): Befund 1 und 2 behoben

Stand: `feat/granola-p1i`, Release-Build `--features gpu-vulkan`, gleicher Aufbau wie oben (`p7b-qg3.ps1 -Variant gpu-auto -CpuPercent 100`, 61,8 min, `auto` nimmt jetzt Qwen3-ASR 1.7B). Rohdaten: `abnahme/p1i-qg3-gemma.json`, `abnahme/p1i-qg3-qwen35.json`, `abnahme/p1i-eval-notes-gemma4.json`, `abnahme/p1i-eval-notes-qwen35.json`.

| Lauf | Enddurchlauf | KI-Notizen | **Stopp → Notizen** | ≤ 180 s | Blöcke |
|---|---|---|---|---|---|
| Gemma 4 E4B (`p1i-qg3-gemma.json`) | 105,5 s | **51,0 s**, 3 Blöcke, Reduce übersprungen (30 Zeilen) | **156,6 s** | **ja** | 0 abgeschnitten, 0 halbiert, 0 verworfen |
| Gemma 4 E4B, zwei weitere Läufe desselben Stands | 106,9 s / 107,3 s | 51,3 s / 51,0 s | 158,1 s / 158,3 s | ja | wie oben |
| Qwen3.5-9B (`p1i-qg3-qwen35.json`) | 109,4 s | 76,7 s, 3 Blöcke, Reduce übersprungen (76 Zeilen) | 186,1 s | **nein** (Enddurchlauf 15 s langsamer als im ersten P7b-Lauf) | 0 abgeschnitten, 0 halbiert, 0 verworfen |
| Vorher (Gemma, P7b) | 130,7 s | 86,3 s, 4 Antworten abgeschnitten, Block 1/2 verworfen | 217,0 s | nein | – |

`--eval-notes` (Standardkontext 16 384, drei Fixtures, Einzeldurchlauf): Gemma 4 E4B Exit 0 (Nutzer 1,000, KI belegt 0,970), Qwen3.5-9B Exit 0 (1,000 / 1,000).

**Was der Fehler wirklich war.** Die Blockgröße allein war es nicht. Ohne Grenze im Prompt läuft die Antwort eines lokalen Modells bei einem echten Block in eine Endlosliste bis zum Kontextende (Gemma: 7 200 Token, `finish_reason: length`; Qwen3.5: zwei von zwei echten 35 000-Zeichen-Blöcken). Ein größerer Antwortplatz hätte das nur verschoben. Deshalb wurde beides gemacht:
- **Budget in Token** (`notes/budget.rs`): Kontext − Antwortreserve (6 144) − Rahmen, Zeichen je Token nur als Rückfall. Der fertige Prompt wird über `/tokenize` des Servers gemessen (Einzeldurchlauf, Blockgröße aus dem Verhältnis am ganzen Transkript, jeder Block vor dem Senden); Blöcke sind gleichmäßig statt „voll, voll, Rest“.
- **Antwort begrenzt:** lokale map-/Reduce-Prompts nennen eine Höchstzahl KI-Einträge (1 je 3 500 Zeichen, mindestens 4). Gemma, echter Block von 9 200 Token: ohne Grenze 7 185 Token und `length` (zweimal reproduziert), mit Grenze 2 800 bis 4 350 Token und `stop` (in allen Läufen, auch in den QG3-Läufen). Qwen3.5 ist unstetiger: ohne Grenze 2 100 Token im Testblock, aber beide 35 000-Zeichen-Blöcke des QG3-Laufs ohne Grenze wurden abgeschnitten; mit Grenze 3 bis 10 beendete jede Antwort regulär (4 100 bis 6 600 Token), Grenze 2 lief weg. Die Zahl verankert das Modell: Qwen schreibt mit Grenze mehr Einträge (76 Zeilen für 60 Minuten statt der 20 des ersten P7b-Laufs) und braucht 76,7 s statt 42,1 s. Der frühere Wert stammt aus einem Lauf ohne Grenze, der (vermutlich zufällig) kurz blieb.
- **Reduce nur bei kurzer Liste** (höchstens 24 Zeilen): er gibt alle Zeilen noch einmal aus (Gemma: 33 Zeilen = 3 977 Token = 25 s, im ersten Anlauf 51 Zeilen = 8 939 Token = 57 s und abgeschnitten). Sonst der deterministische Zusammenschluss, der jeden Eintrag behält.
- **Nie still verwerfen:** abgeschnittene oder ungültige Antwort → Block halbieren (höchstens zwei Stufen), nicht wiederholen; nur was danach noch scheitert, steht in `chunks_failed` (die Oberfläche zeigt dafür „X von Y Abschnitten … Die Notizen sind unvollständig“), der Reduce bekommt die fehlenden Zeiten genannt, und die Simulation meldet `incomplete`.

**Befund 2 (auto nimmt Whisper large-v3):** `AUTO_GPU_CANDIDATES` nennt jetzt Qwen3-ASR 1.7B zuerst. Enddurchlauf gemessen 94,8 bis 109,4 s (STT 63 bis 77 s, RTF 50 bis 61) je nach Rechnerlast bei gleichem Aufbau; das ist die Streuung, die das Gate im Ergebnis mit trägt. Die Notizen der Gemma-Läufe bleiben stabil bei 51 s, das Gate hat damit bei langsamem Enddurchlauf rund 23 s Luft, bei schnellem rund 34 s.
