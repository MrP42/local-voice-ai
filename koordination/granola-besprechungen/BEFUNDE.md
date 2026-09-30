# Befunde des Planers — Besprechungen auf Granola-Niveau - lokal, ohne Abo

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B1 — P2b1: Modellwahl nach FLEURS-de-Benchmark (29.09. 13:00)
- Beobachtet (240 Saetze, 5206 Woerter, `docs/m2-evidence/bench.md` auf feat/granola-p2b1): Parakeet v3 ONNX int8 (heutiges Live-Modell der App) 7,86 % WER; Parakeet v3 GGUF Q8 5,55 % (CPU, RTF 11) bzw. 5,51 % (CUDA); Whisper-turbo Q8 CPU 8,87 %, CUDA 5,61 %; Whisper large-v3 Q5 CUDA 4,65 %; Qwen3-ASR 1.7B Q5 CPU 4,17 % (RTF 4,3); Qwen3-ASR 0.6B 7,16 %.
- Konsequenz: Live-Modell fuer Besprechungen = Parakeet v3 GGUF Q8 (nicht ONNX int8) -> neues Paket P2g (Katalog/Default, Latenz pruefen). Enddurchlauf `auto`: GPU -> Whisper large-v3 oder Qwen3-ASR 1.7B (nach P2f messen), CPU -> Qwen3-ASR 1.7B nur mit Hinweis auf Dauer (60 min ~14 min, QG3 nur mit GPU), sonst Live=Ende.
- Status: erledigt (P2g 0e9b05d6: GGUF Q8 5,51 % WER, Live p95 2,0 s)

## B2 — P1f: Vorlagenliste bricht fremde Seiten bei null-Antwort (29.09. 13:40)
- Beobachtet: volle Playwright-Suite nach P1f 17 rot (workspace, voices, llm-*): TypeError "reading 'map'" in MeetingNotesSettings, weil die Attrappe dort null liefert.
- Beleg: npx playwright test -> 17 failed; nach Fix 118 passed.
- Konsequenz: `result.data ?? []` in MeetingNotesSettings.tsx und TemplatePicker.tsx (im P1f-Commit).
- Status: erledigt (P1f, d0bc230e)

## B3 — P1e: Qwen 3.5 9B liefert leere Antwort (Denkmodus) (29.09. 14:10)
- Beobachtet: --eval-notes mit Qwen3.5-9B -> llm_failed; das Modell denkt vor der Antwort (Probe "2+3": 623 Token), bei langem Prompt ist der 8192-Kontext voll, bevor JSON entsteht.
- Konsequenz: Paket P1g (Denkmodus aus, Kontext je VRAM). Gemma 4 E4B erfuellt AK3 bereits.
- Status: erledigt (P1g 60c4f50a; Rest: Qwen3.5 bei T=0 in 2 Kurz-Fixtures ohne KI-Eintraege -> KNOWN-LIMITATIONS, Gemma bleibt Standard)

## B4 — P1e: toter llama-server gilt als laufend (29.09. 14:10)
- Beobachtet: Gemma 4 12B loest in der CUDA-Laufzeit "illegal memory access" aus; danach meldet is_serving den toten Server als laufend, alle Folgeaufrufe scheitern sofort.
- Konsequenz: Paket P1h (Lebendpruefung + Neustart).
- Status: erledigt (P1h, f146816e)

## B5 — P4b/P4c: Chat nutzt noch LexicalOnly statt LlamaEmbedder (29.09. 15:00)
- Beobachtet: commands/meeting_chat.rs (P4c) arbeitet mit dem Platzhalter-Embedder; P4b liefert LlamaEmbedder, hat den Tausch aber nicht vorgenommen.
- Konsequenz: Tausch im Paket P4f (Eval AK8 braucht Vektoren).
- Status: erledigt (P4f, b0f18199)

## B6 — P4e: Zitat-Tooltip verdeckt Antworttext (29.09. 15:20)
- Beobachtet: Screenshot abnahme/p4e-chat.png, der Tooltip ueberlagert die Antwortzeile.
- Konsequenz: kleine UI-Korrektur (Tooltip unterhalb/seitlich) im Abnahme-Feinschliff M7.
- Status: erledigt (P6d 95e21c66, abnahme/b6-tooltip.png)

## B7 — P3a: AMI-Pruefteil 15,19 % DER, knapp ueber AK7-Grenze 15 % (29.09. 15:50)
- Beobachtet: --eval-diarization; ES2004a 15,30 % (v. a. Wortpausen/Miss), IS1009a 15,09 % (6 % Verwechslung); deutsch 0,94 %; AMI-Entwicklung 14,72 %.
- Konsequenz: laut E21 nicht nachtunen. Owner-Entscheidung Patrick: (a) AK7 fuer AMI als erfuellt im Rahmen der Messunsicherheit werten, (b) Nemotron-3 (8 Sprecher, im Spike AMI 14,0 %) spaeter nachziehen, sobald transcribe.cpp es laedt, (c) Grenze beibehalten und AK7 offen lassen.
- Status: erledigt (Patrick 29.09. abends: Variante a, AK7 erfuellt; Nemotron Folge-Goal)

## B8 — P4f: AK8 verfehlt, Zitate im Format [Sn] gehen verloren (29.09. 16:20)
- Beobachtet: --eval-chat E4B 0,58/0,71 (Streuung), 12B 0,83; 6 Fehlfragen nur wegen Zitatformat [S8]/[Q2:S44] (Antwort inhaltlich richtig), 2 Retrieval (Kompositum, lange Besprechung), 2 Modell.
- Beleg: abnahme/p4f-eval-chat*.json, Commit b0f18199.
- Konsequenz: Paket P4g.
- Status: erledigt (P4g, a4207200)

## B9 — Playwright-Laeufe parallel in mehreren Worktrees stoeren sich (29.09. 20:10)
- Beobachtet: volle Suite 30-37 rot, einzeln gruen; alle Worktrees nutzen Port 1420 mit reuseExistingServer -> Tests laufen gegen fremden Vite-Server.
- Konsequenz: Planer prueft die UI-Suite nur ohne parallele Playwright-Laeufe; dauerhaft Paket P7a (Port je Worktree).
- Status: erledigt (P7a d4ffa6cf)

## B10 — P7c: vorbestehende Lizenzfunde außerhalb des Goals (29.09. 22:40)
- Beobachtet: mp3lame-encoder 0.2.5 ist LGPL-3.0 (deny.toml behauptet das Gegenteil); ASR-Katalog enthält Canary 1B (CC-BY-NC-4.0, nicht vorgewählt, nicht im Besprechungspfad).
- Beleg: docs/m2-evidence/ATTRIBUTION.md (Befund-Absätze), cargo metadata.
- Konsequenz: nicht aus diesem Goal; QG6 betrifft nur neue Crates/Modelle (alle frei). An Patrick gemeldet, Entscheidung außerhalb des Goals.
- Status: erledigt (dokumentiert, an Patrick gemeldet; nicht Goal-Scope)

## B11 — P7b: KI-Notizen mit Gemma verwerfen bei 60 min still den ersten Block (30.09. 00:40)
- Beobachtet: Gemma 4 E4B, erster Map-Block 15.140 Token Prompt bei 16.384 Kontext -> Antwort viermal abgeschnitten, Block verworfen, Notizen decken nur die zweite Hälfte ab, `ok: true`. Kalibrierung 3,35 Z./Token stammt von Qwen3.5, Gemma ~3,0. QG3 mit Gemma 201-217 s (> 180 s), mit Qwen3.5 136,7 s.
- Nebenbefunde: AUTO_GPU_CANDIDATES bevorzugt Whisper large-v3 statt schnellerem Qwen3-ASR (~35 s je 60 min); Live-Modell lädt ohne RAM-Gate.
- Beleg: abnahme/p7b-qg3.md, p7b-qg3-*.json.
- Konsequenz: Paket P1i.
- Status: erledigt (P1i 25e32343: Gemma 60 min 156,6 s, kein verworfener Block; Warnung statt stiller Luecke)

## B12 — P1i: Protokoll-Erzeugung (minutes.rs) verwirft abgeschnittene Blöcke weiter still (30.09.)
- Beobachtet: gleiches Muster wie B11 im älteren Protokoll-Pfad (vor dem Goal vorhanden, nicht die KI-Notizen); ask_json gibt bei Abschneiden auf.
- Nebenbefund: Qwen3.5-9B im QG3-Lauf 186,1 s (> 180 s); das Gate gilt für das Standardmodell Gemma (156,6 s).
- Konsequenz: Folge-Goal (Protokoll-Pfad auf die P1i-Blocklogik umstellen); in docs/KNOWN-LIMITATIONS.md vermerkt.
- Status: offen (30.09.: Patrick holt es in Runde 3 -> P1k)

## B13 — Abnahme 0.20.8: Follow-up-Mail meldet „Sprachmodell hat keinen Text geliefert“ (30.09.)
- Beobachtet: Import „Die drei Schwestern …“ (Märchen); Log: Chat-Runden 1/2 „Antwort 10 Zeichen, kein Beleg: true“ -> followup_empty. Der Entwurf läuft über den Chat mit strenger Belegpflicht; ohne Treffer gibt es keinen Text, die Meldung ist irreführend.
- Beleg: handy.log 30.09. 14:22–14:23, Screenshot Patrick.
- Konsequenz: Paket P6f.
- Status: offen

## B14 — Abnahme 0.20.8: Protokoll-Erzeugung verliert Laufzustand beim Reiterwechsel (30.09.)
- Beobachtet (Patrick): „Erzeugen“ -> „Protokoll wird erzeugt“; Reiter wechseln und zurück -> Hinweis weg, Knopf wieder klickbar, paralleler zweiter Start möglich. Kein Fortschritt, keine Steuerung.
- Konsequenz: P1k (Laufzustand im Backend, Doppelstart abgewiesen, Fortschrittswerte) + P8a (Phase Protokoll/KI-Notizen mit Balken, Pause/Stopp).
- Status: offen

