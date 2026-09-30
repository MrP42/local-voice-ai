# P3e – Mehrsprecher-Praxistest: Sprechertrennung am TTS-Hörspiel

**Frage:** Wie trennt die Sprechertrennung (Sortformer 4spk v2.1 Q8_0, P3a/P3b) eine echte Mehrsprecher-Aufnahme? Getestet wurde Patricks Hörspiel „Emilia, Sofie und Mara und das Tal der drei Wege“ (18:39 min, 7 Stimmen). Die Referenz stammt aus dem Vorlesen-Skript.

**Ergebnis:**
- **Sprecherzahl: 4 erkannt, 7 im Skript.** 4 ist die Obergrenze von Sortformer 4spk. Die Grenze ist aber nicht der Hauptfehler.
- **DER 33,84 %** (Miss 1,53 / FA 1,52 / **Verwechslung 30,79**). Das liegt **weit über der AK7-Grenze von 15 %**.
- **Hauptfehler: Die drei Schwestern Emilia, Sofie und Mara laufen als *ein* Sprecher.** Das gilt über die ganze Datei und auch in frisch gestarteten 3-min-Ausschnitten, in denen nur 4 Stimmen sprechen (dort 2 erkannte Sprecher). Die Stimmen der Schwestern liegen akustisch sehr eng beieinander. Der Erzähler und die drei Nebenstimmen werden praktisch fehlerfrei getrennt, auch in kurzen Einwürfen.
- **Nemotron-3 (8 Sprecher) löst das nicht:** DER 31,95 %. Nemotron findet die drei Nebenstimmen, verschmilzt die Schwestern aber genauso. **Nachtuning der Nachbearbeitung hilft auch nicht:** DER roh 33,23 %, der Fehler steckt schon im Modell. Ein Sprecher-Embedding (ERes2Net, das Modell aus dem abgebrochenen P3d) trennt die Schwestern dagegen gut: Leave-one-out 97,5 %.

Stand 30.09.2026. Release-EXE 0.20.8 (`--features gpu-vulkan`, Haupt-Checkout `target/release`, 30.09. 15:50) als portable Hardlink-Kopie. RTX 4090, i9-13900K, 64 GB.

## Daten und Referenz

- Audio: `Desktop\Emilia,-Sofie-und-Mara-und-das-Tal-der-drei-Wege_2026-09-15_2031.mp3` (44,1 kHz mono, 1119,03 s), mit ffmpeg nach 16 kHz mono PCM umgewandelt.
- Skript: gleicher Name mit `.mp3.json`. Es enthält 488 Sätze, jeweils mit `{text, voice, start_ms, end_ms}`. Die Zeitspannen schließen lückenlos aneinander an (0 bis 1119,019 s) und passen damit direkt zum Audio (Differenz 14 ms). **Eine Ausrichtung über das Transkript war nicht nötig.**
- Prüfung der Ausrichtung: Bei 384 von 487 Satzgrenzen liegt innerhalb von ±50 ms eine Pause (< −55 dBFS). Die Trefferzahl ist am höchsten beim Versatz 0 bis +50 ms (384/378) und fällt bei ±200 ms auf 46/11. Es gibt also keinen Versatz und keine Drift.
- Referenz-RTTM (`tools/p3e_reference.py`): Stimme je Satz aus dem Skript, jede Spanne auf Sprache gekürzt. Gewertet sind 10-ms-Rahmen über −50 dBFS; Lücken unter 300 ms werden überbrückt, Inseln unter 50 ms verworfen. Ergebnis: 526 Turns, 1055,1 s Sprache in 1119,0 s Spannen. Die übrigen Referenzen des Diarisierungskorpus enthalten ebenfalls nur Sprache.
  Empfindlichkeit (Befehl → Ergebnis siehe unten): Die Schwelle verschiebt die DER um höchstens 0,4 Punkte (−45 dB: 33,41 %, −55 dB: 33,94 %). Die ungekürzten Spannen ergeben 35,11 %. Der Verwechslungsanteil bleibt in allen Fällen bei 30,7 bis 30,9 %.

| Stimme im Skript | Sätze | Sprache (s) | F0-Median (IQR), Hz |
|---|---|---|---|
| Erzähler (`die-drei-fragezeichen-erzaehler`) | 228 | 469,2 | 88 (81–96) |
| Emilia | 79 | 213,9 | 258 (242–281) |
| Mara | 90 | 173,9 | 262 (232–291) |
| Sofie | 69 | 164,0 | 267 (242–286) |
| Patrick | 12 | 14,8 | 90 (83–102) |
| Leo Lausemaus | 9 | 14,7 | 320 (281–372) |
| German Father (nur Titel) | 1 | 4,5 | 106 (97–126) |

**7 Stimmen, davon 4 Hauptstimmen und 3 Nebenstimmen mit zusammen 34 s (3,2 %).** Die Schwestern sind geklonte Fish-Speech-Stimmen aus drei verschiedenen Demo-WAVs. F0 ist nur ein grober Hinweis (Autokorrelation, `tools/p3e_pitch.py`, 150 Rahmen je Stimme).

## Weg

- **Sandbox statt installierter App:** Die EXE, ihre DLLs und `resources` sind als Hardlinks in `C:\Users\wolff\lva-spikes\p3e\app` angelegt, dazu die Markerdatei `portable`. Die App liest Einstellungen, Logs und Modelle deshalb aus `app\Data`. Dort liegen Hardlinks des installierten Sortformer-GGUF und von Parakeet int8. `LVA_APPDATA_DIR` gibt es nur im MCP-Pfad. Der Portable-Modus ist der einzige Schalter, der auch `--eval-diarization` und `--import-meeting` vom echten `%APPDATA%` fernhält. `LVA_MEETINGS_DIR` zeigt zusätzlich auf eine frische Sandbox. Patricks laufende App (PID 61196) und der Haupt-Checkout blieben unberührt; `git status` zeigt dort nur das vorbestehende `Cargo.toml`.
- Alle Läufe liefen über `Invoke-P7bApp` (`p7b-common.ps1`): Job-Objekt mit RAM-Deckel, BelowNormal, CPU-Deckel 80 % (App) bzw. 50 % (Nemotron) und Kill-on-close, dazu ein RAM-Start-Gate ≥ 10 GB und ein Zeitlimit. Kein Lauf hinterließ Restprozesse (`runs.json`: `leftovers: []`).
- Die DER wird mit dem App-Werkzeug `--eval-diarization` berechnet, Metrik `der.rs`: 10-ms-Raster, Kragen 0,25 s, Überlappung zählt, optimale 1:1-Zuordnung. Die Aufschlüsselung je Sprecher kommt aus `tools/p3e_analyze.py`, einer Nachbildung derselben Metrik. **Gegenprobe:** 33,84 / 1,53 / 1,52 / 30,79 stimmen mit der App auf die Hundertstel überein.

## Befehl → Ergebnis

| # | Befehl | Ergebnis |
|---|---|---|
| 1 | `python tools/p3e_reference.py --script <mp3.json> --wav diar/hoerspiel.wav --uri hoerspiel --out-dir diar` | `ref/hoerspiel.rttm` (526 Turns, 1055,1 s) + `ref/hoerspiel.spans.rttm`, `ref/ref-summary.json` |
| 2 | `pwsh -File tools/p3e-hoerspiel.ps1 -Mp3 <mp3> -SkipImport` (→ `--eval-diarization p3e\diar --rttm-out … --out …`) | Exit 3 (deutsches Ziel ≤ 5 % verfehlt), 16,0 s, Spitze 1,1 GB. **DER 33,84 %**, roh 33,23 %, **4 von 7 Sprechern**, Sortformer 14,1 s für 1119 s (79× Echtzeit, Vulkan) → `sortformer/eval-diarization.json`, `sortformer/hoerspiel{,.raw}.rttm` |
| 3 | `python tools/p3e_analyze.py --ref ref/hoerspiel.rttm --hyp sortformer/hoerspiel.rttm --hyp-raw … --main erzaehler,emilia,mara,sofie --import-json … --out sortformer/analysis.json` | Tabelle unten; nur Hauptstimmen gewertet: DER 33,43 % (Verwechslung 31,12 %) |
| 4 | `pwsh -File tools/p3e-hoerspiel.ps1 -Mp3 <mp3> -SkipEval` (→ `--import-meeting <mp3> --model parakeet-tdt-0.6b-v3`) | Exit 0, 164,9 s (Parakeet int8 auf der CPU + Sprecher), Spitze 1,8 GB, Status `ready`, 437 Segmente, **4 Sprecher**, 0 ohne Sprecher, Log „speakers applied: 437 assigned, 0 split“ → `sortformer/import-segments.json` (ohne Text) |
| 5 | `p3e_reference.py … --from-s 120 --to-s 300` (dazu 300–480 und 900–1080), dann `p3e-hoerspiel.ps1 -SkipImport -EvalDir p3e/diar-excerpts -EvalName eval-excerpts` | Drei 3-min-Ausschnitte, frisch gestartet, mit genau Erzähler + 3 Schwestern: **je 2 erkannte Sprecher**, DER 35,71 / 41,40 / 35,42 % → `sortformer/eval-excerpts.json` |
| 6 | `pwsh -Command "& tools/p3e-nemotron.ps1 -OutDir … -Wav <3 Ausschnitte>,<ganze Datei>"` (Spike `lva-spikes/m3/run_nemotron.py`, CPU, 4 Threads, offline) | Exit 0, 51,0 s, Spitze 3,5 GB. Ganze Datei: **5 Sprecher, DER 31,95 %** (Verwechslung 30,21). Ausschnitte: je 2 Sprecher, 35,13 / 40,59 / 36,24 % → `nemotron/` |
| 7 | `lva-spikes/m3/venv/Scripts/python lva-spikes/p3e/embed_check.py diar/hoerspiel.wav diar/hoerspiel.rttm <eres2net.onnx> <wespeaker.onnx> --voices emilia,sofie,mara,erzaehler` | je 40 Turns ≥ 1,5 s. ERes2Net: Leave-one-out mit Zentroid **97,5 %** (Zufall 25 %), WeSpeaker: 82,5 % → `embed_check.json` |
| 8 | `python tools/p3e_pitch.py --wav … --ref … --out pitch.json` | F0 je Stimme (Tabelle oben) |
| 9 | Empfindlichkeit: Schritt 1 mit `--threshold-db -45/-55` bzw. `hoerspiel.spans.rttm`, dann Schritt 3 | DER 33,41 / 33,94 / 35,11 %; Verwechslung 30,79 / 30,67 / 30,91 % |

Rohdaten: `abnahme/p3e-hoerspiel/` (`ref/`, `sortformer/`, `nemotron/`, `pitch.json`, `embed_check.json`, `runs.json`). Audio-WAVs und Logs liegen unter `C:\Users\wolff\lva-spikes\p3e` und sind nicht im Repo.

## Je Sprecher (Sortformer, ganze Datei, gewertete Sekunden nach Kragen)

Die Zuordnung der Metrik ist Erzähler → S2, Emilia → S3, Patrick → S1, Leo → S4. Sofie, Mara und German Father bekommen keinen eigenen Hyp-Sprecher.

| Stimme | gewertet (s) | → Hyp | richtig | verwechselt mit | verpasst |
|---|---|---|---|---|---|
| Erzähler | 350,8 | S2 | **100,0 %** | – (1,5 s parallel S3) | 0,0 s |
| Emilia | 167,3 | S3 | **100,0 %** | – | 0,0 s |
| Sofie | 127,2 | – | **0,0 %** | S3 125,0 s (= Emilia) | 2,2 s |
| Mara | 125,2 | – | **0,0 %** | S3 115,3 s (= Emilia) | 9,9 s |
| Leo Lausemaus | 10,2 | S4 | 100,0 % | – | 0,0 s |
| Patrick | 8,8 | S1 | 100,0 % | – (parallel auch S4 5,6 s) | 0,0 s |
| German Father | 4,0 | – | 0,0 % | S1 4,0 s (= Patrick) | 0,0 s |
| **Summe** | **793,6** | | | **Verwechslung 30,79 %** | Miss 1,53 %, FA 1,52 % |

Hyp-Sprecher im Import (Sekunden je Stimme, nach Segmenten): Sprecher 3 = Emilia 228,5 + Mara 169,1 + Sofie 163,0 s. Sprecher 2 = Erzähler 496,7 s. Sprecher 1 = Patrick 11,3 + German Father 5,2 s. Sprecher 4 = Leo 13,5 + Patrick 4,7 s. Die Segmente des Imports sind zu **68,3 % der Zeit** (65,2 % der Segmente) der richtigen Stimme zugeordnet. 26 Segmente umfassen mehr als eine Stimme (Reinheit < 80 %) und wurden nicht geteilt.

## Typische Fehler

1. **Ähnliche Stimmen werden verschmolzen.** Das ist der Kern des Befunds und macht 30,2 der 30,8 Punkte Verwechslung aus. Die drei Mädchenstimmen (F0-Median 258/262/267 Hz, Quartile fast deckungsgleich) laufen durchgehend unter einem Hyp-Sprecher. Das ist kein Drift-Effekt: In jedem 60-s-Fenster mit Schwestern-Dialog liegt die Verwechslung bei 15 bis 52 %. Es ist auch keine Folge der 4-Sprecher-Grenze: Slot S4 blieb bis 512 s frei, S1 war nach dem Titel (0 bis 4,8 s) bis 649 s unbenutzt. Auch die frisch gestarteten Ausschnitte mit genau 4 Stimmen ergeben nur 2 Sprecher.
2. **Mehr Stimmen als Slots.** Die drei Nebenstimmen (23,0 s gewertet = 2,9 % der Referenz) teilen sich S1 und S4, dabei landet Patrick teils parallel auf beiden. Diese Kosten sind gering. Wären die Schwestern getrennt, bliebe mit 4 Slots eine Untergrenze von rund 3 % DER durch die Nebenstimmen. Nemotron (8 Sprecher) trennt alle drei Nebenstimmen korrekt (je 100 %).
3. **Kurze Einwürfe sind kein eigener Fehlerschwerpunkt.** Bei den nicht verschmolzenen Stimmen (Erzähler, Emilia, Leo, Patrick) sind Turns < 1 s (53 Turns) ebenso zu 100 % richtig wie längere. Behandelt man die Schwestern als eine Klasse, liegen Turns < 1 s bei 89,3 %, 1 bis 2 s bei 96,2 % und ≥ 2 s bei 97,5 bis 100 %. Der Miss von 1,5 % hängt an expressiven Kurzsätzen, vor allem bei Mara: 11 Turns mit Stiltags wie `[happy]`, `[yawning]`, `[laughing]` und `[surprised]` werden teilweise nicht als Sprache erkannt.

## Einordnung gegen AK7 (Grenze 15 %)

- **33,84 % liegt klar über 15 %.** Wird nur über den Erzähler, die Schwestern als eine Klasse und die Nebenstimmen gewertet, liegt der Fehler bei wenigen Prozent. Die Metrik bestraft aber zu Recht, dass drei Personen zu einer werden: In der Oberfläche hieße das „Sprecher 3“ für drei Figuren.
- AK7 bleibt formal unberührt. Es ist auf AMI-Prüfteil (15,19 %, B7) und das synthetische deutsche Fixture (0,94 %) definiert, in dem die Stimmen deutlich verschieden sind. Das Hörspiel zeigt eine Grenze, die dieser Testsatz nicht abdeckt: **ähnlich klingende Sprecher gleichen Geschlechts und Alters.** In echten Besprechungen kommt das vor (z. B. mehrere Männer ähnlichen Alters). Die AMI-Messung mit 6 % Verwechslung in IS1009a zeigt dieselbe Richtung, aber in weit geringerem Ausmaß.
- Einschränkung zur Übertragbarkeit: Alle Stimmen stammen aus demselben TTS-Modell (Fish-Speech-Klone). Das kann die Ähnlichkeit gegenüber echten Menschen verstärken. Ein Hörtest durch Patrick fehlt, und ob die Schwestern für Menschen klar unterscheidbar sind, ist nicht gemessen.

## Nachtuning oder Nemotron?

- **Nachtuning der Nachbearbeitung (gap/pad, Preset): nein.** Die rohe Modellausgabe hat dieselbe Verwechslung (DER roh 33,23 %). Nachbearbeitung kann keine Sprecher trennen, die das Modell zusammengelegt hat. E21 („nicht nachtunen“) bleibt sinnvoll.
- **Nemotron-3: löst dieses Problem nicht.** Gemessen wurde mit dem Spike-Werkzeug auf PyTorch-CPU, nicht in der App: 31,95 % statt 33,84 %. Der Gewinn kommt nur von den Nebenstimmen, die Schwestern werden genauso verschmolzen. Für mehr als 4 Personen bleibt Nemotron als Folge-Goal richtig, für ähnliche Stimmen nicht.
- **Hoffnungsvoll ist ein Embedding-Schritt.** ERes2Net (das Modell aus P3d) trennt die Schwestern mit Referenzlabels zu 97,5 % (Leave-one-out gegen Zentroide). Der Cosinus innerhalb einer Stimme liegt bei 0,55 bis 0,71, zwischen den Schwestern bei 0,38 bis 0,53. Das spricht für einen Nachschritt, der einen Sortformer-Sprecher über Turn-Embeddings aufspaltet, wenn er zwei oder mehr stabile Untergruppen enthält. Das gehört zum Folge-Goal „Wiedererkennen (P3d)“. Unüberwacht (ohne Labels) ist das schwerer als der hier gemessene Zentroid-Test; Sofie/Mara (0,528) liegt nahe an Mara/Mara (0,545). Ohne eigene Messung mit Clustering ist das nur ein Hinweis, keine Zusage.
- Bis dahin ist das eine bekannte Grenze. Nutzer können falsch zusammengefasste Sprecher nur über das Umbenennen korrigieren, und das trennt keine Segmente.

## Grenzen

- Eine Aufnahme mit synthetischen Stimmen, je Variante ein Lauf. Die Streuung ist nicht gemessen, weil kein Lauf wiederholt wurde.
- Die Referenz beruht auf Skriptzeiten und einer Pegelschwelle. Die Sprecherzuordnung je Satz ist exakt, die Sprachgrenzen sind geschätzt (Empfindlichkeit ≤ 1,3 Punkte, siehe Schritt 9).
- Das Import-STT war Parakeet int8, weil Patricks Standard-GGUF in der Sandbox nicht vorhanden war. Auf die Sprecher wirkt das nur über die Segmentgrenzen. Die Sprecher-DER (Schritte 2 und 5) ist vom STT unabhängig.
- Nemotron lief im Spike (PyTorch-CPU, `transformers`), nicht über transcribe.cpp in der App. Die Zahlen gelten für das Modell, nicht für eine künftige App-Anbindung.
