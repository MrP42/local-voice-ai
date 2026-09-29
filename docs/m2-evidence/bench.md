# M2 Benchmark: Modellgüte je Satz (FLEURS-de)

Grundlage für die Wahl des Enddurchlauf-Modells (AK4: Enddurchlauf ≤ 6 % WER auf FLEURS-de).
Werkzeuge: `scripts/bench/make_corpus.py` (Korpus), `scripts/bench/sentence_bench.py` (Messung).
Namensnennung der Daten: [ATTRIBUTION.md](ATTRIBUTION.md).

## Ergebnis

<!-- bench:table:begin -->
<!-- bench:table:end -->

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

SYNTH_TABLE

Je Szene: `mic.wav` (nur Nahsprache), `system.wav` (Gegenseite = Referenzsignal), `mic_echo.wav` (Lautsprecher-Echo -6 dB,
RT60 0,3 s, Rauschen -55 dB, Verzögerung je Szene, Mischer aus dem Spike `mix.py`), `mic_echo_hard.wav` (0 dB, tanh-Verzerrung,
150 ppm Drift, Verzögerungssprung 50 → 170 ms bei 50 %, Spike `mix_hard.py`) und `reference.json` (Sprecher, Kanal, Text,
`start_ms`/`end_ms` je Äußerung, Referenztext je Kanal, Wörter, die nur die Gegenseite spricht).

## Grenzen

- **Gelesene Sprache:** FLEURS sind vorgelesene, gut artikulierte Einzelsätze (Wikipedia-nah) in ruhiger Umgebung.
  Echte Besprechungen (Ins-Wort-Fallen, Dialekt, Nebengeräusche, Fachjargon) liegen erfahrungsgemäß deutlich darüber;
  die Werte taugen für den **Modellvergleich** und AK4, nicht als Erwartung für reale Besprechungen.
- Je Modell nur **eine Aufnahme je Satz** (Sprecherwechsel/Geschlecht nicht kontrolliert); 240 Sätze / rund 4.000 Referenzwörter
  geben bei ~5 % WER eine Unsicherheit von etwa ±1,0 Prozentpunkt (95 %). Unterschiede unter einem Prozentpunkt sind kein Befund.
- Die strenge Normierung zählt Ziffer-vs.-Zahlwort und Bindestrich-Schreibweisen als Fehler; Literaturwerte (z. B. Parakeet ~5 % auf
  FLEURS-de) nutzen meist andere Normalisierer und sind nicht 1:1 vergleichbar.
- Die synthetischen Szenen sind TTS: gleichmäßige Stimmen, linearer Raum. Ein realer Lautsprecher-Test bleibt in der Abnahme (P2c).
- Whisper und die Spike-Zeilen laufen ohne Sprachvorgabe (Automatik) bzw. mit den Standardoptionen der Bibliothek.
