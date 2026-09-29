# Befunde des Planers — Besprechungen auf Granola-Niveau - lokal, ohne Abo

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B1 — P2b1: Modellwahl nach FLEURS-de-Benchmark (29.09. 13:00)
- Beobachtet (240 Saetze, 5206 Woerter, `docs/m2-evidence/bench.md` auf feat/granola-p2b1): Parakeet v3 ONNX int8 (heutiges Live-Modell der App) 7,86 % WER; Parakeet v3 GGUF Q8 5,55 % (CPU, RTF 11) bzw. 5,51 % (CUDA); Whisper-turbo Q8 CPU 8,87 %, CUDA 5,61 %; Whisper large-v3 Q5 CUDA 4,65 %; Qwen3-ASR 1.7B Q5 CPU 4,17 % (RTF 4,3); Qwen3-ASR 0.6B 7,16 %.
- Konsequenz: Live-Modell fuer Besprechungen = Parakeet v3 GGUF Q8 (nicht ONNX int8) -> neues Paket P2g (Katalog/Default, Latenz pruefen). Enddurchlauf `auto`: GPU -> Whisper large-v3 oder Qwen3-ASR 1.7B (nach P2f messen), CPU -> Qwen3-ASR 1.7B nur mit Hinweis auf Dauer (60 min ~14 min, QG3 nur mit GPU), sonst Live=Ende.
- Status: offen
