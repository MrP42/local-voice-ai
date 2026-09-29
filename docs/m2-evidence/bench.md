# M2 Benchmark: Modellgüte je Satz (FLEURS-de)

Grundlage für die Wahl des Enddurchlauf-Modells (AK4: Enddurchlauf ≤ 6 % WER auf FLEURS-de).
Werkzeuge: `scripts/bench/make_corpus.py` (Korpus), `scripts/bench/sentence_bench.py` (Messung).
Namensnennung der Daten: [ATTRIBUTION.md](ATTRIBUTION.md).

## Ergebnis

<!-- bench:table:begin -->
_Stand 2026-09-29, automatisch aus den Ergebnis-JSON erzeugt._

| Modell | Engine | Gerät | Sätze | WER FLEURS-de | RTF | Hinweise |
|---|---|---|---|---|---|---|
| Parakeet TDT 0.6B v3 int8 | transcribe-rs (ONNX) | CPU | 240 | **7,86 %** (409/5206) | 16,5× | WER weich 7,38 %; Laden 1,2 s; 1 Sätze weichen von der Rust-Wertung ab |
| Whisper large-v3-turbo Q8 | transcribe-cpp | CPU | 240 | **8,87 %** (462/5206) | 1,3× | WER weich 8,39 %; Laden 0,9 s; 2 Sätze weichen von der Rust-Wertung ab |
| Qwen3-ASR 0.6B Q8 | transcribe-cpp | CPU | 240 | **7,16 %** (373/5206) | 5,3× | WER weich 6,97 %; Laden 1,0 s; 2 Sätze weichen von der Rust-Wertung ab |
| Qwen3-ASR 1.7B Q5_K_M | transcribe-cpp | CPU | 240 | **4,17 %** (217/5206) | 4,3× | WER weich 4,21 %; Laden 1,3 s; 3 Sätze weichen von der Rust-Wertung ab |
| Parakeet TDT 0.6B v3 GGUF Q8 | transcribe-cpp (Spike-Build) | CPU | 240 | **5,55 %** (289/5206) | 11,3× | Spike-Build, nicht App (Modell nicht im App-Katalog installiert); WER weich 5,46 %; Laden 0,8 s |
| Parakeet TDT 0.6B v3 GGUF Q8 | transcribe-cpp (Spike-Build) | CUDA RTX 4090 | 240 | **5,51 %** (287/5206) | 87,7× | Spike-Build, nicht App; WER weich 5,36 %; Laden 0,6 s |
| Whisper large-v3-turbo Q8 | transcribe-cpp (Spike-Build) | CUDA RTX 4090 | 240 | **5,61 %** (292/5206) | 114,9× | Spike-Build, nicht App; WER weich 5,15 %; Laden 0,8 s |
| Whisper large-v3 Q5_K_M | transcribe-cpp (Spike-Build) | CUDA RTX 4090 | 240 | **4,65 %** (242/5206) | 20,6× | Spike-Build, nicht App; WER weich 4,28 %; Laden 0,9 s |
| parakeet-tdt-0.6b-v3-Q8_0.gguf | transcribe-cpp | CPU | 240 | **5,51 %** (287/5206) | 11,3× | WER weich 5,40 %; Laden 0,8 s; 1 Sätze weichen von der Rust-Wertung ab |
<!-- bench:table:end -->

## Folgerung

- **Live-Mitschrift: Parakeet TDT 0.6B v3 als GGUF Q8 (transcribe-cpp) statt ONNX int8.** Gleiches Modell, aber
  5,55 % statt 7,86 % WER auf der CPU (Abstand über zwei Prozentpunkte, also über der Messunsicherheit) bei 11,3× Echtzeit –
  genug Reserve für den Live-Pfad ohne GPU. Voraussetzung: das GGUF-Modell kommt in den App-Katalog; die Zeile stammt aus
  dem Spike-Build mit derselben Bibliothek, nicht aus der App.
- **Enddurchlauf: Qwen3-ASR 1.7B Q5_K_M auf der CPU**, 4,17 % WER bei 4,3× Echtzeit – erfüllt AK4 (≤ 6 %) schon mit dem
  heutigen CPU-Backend der App (eine Stunde Besprechung ≈ 14 min Rechenzeit). **Mit GPU: Whisper large-v3 Q5_K_M**
  (4,65 %, 20,6×) als gleichwertige Alternative; der Unterschied zu Qwen3 1.7B liegt innerhalb der Unsicherheit, die Wahl
  richtet sich also nach Gerät und Laufzeit, nicht nach der WER.
- **Nicht gewählt:** Parakeet ONNX int8 (7,86 %) und Qwen3-ASR 0.6B (7,16 %) verfehlen AK4; Whisper large-v3-turbo Q8
  auf der App-CPU ist mit 1,3× zu langsam und mit 8,87 % zu ungenau.
- **Grenzen der Folgerung:** Alle GPU-Zeilen stammen aus dem Spike-Build; ein CUDA-Backend im Release ist ein eigenes Paket.
  Qwen3-ASR 1.7B wurde nicht auf der GPU gemessen. Whisper-turbo liegt auf der App-CPU (8,87 %) deutlich schlechter als im
  Spike auf CUDA (5,61 %) – gleiche Quantisierung, Ursache (Decoding-Optionen/Sprachvorgabe der App-CLI vs. Bibliotheks-Standard)
  ist nicht geklärt; bis dahin ist die turbo-CPU-Zeile kein Urteil über das Modell. Speicherbedarf je Modell wurde nicht gemessen.

## Methode

- **Stichprobe:** FLEURS `de_de`, Teilmenge `test` (862 Aufnahmen, 347 verschiedene Sätze). Je Satz-ID die
  **erste Aufnahme in Archivreihenfolge**; das Archiv wird gestreamt und nach 240 verschiedenen Sätzen
  abgebrochen (328 Aufnahmen gelesen, rund 55 min Audio, 4 bis 47 s je Satz). Audio float32 → PCM16, 16 kHz, mono.
  Referenz ist die `raw_transcription` (mit Groß-/Kleinschreibung und Satzzeichen).
- **Normierung:** wie `selftest::normalize_word` in der App: Kleinschreibung, Entfernen von `. , ; : ! ? " ' „ “ » « ( ) [ ]`,
  ä/ö/ü/ß → ae/oe/ue/ss. Zahlwörter werden **nicht** auf Ziffern gefaltet, Bindestriche nicht getrennt.
  Diese strenge Wertung ist die Hauptzahl. „WER weich“ (Hinweise) trennt zusätzlich Bindestriche und Schrägstriche.
- **WER** = Summe aller Wortfehler (Ersetzung + Einfügung + Löschung, Wort-Levenshtein) / Summe aller Referenzwörter
  über alle Sätze (kein Mittel der Satz-WER). Ausfälle einzelner Sätze zählen als komplett falsch und werden ausgewiesen.
  Die Python-Wertung wird je Satz gegen die Rust-Wertung der CLI (`--reference`) gegengeprüft; Abweichungen stehen in den Hinweisen.
- **RTF** ist der Geschwindigkeitsfaktor (× Echtzeit) = Summe Audiosekunden / Summe reiner Transkriptionszeit
  (`best_ms` der CLI, 1 Lauf je Satz). **Ladezeit** wird getrennt geführt (Median je Aufruf; die CLI lädt das Modell
  bei jedem Satz neu, das Spike-Programm einmal je Block von 40 Sätzen).
- **Geräte:** CPU-Zeilen laufen über die Release-CLI der App (Build vom 27.09., nur CPU-Backend). Zeilen mit
  „Spike-Build“ laufen über `C:\Users\wolff\lva-spikes\m2\tcbench` (transcribe-cpp 0.1.3 mit `cuda`, gleiche Bibliothek,
  aber **nicht die App**): sie zeigen, was ein GPU-Backend im Release brächte. `parakeet-gguf-cpu` läuft ebenfalls dort,
  weil das GGUF-Modell nicht im App-Katalog installiert ist (kein Download in die App-Daten).
- **Systemschutz:** ein Messprozess zur Zeit, `BelowNormal`, Job-Objekt mit Speicherdeckel (8 GB, Spike 16 GB) und
  Kill-on-close, RAM-Start-Gate (≥ 3 GB frei), Zeitlimit je Aufruf. Parallel liefen Cargo-Builds anderer Arbeiten:
  RTF-Werte sind deshalb eher konservativ (unter Last gemessen).

## Befehle

```
pip install -r scripts/bench/requirements.txt
python scripts/bench/make_corpus.py fleurs && python scripts/bench/make_corpus.py synth
python scripts/bench/make_corpus.py --check                       # Exit 0 = Korpus vollständig
python scripts/bench/sentence_bench.py --models parakeet-onnx parakeet-gguf-cpu qwen3-0.6b whisper-turbo-q8 \
    whisper-turbo-cuda whisper-large-v3-q5-cuda parakeet-gguf-cuda qwen3-1.7b
python scripts/bench/sentence_bench.py --selftest                 # WER-Aggregation mit bekanntem Ergebnis
python -m pytest scripts/bench -q                                 # Skript-Tests (kein Netz, keine Modelle)
```

Ablage: `%LOCALAPPDATA%\lva-bench\` (`fleurs\`, `synth\`, `results\*.json` mit allen Satzzeilen). Nie im Repo, nie im Installer.

## Synthetischer Mehrsprecher-Korpus (für `--simulate-meeting`, AK5/AK6)

Drei Szenen (SAPI-Stimmen Hedda = Ich/Mikrofon, Stefan und Katja = Gegenseite/Loopback), erzeugt aus Vorlagen mit
Fachbegriffen und englischen Einsprengseln, `make_corpus.py synth` (deterministisch, gleiche Seeds = gleiche Texte):

| Szene | Titel | Sprecher | Dauer | Äußerungen | Wörter | Überlappungen Ich/Gegenseite |
|---|---|---|---|---|---|---|
| `scene1_status` | Projektstatus Rechenzentrum-Umzug | Hedda, Stefan | 4,3 min | 45 | 567 | 3 |
| `scene2_angebot` | Angebots- und Vertragsbesprechung | Hedda, Stefan, Katja | 6,4 min | 63 | 813 | 7 |
| `scene3_sprint` | Sprint-Planung und Release-Review (viel Englisch) | Hedda, Katja, Stefan | 8,5 min | 93 | 1181 | 30 |

Je Szene: `mic.wav` (nur Nahsprache), `system.wav` (Gegenseite = Referenzsignal), `mic_echo.wav` (Lautsprecher-Echo -6 dB,
RT60 0,3 s, Rauschen -55 dB, Verzögerung je Szene, Mischer aus dem Spike `mix.py`), `mic_echo_hard.wav` (0 dB, tanh-Verzerrung,
150 ppm Drift, Verzögerungssprung 50 → 170 ms bei 50 %, Spike `mix_hard.py`) und `reference.json` (Sprecher, Kanal, Text,
`start_ms`/`end_ms` je Äußerung, Referenztext je Kanal, Wörter, die nur die Gegenseite spricht).

## Live-Latenz und Live-WER (AK5, P2b2)

**AK5 erfüllt:** Über 12,8 min Aufnahme liegt p95 (Ende der Äußerung → Segment-Event) bei **1.385 ms** (Soll ≤ 5.000 ms).
Gemessen mit der Release-CLI dieses Stands (CPU, Parakeet ONNX wie in der App konfiguriert). Die Szenen laufen im Echtzeit-Takt
durch **denselben** DSP-Thread (Echo-Unterdrückung, Silero-VAD, Segmentierer) und Transkriptions-Worker wie eine Live-Besprechung.

| Lauf | Audio | Modell | Segmente | Latenz p50 / p95 / max | Live-WER gesamt | Ich / Gegenseite | Gegenseite im Ich-Kanal | FLEURS-de |
|---|---|---|---|---|---|---|---|---|
| `-Full`: scene3 + scene1, `mic_echo.wav`, AEC an | 12,8 min | Parakeet TDT 0.6B v3 int8 (ONNX, CPU) | 128 | 960 / **1.385** / 4.604 ms | **6,86 %** (120/1748) | 6,65 % / 7,03 % | 0 % | 7,86 % (240 Sätze) |
| `-Quick`: scene1, `mic_echo.wav`, AEC an | 4,3 min | Parakeet TDT 0.6B v3 int8 (ONNX, CPU) | 41 | 938 / 1.326 / 1.418 ms | 5,29 % (30/567) | 5,97 % / 4,42 % | 0 % | 10,04 % (20 Sätze) |

- **Befehle:** `pwsh apps/local-voice/scripts/m2-bench.ps1 -Full` bzw. `-Quick` (Stand 29.09.2026). Das Skript ruft
  `local-voice-ai.exe --simulate-meeting --realtime --scene <Szene> … --json --out …` in einer `LVA_MEETINGS_DIR`-Sandbox auf und
  danach `scripts/bench/sentence_bench.py` (FLEURS). Rohdaten mit allen Segmentzeiten: `%LOCALAPPDATA%\lva-bench\results\live\`.
- **Latenz** = `emitted_at_ms − vad_end_ms` je `Segments`-Event: `emitted_at_ms` ist die Wanduhr ab Einspeisebeginn, `vad_end_ms`
  das Ende der Äußerung auf der Audio-Achse. Die Blöcke kommen im 30-ms-Takt erst nach ihrer Aufnahmedauer an, wie bei einem
  Capture-Gerät. Perzentile nach Nearest-Rank. Die Zeit bis zur Anzeige in React ist nicht enthalten (erwartet < 50 ms).
- **Zusammensetzung:** Median ≈ 600 ms VAD-Nachlauf + ~350 ms Transkription und Speichern. Das Maximum von 4,6 s ist der einzige
  Schnitt an der Höchstlänge (15 s) im Lauf: Der Segmentierer schneidet im letzten Viertel an der leisesten Stelle, entscheidet
  aber erst bei 15 s. Für Monologe über 11 s ist das der schlimmste Fall, er liegt weiter unter 5 s.
- **`vad_end_ms` ist abgeleitet** (Blockende minus 300 ms Nachlaufpolster), weil der Harness die Metadaten des Segmentierers
  nicht sieht. Für Segmente, die mit dem VAD-Nachlauf enden, ist der Wert exakt. Beim Schnitt an der Höchstlänge und beim letzten
  Segment liegt er bis zu 300 ms zu früh, die Latenz ist dort also höchstens 300 ms zu hoch (konservativ).
- **Live-WER** = Summe der Wortfehler beider Kanäle / Summe der Referenzwörter. Berechnet in Rust über
  `selftest::SelfTestResult::build` (eine WER-Implementierung, Normierung wie oben) gegen `reference_text` aus `reference.json`.
  Sie enthält Segmentierung, Echo-Unterdrückung und Halluzinationsfilter, ist also nicht mit der Satz-WER vergleichbar.
- **Parakeet GGUF Q8 (Befund B1) nicht gemessen:** Das Modell ist nicht im App-Katalog installiert. Nachholen mit
  `m2-bench.ps1 -Full -Model <Katalog-ID>`, sobald es dort liegt.
- **Systemschutz:** Die App läuft im Job-Objekt (BelowNormal, 8 GB Speicherdeckel, 50 % CPU-Hard-Cap, Kill-on-close) mit
  RAM-Start-Gate (≥ 4 GB frei) und Zeitlimit (1,5 × Audiodauer + 5 min). Es läuft immer nur ein Messprozess.

## Grenzen

- **Gelesene Sprache:** FLEURS sind vorgelesene, gut artikulierte Einzelsätze (Wikipedia-nah) in ruhiger Umgebung.
  Echte Besprechungen (Ins-Wort-Fallen, Dialekt, Nebengeräusche, Fachjargon) liegen erfahrungsgemäß deutlich darüber;
  die Werte taugen für den **Modellvergleich** und AK4, nicht als Erwartung für reale Besprechungen.
- Je Modell nur **eine Aufnahme je Satz** (Sprecherwechsel/Geschlecht nicht kontrolliert); 240 Sätze / 5.206 Referenzwörter
  geben bei ~5 % WER eine Unsicherheit von etwa ±1,0 Prozentpunkt (95 %). Unterschiede unter einem Prozentpunkt sind kein Befund.
- Die strenge Normierung zählt Ziffer-vs.-Zahlwort und Bindestrich-Schreibweisen als Fehler; Literaturwerte (z. B. Parakeet ~5 % auf
  FLEURS-de) nutzen meist andere Normalisierer und sind nicht 1:1 vergleichbar.
- Die synthetischen Szenen sind TTS: gleichmäßige Stimmen, linearer Raum. Ein realer Lautsprecher-Test bleibt in der Abnahme (P2c).
- Whisper und die Spike-Zeilen laufen ohne Sprachvorgabe (Automatik) bzw. mit den Standardoptionen der Bibliothek.
